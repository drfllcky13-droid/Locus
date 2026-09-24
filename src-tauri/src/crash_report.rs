//! Crash reports that never leave the machine by themselves and never carry case data: a panic
//! writes the app version, system, the message (with anything that looks like a file path
//! removed) and a backtrace of Locus's own code to the app's log folder. On the next start the
//! app offers to show it, and the user can copy it and send it on if they choose.

use crate::commands::CmdResult;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Replace anything that looks like a file path (a drive letter, a slash or a backslash in a
/// word) with `<path>`, so a case's folder or evidence names can't end up in a report.
pub fn scrub(s: &str) -> String {
    s.split_inclusive(char::is_whitespace)
        .map(|w| {
            let t = w.trim_end();
            let looks = t.contains('\\')
                || t.contains('/')
                || (t.len() > 2
                    && t.as_bytes()[1] == b':'
                    && t.as_bytes()[0].is_ascii_alphabetic());
            if looks {
                format!("<path>{}", &w[t.len()..])
            } else {
                w.to_string()
            }
        })
        .collect()
}

fn dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_log_dir()
        .ok()
        .map(|d| d.join("crash-reports"))
}

fn write(dir: &Path, kind: &str, message: &str, detail: &str) {
    let _ = std::fs::create_dir_all(dir);
    let at = locus_core::timestamp();
    let text = format!(
        "Locus crash report\r\n\
         Version: {}\r\nSystem: {} {}\r\nWhen: {at}\r\nKind: {kind}\r\n\
         \r\nMessage:\r\n{}\r\n\r\nWhere:\r\n{}\r\n\
         \r\nThis report holds no case data: file paths are removed. Nothing was sent anywhere.\r\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        scrub(message),
        scrub(detail),
    );
    let name = format!("crash-{}.txt", at.replace([':', '.'], "-"));
    let _ = std::fs::write(dir.join(name), text);
}

/// Install the panic hook: every panic, on any thread, writes a report.
pub fn install(app: &AppHandle) {
    let Some(d) = dir(app) else { return };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let place = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let bt = std::backtrace::Backtrace::force_capture();
        // Only Locus's own frames: names of functions, not data.
        let ours: Vec<String> = bt
            .to_string()
            .lines()
            .filter(|l| l.contains("locus"))
            .take(40)
            .map(|l| l.trim().to_string())
            .collect();
        write(
            &d,
            "panic",
            &message,
            &format!("{place}\r\n{}", ours.join("\r\n")),
        );
        previous(info);
    }));
}

#[derive(Serialize)]
pub struct CrashReport {
    pub name: String,
    pub text: String,
}

/// Reports written since they were last cleared.
#[tauri::command]
pub fn crash_reports(app: AppHandle) -> Vec<CrashReport> {
    let Some(d) = dir(&app) else { return vec![] };
    let mut out: Vec<CrashReport> = std::fs::read_dir(&d)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.ends_with(".txt").then(|| CrashReport {
                text: std::fs::read_to_string(e.path()).unwrap_or_default(),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Remove the saved reports (after the user has read or sent them).
#[tauri::command]
pub fn crash_reports_clear(app: AppHandle) -> CmdResult<()> {
    if let Some(d) = dir(&app) {
        let _ = std::fs::remove_dir_all(d);
    }
    Ok(())
}

/// An error the view couldn't handle (an uncaught exception), written like a panic.
#[tauri::command]
pub fn crash_report_view(app: AppHandle, message: String, stack: String) {
    if let Some(d) = dir(&app) {
        write(&d, "view error", &message, &stack);
    }
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn paths_never_reach_a_report() {
        let s = scrub("Could not open E:\\Cases\\2026-00417\\hall.e57: denied (see /home/x/y)");
        assert!(
            !s.contains("Cases") && !s.contains("hall.e57") && !s.contains("home"),
            "{s}"
        );
        assert!(
            s.starts_with("Could not open <path> denied (see <path>"),
            "{s}"
        );
    }
}
