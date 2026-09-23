//! Point-cloud commands: scene description, node protocol, picking, measurement and
//! cleanup. Geometry and math live in locus-octree and locus-analysis; this file only
//! resolves ids, calls them, and records results through locus-core.

use crate::commands::{blocking, err, AppState, CmdResult};
use locus_analysis::measure;
use locus_core::{
    CleanupRecord, CleanupScan, EvidenceRecord, MeasurementRecord, OctreeRecord, Project,
};
use locus_octree::cleanup::{self, Removal};
use locus_octree::scene::{build_scan, write_bitmap, Resolved, ScanKey, Scene};
use locus_octree::BuildProgress;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use tauri::http::{header, Response};
use tauri::{AppHandle, Emitter, Manager};

// ---------- background octree builds ----------

pub struct Job {
    pub root: PathBuf,
    pub rec: EvidenceRecord,
    pub scan: usize,
}

/// One worker builds octrees in order, so two large builds never compete for memory.
pub struct Builder(Mutex<mpsc::Sender<Job>>);

#[derive(Clone, Serialize)]
struct BuildEvent {
    scan: String,
    name: String,
    progress: Option<BuildProgress>,
    error: Option<String>,
}

impl Builder {
    pub fn start(app: AppHandle) -> Builder {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for job in rx {
                let key = ScanKey {
                    evidence_id: job.rec.id,
                    scan_idx: job.scan,
                };
                let name = job.rec.contents.scans[job.scan].name.clone();
                let event = |progress, error| BuildEvent {
                    scan: key.to_string(),
                    name: name.clone(),
                    progress,
                    error,
                };
                let mut last = std::time::Instant::now();
                let result = build_scan(&job.root, &job.rec, job.scan, &mut |p| {
                    if last.elapsed().as_millis() > 200 {
                        last = std::time::Instant::now();
                        let _ = app.emit("octree-progress", event(Some(p), None));
                    }
                });
                let state = app.state::<AppState>();
                let mut guard = state.project.lock().unwrap();
                let Some(project) = guard.as_mut().filter(|p| p.root() == job.root) else {
                    continue; // the project was closed while this was building
                };
                let (status, points, detail) = match &result {
                    Ok(meta) => ("built", meta.points, String::new()),
                    Err(e) => ("failed", 0, e.to_string()),
                };
                let recorded = project.record_octree(job.rec.id, job.scan, status, points, &detail);
                if let Ok(scene) = Scene::load(project) {
                    *state.scene.write().unwrap() = scene;
                }
                let error = result
                    .err()
                    .map(|e| e.to_string())
                    .or(recorded.err().map(|e| e.to_string()));
                let _ = app.emit("octree-progress", event(None, error));
                let _ = app.emit("scene-changed", ());
            }
        });
        Builder(Mutex::new(tx))
    }

    /// Queue every scan of `records` that has no built octree on disk.
    pub fn queue_missing(&self, project: &Project, records: &[EvidenceRecord]) -> CmdResult<()> {
        let built: BTreeMap<(i64, usize), OctreeRecord> = project
            .octrees()
            .map_err(err)?
            .into_iter()
            .map(|o| ((o.evidence_id, o.scan_idx), o))
            .collect();
        for rec in records {
            for scan in 0..rec.contents.scans.len() {
                let done = built
                    .get(&(rec.id, scan))
                    .is_some_and(|o| o.status == "built")
                    && project.octree_dir(rec.id, scan).join("meta.json").is_file();
                if !done && rec.unit.is_some() {
                    let job = Job {
                        root: project.root().into(),
                        rec: rec.clone(),
                        scan,
                    };
                    self.0.lock().unwrap().send(job).map_err(err)?;
                }
            }
        }
        Ok(())
    }
}

/// After opening a project or importing into it: refresh the scene, queue missing builds.
pub fn refresh(app: &AppHandle, project: &Project) -> CmdResult<()> {
    let state = app.state::<AppState>();
    *state.scene.write().unwrap() = Scene::load(project).map_err(err)?;
    app.state::<Builder>()
        .queue_missing(project, &project.evidence().map_err(err)?)
}

// ---------- node protocol ----------

/// `locus://localhost/node/<scan>/<node>` (http://locus.localhost/... on Windows).
pub fn protocol(app: &AppHandle, request: tauri::http::Request<Vec<u8>>) -> Response<Vec<u8>> {
    let parts: Vec<&str> = request.uri().path().trim_matches('/').split('/').collect();
    let served = match parts.as_slice() {
        ["node", scan, node] => match (ScanKey::parse(scan), node.parse::<usize>()) {
            (Some(key), Ok(node)) => app
                .state::<AppState>()
                .scene
                .read()
                .unwrap()
                .serve(key, node)
                .map_err(err),
            _ => Err("bad node address".into()),
        },
        _ => Err("unknown path".into()),
    };
    match served {
        Ok(bytes) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(bytes)
            .unwrap(),
        Err(e) => Response::builder()
            .status(404)
            .body(e.into_bytes())
            .unwrap(),
    }
}

// ---------- scene description ----------

#[derive(Serialize)]
struct NodeView {
    name: String,
    min: [f64; 3],
    size: f64,
    count: u32,
    spacing: f64,
    children: u8,
}

#[derive(Serialize)]
struct ScanView {
    key: String,
    name: String,
    pose: [f64; 16],
    min: [f64; 3],
    size: f64,
    points: u64,
    has_color: bool,
    has_intensity: bool,
    nodes: Vec<NodeView>,
}

#[derive(Serialize)]
pub struct SceneView {
    /// Render origin in the project frame (meters); the GPU works relative to it.
    origin: [f64; 3],
    scans: Vec<ScanView>,
}

/// Octree hierarchies for every built scan. Sent once, and again after "scene-changed".
#[tauri::command]
pub async fn scene_view(app: AppHandle) -> CmdResult<SceneView> {
    blocking(app, |s| {
        let scene = s.scene.read().unwrap();
        let scans = scene
            .scans
            .values()
            .map(|c| ScanView {
                key: c.key.to_string(),
                name: c.name.clone(),
                pose: c.pose,
                min: c.tree.meta.min,
                size: c.tree.meta.size,
                points: c.tree.meta.points,
                has_color: c.tree.meta.has_color,
                has_intensity: c.tree.meta.has_intensity,
                nodes: c
                    .tree
                    .nodes
                    .iter()
                    .map(|n| NodeView {
                        name: n.name.clone(),
                        min: n.min,
                        size: n.size,
                        count: n.count,
                        spacing: n.spacing,
                        children: n.children,
                    })
                    .collect(),
            })
            .collect();
        Ok(SceneView {
            origin: scene.origin(),
            scans,
        })
    })
    .await
}

/// Analysis state that changes as the examiner works. Small; sent after every change.
#[derive(Serialize)]
pub struct StateView {
    revisions: BTreeMap<String, u64>,
    octrees: Vec<OctreeRecord>,
    measurements: Vec<MeasurementRecord>,
    cleanups: Vec<CleanupRecord>,
    point_sigma_m: f64,
}

fn state_view(s: &AppState) -> CmdResult<StateView> {
    let guard = s.project.lock().unwrap();
    let p = guard.as_ref().ok_or("Open or create a project first.")?;
    let scene = s.scene.read().unwrap();
    Ok(StateView {
        revisions: scene
            .scans
            .iter()
            .map(|(k, c)| (k.to_string(), c.revision))
            .collect(),
        octrees: p.octrees().map_err(err)?,
        measurements: p.measurements().map_err(err)?,
        cleanups: p.cleanups().map_err(err)?,
        point_sigma_m: p.point_sigma().map_err(err)?,
    })
}

#[tauri::command]
pub async fn analysis_state(app: AppHandle) -> CmdResult<StateView> {
    blocking(app, state_view).await
}

// ---------- picking and measurement ----------

/// What the GPU pick pass identified: a served point, never a coordinate.
#[derive(Deserialize)]
pub struct Pick {
    pub scan: String,
    node: usize,
    k: usize,
    pub revision: u64,
}

pub fn resolve(scene: &Scene, pick: &Pick) -> CmdResult<Resolved> {
    let key = ScanKey::parse(&pick.scan).ok_or("bad scan key")?;
    scene
        .resolve(key, pick.node, pick.k, pick.revision)
        .map_err(err)
}

#[tauri::command]
pub async fn pick_resolve(app: AppHandle, pick: Pick) -> CmdResult<Resolved> {
    blocking(app, move |s| resolve(&s.scene.read().unwrap(), &pick)).await
}

fn compute(kind: &str, pts: &[measure::P3], sigma: f64) -> CmdResult<Value> {
    let n = pts.len();
    Ok(match (kind, n) {
        ("distance", 2) => json!(measure::distance(pts[0], pts[1], sigma)),
        ("angle", 3) => json!(measure::angle(pts[0], pts[1], pts[2], sigma).map_err(err)?),
        ("area", 3..) => json!(measure::polygon_area(pts, sigma).map_err(err)?),
        ("height", 4..) => {
            json!(measure::height_above_plane(&pts[..n - 1], pts[n - 1], sigma).map_err(err)?)
        }
        _ => return Err(format!("{kind} cannot be measured from {n} points")),
    })
}

/// Measure from picked points. Every coordinate is re-resolved here from the stored f64
/// data; the client's own idea of where the points are is never used.
#[tauri::command]
pub async fn measure(app: AppHandle, kind: String, picks: Vec<Pick>) -> CmdResult<StateView> {
    blocking(app, move |s| {
        let resolved: Vec<Resolved> = {
            let scene = s.scene.read().unwrap();
            picks.iter().map(|p| resolve(&scene, p)).collect::<CmdResult<_>>()?
        };
        {
            let mut guard = s.project.lock().unwrap();
            let p = guard.as_mut().ok_or("Open or create a project first.")?;
            let sigma = p.point_sigma().map_err(err)?;
            let pts: Vec<measure::P3> = resolved.iter().map(|r| r.project).collect();
            let mut result = compute(&kind, &pts, sigma)?;
            result["sigma_point_m"] = json!(sigma);
            let points = json!(resolved
                .iter()
                .map(|r| json!({ "scan": r.scan.to_string(), "index": r.index, "project": r.project }))
                .collect::<Vec<_>>());
            p.add_measurement(&kind, points, result).map_err(err)?;
        }
        state_view(s)
    })
    .await
}

#[tauri::command]
pub async fn measurement_delete(app: AppHandle, id: i64) -> CmdResult<StateView> {
    blocking(app, move |s| {
        s.project
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("no project")?
            .delete_measurement(id)
            .map_err(err)?;
        state_view(s)
    })
    .await
}

#[tauri::command]
pub async fn set_point_sigma(app: AppHandle, meters: f64) -> CmdResult<StateView> {
    if !(meters > 0.0 && meters < 1.0) {
        return Err("Point uncertainty must be between 0 and 1 m.".into());
    }
    blocking(app, move |s| {
        s.project
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("no project")?
            .set_setting(locus_core::POINT_SIGMA_KEY, &meters.to_string())
            .map_err(err)?;
        state_view(s)
    })
    .await
}

// ---------- cleanup ----------

#[derive(Deserialize, Serialize, Clone, Copy)]
pub struct Region {
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
// A short-lived request per user action; boxing the lasso matrix would buy nothing.
#[allow(clippy::large_enum_variant)]
pub enum CleanupRequest {
    BoxDelete {
        region: Region,
    },
    LassoDelete {
        view_proj: [f64; 16],
        origin: [f64; 3],
        polygon: Vec<[f64; 2]>,
        depth: cleanup::LassoDepth,
        #[serde(default)]
        clip: cleanup::Clip,
    },
    Outliers {
        k: usize,
        std_mult: f64,
        region: Option<Region>,
    },
    Voxel {
        size: f64,
        region: Option<Region>,
    },
}

impl CleanupRequest {
    fn kind(&self) -> &'static str {
        match self {
            CleanupRequest::BoxDelete { .. } => "box_delete",
            CleanupRequest::LassoDelete { .. } => "lasso_delete",
            CleanupRequest::Outliers { .. } => "outliers",
            CleanupRequest::Voxel { .. } => "voxel",
        }
    }

    fn run(&self, scene: &Scene, progress: &mut dyn FnMut(u64, u64)) -> CmdResult<Removal> {
        let r = |o: &Option<Region>| o.map(|r| (r.min, r.max));
        match self {
            CleanupRequest::BoxDelete { region } => {
                cleanup::box_delete(scene, region.min, region.max)
            }
            CleanupRequest::LassoDelete {
                view_proj,
                origin,
                polygon,
                depth,
                clip,
            } => cleanup::lasso_delete(scene, view_proj, origin, polygon, *depth, clip),
            CleanupRequest::Outliers {
                k,
                std_mult,
                region,
            } => {
                if *k == 0 || *k > 64 || !std_mult.is_finite() || *std_mult <= 0.0 {
                    return Err("Choose 1 to 64 neighbours and a positive threshold.".into());
                }
                cleanup::outliers(scene, *k, *std_mult, r(region), progress)
            }
            CleanupRequest::Voxel { size, region } => {
                if !size.is_finite() || *size <= 0.0 {
                    return Err("Voxel size must be positive.".into());
                }
                cleanup::voxel_downsample(scene, *size, r(region), progress)
            }
        }
        .map_err(err)
    }
}

/// Run a cleanup operation, store what it removed as derived bitmaps, and record it.
#[tauri::command]
pub async fn cleanup_apply(app: AppHandle, request: CleanupRequest) -> CmdResult<StateView> {
    let emitter = app.clone();
    blocking(app, move |s| {
        let removal = {
            let scene = s.scene.read().unwrap();
            request.run(&scene, &mut |done, total| {
                let _ = emitter.emit("cleanup-progress", (done, total));
            })?
        };
        if removal.is_empty() {
            return Err("Nothing to remove: no visible points matched.".into());
        }
        {
            let mut guard = s.project.lock().unwrap();
            let p = guard.as_mut().ok_or("Open or create a project first.")?;
            let stamp = chrono_like_stamp();
            let scans = removal
                .iter()
                .map(|(k, bm)| {
                    let (file, sha256) =
                        write_bitmap(p.root(), &format!("{stamp}-{k}"), bm).map_err(err)?;
                    Ok(CleanupScan {
                        evidence_id: k.evidence_id,
                        scan_idx: k.scan_idx,
                        removed: bm.len(),
                        file,
                        sha256,
                    })
                })
                .collect::<CmdResult<Vec<_>>>()?;
            let params = serde_json::to_value(&request).map_err(err)?;
            p.add_cleanup(request.kind(), params, scans).map_err(err)?;
            s.scene.write().unwrap().reload_removed(p).map_err(err)?;
        }
        state_view(s)
    })
    .await
}

/// How many visible points a cleanup would remove, without recording anything.
#[tauri::command]
pub async fn cleanup_preview(app: AppHandle, request: CleanupRequest) -> CmdResult<u64> {
    blocking(app, move |s| {
        let scene = s.scene.read().unwrap();
        let removal = request.run(&scene, &mut |_, _| {})?;
        Ok(removal.iter().map(|(_, bm)| bm.len()).sum())
    })
    .await
}

/// Unique, sortable file stem for a cleanup's bitmaps.
fn chrono_like_stamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}{:09}", d.as_secs(), d.subsec_nanos())
}

/// Undo (`active: false`) or redo a cleanup operation.
#[tauri::command]
pub async fn cleanup_set_active(app: AppHandle, id: i64, active: bool) -> CmdResult<StateView> {
    blocking(app, move |s| {
        {
            let mut guard = s.project.lock().unwrap();
            let p = guard.as_mut().ok_or("Open or create a project first.")?;
            p.set_cleanup_active(id, active).map_err(err)?;
            s.scene.write().unwrap().reload_removed(p).map_err(err)?;
        }
        state_view(s)
    })
    .await
}

/// Project to open at startup, from `LOCUS_OPEN` and `LOCUS_EXAMINER` (used by scripted
/// runs such as performance measurements; opening still goes through the normal path).
#[derive(Serialize)]
pub struct Startup {
    open: Option<String>,
    examiner: Option<String>,
}

#[tauri::command]
pub fn startup() -> Startup {
    Startup {
        open: std::env::var("LOCUS_OPEN").ok(),
        examiner: std::env::var("LOCUS_EXAMINER").ok(),
    }
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    webview: String,
}

/// Licences and attributions of the third-party data bundled in the app (also shipped as
/// THIRD_PARTY_NOTICES.txt next to it).
#[tauri::command]
pub fn third_party_notices() -> &'static str {
    include_str!("../../THIRD_PARTY_NOTICES.txt")
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        webview: tauri::webview_version().unwrap_or_default(),
    }
}
