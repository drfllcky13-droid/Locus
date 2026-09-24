//! The licence: read from the app's config folder at startup, checked against the public key
//! below, and consulted by the commands that belong to a tier (`require`). See
//! docs/methods/licensing.md.

use crate::commands::CmdResult;
use locus_core::license::{verify, Feature, License, Tier};
use serde::Serialize;
use std::sync::RwLock;
use tauri::{AppHandle, Manager};

/// The publisher's public key (its private key is kept outside the repository).
const PUBLIC_KEY: &str = "33a8fca39f5e99510b1e0dd5d8d040a33b1070d7cf121f170462b4ddb269ba66";

const FILE: &str = "license.locus-license";

#[derive(Debug, Clone, Serialize)]
pub struct LicenseInfo {
    /// The tier in force.
    pub tier: Tier,
    pub tier_name: &'static str,
    /// "licensed", "evaluation" (no licence file) or "invalid" (a file that doesn't verify:
    /// the app then runs as an evaluation and says why).
    pub status: &'static str,
    pub license: Option<License>,
    pub problem: Option<String>,
}

static STATE: RwLock<Option<LicenseInfo>> = RwLock::new(None);

/// Without a licence the app runs as an unlicensed evaluation with every tool, labelled as
/// such (whether that stays for a commercial release is the publisher's decision).
fn evaluation(problem: Option<String>) -> LicenseInfo {
    LicenseInfo {
        tier: Tier::AnalystPlus,
        tier_name: "Evaluation (unlicensed)",
        status: if problem.is_some() {
            "invalid"
        } else {
            "evaluation"
        },
        license: None,
        problem,
    }
}

fn check(text: &str) -> Result<License, String> {
    let key: [u8; 32] = hex::decode(PUBLIC_KEY)
        .ok()
        .and_then(|b| b.try_into().ok())
        .expect("the public key is 32 hex bytes");
    verify(text, &key, &locus_core::timestamp()[..10])
}

fn licensed(l: License) -> LicenseInfo {
    LicenseInfo {
        tier: l.tier,
        tier_name: l.tier.name(),
        status: "licensed",
        license: Some(l),
        problem: None,
    }
}

/// Read the licence at startup.
pub fn load(app: &AppHandle) {
    let info = match app.path().app_config_dir().map(|d| d.join(FILE)) {
        Ok(p) if p.is_file() => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| check(&t))
        {
            Ok(l) => licensed(l),
            Err(e) => evaluation(Some(e)),
        },
        _ => evaluation(None),
    };
    *STATE.write().unwrap() = Some(info);
}

fn current() -> LicenseInfo {
    STATE
        .read()
        .unwrap()
        .clone()
        .unwrap_or_else(|| evaluation(None))
}

/// Refuse a command outside the licence's tier.
pub fn require(f: Feature) -> CmdResult<()> {
    let i = current();
    if i.tier.allows(f) {
        Ok(())
    } else {
        Err(format!(
            "This needs a higher licence: the {} licence doesn't include it.",
            i.tier_name
        ))
    }
}

#[tauri::command]
pub fn license_info() -> LicenseInfo {
    current()
}

/// Check a licence file and, if it verifies, keep a copy in the app's config folder.
#[tauri::command]
pub fn license_install(app: AppHandle, path: String) -> CmdResult<LicenseInfo> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let l = check(&text).map_err(|e| {
        let e = e[..1].to_uppercase() + &e[1..];
        format!("{e}.")
    })?;
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(FILE), text).map_err(|e| e.to_string())?;
    let info = licensed(l);
    *STATE.write().unwrap() = Some(info.clone());
    Ok(info)
}
