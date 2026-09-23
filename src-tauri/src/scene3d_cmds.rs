//! 3-D scene commands: scenes stored as revisions like diagrams, the diagram revision an
//! extrusion was built from, surface snapping on the point cloud, and the sun's position.

use crate::commands::{blocking, err, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_analysis::surface::{self, Surface};
use locus_core::{HistoryEntry, Revision};
use serde_json::Value;
use tauri::AppHandle;

#[tauri::command]
pub async fn scenes(app: AppHandle) -> CmdResult<Vec<Revision>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open or create a project first.")?
            .scenes()
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn scene_create(app: AppHandle, name: String, document: Value) -> CmdResult<Revision> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.create_scene(&name, &document).map_err(err)
    })
    .await
}

/// Save a new revision (nothing is written if the document and name are unchanged).
#[tauri::command]
pub async fn scene_save(
    app: AppHandle,
    scene_id: i64,
    name: String,
    document: Value,
) -> CmdResult<Revision> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.save_scene(scene_id, &name, &document).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn scene_history(app: AppHandle, scene_id: i64) -> CmdResult<Vec<HistoryEntry>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open or create a project first.")?
            .scene_history(scene_id)
            .map_err(err)
    })
    .await
}

/// A specific diagram revision: an extrusion is built from the revision it names, not
/// whatever the diagram looks like now.
#[tauri::command]
pub async fn diagram_revision(app: AppHandle, revision_id: i64) -> CmdResult<Revision> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open or create a project first.")?
            .diagram_revision(revision_id)
            .map_err(err)?
            .ok_or(format!("No diagram revision {revision_id}."))
    })
    .await
}

/// The surface at a picked point, for snapping a model to the cloud: a plane fitted to the
/// visible points within `radius` (m) of the pick, which is re-resolved here from stored
/// data. The normal faces `toward` (the camera's project position).
#[tauri::command]
pub async fn surface_at(
    app: AppHandle,
    pick: Pick,
    radius: f64,
    toward: [f64; 3],
) -> CmdResult<Surface> {
    if !(radius > 0.0 && radius <= 1.0) {
        return Err("Choose a radius between 0 and 1 m.".into());
    }
    blocking(app, move |s| {
        let scene = s.scene.read().unwrap();
        let p = resolve(&scene, &pick)?.project;
        let near = scene.points_within(p, radius).map_err(err)?;
        surface::surface_at(p, &near, toward)
            .map_err(|e| format!("No surface to snap to within {radius} m of that point: {e}."))
    })
    .await
}

/// The sun's azimuth and elevation at a place (degrees) and time (UTC seconds).
#[tauri::command]
pub fn sun_position(lat: f64, lon: f64, unix: f64) -> CmdResult<locus_analysis::sun::SunPosition> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || !unix.is_finite() {
        return Err("Latitude must be −90° to 90° and longitude −180° to 180°.".into());
    }
    Ok(locus_analysis::sun::sun_position(lat, lon, unix))
}

/// Evaluate an animation: every mover's state at each `step` (s), the plausibility flags, the
/// assumed segments, warnings and limitations (locus-analysis animation). Nothing is stored;
/// the animation is saved with its scene.
#[tauri::command]
pub async fn animation_evaluate(
    app: AppHandle,
    animation: locus_analysis::animation::Animation,
    step: f64,
) -> CmdResult<locus_analysis::animation::Evaluation> {
    blocking(app, move |_| {
        locus_analysis::animation::evaluate(&animation, step).map_err(|e| {
            let e = e.to_string();
            e[..1].to_uppercase() + &e[1..] + "."
        })
    })
    .await
}
