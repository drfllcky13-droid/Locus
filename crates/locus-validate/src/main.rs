//! Synthetic ground-truth scenes and the end-to-end accuracy suite (built in Phase 13).
//!
//! Available now: `locus-validate gen-scene --scans N --points-per-scan M --out FILE.e57
//! [--seed S]`, which writes a synthetic multi-station E57 for performance and pipeline
//! testing.

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
        _ => {
            eprintln!("locus-validate: no validation scenarios yet (Phase 13). Try `gen-scene`.");
            ExitCode::FAILURE
        }
    }
}
