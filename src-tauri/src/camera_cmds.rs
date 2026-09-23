//! Camera matching, subject height and witness perspective commands. Every scan point is
//! resolved again here from stored data, and the photo is named by its evidence record, so
//! the stored run says exactly what it was built from.

use crate::analysis_cmds::photo_ref;
use crate::commands::{blocking, err, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_analysis::camera::{self, HeightInput, PairInput, Parameters, Run, Sight, WitnessRun};
use locus_analysis::trajectory::PointSource;
use locus_core::{AnalysisRecord, Project};
use locus_octree::scene::Scene;
use serde::Deserialize;
use tauri::AppHandle;

fn capital(e: &str) -> String {
    e[..1].to_uppercase() + &e[1..] + "."
}

fn source(pick: &Pick, index: u32) -> PointSource {
    PointSource {
        scan: pick.scan.clone(),
        index,
        revision: pick.revision,
    }
}

/// A pixel in the photo and the scan point clicked for it.
#[derive(Deserialize)]
pub struct PairPick {
    pub px: [f64; 2],
    pub pick: Pick,
}

#[derive(Deserialize)]
pub struct CameraRequest {
    /// The photo or frame: an image in the evidence, and its size in pixels.
    pub photo: i64,
    pub size: [u32; 2],
    pub pairs: Vec<PairPick>,
    /// The scan point σ is the project's, whatever is sent.
    pub parameters: Parameters,
    #[serde(default)]
    pub subjects: Vec<HeightInput>,
}

fn camera_run(scene: &Scene, project: &Project, req: &CameraRequest) -> CmdResult<Run> {
    let evidence = project.evidence().map_err(err)?;
    let photo = photo_ref(&evidence, req.photo, "The camera's photo")?;
    if req.size[0] == 0 || req.size[1] == 0 {
        return Err("The photo's size is missing.".into());
    }
    let mut pairs = vec![];
    for p in &req.pairs {
        let r = resolve(scene, &p.pick)?;
        pairs.push(PairInput {
            px: p.px,
            world: r.project,
            source: Some(source(&p.pick, r.index)),
        });
    }
    let mut params = req.parameters.clone();
    params.point_sigma = project.point_sigma().map_err(err)?;
    if !(params.pick_sigma_px > 0.0 && params.pick_sigma_px <= 20.0) {
        return Err("Give a pick uncertainty between 0 and 20 pixels.".into());
    }
    if !(100..=20_000).contains(&params.draws) {
        return Err("Use between 100 and 20,000 Monte Carlo draws.".into());
    }
    for s in &req.subjects {
        if s.label.trim().is_empty() {
            return Err("Give each subject a label.".into());
        }
    }
    camera::run(Some(photo), pairs, req.size, params, &req.subjects)
        .map_err(|e| capital(&e.to_string()))
}

/// Solve the camera and the subjects' heights without storing anything.
#[tauri::command]
pub async fn camera_preview(app: AppHandle, request: CameraRequest) -> CmdResult<Run> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        camera_run(&s.scene.read().unwrap(), p, &request)
    })
    .await
}

/// Solve and store the camera match as an analysis record (audit-logged).
#[tauri::command]
pub async fn camera_save(
    app: AppHandle,
    name: String,
    request: CameraRequest,
    revises: Option<i64>,
) -> CmdResult<AnalysisRecord> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let run = camera_run(&s.scene.read().unwrap(), p, &request)?;
        let record = serde_json::to_value(&run).map_err(err)?;
        p.add_analysis("camera", camera::METHOD, &name, &record, revises)
            .map_err(err)
    })
    .await
}

#[derive(Deserialize)]
pub struct Target {
    pub label: String,
    pub pick: Pick,
}

fn radius() -> f64 {
    0.03
}
fn clearance() -> f64 {
    0.1
}

#[derive(Deserialize)]
pub struct WitnessRequest {
    /// The floor point the witness stands on, and the eye's height above it (m).
    pub floor: Pick,
    pub eye_height: f64,
    /// Where the view looks, and its horizontal field of view (degrees).
    pub look_at: [f64; 3],
    pub fov_deg: f64,
    pub targets: Vec<Target>,
    /// Scan points within this distance of a line of sight block it (m), except within
    /// `end_clearance` of its ends.
    #[serde(default = "radius")]
    pub radius: f64,
    #[serde(default = "clearance")]
    pub end_clearance: f64,
}

/// The scan points within `radius` of the segment from `a` to `b`: balls along it, each
/// point once.
fn points_near_segment(
    scene: &Scene,
    a: [f64; 3],
    b: [f64; 3],
    radius: f64,
) -> CmdResult<Vec<[f64; 3]>> {
    let d: [f64; 3] = std::array::from_fn(|k| b[k] - a[k]);
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let step = 0.1f64.max(2.0 * radius);
    let n = (len / step).ceil().max(1.0) as usize;
    let ball = (radius * radius + (step / 2.0).powi(2)).sqrt();
    let mut pts = vec![];
    for i in 0..=n {
        let t = (i as f64 * step).min(len) / len.max(1e-12);
        let c = std::array::from_fn(|k| a[k] + t * d[k]);
        pts.extend(scene.points_within(c, ball).map_err(err)?);
    }
    pts.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
    pts.dedup();
    Ok(pts)
}

fn witness_run(scene: &Scene, req: &WitnessRequest) -> CmdResult<WitnessRun> {
    if !(req.eye_height > 0.2 && req.eye_height < 3.0) {
        return Err("Give an eye height between 0.2 and 3 m.".into());
    }
    if !(req.fov_deg > 5.0 && req.fov_deg < 150.0) {
        return Err("Give a field of view between 5° and 150°.".into());
    }
    if !(req.radius > 0.0 && req.radius <= 0.5 && req.end_clearance >= 0.0) {
        return Err("Give a sight-line radius up to 0.5 m.".into());
    }
    let floor = resolve(scene, &req.floor)?;
    let eye = [
        floor.project[0],
        floor.project[1],
        floor.project[2] + req.eye_height,
    ];
    let mut sights: Vec<Sight> = vec![];
    for t in &req.targets {
        let r = resolve(scene, &t.pick)?;
        let pts = points_near_segment(scene, eye, r.project, req.radius)?;
        let mut s = camera::line_of_sight(
            &t.label,
            eye,
            r.project,
            &pts,
            req.radius,
            req.end_clearance,
        );
        s.target_source = Some(source(&t.pick, r.index));
        sights.push(s);
    }
    let mut w = camera::witness(
        floor.project,
        req.eye_height,
        req.look_at,
        req.fov_deg,
        sights,
    );
    w.floor_source = Some(source(&req.floor, floor.index));
    Ok(w)
}

/// A witness's eye position and lines of sight, without storing anything.
#[tauri::command]
pub async fn witness_preview(app: AppHandle, request: WitnessRequest) -> CmdResult<WitnessRun> {
    blocking(app, move |s| {
        witness_run(&s.scene.read().unwrap(), &request)
    })
    .await
}

/// Store a witness perspective as an analysis record (audit-logged).
#[tauri::command]
pub async fn witness_save(
    app: AppHandle,
    name: String,
    request: WitnessRequest,
    revises: Option<i64>,
) -> CmdResult<AnalysisRecord> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let run = witness_run(&s.scene.read().unwrap(), &request)?;
        let record = serde_json::to_value(&run).map_err(err)?;
        p.add_analysis("witness", camera::WITNESS_METHOD, &name, &record, revises)
            .map_err(err)
    })
    .await
}
