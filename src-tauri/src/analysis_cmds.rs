//! Analysis tool commands. Every pick is resolved again here from stored data (as for
//! measurements); the client's idea of where a point is never enters an analysis. A preview
//! computes without storing; a run stores an immutable, audit-logged record.

use crate::commands::{blocking, err, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_analysis::defect::fit_defect;
use locus_analysis::surface;
use locus_analysis::trajectory::{
    self, FittedPlane, InputPoint, Parameters, PhotoRef, PointSource, Run,
};
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
    /// 1σ of this point (m), when the picked point itself is used.
    pub sigma: f64,
    /// For defects: "fitted" (the hole's centre from its rim; the default) or "manual" (the
    /// picked point, with `override_reason`).
    #[serde(default = "fitted")]
    pub centre: String,
    #[serde(default)]
    pub override_reason: Option<String>,
    /// A photograph of this defect (an image in the evidence).
    #[serde(default)]
    pub photo: Option<i64>,
}

fn fitted() -> String {
    "fitted".into()
}

fn hole_radius() -> f64 {
    0.03
}

#[derive(Deserialize)]
pub struct TrajectoryRequest {
    pub points: Vec<TrajectoryPick>,
    pub parameters: Parameters,
    /// Radius of the plane fitted around each defect, for angles to its surface (m).
    pub plane_radius: f64,
    /// Radius around a click searched for the hole and its rim (m).
    #[serde(default = "hole_radius")]
    pub hole_radius: f64,
}

/// An image in the evidence, as an analysis records it.
pub(crate) fn photo_ref(
    evidence: &[locus_core::EvidenceRecord],
    id: i64,
    label: &str,
) -> CmdResult<PhotoRef> {
    let e = evidence
        .iter()
        .find(|e| e.id == id && !e.contents.images.is_empty())
        .ok_or(format!("{label}: evidence item {id} is not an image."))?;
    Ok(PhotoRef {
        evidence_id: id,
        name: e.contents.images[0].name.clone(),
        file: e.stored_path.clone(),
        sha256: e.sha256.clone(),
    })
}

fn trajectory_run(scene: &Scene, project: &Project, req: &TrajectoryRequest) -> CmdResult<Run> {
    if req.points.len() < 2 {
        return Err(
            "Pick at least two points: entry and exit defects, or both ends of a rod.".into(),
        );
    }
    let evidence = project.evidence().map_err(err)?;
    let inputs = req
        .points
        .iter()
        .enumerate()
        .map(|(n, p)| {
            let label = format!("Point {} ({} on {})", n + 1, p.kind, p.surface);
            let r = resolve(scene, &p.pick)?;
            let at = r.project;
            let source = Some(PointSource {
                scan: p.pick.scan.clone(),
                index: r.index,
                revision: p.pick.revision,
            });
            let photo = p
                .photo
                .map(|id| photo_ref(&evidence, id, &label))
                .transpose()?;
            if p.kind == "rod" {
                if !(p.sigma.is_finite() && p.sigma > 0.0) {
                    return Err(format!("{label}: give a positive uncertainty."));
                }
                return Ok(InputPoint {
                    kind: p.kind.clone(),
                    surface: p.surface.clone(),
                    point: at,
                    sigma: p.sigma,
                    centre: "rod".into(),
                    source,
                    photo,
                    ..Default::default()
                });
            }
            // The surface's plane (for angles to it).
            let near = scene.points_within(at, req.plane_radius).map_err(err)?;
            let plane = surface::surface_at(at, &near, at)
                .ok()
                .map(|s| FittedPlane {
                    point: s.point,
                    normal: s.normal,
                    rms: s.rms,
                    points: s.points,
                });
            let base = InputPoint {
                kind: p.kind.clone(),
                surface: p.surface.clone(),
                plane,
                picked: Some(at),
                source,
                photo,
                ..Default::default()
            };
            if p.centre == "manual" {
                let reason = p.override_reason.as_deref().unwrap_or("").trim();
                if reason.is_empty() {
                    return Err(format!(
                        "{label}: a manual centre needs a reason (it is stored with the analysis)."
                    ));
                }
                if !(p.sigma.is_finite() && p.sigma > 0.0) {
                    return Err(format!("{label}: give a positive uncertainty."));
                }
                return Ok(InputPoint {
                    point: at,
                    sigma: p.sigma,
                    centre: "manual".into(),
                    override_reason: Some(reason.into()),
                    ..base
                });
            }
            let around = scene.points_within(at, req.hole_radius).map_err(err)?;
            let f = fit_defect(at, &around).map_err(|e| {
                format!(
                    "{label}: no hole centre ({e}). Click nearer the hole, or use the picked point as a manual centre with a reason."
                )
            })?;
            Ok(InputPoint {
                point: f.centre,
                sigma: f.centre_sigma,
                centre: "fitted".into(),
                defect: Some(f),
                ..base
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
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        trajectory_run(&s.scene.read().unwrap(), p, &request)
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
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let run = trajectory_run(&s.scene.read().unwrap(), p, &request)?;
        let record = serde_json::to_value(&run).map_err(err)?;
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

pub(crate) fn meta(p: &Project, a: &AnalysisRecord) -> CmdResult<locus_report::analysis::Meta> {
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
        case_number: p.setting("case_number").map_err(err)?,
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
                // Defect photos, each checked against the hash recorded in the run.
                let images = locus_report::trajectory::photo_files(&run)
                    .into_iter()
                    .map(|(file, sha)| {
                        Ok((
                            file.clone(),
                            crate::diagram_cmds::read_checked(p.root(), &file, &sha)?,
                        ))
                    })
                    .collect::<CmdResult<Vec<_>>>()?;
                (
                    locus_report::trajectory::report(&meta(p, &a)?, &run),
                    images,
                )
            }
            "bloodstain" => {
                let run: locus_analysis::bloodstain::Run =
                    serde_json::from_value(a.record.clone()).map_err(err)?;
                (
                    locus_report::bloodstain::report(&meta(p, &a)?, &run),
                    vec![],
                )
            }
            "camera" => {
                let run: locus_analysis::camera::Run =
                    serde_json::from_value(a.record.clone()).map_err(err)?;
                // The photo, checked against the hash recorded in the run.
                let images = locus_report::camera::photo_files(&run)
                    .into_iter()
                    .map(|(file, sha)| {
                        Ok((
                            file.clone(),
                            crate::diagram_cmds::read_checked(p.root(), &file, &sha)?,
                        ))
                    })
                    .collect::<CmdResult<Vec<_>>>()?;
                (locus_report::camera::report(&meta(p, &a)?, &run), images)
            }
            "witness" => {
                let run: locus_analysis::camera::WitnessRun =
                    serde_json::from_value(a.record.clone()).map_err(err)?;
                (
                    locus_report::camera::witness_report(&meta(p, &a)?, &run),
                    vec![],
                )
            }
            "skid" | "yaw" | "momentum" | "crush" => {
                let m = meta(p, &a)?;
                let r = &a.record;
                let doc = match a.tool.as_str() {
                    "skid" => locus_report::crash::skid(
                        &m,
                        &serde_json::from_value(r.clone()).map_err(err)?,
                    ),
                    "yaw" => locus_report::crash::yaw(
                        &m,
                        &serde_json::from_value(r.clone()).map_err(err)?,
                    ),
                    "momentum" => locus_report::crash::momentum(
                        &m,
                        &serde_json::from_value(r.clone()).map_err(err)?,
                    ),
                    _ => locus_report::crash::crush(
                        &m,
                        &serde_json::from_value(r.clone()).map_err(err)?,
                    ),
                };
                (doc, vec![])
            }
            "edr" => (
                locus_report::crash::edr(
                    &meta(p, &a)?,
                    &serde_json::from_value(a.record.clone()).map_err(err)?,
                ),
                vec![],
            ),
            "photogrammetry" => (
                locus_report::photo::report(
                    &meta(p, &a)?,
                    &serde_json::from_value(a.record.clone()).map_err(err)?,
                ),
                vec![],
            ),
            "crush_volume" => (
                locus_report::crash::volume(
                    &meta(p, &a)?,
                    &serde_json::from_value(a.record.clone()).map_err(err)?,
                ),
                vec![],
            ),
            "animation" => {
                let run: locus_analysis::tds::TdsRun =
                    serde_json::from_value(a.record.clone()).map_err(err)?;
                // The scene's renders, listed with it.
                let renders = p
                    .analyses()
                    .map_err(err)?
                    .into_iter()
                    .filter(|x| x.tool == "render" && x.withdrawn.is_none())
                    .filter_map(|x| {
                        serde_json::from_value::<locus_analysis::animation::RenderRecord>(x.record)
                            .ok()
                            .filter(|r| r.scene_id == run.scene_id)
                            .map(|r| (x.id, r))
                    })
                    .collect::<Vec<_>>();
                (
                    locus_report::animation::report(&meta(p, &a)?, &run, &renders),
                    vec![],
                )
            }
            t => return Err(format!("No report for {t} analyses yet.")),
        };
        let (report, images) = report;
        let out = locus_report::analysis::pdf(&report, images)?;
        std::fs::write(&path, &out.pdf).map_err(|e| format!("Could not write {path}: {e}"))?;
        let (sha256, bytes) =
            locus_core::hash::sha256_reader(out.pdf.as_slice(), &mut |_| {}).map_err(err)?;
        p.record_analysis_report(id, &path, &sha256, bytes)
            .map_err(err)?;
        Ok(sha256)
    })
    .await
}

/// The project's case number (printed on every report), if set.
#[tauri::command]
pub async fn case_number(app: AppHandle) -> CmdResult<Option<String>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open or create a project first.")?
            .setting("case_number")
            .map_err(err)
    })
    .await
}

/// Set the project's case number (audit-logged as a setting change).
#[tauri::command]
pub async fn case_number_set(app: AppHandle, value: String) -> CmdResult<()> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        guard
            .as_mut()
            .ok_or("Open or create a project first.")?
            .set_setting("case_number", value.trim())
            .map_err(err)
    })
    .await
}
