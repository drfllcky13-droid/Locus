//! Diagram commands: create, save revisions, list, history, and solving hand measurements.

use crate::commands::{blocking, err, CmdResult};
use locus_analysis::handmeasure::{self, HandError, Side, Solved, Tape, P2};
use locus_core::{DiagramRevision, HistoryEntry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path};
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
        // Underlay images, each checked against the hash the diagram recorded.
        let files = d
            .underlays()
            .into_iter()
            .map(|(file, sha)| Ok((file.to_string(), read_checked(p.root(), file, sha)?)))
            .collect::<CmdResult<Vec<_>>>()?;
        let out = locus_report::diagram::pdf(&d, &locus_report::diagram::symbols(), &o, files)?;
        std::fs::write(&path, &out.pdf).map_err(|e| format!("Could not write {path}: {e}"))?;
        let (sha256, bytes) =
            locus_core::hash::sha256_reader(out.pdf.as_slice(), &mut |_| {}).map_err(err)?;
        p.record_diagram_export(rev.revision_id, scale, &path, &sha256, bytes)
            .map_err(err)?;
        Ok(sha256)
    })
    .await
}

// ---------- underlays ----------

/// Read a project file named by a diagram (relative, inside the project) and check its hash.
fn read_checked(root: &Path, file: &str, sha256: &str) -> CmdResult<Vec<u8>> {
    let rel = Path::new(file);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("Not a project file: {file}"));
    }
    let bytes = std::fs::read(root.join(rel)).map_err(|e| format!("Could not read {file}: {e}"))?;
    let (sha, _) = locus_core::hash::sha256_reader(bytes.as_slice(), &mut |_| {}).map_err(err)?;
    if sha != sha256 {
        return Err(format!(
            "{file} has changed since it was placed (SHA-256 {sha}, expected {sha256})."
        ));
    }
    Ok(bytes)
}

#[derive(Serialize)]
pub struct UnderlayImage {
    evidence_id: i64,
    name: String,
    file: String,
    sha256: String,
    width: u32,
    height: u32,
}

/// Photos and aerial images in the evidence store that can go under a diagram.
#[tauri::command]
pub async fn underlay_images(app: AppHandle) -> CmdResult<Vec<UnderlayImage>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        Ok(p.evidence()
            .map_err(err)?
            .into_iter()
            .filter_map(|e| {
                let i = e.contents.images.first()?;
                Some(UnderlayImage {
                    evidence_id: e.id,
                    name: i.name.clone(),
                    file: e.stored_path.clone(),
                    sha256: e.sha256.clone(),
                    width: i.width,
                    height: i.height,
                })
            })
            .collect())
    })
    .await
}

/// An underlay's image bytes (PNG or JPEG), after checking its hash.
#[tauri::command]
pub async fn underlay_bytes(
    app: AppHandle,
    file: String,
    sha256: String,
) -> CmdResult<tauri::ipc::Response> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        Ok(tauri::ipc::Response::new(read_checked(
            p.root(),
            &file,
            &sha256,
        )?))
    })
    .await
}

#[derive(Serialize)]
pub struct SliceView {
    file: String,
    sha256: String,
    origin: [f64; 2],
    resolution: f64,
    width: u32,
    height: u32,
    points: u64,
}

/// Rasterise the scene between two heights (top-down, `resolution` m per pixel) into a PNG
/// in the project, and log it.
#[tauri::command]
pub async fn underlay_slice(
    app: AppHandle,
    z_min: f64,
    z_max: f64,
    resolution: f64,
) -> CmdResult<SliceView> {
    blocking(app, move |s| {
        let slice = {
            let scene = s.scene.read().unwrap();
            locus_octree::slice::slice(&scene, (z_min, z_max), resolution)
                .map_err(err)?
                .map_err(|e| format!("Cannot slice: {e}."))?
        };
        let mut png_bytes = vec![];
        {
            let mut enc = png::Encoder::new(&mut png_bytes, slice.width, slice.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(err)?;
            w.write_image_data(&slice.rgba).map_err(err)?;
        }
        let (sha256, bytes) =
            locus_core::hash::sha256_reader(png_bytes.as_slice(), &mut |_| {}).map_err(err)?;
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let file = format!("derived/slices/{}.png", &sha256[..16]);
        let path = p.root().join(&file);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
        std::fs::write(&path, &png_bytes).map_err(err)?;
        let registration = p.applied_registration().map_err(err)?;
        p.record_underlay(
            "diagram.underlay_sliced",
            json!({
                "file": file,
                "sha256": sha256,
                "bytes": bytes,
                "z_min": z_min,
                "z_max": z_max,
                "resolution": resolution,
                "origin": slice.origin,
                "width": slice.width,
                "height": slice.height,
                "points": slice.points,
                "registration": registration,
            }),
        )
        .map_err(err)?;
        Ok(SliceView {
            file,
            sha256,
            origin: slice.origin,
            resolution,
            width: slice.width,
            height: slice.height,
            points: slice.points,
        })
    })
    .await
}

/// Log an image underlay's calibration: its known points, the fit and the residuals.
#[tauri::command]
pub async fn underlay_calibrated(app: AppHandle, details: Value) -> CmdResult<()> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.record_underlay("diagram.underlay_calibrated", details)
            .map_err(err)
    })
    .await
}
