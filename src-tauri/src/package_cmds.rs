//! The portable case package: a folder with the viewer (this app, which opens read-only when
//! it finds the package's manifest beside it), a copy of the case's data, freshly printed
//! reports and the renders, all listed with their SHA-256 in `manifest.json`. See
//! docs/methods/case-package.md.

use crate::commands::{blocking, err, CmdResult};
use locus_core::package::{self, Manifest, PackageCheck};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub const VIEWER_EXE: &str = if cfg!(windows) {
    "Lotus Viewer.exe"
} else {
    "Lotus Viewer"
};

const README: &str = "Lotus case package\r\n\
\r\n\
Open \"Lotus Viewer\" in this folder to view the case. Nothing needs installing and no \r\n\
administrator rights are needed. On Windows it uses Microsoft Edge WebView2, which is part \r\n\
of Windows 11 and of up-to-date Windows 10.\r\n\
\r\n\
The viewer is read-only: it opens the case's database read-only and cannot change anything. \r\n\
On opening, it checks every file in this folder against manifest.json and shows the result \r\n\
and the package hash (the SHA-256 of manifest.json) under Help > About.\r\n\
\r\n\
reports/  the case report and every analysis, diagram and registration report, as PDF\r\n\
videos/   the renders (MP4), each checked against its logged SHA-256\r\n\
case/     the case's database and derived data (point clouds), read by the viewer\r\n";

/// A file name from a record's name: letters, digits, spaces, `-`, `_` and `.`.
fn safe(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    t.trim().chars().take(80).collect()
}

fn copy_dir(from: &Path, to: &Path) -> CmdResult<()> {
    std::fs::create_dir_all(to).map_err(err)?;
    for e in std::fs::read_dir(from).map_err(err)? {
        let e = e.map_err(err)?;
        let p = e.path();
        let q = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &q)?;
        } else {
            std::fs::copy(&p, &q).map_err(|x| format!("Could not copy {}: {x}", p.display()))?;
        }
    }
    Ok(())
}

fn set_read_only(dir: &Path) -> CmdResult<()> {
    for e in std::fs::read_dir(dir).map_err(err)? {
        let p = e.map_err(err)?.path();
        if p.is_dir() {
            set_read_only(&p)?;
        } else {
            let mut perm = std::fs::metadata(&p).map_err(err)?.permissions();
            perm.set_readonly(true);
            std::fs::set_permissions(&p, perm).map_err(err)?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
pub struct Made {
    pub path: String,
    pub hash: String,
    pub files: usize,
    pub not_included: Vec<String>,
}

/// Make a case package in a new folder `<name> case package` inside `parent`.
#[tauri::command]
pub async fn package_export(
    app: AppHandle,
    parent: String,
    name: String,
    include_evidence: bool,
) -> CmdResult<Made> {
    crate::license_cmds::require(locus_core::license::Feature::CasePackage)?;
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let name = if name.trim().is_empty() {
            p.name().map_err(err)?
        } else {
            name
        };
        let root = PathBuf::from(&parent).join(format!("{} case package", safe(&name)));
        if root.exists() {
            return Err(format!("{} already exists.", root.display()));
        }
        let fail = |e: String| {
            let _ = std::fs::remove_dir_all(&root);
            e
        };
        let made = (|| -> CmdResult<(String, Manifest)> {
            let case = root.join("case");
            let (reports, videos) = (root.join("reports"), root.join("videos"));
            for d in [&case, &reports, &videos] {
                std::fs::create_dir_all(d).map_err(err)?;
            }
            // The data: a consistent copy of the database, and the derived point clouds.
            p.copy_database(&case.join("project.sqlite")).map_err(err)?;
            for (sub, want) in [
                ("derived", true),
                ("assets", true),
                ("evidence", include_evidence),
            ] {
                let from = p.root().join(sub);
                if want && from.is_dir() {
                    copy_dir(&from, &case.join(sub))?;
                }
            }
            // Reports, printed now from the saved records.
            let data = crate::export_cmds::case_data(p)?;
            let head = data.audit_head.clone();
            let pdf = locus_report::analysis::pdf(&locus_report::case::report(&data), vec![])?;
            std::fs::write(reports.join("Case report.pdf"), &pdf.pdf).map_err(err)?;
            let mut not_included = vec![];
            for a in p.analyses().map_err(err)? {
                if a.withdrawn.is_some() {
                    continue;
                }
                if a.tool == "render" {
                    // The render's file, only if it is still exactly what was logged.
                    let file = a.record["file"].as_str().unwrap_or_default().to_string();
                    let want = a.record["sha256"].as_str().unwrap_or_default();
                    let ok = locus_core::hash::sha256_file(Path::new(&file), &mut |_| {})
                        .is_ok_and(|(h, _)| h == want);
                    if ok {
                        let to = videos.join(format!("{} {}.mp4", a.id, safe(&a.name)));
                        std::fs::copy(&file, &to).map_err(err)?;
                    } else {
                        not_included.push(format!(
                            "render {} \"{}\": {} is missing or no longer matches its SHA-256",
                            a.id, a.name, file
                        ));
                    }
                    continue;
                }
                match crate::analysis_cmds::analysis_pdf(p, a.id) {
                    Ok(pdf) => {
                        let f = format!("Analysis {} {} - {}.pdf", a.id, a.tool, safe(&a.name));
                        std::fs::write(reports.join(f), pdf).map_err(err)?;
                    }
                    Err(e) => {
                        not_included.push(format!("analysis {} \"{}\" report: {e}", a.id, a.name))
                    }
                }
            }
            for d in p.diagrams().map_err(err)? {
                // The largest standard scale that fits on A3 landscape.
                let pdf = [50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0]
                    .iter()
                    .find_map(|&scale| {
                        let (_, dd, o, files) = crate::diagram_cmds::printable(
                            p,
                            d.document_id,
                            scale,
                            locus_report::diagram::Paper::A3,
                            true,
                        )
                        .ok()?;
                        locus_report::diagram::pdf(
                            &dd,
                            &locus_report::diagram::symbols(),
                            &o,
                            files,
                        )
                        .ok()
                        .map(|r| (scale, r.pdf))
                    });
                match pdf {
                    Some((scale, bytes)) => std::fs::write(
                        reports.join(format!(
                            "Diagram {} {} 1-{scale}.pdf",
                            d.document_id,
                            safe(&d.name)
                        )),
                        bytes,
                    )
                    .map_err(err)?,
                    None => not_included.push(format!(
                        "diagram {} \"{}\": empty, or too large for A3 at 1:5000",
                        d.document_id, d.name
                    )),
                }
            }
            if let Some(id) = p.applied_registration().map_err(err)? {
                let pdf = crate::register_cmds::registration_pdf(p, id)?;
                std::fs::write(reports.join(format!("Registration {id}.pdf")), pdf).map_err(err)?;
            }
            // The viewer: this program.
            let exe = std::env::current_exe().map_err(err)?;
            std::fs::copy(&exe, root.join(VIEWER_EXE))
                .map_err(|e| format!("Could not copy the viewer: {e}"))?;
            std::fs::write(root.join("README.txt"), README).map_err(err)?;
            std::fs::write(
                root.join("THIRD_PARTY_NOTICES.txt"),
                include_str!("../../THIRD_PARTY_NOTICES.txt"),
            )
            .map_err(err)?;
            let m = Manifest {
                kind: package::KIND.into(),
                version: 1,
                project: p.name().map_err(err)?,
                case_number: p.setting("case_number").map_err(err)?,
                made_by: p.examiner().to_string(),
                made_at: locus_core::timestamp(),
                app_version: env!("CARGO_PKG_VERSION").into(),
                source_state_head: head,
                evidence_included: include_evidence,
                not_included,
                files: vec![],
            };
            let hash = package::write_manifest(&root, m.clone(), &[]).map_err(err)?;
            set_read_only(&root)?;
            Ok((hash, m))
        })()
        .map_err(fail)?;
        let (hash, m) = made;
        let files = package::verify(&root, &[], &mut |_| {})
            .map_err(err)?
            .checked;
        let manifest = root.join(package::MANIFEST).to_string_lossy().into_owned();
        crate::export_cmds::log_written(
            p,
            "case package",
            &manifest,
            serde_json::json!({
                "package_hash": hash,
                "folder": root.to_string_lossy(),
                "files": files,
                "evidence_included": include_evidence,
                "not_included": m.not_included,
            }),
        )?;
        Ok(Made {
            path: root.to_string_lossy().into_owned(),
            hash,
            files,
            not_included: m.not_included,
        })
    })
    .await
}

/// The package this program was started from: its own folder, when a case package's
/// manifest is there (or `LOCUS_PACKAGE`, for tests).
pub fn package_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("LOCUS_PACKAGE") {
        return Some(PathBuf::from(d));
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    package::is_package(&dir).then_some(dir)
}

#[derive(Serialize)]
pub struct Opened {
    pub project: crate::commands::ProjectInfo,
    pub check: PackageCheck,
    pub folder: String,
}

/// Open the package this program was started from: check every file against the manifest,
/// then open the case read-only.
#[tauri::command]
pub async fn package_open(
    app: AppHandle,
    on_progress: tauri::ipc::Channel<u64>,
) -> CmdResult<Opened> {
    let dir = package_dir().ok_or("This program wasn't started from a case package.")?;
    let handle = app.clone();
    blocking(app, move |s| {
        let check = package::verify(&dir, &[], &mut |b| {
            let _ = on_progress.send(b);
        })
        .map_err(err)?;
        let p = locus_core::Project::open_read_only(&dir.join("case")).map_err(err)?;
        let info = crate::commands::info(&p, None)?;
        crate::scene_cmds::refresh(&handle, &p)?;
        *s.project.lock().unwrap() = Some(p);
        Ok(Opened {
            project: info,
            check,
            folder: dir.to_string_lossy().into_owned(),
        })
    })
    .await
}

/// Open one of the package's reports or videos with the system's viewer.
#[tauri::command]
pub async fn package_file_open(file: String) -> CmdResult<()> {
    let dir = package_dir().ok_or("Not a case package.")?;
    let rel = Path::new(&file);
    if !rel
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!("Not a file in the package: {file}"));
    }
    let path = dir.join(rel);
    if !path.is_file() {
        return Err(format!("{file} isn't in the package."));
    }
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(&path).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&path).spawn();
    r.map(|_| ())
        .map_err(|e| format!("Could not open {file}: {e}"))
}
