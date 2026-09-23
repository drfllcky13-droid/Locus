//! Photogrammetry: a user-installed COLMAP run as a separate process (never bundled; see
//! docs/phase8-colmap-licence-review.txt). The setup (where COLMAP is, CPU-only features) is
//! the machine's, kept in the app's config folder; each run records COLMAP's version and
//! executable hash. A run works in the project's `derived/photogrammetry/<run>/` (regenerable);
//! its scaled point cloud is written as E57, imported as evidence (hashed, read-only) and the run
//! stored as an audit-logged analysis record with the full provenance.

use crate::commands::{blocking, err, AppState, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_photo::colmap::{self, Build, Matcher, Reconstruction, Settings};
use locus_photo::exif::GpsTags;
use locus_photo::georef::{self, Click, ScaleRecord, ScaleRequest, Triangulated};
use locus_photo::record::{self, ColmapInfo, Output, PhotoRun, Source, SourceImage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// The oldest COLMAP Locus is written for.
const OLDEST: (u32, u32, u32) = colmap::MIN_VERSION;
pub const RELEASES: &str = "https://github.com/colmap/colmap/releases";

#[derive(Serialize, Deserialize, Default, Clone)]
struct SetupFile {
    colmap_path: Option<String>,
    #[serde(default)]
    cpu_only: bool,
}

fn setup_file(app: &AppHandle) -> CmdResult<PathBuf> {
    let dir = app.path().app_config_dir().map_err(err)?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir.join("photogrammetry.json"))
}

fn load_setup(app: &AppHandle) -> SetupFile {
    setup_file(app)
        .ok()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// A chosen path to the executable: COLMAP.bat (its release folder) means bin\colmap.exe.
fn exe_of(path: &Path) -> PathBuf {
    let is_bat = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("bat"));
    if is_bat || path.is_dir() {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        for c in [dir.join("bin").join("colmap.exe"), dir.join("colmap.exe")] {
            if c.exists() {
                return c;
            }
        }
    }
    path.to_path_buf()
}

/// Places COLMAP is commonly found: every folder on PATH, and the usual install folders.
fn candidates() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("colmap.exe"))
                .collect()
        })
        .unwrap_or_default();
    for base in ["ProgramFiles", "LOCALAPPDATA"] {
        if let Some(b) = std::env::var_os(base) {
            v.push(Path::new(&b).join("COLMAP").join("bin").join("colmap.exe"));
        }
    }
    v.into_iter().filter(|p| p.exists()).collect()
}

#[derive(Serialize, Clone)]
pub struct Found {
    path: String,
    banner: String,
    version: (u32, u32, u32),
    cuda: bool,
    sha256: String,
    supported: bool,
}

#[derive(Serialize)]
pub struct SetupView {
    configured: Option<String>,
    cpu_only: bool,
    found: Option<Found>,
    error: Option<String>,
    /// Other copies found on this machine.
    candidates: Vec<String>,
    releases: &'static str,
    oldest: (u32, u32, u32),
}

fn examine(exe: &Path) -> Result<Found, String> {
    let b = colmap::detect(exe)?;
    let (sha256, _) = locus_core::hash::sha256_file(exe, &mut |_| {}).map_err(err)?;
    Ok(Found {
        path: exe.display().to_string(),
        supported: b.version >= OLDEST,
        banner: b.banner,
        version: b.version,
        cuda: b.cuda,
        sha256,
    })
}

fn setup_view(app: &AppHandle) -> SetupView {
    let s = load_setup(app);
    let cands = candidates();
    let (found, error) = match s.colmap_path.as_deref().map(|p| exe_of(Path::new(p))) {
        Some(exe) => match examine(&exe) {
            Ok(f) => (Some(f), None),
            Err(e) => (None, Some(e)),
        },
        None => (None, None),
    };
    SetupView {
        configured: s.colmap_path,
        cpu_only: s.cpu_only,
        found,
        error,
        candidates: cands.iter().map(|p| p.display().to_string()).collect(),
        releases: RELEASES,
        oldest: OLDEST,
    }
}

/// Where COLMAP is and what it is.
#[tauri::command]
pub async fn photo_setup(app: AppHandle) -> CmdResult<SetupView> {
    tauri::async_runtime::spawn_blocking(move || Ok(setup_view(&app)))
        .await
        .map_err(err)?
}

/// Set COLMAP's path (None to clear) and the CPU-only setting.
#[tauri::command]
pub async fn photo_setup_set(
    app: AppHandle,
    path: Option<String>,
    cpu_only: bool,
) -> CmdResult<SetupView> {
    tauri::async_runtime::spawn_blocking(move || {
        let colmap_path = match path.filter(|p| !p.trim().is_empty()) {
            Some(p) => {
                let exe = exe_of(Path::new(p.trim()));
                examine(&exe).map_err(|e| format!("{e}."))?;
                Some(exe.display().to_string())
            }
            None => None,
        };
        let f = SetupFile {
            colmap_path,
            cpu_only,
        };
        std::fs::write(
            setup_file(&app)?,
            serde_json::to_vec_pretty(&f).map_err(err)?,
        )
        .map_err(err)?;
        Ok(setup_view(&app))
    })
    .await
    .map_err(err)?
}

// ---------- sources ----------

#[derive(Serialize)]
pub struct SourceImageView {
    evidence_id: i64,
    name: String,
    sha256: String,
    file: String,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
pub struct SourceVideoView {
    evidence_id: i64,
    name: String,
    sha256: String,
}

#[derive(Serialize)]
pub struct Sources {
    images: Vec<SourceImageView>,
    videos: Vec<SourceVideoView>,
}

/// The project's photos and videos.
#[tauri::command]
pub async fn photo_sources(app: AppHandle) -> CmdResult<Sources> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        let (mut images, mut videos) = (vec![], vec![]);
        for e in p.evidence().map_err(err)? {
            let name = Path::new(&e.original_path)
                .file_name()
                .map_or(e.original_path.clone(), |n| n.to_string_lossy().to_string());
            if e.contents.format == "Video" {
                videos.push(SourceVideoView {
                    evidence_id: e.id,
                    name,
                    sha256: e.sha256.clone(),
                });
            } else if let Some(i) = e
                .contents
                .images
                .first()
                .filter(|_| matches!(e.contents.format.as_str(), "JPEG" | "PNG"))
            {
                images.push(SourceImageView {
                    evidence_id: e.id,
                    name,
                    sha256: e.sha256.clone(),
                    file: e.stored_path.clone(),
                    width: i.width,
                    height: i.height,
                });
            }
        }
        Ok(Sources { images, videos })
    })
    .await
}

// ---------- runs ----------

#[derive(Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceRequest {
    Photos { evidence_ids: Vec<i64> },
    Video { evidence_id: i64, interval: f64 },
}

#[derive(Deserialize, Clone)]
pub struct RunRequest {
    source: SourceRequest,
    camera_model: String,
    single_camera: bool,
    dense: bool,
    max_image_size: u32,
    #[serde(default)]
    dense_max_image_size: Option<u32>,
}

struct Job {
    dir: PathBuf,
    images: PathBuf,
    colmap: ColmapInfo,
    settings: Settings,
    source: Source,
    images_total: usize,
    recon: Reconstruction,
    gps: BTreeMap<String, GpsTags>,
}

#[derive(Default)]
pub struct PhotoState {
    job: Mutex<Option<Job>>,
    cancel: Arc<AtomicBool>,
    running: AtomicBool,
}

#[derive(Serialize, Clone)]
pub struct JobView {
    images_total: usize,
    registered: Vec<String>,
    sparse_points: usize,
    mean_error_px: f64,
    other_models: Vec<usize>,
    dense: bool,
    note: Option<String>,
    /// Registered photos with a GPS position, and with an RTK fixed one.
    gps: usize,
    rtk: usize,
    seconds: f64,
}

fn job_view(j: &Job) -> JobView {
    JobView {
        images_total: j.images_total,
        registered: j
            .recon
            .model
            .images
            .values()
            .map(|i| i.name.clone())
            .collect(),
        sparse_points: j.recon.model.points.len(),
        mean_error_px: j.recon.model.mean_error(),
        other_models: j.recon.other_models.clone(),
        dense: j.recon.dense_ply.is_some(),
        note: j.recon.note.clone(),
        gps: j
            .recon
            .model
            .images
            .values()
            .filter(|i| j.gps.get(&i.name).is_some_and(|g| g.latitude.is_some()))
            .count(),
        rtk: j
            .recon
            .model
            .images
            .values()
            .filter(|i| j.gps.get(&i.name).is_some_and(|g| g.rtk_flag == Some(50)))
            .count(),
        seconds: j.recon.stages.iter().map(|s| s.seconds).sum(),
    }
}

#[derive(Serialize, Clone)]
struct ProgressEvent {
    stage: String,
    done: u32,
    total: u32,
    line: String,
}

#[derive(Serialize, Clone)]
struct DoneEvent {
    job: Option<JobView>,
    error: Option<String>,
}

/// Link (or copy) an evidence file into the run's image folder, read-only either way.
fn place(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::hard_link(src, dst).or_else(|_| std::fs::copy(src, dst).map(|_| ()))
}

/// Start a reconstruction in the background: progress arrives as "photo-progress" events and
/// the end as "photo-done". Only one runs at a time.
#[tauri::command]
pub async fn photo_run(app: AppHandle, request: RunRequest) -> CmdResult<()> {
    let st = app.state::<PhotoState>();
    if st.running.swap(true, Ordering::SeqCst) {
        return Err("A reconstruction is already running.".into());
    }
    st.cancel.store(false, Ordering::SeqCst);
    let handle = app.clone();
    std::thread::spawn(move || {
        let r = run_job(&handle, request);
        let st = handle.state::<PhotoState>();
        let done = match r {
            Ok(job) => {
                let view = job_view(&job);
                *st.job.lock().unwrap() = Some(job);
                DoneEvent {
                    job: Some(view),
                    error: None,
                }
            }
            Err(e) => DoneEvent {
                job: None,
                error: Some(e),
            },
        };
        st.running.store(false, Ordering::SeqCst);
        let _ = handle.emit("photo-done", done);
    });
    Ok(())
}

fn run_job(app: &AppHandle, req: RunRequest) -> CmdResult<Job> {
    let setup = load_setup(app);
    let exe = exe_of(Path::new(setup.colmap_path.as_deref().ok_or(
        "Set up COLMAP first: install it and choose its colmap.exe or COLMAP.bat.",
    )?));
    let found = examine(&exe)?;
    if !found.supported {
        return Err(format!(
            "{} is older than COLMAP {}.{}.{}, the oldest Locus runs.",
            found.banner, OLDEST.0, OLDEST.1, OLDEST.2
        ));
    }
    let build = Build {
        version: found.version,
        banner: found.banner.clone(),
        cuda: found.cuda,
    };
    // Gather the inputs from the project.
    let state = app.state::<AppState>();
    let (root, evidence) = {
        let guard = state.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        (p.root().to_path_buf(), p.evidence().map_err(err)?)
    };
    let run_id = chrono_id();
    let dir = root.join("derived").join("photogrammetry").join(&run_id);
    let images = dir.join("images");
    std::fs::create_dir_all(&images).map_err(err)?;
    let st = app.state::<PhotoState>();
    let cancel = st.cancel.clone();
    let emit = |stage: &str, done: u32, total: u32, line: &str| {
        let _ = app.emit(
            "photo-progress",
            ProgressEvent {
                stage: stage.into(),
                done,
                total,
                line: line.into(),
            },
        );
    };
    let mut gps = BTreeMap::new();
    let (source, matcher) = match &req.source {
        SourceRequest::Photos { evidence_ids } => {
            if evidence_ids.len() < 3 {
                return Err("Choose at least 3 photos.".into());
            }
            let mut items = vec![];
            for id in evidence_ids {
                let e = evidence
                    .iter()
                    .find(|e| e.id == *id)
                    .ok_or(format!("No evidence {id}."))?;
                let orig = Path::new(&e.original_path)
                    .file_name()
                    .map_or("photo".into(), |n| n.to_string_lossy().to_string());
                let name = format!("{}_{}", e.id, orig);
                let src = root.join(&e.stored_path);
                place(&src, &images.join(&name)).map_err(err)?;
                if let Ok(bytes) = std::fs::read(&src) {
                    gps.insert(name.clone(), locus_photo::exif::read_jpeg(&bytes));
                }
                items.push(SourceImage {
                    evidence_id: e.id,
                    name,
                    sha256: e.sha256.clone(),
                });
            }
            (Source::Photos { items }, Matcher::Exhaustive)
        }
        SourceRequest::Video {
            evidence_id,
            interval,
        } => {
            let e = evidence
                .iter()
                .find(|e| e.id == *evidence_id)
                .ok_or(format!("No evidence {evidence_id}."))?;
            let s = locus_photo::video::sample_frames(
                &root.join(&e.stored_path),
                *interval,
                &images,
                &cancel,
                &mut |t, d| emit("frames", (t * 10.0) as u32, (d * 10.0) as u32, ""),
            )?;
            (
                Source::Video {
                    evidence_id: e.id,
                    name: Path::new(&e.original_path)
                        .file_name()
                        .map_or("video".into(), |n| n.to_string_lossy().to_string()),
                    sha256: e.sha256.clone(),
                    interval: *interval,
                    first_timestamp: s.first_timestamp,
                    duration: s.duration,
                    frames: s
                        .frames
                        .iter()
                        .map(|f| {
                            (
                                f.file.file_name().unwrap().to_string_lossy().to_string(),
                                f.time,
                            )
                        })
                        .collect(),
                },
                Matcher::Sequential,
            )
        }
    };
    let images_total = match &source {
        Source::Photos { items } => items.len(),
        Source::Video { frames, .. } => frames.len(),
    };
    let settings = Settings {
        cpu_only: setup.cpu_only,
        matcher,
        // Video frames all come from one camera.
        single_camera: req.single_camera || matches!(source, Source::Video { .. }),
        camera_model: req.camera_model.clone(),
        dense: req.dense,
        max_image_size: req.max_image_size,
        dense_max_image_size: req.dense_max_image_size.unwrap_or(2000),
    };
    let mut last = (String::new(), (0u32, 0u32));
    let recon = colmap::reconstruct(
        &exe,
        &build,
        &settings,
        &images,
        &dir.join("work"),
        &cancel,
        &mut |stage, line| {
            // Carry the stage's last count through lines that don't state one.
            let (d, t) = match colmap::progress(stage, line) {
                Some(p) => {
                    last = (stage.to_string(), p);
                    p
                }
                None if last.0 == stage => last.1,
                None => (0, 0),
            };
            emit(stage, d, t, line);
        },
    )
    .map_err(|e| {
        if e == colmap::RunError::Cancelled {
            "Cancelled.".to_string()
        } else {
            format!("{e}")
        }
    })?;
    Ok(Job {
        dir,
        images,
        colmap: ColmapInfo {
            path: found.path,
            banner: found.banner,
            sha256: found.sha256,
        },
        settings,
        source,
        images_total,
        recon,
        gps,
    })
}

/// A run's folder name: UTC time, sortable.
fn chrono_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("run-{}", t.as_millis())
}

#[tauri::command]
pub fn photo_cancel(app: AppHandle) {
    app.state::<PhotoState>()
        .cancel
        .store(true, Ordering::SeqCst);
}

/// The finished reconstruction waiting to be scaled and imported, if any.
#[tauri::command]
pub fn photo_job(app: AppHandle) -> Option<JobView> {
    let st = app.state::<PhotoState>();
    let j = st.job.lock().unwrap();
    j.as_ref().map(job_view)
}

/// A registered photo's bytes (from the run's image folder), for clicking targets.
#[tauri::command]
pub async fn photo_image(app: AppHandle, name: String) -> CmdResult<tauri::ipc::Response> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<PhotoState>();
        let j = st.job.lock().unwrap();
        let j = j.as_ref().ok_or("No reconstruction.")?;
        if name.contains(['/', '\\']) || name.contains("..") {
            return Err("bad image name".into());
        }
        Ok(tauri::ipc::Response::new(
            std::fs::read(j.images.join(&name)).map_err(err)?,
        ))
    })
    .await
    .map_err(err)?
}

/// A target clicked in two or more photos, triangulated.
#[tauri::command]
pub fn photo_triangulate(app: AppHandle, clicks: Vec<Click>) -> CmdResult<Triangulated> {
    let st = app.state::<PhotoState>();
    let j = st.job.lock().unwrap();
    let j = j.as_ref().ok_or("No reconstruction.")?;
    georef::triangulate_clicks(&j.recon.model, &clicks).map_err(|e| capital(&e))
}

fn capital(e: &str) -> String {
    let mut s = e[..1].to_uppercase() + &e[1..];
    if !s.ends_with('.') {
        s.push('.');
    }
    s
}

/// Control points may be picked on a scan instead of typed.
#[derive(Deserialize)]
pub struct GcpRequest {
    label: String,
    clicks: Vec<Click>,
    #[serde(default)]
    world: Option<[f64; 3]>,
    #[serde(default)]
    pick: Option<Pick>,
    #[serde(default)]
    check: bool,
}

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ScaleIn {
    Gps,
    Distances { items: Vec<georef::DistanceItem> },
    Gcps { items: Vec<GcpRequest> },
}

fn scale_request(app: &AppHandle, s: ScaleIn) -> CmdResult<ScaleRequest> {
    Ok(match s {
        ScaleIn::Gps => ScaleRequest::Gps,
        ScaleIn::Distances { items } => ScaleRequest::Distances { items },
        ScaleIn::Gcps { items } => {
            let state = app.state::<AppState>();
            let scene = state.scene.read().unwrap();
            let mut out = vec![];
            for it in items {
                let world = match (it.world, &it.pick) {
                    (_, Some(p)) => resolve(&scene, p)?.project,
                    (Some(w), None) => w,
                    (None, None) => {
                        return Err(format!(
                            "{}: give its coordinates or pick it on a scan.",
                            it.label
                        ))
                    }
                };
                out.push(georef::GcpItem {
                    label: it.label,
                    clicks: it.clicks,
                    world,
                    check: it.check,
                });
            }
            ScaleRequest::Gcps { items: out }
        }
    })
}

/// The scaling, without importing (to check its residuals).
#[tauri::command]
pub async fn photo_scale(app: AppHandle, scale: ScaleIn) -> CmdResult<ScaleRecord> {
    tauri::async_runtime::spawn_blocking(move || {
        let req = scale_request(&app, scale)?;
        let st = app.state::<PhotoState>();
        let j = st.job.lock().unwrap();
        let j = j.as_ref().ok_or("No reconstruction.")?;
        georef::solve(&j.recon.model, &req, &j.gps).map_err(|e| capital(&e))
    })
    .await
    .map_err(err)?
}

#[derive(Serialize)]
pub struct Imported {
    record: locus_core::AnalysisRecord,
    evidence_id: i64,
}

/// Scale the reconstruction, write it as E57, import it as evidence and store the run.
#[tauri::command]
pub async fn photo_import(app: AppHandle, name: String, scale: ScaleIn) -> CmdResult<Imported> {
    tauri::async_runtime::spawn_blocking(move || import(&app, name, scale))
        .await
        .map_err(err)?
}

fn import(app: &AppHandle, name: String, scale: ScaleIn) -> CmdResult<Imported> {
    let req = scale_request(app, scale)?;
    let ps = app.state::<PhotoState>();
    let guard_job = ps.job.lock().unwrap();
    let j = guard_job.as_ref().ok_or("No reconstruction.")?;
    let sc = georef::solve(&j.recon.model, &req, &j.gps).map_err(|e| capital(&e))?;
    // The points: COLMAP's dense cloud when there is one, else the sparse points.
    let (xyz, rgb, from) = match &j.recon.dense_ply {
        Some(f) => {
            let p = locus_photo::ply::read(&std::fs::read(f).map_err(err)?)?;
            (p.xyz, p.rgb, "dense")
        }
        None => (
            j.recon.model.points.iter().map(|p| p.xyz).collect(),
            j.recon.model.points.iter().map(|p| p.rgb).collect(),
            "sparse",
        ),
    };
    let xyz: Vec<[f64; 3]> = xyz.iter().map(|p| sc.transform.apply(*p)).collect();
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let file = j.dir.join(format!("{safe}.e57"));
    let guid = j.dir.file_name().unwrap().to_string_lossy().to_string();
    locus_io::write_e57(&file, &guid, &name, &xyz, &rgb).map_err(err)?;
    let state = app.state::<AppState>();
    let mut guard = state.project.lock().unwrap();
    let project = guard.as_mut().ok_or("Open or create a project first.")?;
    let pv = locus_io::preview(&file, &mut |_| {}).map_err(err)?;
    let rec = locus_io::commit(
        project,
        &pv,
        Some(locus_core::LinearUnit::Meter),
        &mut |_| {},
    )
    .map_err(err)?;
    app.state::<crate::scene_cmds::Builder>()
        .queue_missing(project, std::slice::from_ref(&rec))?;
    let mut warnings = sc.warnings.clone();
    if let Some(n) = &j.recon.note {
        warnings.push(n.clone());
    }
    let registered: Vec<String> = j
        .recon
        .model
        .images
        .values()
        .map(|i| i.name.clone())
        .collect();
    if registered.len() < j.images_total {
        warnings.push(format!(
            "COLMAP placed {} of the {} images; the rest didn't match well enough (too little overlap, blur, or plain surfaces).",
            registered.len(),
            j.images_total
        ));
    }
    if !j.recon.other_models.is_empty() {
        warnings.push(format!(
            "Some images formed separate reconstructions ({:?} images) that couldn't be joined to the main one; only the largest is used.",
            j.recon.other_models
        ));
    }
    let run = PhotoRun {
        method: record::PHOTO_METHOD.into(),
        name: name.clone(),
        colmap: j.colmap.clone(),
        settings: j.settings.clone(),
        source: j.source.clone(),
        stages: j.recon.stages.clone(),
        images_total: j.images_total,
        registered,
        sparse_points: j.recon.model.points.len(),
        mean_error_px: j.recon.model.mean_error(),
        other_models: j.recon.other_models.clone(),
        dense_note: j.recon.note.clone(),
        summary: format!(
            "{name}: {} of {} images, {} points ({from}), scaled by {} (scale ±{:.2} %, RMS {:.1} mm)",
            j.recon.model.images.len(),
            j.images_total,
            xyz.len(),
            match sc.method.as_str() {
                "gps" => "the photos' GPS",
                "distances" => "known distances",
                _ => "control points",
            },
            sc.scale_sigma_rel * 100.0,
            sc.rms * 1000.0
        ),
        scale: sc,
        output: Output {
            evidence_id: rec.id,
            sha256: rec.sha256.clone(),
            file: rec.stored_path.clone(),
            points: xyz.len(),
            from: from.into(),
        },
        warnings,
        assumptions: record::PHOTO_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: record::PHOTO_LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    };
    let stored = project
        .add_analysis(
            "photogrammetry",
            record::PHOTO_METHOD,
            &name,
            &serde_json::to_value(&run).map_err(err)?,
            None,
        )
        .map_err(err)?;
    if let Ok(info) = crate::commands::info(project, None) {
        let _ = app.emit(
            "project-changed",
            serde_json::to_value(&info).unwrap_or_default(),
        );
    }
    Ok(Imported {
        record: stored,
        evidence_id: rec.id,
    })
}
