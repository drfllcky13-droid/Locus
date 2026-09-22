//! Synthetic ground-truth scenes and the end-to-end accuracy suite (built in Phase 13).
//!
//! Available now:
//! - `locus-validate gen-scene --scans N --points-per-scan M --out FILE.e57 [--seed S]`
//!   writes a synthetic multi-station E57 for performance and pipeline testing.
//! - `locus-validate import --project DIR --examiner NAME [--unit meter] FILE...` imports
//!   files and builds their octrees exactly as the app does, reporting time and memory.

mod import;
mod scene;

use std::process::ExitCode;

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: Option<T>) -> Result<T, String> {
    match args.iter().position(|a| a == name) {
        Some(i) => args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{name} needs a value")),
        None => default.ok_or_else(|| format!("{name} is required")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen-scene") => {
            let run = || -> Result<(), String> {
                let out: String = arg(&args, "--out", None)?;
                let opts = scene::Options {
                    scans: arg(&args, "--scans", Some(4))?,
                    points_per_scan: arg(&args, "--points-per-scan", Some(1_000_000))?,
                    seed: arg(&args, "--seed", Some(1))?,
                };
                let t = std::time::Instant::now();
                let n = scene::generate(std::path::Path::new(&out), &opts, &mut |s, i| {
                    if i % 10_000_000 == 0 {
                        eprintln!("scan {} of {}: {} points", s + 1, opts.scans, i);
                    }
                })
                .map_err(|e| e.to_string())?;
                eprintln!("wrote {n} points to {out} in {:.0?}", t.elapsed());
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
        _ => {
            eprintln!("locus-validate: no validation scenarios yet (Phase 13). Try `gen-scene` or `import`.");
            ExitCode::FAILURE
        }
    }
}
