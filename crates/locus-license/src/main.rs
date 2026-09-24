//! Makes Lotus licence files. The private key stays with the publisher, never in the
//! repository or the app (docs/methods/licensing.md).
//!
//! - `locus-license keygen --out KEY.txt` writes a new private key (hex) and prints its public
//!   key, to put in `src-tauri/src/license_cmds.rs`.
//! - `locus-license sign --key KEY.txt --id ID --licensee NAME --tier diagram|analyst|analyst_plus
//!   [--expires YYYY-MM-DD] --out FILE.locus-license` writes a signed licence.

use locus_core::license::{public_key, sign, License, Tier};
use std::process::ExitCode;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn run(args: &[String]) -> Result<(), String> {
    let need = |n: &str| arg(args, n).ok_or(format!("{n} is required"));
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let out = need("--out")?;
            if std::path::Path::new(&out).exists() {
                return Err(format!("{out} exists; not overwriting a key"));
            }
            let mut secret = [0u8; 32];
            getrandom::fill(&mut secret).map_err(|e| e.to_string())?;
            std::fs::write(&out, hex::encode(secret)).map_err(|e| format!("{out}: {e}"))?;
            println!("private key written to {out}: keep it safe, and out of the repository");
            println!("public key: {}", hex::encode(public_key(&secret)));
            Ok(())
        }
        Some("sign") => {
            let key = std::fs::read_to_string(need("--key")?).map_err(|e| e.to_string())?;
            let secret: [u8; 32] = hex::decode(key.trim())
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or("the key file isn't a 32-byte hex key")?;
            let tier: Tier = serde_json::from_value(serde_json::json!(need("--tier")?))
                .map_err(|_| "--tier is diagram, analyst or analyst_plus")?;
            let l = License {
                id: need("--id")?,
                licensee: need("--licensee")?,
                tier,
                issued: locus_core::timestamp()[..10].to_string(),
                expires: arg(args, "--expires"),
            };
            let out = need("--out")?;
            let f = sign(l, &secret);
            std::fs::write(
                &out,
                serde_json::to_string_pretty(&f).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("{out}: {e}"))?;
            println!("licence written to {out}");
            Ok(())
        }
        _ => Err("keygen or sign (see the top of src/main.rs)".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("locus-license: {e}");
            ExitCode::FAILURE
        }
    }
}
