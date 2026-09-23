//! Diagram commands: create, save revisions, list, history, and solving hand measurements.

use crate::commands::{blocking, err, CmdResult};
use locus_analysis::handmeasure::{self, HandError, Side, Solved, Tape, P2};
use locus_core::{DiagramRevision, HistoryEntry};
use serde::Deserialize;
use serde_json::Value;
use tauri::AppHandle;

#[tauri::command]
pub async fn diagrams(app: AppHandle) -> CmdResult<Vec<DiagramRevision>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open or create a project first.")?
            .diagrams()
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn diagram_create(
    app: AppHandle,
    name: String,
    document: Value,
) -> CmdResult<DiagramRevision> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.create_diagram(&name, &document).map_err(err)
    })
    .await
}

/// Save a new revision (nothing is written if the document and name are unchanged).
#[tauri::command]
pub async fn diagram_save(
    app: AppHandle,
    diagram_id: i64,
    name: String,
    document: Value,
) -> CmdResult<DiagramRevision> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.save_diagram(diagram_id, &name, &document).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn diagram_history(app: AppHandle, diagram_id: i64) -> CmdResult<Vec<HistoryEntry>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open or create a project first.")?
            .diagram_history(diagram_id)
            .map_err(err)
    })
    .await
}

/// A hand measurement to solve, with the reference positions already looked up.
#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum HandRequest {
    BaselineOffset {
        from: P2,
        to: P2,
        along: f64,
        offset: f64,
        side: Side,
    },
    Triangulation {
        refs: Vec<(P2, f64)>,
        side: Option<Side>,
    },
}

/// Solve a hand measurement. `known_sigma` is the reference points' 1σ per axis (m); the
/// tape's 1σ is `tape_fixed + tape_per_metre × distance`.
#[tauri::command]
pub fn hand_solve(
    request: HandRequest,
    known_sigma: f64,
    tape_fixed: f64,
    tape_per_metre: f64,
) -> CmdResult<Solved> {
    let tape = Tape {
        fixed: tape_fixed,
        per_metre: tape_per_metre,
    };
    match request {
        HandRequest::BaselineOffset {
            from,
            to,
            along,
            offset,
            side,
        } => handmeasure::baseline_offset(from, to, along, offset, side, known_sigma, tape),
        HandRequest::Triangulation { refs, side } => {
            handmeasure::triangulate(&refs, side, known_sigma, tape)
        }
    }
    .map_err(|e: HandError| e.to_string())
}

/// Print the diagram's newest saved revision to PDF at 1:`scale`, and log the export.
/// Returns the PDF's SHA-256.
#[tauri::command]
pub async fn diagram_pdf(
    app: AppHandle,
    diagram_id: i64,
    scale: f64,
    paper: locus_report::diagram::Paper,
    landscape: bool,
    path: String,
) -> CmdResult<String> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let rev = p
            .diagram_latest(diagram_id)
            .map_err(err)?
            .ok_or(format!("No diagram {diagram_id}."))?;
        let d: locus_report::diagram::Diagram =
            serde_json::from_value(rev.document.clone()).map_err(err)?;
        let o = locus_report::diagram::PrintOptions {
            scale,
            paper,
            landscape,
            title: rev.name.clone(),
            details: vec![
                ("Project".into(), p.name().map_err(err)?),
                (
                    "Revision".into(),
                    format!("{} (SHA-256 {})", rev.number, &rev.sha256[..16]),
                ),
                ("Drawn by".into(), rev.created_by.clone()),
                (
                    "Printed".into(),
                    format!("{}, {}", locus_core::timestamp(), p.examiner()),
                ),
            ],
        };
        let out = locus_report::diagram::pdf(&d, &locus_report::diagram::symbols(), &o)?;
        std::fs::write(&path, &out.pdf).map_err(|e| format!("Could not write {path}: {e}"))?;
        let (sha256, bytes) =
            locus_core::hash::sha256_reader(out.pdf.as_slice(), &mut |_| {}).map_err(err)?;
        p.record_diagram_export(rev.revision_id, scale, &path, &sha256, bytes)
            .map_err(err)?;
        Ok(sha256)
    })
    .await
}
