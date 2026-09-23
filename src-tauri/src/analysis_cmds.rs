//! Analysis tool commands. Every pick is resolved again here from stored data (as for
//! measurements); the client's idea of where a point is never enters an analysis. A preview
//! computes without storing; a run stores an immutable, audit-logged record.

use crate::commands::{blocking, err, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_analysis::surface;
use locus_analysis::trajectory::{self, FittedPlane, InputPoint, Parameters, Run};
use locus_core::{AnalysisRecord, Project};
use locus_octree::scene::Scene;
use serde::Deserialize;
use tauri::AppHandle;

/// A trajectory point as the examiner picked it.
#[derive(Deserialize)]
pub struct TrajectoryPick {
    pub pick: Pick,
    /// "entry", "exit" or "rod".
    pub kind: String,
    pub surface: String,
    /// 1σ of this point (m).
    pub sigma: f64,
}

#[derive(Deserialize)]
pub struct TrajectoryRequest {
    pub points: Vec<TrajectoryPick>,
    pub parameters: Parameters,
    /// Radius of the plane fitted around each defect, for angles to its surface (m).
    pub plane_radius: f64,
}

fn trajectory_run(scene: &Scene, req: &TrajectoryRequest) -> CmdResult<Run> {
    if req.points.len() < 2 {
        return Err(
            "Pick at least two points: entry and exit defects, or both ends of a rod.".into(),
        );
    }
    let inputs = req
        .points
        .iter()
        .map(|p| {
            if !(p.sigma.is_finite() && p.sigma > 0.0) {
                return Err("Every point needs a positive uncertainty.".to_string());
            }
            let at = resolve(scene, &p.pick)?.project;
            // Defects get the plane of their surface; rod ends don't lie on one.
            let plane = if p.kind == "rod" {
                None
            } else {
                let near = scene.points_within(at, req.plane_radius).map_err(err)?;
                surface::surface_at(at, &near, at)
                    .ok()
                    .map(|s| FittedPlane {
                        point: s.point,
                        normal: s.normal,
                        rms: s.rms,
                        points: s.points,
                    })
            };
            Ok(InputPoint {
                kind: p.kind.clone(),
                surface: p.surface.clone(),
                point: at,
                sigma: p.sigma,
                plane,
            })
        })
        .collect::<CmdResult<Vec<_>>>()?;
    let p = &req.parameters;
    if !(p.cone_deg >= 0.0 && p.cone_deg < 60.0 && p.band[0] < p.band[1] && p.max_range > 0.0) {
        return Err(
            "Check the cone (0–60°), the height band (low below high) and the range.".into(),
        );
    }
    trajectory::run(inputs, req.parameters.clone()).map_err(|e| {
        let e = e.to_string();
        e[..1].to_uppercase() + &e[1..] + "."
    })
}

/// Compute a trajectory without storing it.
#[tauri::command]
pub async fn trajectory_preview(app: AppHandle, request: TrajectoryRequest) -> CmdResult<Run> {
    blocking(app, move |s| {
        trajectory_run(&s.scene.read().unwrap(), &request)
    })
    .await
}

/// Compute a trajectory and store it as an analysis record (audit-logged).
#[tauri::command]
pub async fn trajectory_save(
    app: AppHandle,
    name: String,
    request: TrajectoryRequest,
    revises: Option<i64>,
) -> CmdResult<AnalysisRecord> {
    blocking(app, move |s| {
        let run = trajectory_run(&s.scene.read().unwrap(), &request)?;
        let record = serde_json::to_value(&run).map_err(err)?;
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.add_analysis("trajectory", trajectory::METHOD, &name, &record, revises)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn analyses(app: AppHandle) -> CmdResult<Vec<AnalysisRecord>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open or create a project first.")?
            .analyses()
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn analysis_withdraw(
    app: AppHandle,
    id: i64,
    reason: String,
) -> CmdResult<Vec<AnalysisRecord>> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.withdraw_analysis(id, &reason).map_err(err)?;
        p.analyses().map_err(err)
    })
    .await
}

fn meta(p: &Project, a: &AnalysisRecord) -> CmdResult<locus_report::analysis::Meta> {
    Ok(locus_report::analysis::Meta {
        project: p.name().map_err(err)?,
        record_id: a.id,
        name: a.name.clone(),
        method: a.method.clone(),
        sha256: a.sha256.clone(),
        revises: a.revises,
        created_at: a.created_at.clone(),
        created_by: a.created_by.clone(),
        withdrawn: a
            .withdrawn
            .as_ref()
            .map(|w| format!("{} by {}: {}", w.at, w.by, w.reason)),
        printed_by: p.examiner().to_string(),
        printed_at: locus_core::timestamp(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        audit_head: p
            .audit_log()
            .map_err(err)?
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_default(),
    })
}

/// Write an analysis's PDF report (built from the stored record) and log it.
#[tauri::command]
pub async fn analysis_report(app: AppHandle, id: i64, path: String) -> CmdResult<String> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let a = p
            .analysis(id)
            .map_err(err)?
            .ok_or(format!("No analysis {id}."))?;
        let report = match a.tool.as_str() {
            "trajectory" => {
                let run: Run = serde_json::from_value(a.record.clone()).map_err(err)?;
                locus_report::trajectory::report(&meta(p, &a)?, &run)
            }
            t => return Err(format!("No report for {t} analyses yet.")),
        };
        let out = locus_report::analysis::pdf(&report)?;
        std::fs::write(&path, &out.pdf).map_err(|e| format!("Could not write {path}: {e}"))?;
        let (sha256, bytes) =
            locus_core::hash::sha256_reader(out.pdf.as_slice(), &mut |_| {}).map_err(err)?;
        p.record_analysis_report(id, &path, &sha256, bytes)
            .map_err(err)?;
        Ok(sha256)
    })
    .await
}
