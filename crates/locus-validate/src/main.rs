//! Synthetic ground-truth scenes and the end-to-end accuracy suite (built in Phase 13).
//!
//! Available now:
//! - `locus-validate gen-scene --out FILE.e57 [options]` writes a synthetic multi-station
//!   E57 and its ground truth (`FILE.e57.truth.json`: true poses, targets, injected faults).
//!   Options: `--scans N` (4), `--points-per-scan M` (1000000), `--seed S` (1),
//!   `--no-targets`, `--tilt-deg D` (0.5), `--noise-mm R` (1), `--outliers F` (0),
//!   `--stored-pose true|none|M,DEG` (true; `M,DEG` perturbs by M metres and DEG degrees),
//!   `--move-sphere K,FROM,DX,DY,DZ` (sphere K moved by DX,DY,DZ m from scan FROM on).
//! - `locus-validate gen-trajectory|gen-bloodstain|gen-camera|gen-crush --out DIR [--seed S]
//!   [--options FILE.json]` write ground truth for the analysis tools (see `gen.rs`):
//!   `truth.json`, `scene.e57`, and images. `gen-bloodstain` also takes
//!   `--flight straight|ballistic`.
//! - `locus-validate import --project DIR --examiner NAME [--unit meter] FILE...` imports
//!   files and builds their octrees exactly as the app does, reporting time and memory.
//! - `locus-validate run --out REPORT.pdf [--full] [--json FILE]` runs every tool against
//!   synthetic ground truth and prints the validation report (docs/methods/validation.md);
//!   `--full` is the release size. Exits non-zero if any bound isn't met.

mod gen;
mod import;
mod report;
mod suite;

use std::process::ExitCode;

pub(crate) fn arg<T: std::str::FromStr>(
    args: &[String],
    name: &str,
    default: Option<T>,
) -> Result<T, String> {
    match args.iter().position(|a| a == name) {
        Some(i) => args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{name} needs a value")),
        None => default.ok_or_else(|| format!("{name} is required")),
    }
}

/// Comma-separated numbers after `name`, if the flag is present.
fn list(args: &[String], name: &str, n: usize) -> Result<Option<Vec<f64>>, String> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    let v: Vec<f64> = args
        .get(i + 1)
        .ok_or(format!("{name} needs a value"))?
        .split(',')
        .map(|x| {
            x.trim()
                .parse()
                .map_err(|_| format!("{name}: bad number {x:?}"))
        })
        .collect::<Result<_, _>>()?;
    if v.len() != n {
        return Err(format!("{name} needs {n} comma-separated values"));
    }
    Ok(Some(v))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen-scene") => {
            let run = || -> Result<(), String> {
                let out: String = arg(&args, "--out", None)?;
                let stored: String = arg(&args, "--stored-pose", Some("true".into()))?;
                let opts = locus_synth::Options {
                    scans: arg(&args, "--scans", Some(4))?,
                    points_per_scan: arg(&args, "--points-per-scan", Some(1_000_000))?,
                    seed: arg(&args, "--seed", Some(1))?,
                    targets: !args.iter().any(|a| a == "--no-targets"),
                    tilt_deg: arg(&args, "--tilt-deg", Some(0.5))?,
                    range_noise_m: arg::<f64>(&args, "--noise-mm", Some(1.0))? / 1000.0,
                    outliers: arg(&args, "--outliers", Some(0.0))?,
                    stored_pose: match stored.as_str() {
                        "true" => locus_synth::StoredPose::True,
                        "none" => locus_synth::StoredPose::None,
                        _ => {
                            let v = list(&args, "--stored-pose", 2)
                                .map_err(|_| "--stored-pose is true, none or METRES,DEGREES")?
                                .expect("flag present");
                            locus_synth::StoredPose::Perturbed {
                                metres: v[0],
                                degrees: v[1],
                            }
                        }
                    },
                    moved_sphere: list(&args, "--move-sphere", 5)?.map(|v| {
                        locus_synth::MovedSphere {
                            sphere: v[0] as usize,
                            from_scan: v[1] as usize,
                            offset: [v[2], v[3], v[4]],
                        }
                    }),
                };
                let truth = locus_synth::truth(&opts);
                if let Some(m) = opts.moved_sphere {
                    if m.sphere >= truth.spheres.len() {
                        return Err(format!(
                            "--move-sphere: there are {} spheres",
                            truth.spheres.len()
                        ));
                    }
                }
                let out = std::path::Path::new(&out);
                let t = std::time::Instant::now();
                let n = locus_synth::write_e57(out, &opts, &truth, &mut |s, i| {
                    if i % 10_000_000 == 0 {
                        eprintln!("scan {} of {}: {} points", s + 1, opts.scans, i);
                    }
                })
                .map_err(|e| e.to_string())?;
                let sidecar = locus_synth::truth_path(out);
                locus_synth::write_truth(&truth, &sidecar).map_err(|e| e.to_string())?;
                eprintln!(
                    "wrote {n} points to {} in {:.0?}; ground truth in {}",
                    out.display(),
                    t.elapsed(),
                    sidecar.display()
                );
                Ok(())
            };
            match run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("gen-scene: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(cmd @ ("gen-trajectory" | "gen-bloodstain" | "gen-camera" | "gen-crush")) => {
            let r = match cmd {
                "gen-trajectory" => gen::trajectory(&args),
                "gen-bloodstain" => gen::bloodstain(&args),
                "gen-crush" => gen::crush(&args),
                _ => gen::camera(&args),
            };
            match r {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{cmd}: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("gen-video") => {
            // An H.264 MP4 from a folder of images, in name order (Media Foundation; Windows):
            // for testing video import. `--scale` shrinks them (H.264 takes up to 4096 px wide).
            let run = || -> Result<(), String> {
                let dir: String = arg(&args, "--images", None)?;
                let out: String = arg(&args, "--out", None)?;
                let fps: u32 = arg(&args, "--fps", Some(2))?;
                let scale: f64 = arg(&args, "--scale", Some(0.5))?;
                let mut files: Vec<_> = std::fs::read_dir(&dir)
                    .map_err(|e| format!("{dir}: {e}"))?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_file())
                    .collect();
                files.sort();
                let frames = files
                    .iter()
                    .map(|f| locus_photo::video_dev::load_image(f, scale))
                    .collect::<Result<Vec<_>, _>>()?;
                locus_photo::video_dev::write_mp4(std::path::Path::new(&out), &frames, fps)?;
                eprintln!(
                    "wrote {} frames ({}×{}) at {fps} fps to {out}",
                    frames.len(),
                    frames[0].width,
                    frames[0].height
                );
                Ok(())
            };
            match run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("gen-video: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("import") => {
            let run = || -> Result<(), String> {
                let project: String = arg(&args, "--project", None)?;
                let examiner: String = arg(&args, "--examiner", None)?;
                let unit = match args.iter().position(|a| a == "--unit") {
                    Some(i) => Some(
                        serde_json::from_value(serde_json::json!(args
                            .get(i + 1)
                            .ok_or("--unit needs a value")?))
                        .map_err(|_| "unknown unit")?,
                    ),
                    None => None,
                };
                let files: Vec<String> = args
                    .iter()
                    .enumerate()
                    .skip(1)
                    .filter(|(i, a)| !a.starts_with("--") && !args[i - 1].starts_with("--"))
                    .map(|(_, a)| a.clone())
                    .collect();
                import::run(std::path::Path::new(&project), &examiner, &files, unit)
            };
            match run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("import: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("run") => {
            let run = || -> Result<bool, String> {
                let out: String = arg(&args, "--out", None)?;
                let full = args.iter().any(|a| a == "--full");
                let t = std::time::Instant::now();
                let results = suite::all(full, &mut |name| eprintln!("validating {name}…"));
                let commit = std::env::var("LOCUS_COMMIT").ok().unwrap_or_else(|| {
                    std::process::Command::new("git")
                        .args(["rev-parse", "--short", "HEAD"])
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_else(|| "unknown".into())
                });
                let run = report::Run {
                    version: env!("CARGO_PKG_VERSION").into(),
                    commit,
                    made_at: locus_core::timestamp(),
                    machine: format!(
                        "{} {}, {} threads",
                        std::env::consts::OS,
                        std::env::consts::ARCH,
                        std::thread::available_parallelism().map_or(1, |n| n.get())
                    ),
                    full,
                    seconds: t.elapsed().as_secs_f64(),
                };
                let doc = report::report(&run, &results);
                let pdf = locus_report::analysis::pdf(&doc, vec![])?;
                std::fs::write(&out, &pdf.pdf).map_err(|e| format!("{out}: {e}"))?;
                if let Some(i) = args.iter().position(|a| a == "--json") {
                    let j = args.get(i + 1).ok_or("--json needs a file")?;
                    std::fs::write(
                        j,
                        serde_json::to_vec_pretty(&results).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| format!("{j}: {e}"))?;
                }
                for r in &results {
                    eprintln!(
                        "{}: {} cases, mean {:.4} {}, worst {:.4}{}{}",
                        r.tool,
                        r.errors.len(),
                        r.mean(),
                        r.unit,
                        r.max(),
                        r.coverage
                            .map_or(String::new(), |(i, n)| format!(", coverage {i}/{n}")),
                        match &r.bound {
                            Some((_, true)) => ", bound met",
                            Some((_, false)) => ", BOUND NOT MET",
                            None => "",
                        }
                    );
                }
                eprintln!("report: {out} ({} need attention)", doc.warnings.len());
                Ok(results.iter().all(|r| r.passed()))
            };
            match run() {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("run: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("locus-validate: run, gen-scene, gen-trajectory, gen-bloodstain, gen-camera, gen-crush or import (see the top of src/main.rs).");
            ExitCode::FAILURE
        }
    }
}
