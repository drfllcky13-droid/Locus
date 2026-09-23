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
