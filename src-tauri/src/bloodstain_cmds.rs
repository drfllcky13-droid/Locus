//! Bloodstain area-of-origin commands. The alignment pairs' scan points are resolved again
//! here from stored data, the photo is named by its evidence record, and the ellipse is
//! fitted here from the edge points, so the stored run says exactly what it was built from.

use crate::analysis_cmds::photo_ref;
use crate::commands::{blocking, err, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_analysis::bloodstain::{
    self, align_photo_rectified, rectify, stain, stain_edges, stain_from_photo, AlignPair,
    Alignment, AutoEdge, Parameters, Run, StainInput, StainResult, CORNER_SIGMA_PX,
};
use locus_analysis::surface;
use locus_analysis::trajectory::PointSource;
use locus_core::{AnalysisRecord, EvidenceRecord, Project};
use locus_octree::scene::Scene;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// A pixel in the stain's photo and the scan point clicked for it.
#[derive(Deserialize)]
pub struct PairPick {
    pub px: [f64; 2],
    pub pick: Pick,
}

/// How to place a photo on its surface.
#[derive(Deserialize)]
pub struct AlignRequest {
    pub pairs: Vec<PairPick>,
    /// Where the examiner is looking from: the surface's normal is taken on this side.
    pub eye: [f64; 3],
    /// Radius of the plane fitted to the scan around the pairs (m).
    #[serde(default = "plane_radius")]
    pub plane_radius: f64,
    /// A scale's four corners, for a photo not taken square on.
    #[serde(default)]
    pub scale: Option<ScaleCorners>,
}

/// Four corners of a rectangle on a scale lying on the surface, clicked in order around it
/// (the first two span its width), and its size.
#[derive(Deserialize)]
pub struct ScaleCorners {
    pub corners_px: [[f64; 2]; 4],
    /// Width and height (m).
    pub size: [f64; 2],
    /// 1σ of a corner click (px); `CORNER_SIGMA_PX` when not given.
    #[serde(default)]
    pub corner_sigma_px: Option<f64>,
}

fn plane_radius() -> f64 {
    0.1
}

#[derive(Deserialize)]
pub struct StainRequest {
    pub label: String,
    pub surface: String,
    /// The photo: an image in the evidence.
    pub photo: i64,
    pub align: AlignRequest,
    pub edges: Vec<[f64; 2]>,
    #[serde(default)]
    pub auto_edge: Option<AutoEdge>,
    pub tail_px: [f64; 2],
    #[serde(default)]
    pub excluded: Option<String>,
}

#[derive(Deserialize)]
pub struct BloodstainRequest {
    pub stains: Vec<StainRequest>,
    pub parameters: Parameters,
}

fn align(
    scene: &Scene,
    req: &AlignRequest,
    point_sigma: f64,
) -> CmdResult<(Alignment, Vec<PointSource>)> {
    if req.pairs.len() < 2 {
        return Err("Give at least two point pairs (three to check the fit).".into());
    }
    let mut pairs = vec![];
    let mut sources = vec![];
    for p in &req.pairs {
        let r = resolve(scene, &p.pick)?;
        pairs.push(AlignPair {
            px: p.px,
            world: r.project,
        });
        sources.push(PointSource {
            scan: p.pick.scan.clone(),
            index: r.index,
            revision: p.pick.revision,
        });
    }
    let k = pairs.len() as f64;
    let centre = [0, 1, 2].map(|i| pairs.iter().map(|p| p.world[i]).sum::<f64>() / k);
    let near = scene.points_within(centre, req.plane_radius).map_err(err)?;
    let plane = surface::surface_at(centre, &near, req.eye).map_err(|e| {
        format!("No surface under the scan points ({e}). Pick them on the stain's surface.")
    })?;
    let rect = req
        .scale
        .as_ref()
        .map(|c| {
            rectify(
                c.corners_px,
                c.size,
                c.corner_sigma_px.unwrap_or(CORNER_SIGMA_PX),
            )
        })
        .transpose()
        .map_err(|e| capital(&e.to_string()))?;
    let mut a = align_photo_rectified(&pairs, plane.point, plane.normal, point_sigma, rect)
        .map_err(|e| capital(&e.to_string()))?;
    a.plane_rms = plane.rms;
    Ok((a, sources))
}

fn capital(e: &str) -> String {
    e[..1].to_uppercase() + &e[1..] + "."
}

fn stain_input(
    scene: &Scene,
    evidence: &[EvidenceRecord],
    point_sigma: f64,
    s: &StainRequest,
) -> CmdResult<StainInput> {
    let label = format!("Stain {}", s.label);
    let photo = photo_ref(evidence, s.photo, &label)?;
    let (alignment, sources) =
        align(scene, &s.align, point_sigma).map_err(|e| format!("{label}: {e}"))?;
    let mut input = stain_from_photo(&s.label, &s.surface, alignment, s.edges.clone(), s.tail_px)
        .map_err(|e| format!("{label}: {}", capital(&e.to_string())))?;
    input.photo = Some(photo);
    input.sources = sources;
    input.auto_edge = s.auto_edge;
    input.excluded = s
        .excluded
        .as_ref()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    if s.excluded.is_some() && input.excluded.is_none() {
        return Err(format!("{label}: give the reason it is excluded."));
    }
    Ok(input)
}

fn bloodstain_run(scene: &Scene, project: &Project, req: &BloodstainRequest) -> CmdResult<Run> {
    let evidence = project.evidence().map_err(err)?;
    let sigma = project.point_sigma().map_err(err)?;
    let inputs = req
        .stains
        .iter()
        .map(|s| stain_input(scene, &evidence, sigma, s))
        .collect::<CmdResult<Vec<_>>>()?;
    let mut p = req.parameters.clone();
    if let Some(r) = &p.include_not_upward {
        if r.trim().is_empty() {
            return Err("Give the reason for including stains not clearly moving upward.".into());
        }
    }
    if !(10..=20_000).contains(&p.bootstrap) {
        return Err("Use between 10 and 20,000 bootstrap resamples.".into());
    }
    p.include_not_upward = p.include_not_upward.map(|r| r.trim().to_string());
    bloodstain::run(inputs, p).map_err(|e| capital(&e.to_string()))
}

/// Place a photo on its surface (for the overlay and the pairs' residuals).
#[tauri::command]
pub async fn bloodstain_align(app: AppHandle, request: AlignRequest) -> CmdResult<Alignment> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        let sigma = p.point_sigma().map_err(err)?;
        Ok(align(&s.scene.read().unwrap(), &request, sigma)?.0)
    })
    .await
}

/// The edge of the stain around `seed` in a greyscale crop of its photo (row-major; pixel
/// coordinates in the crop).
#[tauri::command]
pub async fn bloodstain_edges(
    luma: Vec<u8>,
    width: usize,
    height: usize,
    seed: [f64; 2],
    threshold: u8,
) -> CmdResult<Vec<[f64; 2]>> {
    stain_edges(&luma, width, height, seed, threshold).map_err(|e| capital(&e.to_string()))
}

#[derive(Serialize)]
pub struct StainPreview {
    input: StainInput,
    result: StainResult,
}

/// One stain's ellipse, impact angle and direction, without storing anything.
#[tauri::command]
pub async fn bloodstain_stain(
    app: AppHandle,
    request: StainRequest,
    parameters: Parameters,
) -> CmdResult<StainPreview> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        let input = stain_input(
            &s.scene.read().unwrap(),
            &p.evidence().map_err(err)?,
            p.point_sigma().map_err(err)?,
            &request,
        )?;
        let result = stain(&input, &parameters).map_err(|e| capital(&e.to_string()))?;
        Ok(StainPreview { input, result })
    })
    .await
}

/// Compute the area of origin without storing it.
#[tauri::command]
pub async fn bloodstain_preview(app: AppHandle, request: BloodstainRequest) -> CmdResult<Run> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        bloodstain_run(&s.scene.read().unwrap(), p, &request)
    })
    .await
}

/// Compute the area of origin and store it as an analysis record (audit-logged).
#[tauri::command]
pub async fn bloodstain_save(
    app: AppHandle,
    name: String,
    request: BloodstainRequest,
    revises: Option<i64>,
) -> CmdResult<AnalysisRecord> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let run = bloodstain_run(&s.scene.read().unwrap(), p, &request)?;
        let record = serde_json::to_value(&run).map_err(err)?;
        p.add_analysis("bloodstain", bloodstain::METHOD, &name, &record, revises)
            .map_err(err)
    })
    .await
}
