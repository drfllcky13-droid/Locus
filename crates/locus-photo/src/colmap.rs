//! Running a user-installed COLMAP as a separate process (never linked; see
//! docs/phase8-colmap-licence-review.txt): its build information, the stages of a
//! reconstruction as command lines, progress read from its log, and cancel.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The oldest COLMAP this orchestration is written for (the 3.9 option names).
pub const MIN_VERSION: (u32, u32, u32) = (3, 9, 0);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Build {
    pub version: (u32, u32, u32),
    /// The line COLMAP prints, e.g. "COLMAP 4.2.0 (Commit be5e291 on 2026-08-31 with CUDA)".
    pub banner: String,
    pub cuda: bool,
}

/// COLMAP's banner line, from `colmap -h`.
pub fn parse_banner(text: &str) -> Option<Build> {
    let line = text.lines().find(|l| l.contains("COLMAP "))?;
    let i = line.find("COLMAP ")? + 7;
    let v: Vec<u32> = line[i..]
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()?
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let version = (
        *v.first()?,
        *v.get(1).unwrap_or(&0),
        *v.get(2).unwrap_or(&0),
    );
    let banner = line[line.find("COLMAP ")?..].trim().to_string();
    Some(Build {
        version,
        cuda: banner.contains("with CUDA"),
        banner,
    })
}

/// Run `colmap -h` and read its build.
pub fn detect(exe: &Path) -> Result<Build, String> {
    let out = Command::new(exe)
        .arg("-h")
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .output()
        .map_err(|e| format!("could not run {}: {e}", exe.display()))?;
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    parse_banner(&text).ok_or_else(|| format!("{} didn't identify itself as COLMAP", exe.display()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matcher {
    /// Every image against every other (photos).
    Exhaustive,
    /// Each image against its neighbours in name order (video frames).
    Sequential,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Feature extraction and matching on the CPU: COLMAP's own SIFT (from VLFeat, BSD), never
    /// SiftGPU (non-commercial terms).
    pub cpu_only: bool,
    pub matcher: Matcher,
    /// One camera for every image (same body and lens, fixed zoom).
    pub single_camera: bool,
    /// COLMAP camera model: SIMPLE_RADIAL, RADIAL, OPENCV, …
    pub camera_model: String,
    /// Dense reconstruction (needs CUDA).
    pub dense: bool,
    /// Longest image side for feature extraction (px); 0 for COLMAP's default.
    pub max_image_size: u32,
    /// Longest image side for the dense cloud (px); 0 for full size. PatchMatch's time grows
    /// with the pixel count: 4800 px took 26 min for 14 photos where 2000 px takes minutes.
    #[serde(default = "dense_size")]
    pub dense_max_image_size: u32,
    /// The camera's starting parameters when the images carry no focal length (video frames,
    /// photos without EXIF), from a stated field of view; COLMAP treats them as a prior.
    #[serde(default)]
    pub camera_params: Option<String>,
    /// The horizontal field of view they came from (°).
    #[serde(default)]
    pub fov_deg: Option<f64>,
}

/// Starting parameters for a camera model from its image size and horizontal field of view:
/// the focal length (for fisheye models the equidistant f = (w/2) / (fov/2 in radians); for the
/// others the pinhole f = (w/2) / tan(fov/2)), the principal point at the centre, no distortion.
/// Fisheye models need them when images carry no focal length: COLMAP can't recover a fisheye
/// focal length from matches alone and treats every pair as degenerate.
pub fn initial_params(
    model: &str,
    width: u32,
    height: u32,
    fov_deg: f64,
) -> Result<String, String> {
    if !(fov_deg > 1.0 && fov_deg < 250.0) {
        return Err("the field of view must be between 1° and 250°".into());
    }
    let half = (fov_deg / 2.0).to_radians();
    let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
    let fisheye = model.contains("FISHEYE");
    if !fisheye && fov_deg >= 179.0 {
        return Err("a field of view that wide needs a fisheye camera model".into());
    }
    let f = if fisheye { cx / half } else { cx / half.tan() };
    let v: Vec<f64> = match model {
        "SIMPLE_PINHOLE" => vec![f, cx, cy],
        "PINHOLE" => vec![f, f, cx, cy],
        "SIMPLE_RADIAL" => vec![f, cx, cy, 0.0],
        "RADIAL" => vec![f, cx, cy, 0.0, 0.0],
        "OPENCV" | "OPENCV_FISHEYE" => vec![f, f, cx, cy, 0.0, 0.0, 0.0, 0.0],
        m => return Err(format!("camera model {m} isn't supported")),
    };
    Ok(v.iter()
        .map(|x| format!("{x:.3}"))
        .collect::<Vec<_>>()
        .join(","))
}

fn dense_size() -> u32 {
    2000
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            cpu_only: false,
            matcher: Matcher::Exhaustive,
            single_camera: false,
            camera_model: "OPENCV".into(),
            dense: true,
            // More than COLMAP's 3200 px: on ETH3D's pipes, 4800 px features took the 99th
            // percentile distance error from 1.02 % to 0.60 %. The full 6048 px crashed
            // COLMAP 4.2's CPU SIFT.
            max_image_size: 4800,
            dense_max_image_size: 2000,
            camera_params: None,
            fov_deg: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Stage {
    pub name: String,
    pub args: Vec<String>,
}

fn stage(name: &str, args: &[&str]) -> Stage {
    Stage {
        name: name.into(),
        args: std::iter::once(name)
            .chain(args.iter().copied())
            .map(String::from)
            .collect(),
    }
}

/// The sparse stages of a reconstruction of `images` into the workspace `work`: extraction,
/// matching and mapping; and, when dense was asked for but can't run, why not.
pub fn stages(
    build: &Build,
    s: &Settings,
    images: &Path,
    work: &Path,
) -> (Vec<Stage>, Option<String>) {
    let p = |x: &Path| x.to_string_lossy().to_string();
    let (db, sparse) = (work.join("database.db"), work.join("sparse"));
    // COLMAP 4 renamed the SIFT options.
    let (fe, fm) = if build.version.0 >= 4 {
        ("FeatureExtraction", "FeatureMatching")
    } else {
        ("SiftExtraction", "SiftMatching")
    };
    let gpu = if s.cpu_only { "0" } else { "1" };
    let mut extract = vec![
        "--database_path".to_string(),
        p(&db),
        "--image_path".into(),
        p(images),
        "--ImageReader.camera_model".into(),
        s.camera_model.clone(),
        "--ImageReader.single_camera".into(),
        (s.single_camera as u8).to_string(),
        format!("--{fe}.use_gpu"),
        gpu.into(),
    ];
    if s.max_image_size > 0 {
        extract.extend([
            format!("--{fe}.max_image_size"),
            s.max_image_size.to_string(),
        ]);
    }
    if let Some(cp) = &s.camera_params {
        extract.extend(["--ImageReader.camera_params".into(), cp.clone()]);
    }
    let matcher = match s.matcher {
        Matcher::Exhaustive => "exhaustive_matcher",
        Matcher::Sequential => "sequential_matcher",
    };
    let out = vec![
        Stage {
            name: "feature_extractor".into(),
            args: std::iter::once("feature_extractor".to_string())
                .chain(extract)
                .collect(),
        },
        stage(
            matcher,
            &["--database_path", &p(&db), &format!("--{fm}.use_gpu"), gpu],
        ),
        stage(
            "mapper",
            &[
                "--database_path",
                &p(&db),
                "--image_path",
                &p(images),
                "--output_path",
                &p(&sparse),
            ],
        ),
    ];
    let why = (s.dense && !build.cuda).then(|| {
        format!(
            "This COLMAP ({}) was built without CUDA, so the dense point cloud can't be made; the sparse reconstruction runs on its own. Install COLMAP's CUDA build (and an NVIDIA GPU) for dense.",
            build.banner
        )
    });
    (out, why)
}

/// The dense stages for one sparse `model`: undistortion, PatchMatch stereo and fusion into
/// `work/dense/fused.ply`.
pub fn dense_stages(s: &Settings, images: &Path, model: &Path, work: &Path) -> Vec<Stage> {
    let p = |x: &Path| x.to_string_lossy().to_string();
    let dense = p(&work.join("dense"));
    let mut und = vec![
        "--image_path".to_string(),
        p(images),
        "--input_path".into(),
        p(model),
        "--output_path".into(),
        dense.clone(),
        "--output_type".into(),
        "COLMAP".into(),
    ];
    if s.dense_max_image_size > 0 {
        und.extend([
            "--max_image_size".into(),
            s.dense_max_image_size.to_string(),
        ]);
    }
    vec![
        Stage {
            name: "image_undistorter".into(),
            args: std::iter::once("image_undistorter".to_string())
                .chain(und)
                .collect(),
        },
        stage(
            "patch_match_stereo",
            &[
                "--workspace_path",
                &dense,
                "--PatchMatchStereo.geom_consistency",
                "1",
            ],
        ),
        stage(
            "stereo_fusion",
            &[
                "--workspace_path",
                &dense,
                "--output_path",
                &p(&work.join("dense").join("fused.ply")),
            ],
        ),
    ]
}

/// How far a stage has got, from one of its log lines: (done, total).
pub fn progress(stage: &str, line: &str) -> Option<(u32, u32)> {
    // "Processed file [3/14]", "Matching block [1/3, 2/3]", "Processing view 3 / 14",
    // "Fusing image [3/14]", "Undistorting image [3/14]".
    let key = match stage {
        "feature_extractor" => "Processed file [",
        // COLMAP 4 logs "Processing block", 3.x "Matching block".
        "exhaustive_matcher" | "sequential_matcher" if line.contains("Processing block [") => {
            "Processing block ["
        }
        "exhaustive_matcher" | "sequential_matcher" => "Matching block [",
        "image_undistorter" => "Undistorting image [",
        "patch_match_stereo" => "Processing view ",
        "stereo_fusion" => "Fusing image [",
        "mapper" => {
            // "Registering image #7 (8)": images registered so far, total unknown.
            let i = line.find("Registering image #")?;
            let r = &line[i..];
            let a = r.find('(')? + 1;
            let b = r[a..].find(')')? + a;
            // "(8)" in COLMAP 3, "(num_reg_frames=10)" in COLMAP 4.
            let n = r[a..b].rsplit('=').next()?.trim();
            return Some((n.parse().ok()?, 0));
        }
        _ => return None,
    };
    let i = line.find(key)? + key.len();
    let rest = &line[i..];
    let nums: Vec<u32> = rest
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(2)
        .filter_map(|s| s.parse().ok())
        .collect();
    (nums.len() == 2).then(|| (nums[0], nums[1]))
}

/// What a stage did.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StageRecord {
    pub name: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunError {
    Cancelled,
    Failed { stage: String, message: String },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Cancelled => f.write_str("cancelled"),
            RunError::Failed { stage, message } => write!(f, "COLMAP {stage} failed: {message}"),
        }
    }
}

/// Run one stage: every log line (stdout and stderr) goes to `on_line` and to `log`; `cancel`
/// is checked every 100 ms and kills the process.
pub fn run_stage(
    exe: &Path,
    st: &Stage,
    cancel: &AtomicBool,
    log: &mut dyn std::io::Write,
    on_line: &mut dyn FnMut(&str),
) -> Result<StageRecord, RunError> {
    let t0 = Instant::now();
    let fail = |m: String| RunError::Failed {
        stage: st.name.clone(),
        message: m,
    };
    let mut child = Command::new(exe)
        .args(&st.args)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| fail(format!("could not start: {e}")))?;
    let (tx, rx) = mpsc::channel::<String>();
    let pump = |r: Box<dyn Read + Send>, tx: mpsc::Sender<String>| {
        std::thread::spawn(move || {
            for l in BufReader::new(r).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        })
    };
    let a = pump(Box::new(child.stdout.take().unwrap()), tx.clone());
    let b = pump(Box::new(child.stderr.take().unwrap()), tx);
    let mut tail: Vec<String> = vec![];
    let status = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(l) => {
                let _ = writeln!(log, "{l}");
                on_line(&l);
                tail.push(l);
                if tail.len() > 20 {
                    tail.remove(0);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break child.wait().map_err(|e| fail(e.to_string()))?;
            }
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writeln!(log, "-- cancelled");
            return Err(RunError::Cancelled);
        }
    };
    let _ = (a.join(), b.join());
    if !status.success() {
        let code = status.code();
        // Windows' fail-fast, access-violation and out-of-memory codes.
        let hint = match code.map(|c| c as u32) {
            Some(0xC000_0409 | 0xC000_0005 | 0xC000_0017) => {
                " (COLMAP stopped unexpectedly, often from running out of memory: try a smaller image size)"
            }
            _ => "",
        };
        return Err(fail(format!(
            "exit code {code:?}{hint}; last lines:\n{}",
            tail.join("\n")
        )));
    }
    Ok(StageRecord {
        name: st.name.clone(),
        args: st.args.clone(),
        exit_code: status.code(),
        seconds: t0.elapsed().as_secs_f64(),
    })
}

/// The sparse models COLMAP wrote (`sparse/0`, `sparse/1`, …), in order.
pub fn sparse_models(work: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(work.join("sparse"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().parse::<u32>().is_ok())
        })
        .collect();
    v.sort();
    v
}

/// The stage converting a binary model to text (`cameras.txt`, `images.txt`, `points3D.txt`).
pub fn to_text(model: &Path, out: &Path) -> Stage {
    stage(
        "model_converter",
        &[
            "--input_path",
            &model.to_string_lossy(),
            "--output_path",
            &out.to_string_lossy(),
            "--output_type",
            "TXT",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_gives_version_and_cuda() {
        let b = parse_banner("junk\nCOLMAP 4.2.0 (Commit be5e291 on 2026-08-31 with CUDA)\nUsage:")
            .unwrap();
        assert_eq!(b.version, (4, 2, 0));
        assert!(b.cuda);
        let b = parse_banner(
            "I2026 option_manager.cc:1317] COLMAP 3.9.1 (Commit 0f2a5 on 2024-01-01 without CUDA)",
        )
        .unwrap();
        assert_eq!(b.version, (3, 9, 1));
        assert!(!b.cuda);
        assert_eq!(
            b.banner,
            "COLMAP 3.9.1 (Commit 0f2a5 on 2024-01-01 without CUDA)"
        );
        assert!(parse_banner("nothing here").is_none());
    }

    #[test]
    fn stages_follow_the_settings() {
        let b4 = parse_banner("COLMAP 4.2.0 (Commit x on y with CUDA)").unwrap();
        let s = Settings {
            cpu_only: true,
            ..Settings::default()
        };
        let (st, why) = stages(&b4, &s, Path::new("img"), Path::new("w"));
        assert!(why.is_none());
        let names: Vec<_> = st.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["feature_extractor", "exhaustive_matcher", "mapper"]);
        let d = dense_stages(
            &s,
            Path::new("img"),
            Path::new("w/sparse/1"),
            Path::new("w"),
        );
        assert_eq!(d.len(), 3);
        assert!(d[0].args.join(" ").contains("--input_path w/sparse/1"));
        let fe = st[0].args.join(" ");
        assert!(fe.contains("--FeatureExtraction.use_gpu 0"), "{fe}");
        assert!(st[1].args.join(" ").contains("--FeatureMatching.use_gpu 0"));
        // COLMAP 3 names; no CUDA: sparse only, with the reason.
        let b3 = parse_banner("COLMAP 3.9.1 (Commit x on y without CUDA)").unwrap();
        let s = Settings {
            matcher: Matcher::Sequential,
            ..Settings::default()
        };
        let (st, why) = stages(&b3, &s, Path::new("img"), Path::new("w"));
        assert_eq!(st.len(), 3);
        assert_eq!(st[1].name, "sequential_matcher");
        assert!(st[0].args.join(" ").contains("--SiftExtraction.use_gpu 1"));
        assert!(why.unwrap().contains("without CUDA"));
    }

    #[test]
    fn a_field_of_view_gives_the_starting_focal_length() {
        // ETH3D's pipes at half size: 3024 px wide, fx 1715 (fisheye): half-angle 1512/1715 rad.
        let fov = 2.0 * (1512.0f64 / 1715.0).to_degrees();
        let p = initial_params("OPENCV_FISHEYE", 3024, 2016, fov).unwrap();
        assert_eq!(
            p,
            "1715.000,1715.000,1512.000,1008.000,0.000,0.000,0.000,0.000"
        );
        // Pinhole: 90° over 2000 px is f = 1000.
        assert_eq!(
            initial_params("SIMPLE_RADIAL", 2000, 1000, 90.0).unwrap(),
            "1000.000,1000.000,500.000,0.000"
        );
        assert!(initial_params("OPENCV", 2000, 1000, 200.0).is_err());
        let b = parse_banner("COLMAP 4.2.0 (Commit x on y with CUDA)").unwrap();
        let s = Settings {
            camera_params: Some(p.clone()),
            ..Settings::default()
        };
        let (st, _) = stages(&b, &s, Path::new("i"), Path::new("w"));
        assert!(st[0]
            .args
            .join(" ")
            .contains(&format!("--ImageReader.camera_params {p}")));
    }

    #[test]
    fn progress_is_read_from_the_log() {
        assert_eq!(
            progress(
                "exhaustive_matcher",
                "I0923 pairing.cc:212] Processing block [2/4, 1/1]"
            ),
            Some((2, 4))
        );
        assert_eq!(
            progress(
                "feature_extractor",
                "I0923 feature_extraction.cc:258] Processed file [3/14]"
            ),
            Some((3, 14))
        );
        assert_eq!(
            progress("exhaustive_matcher", "Matching block [1/3, 2/3] in 0.5s"),
            Some((1, 3))
        );
        assert_eq!(progress("mapper", "Registering image #7 (8)"), Some((8, 0)));
        assert_eq!(
            progress(
                "mapper",
                "incremental_pipeline.cc:620] Registering image #9 (num_reg_frames=10)"
            ),
            Some((10, 0))
        );
        assert_eq!(
            progress(
                "patch_match_stereo",
                "=== Processing view 3 / 14 for 10_DSC_0642.JPG ==="
            ),
            Some((3, 14))
        );
        assert_eq!(
            progress("stereo_fusion", "Fusing image [12/14] in 1s"),
            Some((12, 14))
        );
        assert_eq!(progress("feature_extractor", "Elapsed time: 0.1"), None);
    }
}

/// What a reconstruction produced.
#[derive(Debug, Clone)]
pub struct Reconstruction {
    pub stages: Vec<StageRecord>,
    /// The largest sparse model (most registered images) and its folder.
    pub model: crate::model::Model,
    pub model_dir: PathBuf,
    /// Other sparse models COLMAP wrote (disconnected groups of images), by image count.
    pub other_models: Vec<usize>,
    pub dense_ply: Option<PathBuf>,
    /// Why dense wasn't made, when it was asked for.
    pub note: Option<String>,
}

/// Run a whole reconstruction of `images` in the workspace `work` (created): the sparse stages,
/// each model converted to text, the largest kept, then dense on it when asked and possible.
/// `on_line(stage, line)` sees every log line; the full log is written to `work/colmap.log`.
pub fn reconstruct(
    exe: &Path,
    build: &Build,
    s: &Settings,
    images: &Path,
    work: &Path,
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str, &str),
) -> Result<Reconstruction, RunError> {
    let fail = |m: String| RunError::Failed {
        stage: "setup".into(),
        message: m,
    };
    std::fs::create_dir_all(work.join("sparse")).map_err(|e| fail(e.to_string()))?;
    let mut log =
        std::fs::File::create(work.join("colmap.log")).map_err(|e| fail(e.to_string()))?;
    let mut records = vec![];
    let mut run = |st: &Stage, records: &mut Vec<StageRecord>| -> Result<(), RunError> {
        let _ = writeln!(log, "== colmap {}", st.args.join(" "));
        let name = st.name.clone();
        records.push(run_stage(exe, st, cancel, &mut log, &mut |l| {
            on_line(&name, l)
        })?);
        Ok(())
    };
    let (sparse, note) = stages(build, s, images, work);
    for st in &sparse {
        run(st, &mut records)?;
    }
    let mut models = vec![];
    for m in sparse_models(work) {
        let txt = work.join("text").join(m.file_name().unwrap());
        std::fs::create_dir_all(&txt).map_err(|e| fail(e.to_string()))?;
        run(&to_text(&m, &txt), &mut records)?;
        let read = |f: &str| std::fs::read_to_string(txt.join(f)).map_err(|e| fail(e.to_string()));
        let model = crate::model::read(
            &read("cameras.txt")?,
            &read("images.txt")?,
            &read("points3D.txt")?,
        )
        .map_err(fail)?;
        models.push((m, model));
    }
    models.sort_by_key(|(_, m)| std::cmp::Reverse(m.images.len()));
    if models.is_empty() {
        return Err(RunError::Failed {
            stage: "mapper".into(),
            message: "COLMAP registered no images: check the photos overlap and are sharp".into(),
        });
    }
    let (model_dir, model) = models.remove(0);
    let mut dense_ply = None;
    if s.dense && build.cuda {
        for st in dense_stages(s, images, &model_dir, work) {
            run(&st, &mut records)?;
        }
        dense_ply = Some(work.join("dense").join("fused.ply"));
    }
    Ok(Reconstruction {
        stages: records,
        model,
        model_dir,
        other_models: models.iter().map(|(_, m)| m.images.len()).collect(),
        dense_ply,
        note,
    })
}
