//! Tauri commands: a thin layer over locus-core and locus-io. No logic lives here
//! beyond argument checks and moving blocking work off the async runtime.

use locus_core::{EvidenceRecord, IntegrityReport, LinearUnit, Project};
use locus_io::{Preview, Progress};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct AppState {
    pub(crate) project: Mutex<Option<Project>>,
    /// The last preview shown to the examiner; commit only ever imports this one.
    preview: Mutex<Option<Preview>>,
    /// The open project's point clouds. Lock order: project, then scene.
    pub(crate) scene: RwLock<locus_octree::scene::Scene>,
}

pub(crate) type CmdResult<T> = Result<T, String>;

pub(crate) fn err(e: impl ToString) -> String {
    e.to_string()
}

/// Run blocking work (file I/O, SQLite) on a worker thread.
pub(crate) async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> CmdResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(err)?
}

#[derive(Serialize)]
pub struct ProjectInfo {
    name: String,
    root: PathBuf,
    examiner: String,
    evidence: Vec<EvidenceRecord>,
    audit_entries: i64,
    /// Hash of the newest audit entry; recording it outside the project anchors the log.
    audit_head: String,
    /// Latest evidence re-hash (on open, or from Verify Evidence); None for a new project.
    integrity: Option<IntegrityReport>,
}

fn info(p: &Project, integrity: Option<IntegrityReport>) -> CmdResult<ProjectInfo> {
    let log = p.audit_log().map_err(err)?;
    let head = log.last();
    Ok(ProjectInfo {
        name: p.name().map_err(err)?,
        root: p.root().into(),
        examiner: p.examiner().into(),
        evidence: p.evidence().map_err(err)?,
        audit_entries: head.map_or(0, |e| e.seq),
        audit_head: head.map(|e| e.hash.clone()).unwrap_or_default(),
        integrity,
    })
}

fn examiner(name: &str) -> CmdResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter the examiner's name; it is recorded with every change.".into());
    }
    Ok(name.into())
}

#[tauri::command]
pub async fn project_create(
    app: AppHandle,
    parent: PathBuf,
    name: String,
    examiner_name: String,
) -> CmdResult<ProjectInfo> {
    let examiner = examiner(&examiner_name)?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Enter a project name.".into());
    }
    let folder: String = name
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let root = parent.join(format!("{folder}.locus"));
    let handle = app.clone();
    blocking(app, move |s| {
        let p = Project::create(&root, &name, &examiner).map_err(err)?;
        let i = info(&p, None)?;
        crate::scene_cmds::refresh(&handle, &p)?;
        *s.project.lock().unwrap() = Some(p);
        Ok(i)
    })
    .await
}

#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    root: PathBuf,
    examiner_name: String,
    on_progress: Channel<u64>,
) -> CmdResult<ProjectInfo> {
    let examiner = examiner(&examiner_name)?;
    let handle = app.clone();
    blocking(app, move |s| {
        let p = Project::open_with_progress(&root, &examiner, &mut |b| {
            let _ = on_progress.send(b);
        })
        .map_err(err)?;
        let i = info(&p, Some(p.integrity_on_open().clone()))?;
        crate::scene_cmds::refresh(&handle, &p)?;
        *s.project.lock().unwrap() = Some(p);
        Ok(i)
    })
    .await
}

#[tauri::command]
pub async fn import_preview(
    app: AppHandle,
    path: PathBuf,
    on_progress: Channel<Progress>,
) -> CmdResult<Preview> {
    blocking(app, move |s| {
        let pv = locus_io::preview(&path, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(err)?;
        *s.preview.lock().unwrap() = Some(pv.clone());
        Ok(pv)
    })
    .await
}

#[derive(Serialize)]
pub struct CommitResult {
    project: ProjectInfo,
    evidence_id: i64,
    /// Set when the evidence was imported but a derived step (e.g. panorama extraction) failed.
    warning: Option<String>,
}

#[tauri::command]
pub async fn import_commit(
    app: AppHandle,
    sha256: String,
    unit: Option<LinearUnit>,
    on_progress: Channel<Progress>,
) -> CmdResult<CommitResult> {
    let handle = app.clone();
    blocking(app, move |s| {
        let pv = s
            .preview
            .lock()
            .unwrap()
            .clone()
            .filter(|p| p.sha256 == sha256)
            .ok_or("This file was not previewed; preview it again before importing.")?;
        let mut guard = s.project.lock().unwrap();
        let project = guard.as_mut().ok_or("Open or create a project first.")?;
        let rec = locus_io::commit(project, &pv, unit, &mut |p| {
            let _ = on_progress.send(p);
        })
        .map_err(err)?;
        *s.preview.lock().unwrap() = None;
        handle
            .state::<crate::scene_cmds::Builder>()
            .queue_missing(project, std::slice::from_ref(&rec))?;
        let warning = if rec.contents.format == "E57" && !rec.contents.images.is_empty() {
            extract_panoramas(project.root(), &rec).err()
        } else {
            None
        };
        Ok(CommitResult {
            project: info(project, None)?,
            evidence_id: rec.id,
            warning,
        })
    })
    .await
}

/// Derived copies of embedded E57 images, regenerable from the evidence at any time.
fn extract_panoramas(root: &Path, rec: &EvidenceRecord) -> Result<(), String> {
    let out = root
        .join("derived")
        .join("e57-images")
        .join(rec.id.to_string());
    locus_io::extract_e57_images(&root.join(&rec.stored_path), &out)
        .map(|_| ())
        .map_err(|e| format!("Imported, but its embedded images could not be extracted: {e}"))
}

#[tauri::command]
pub async fn evidence_verify(app: AppHandle, on_progress: Channel<u64>) -> CmdResult<ProjectInfo> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let project = guard.as_mut().ok_or("Open or create a project first.")?;
        let report = project
            .verify_evidence(&mut |b| {
                let _ = on_progress.send(b);
            })
            .map_err(err)?;
        info(project, Some(report))
    })
    .await
}
