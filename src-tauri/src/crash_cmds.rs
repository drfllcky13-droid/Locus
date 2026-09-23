//! Crash reconstruction commands: skid, yaw, momentum and crush energy. Marks measured on the
//! cloud are resolved again here from stored data, so the stored run says exactly what it was
//! built from.

use crate::commands::{blocking, err, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_analysis::crash::{
    self, Input, MomentumVehicle, SkidSegment, YawRadius, CRUSH_METHOD, DRAWS, MOMENTUM_METHOD,
    SKID_METHOD, YAW_METHOD,
};
use locus_analysis::measure::P3;
use locus_analysis::trajectory::PointSource;
use locus_core::AnalysisRecord;
use locus_octree::scene::Scene;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

fn capital(e: &str) -> String {
    e[..1].to_uppercase() + &e[1..] + "."
}

/// Picked points resolved from stored data, with their sources.
fn resolve_all(scene: &Scene, picks: &[Pick]) -> CmdResult<(Vec<P3>, Vec<PointSource>)> {
    let mut pts = vec![];
    let mut src = vec![];
    for p in picks {
        let r = resolve(scene, p)?;
        pts.push(r.project);
        src.push(PointSource {
            scan: p.scan.clone(),
            index: r.index,
            revision: p.revision,
        });
    }
    Ok((pts, src))
}

fn polyline_length(pts: &[P3]) -> f64 {
    pts.windows(2)
        .map(|w| {
            (0..3)
                .map(|k| (w[1][k] - w[0][k]).powi(2))
                .sum::<f64>()
                .sqrt()
        })
        .sum()
}

#[derive(Serialize)]
pub struct MarkLength {
    pub length: f64,
    pub points: Vec<P3>,
}

/// The length of a mark picked on the cloud as a polyline (m).
#[tauri::command]
pub async fn crash_mark_length(app: AppHandle, picks: Vec<Pick>) -> CmdResult<MarkLength> {
    blocking(app, move |s| {
        let (points, _) = resolve_all(&s.scene.read().unwrap(), &picks)?;
        if points.len() < 2 {
            return Err("Pick at least two points along the mark.".into());
        }
        Ok(MarkLength {
            length: polyline_length(&points),
            points,
        })
    })
    .await
}

/// A skid stretch as sent: its inputs, and the mark's points when measured on the cloud.
#[derive(Deserialize)]
pub struct SkidSegmentRequest {
    pub label: String,
    pub distance: Input,
    pub drag: Input,
    pub braking: Input,
    pub grade: Input,
    #[serde(default)]
    pub path: Vec<Pick>,
}

#[derive(Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum CrashRequest {
    Skid {
        segments: Vec<SkidSegmentRequest>,
        end_speed: Input,
    },
    Yaw {
        #[serde(default)]
        chord: Option<Input>,
        #[serde(default)]
        ordinate: Option<Input>,
        #[serde(default)]
        points: Vec<Pick>,
        drag: Input,
        superelevation: Input,
        #[serde(default)]
        cg_offset: f64,
    },
    Momentum {
        vehicles: [MomentumVehicle; 2],
    },
    Crush {
        label: String,
        a: Input,
        b: Input,
        stiffness_source: String,
        width: Input,
        depths: Vec<Input>,
        pdof_deg: Input,
        mass: Input,
    },
}

fn crash_run(
    scene: &Scene,
    point_sigma: f64,
    req: &CrashRequest,
) -> CmdResult<(&'static str, &'static str, serde_json::Value)> {
    let e = |x: crash::CrashError| capital(&x.to_string());
    let seed = 1;
    Ok(match req {
        CrashRequest::Skid {
            segments,
            end_speed,
        } => {
            let mut segs = vec![];
            for s in segments {
                let (path, sources) = resolve_all(scene, &s.path)?;
                if path.len() >= 2 {
                    // The distance must be the mark as measured on the cloud.
                    let l = polyline_length(&path);
                    if (s.distance.value - l).abs() > 1e-3 {
                        return Err(format!(
                            "{}: the distance ({:.3} m) isn't the length of the mark picked on the cloud ({l:.3} m).",
                            s.label, s.distance.value
                        ));
                    }
                }
                segs.push(SkidSegment {
                    label: s.label.clone(),
                    distance: s.distance,
                    drag: s.drag,
                    braking: s.braking,
                    grade: s.grade,
                    path,
                    sources,
                });
            }
            let r = crash::skid(segs, *end_speed, DRAWS, seed).map_err(e)?;
            ("skid", SKID_METHOD, serde_json::to_value(r).map_err(err)?)
        }
        CrashRequest::Yaw {
            chord,
            ordinate,
            points,
            drag,
            superelevation,
            cg_offset,
        } => {
            let radius = if points.is_empty() {
                YawRadius::Chord {
                    chord: chord.ok_or(
                        "Give the chord and middle ordinate, or pick points along the mark.",
                    )?,
                    ordinate: ordinate.ok_or(
                        "Give the chord and middle ordinate, or pick points along the mark.",
                    )?,
                }
            } else {
                let (points, sources) = resolve_all(scene, points)?;
                YawRadius::Points { points, sources }
            };
            let r = crash::yaw(
                radius,
                *drag,
                *superelevation,
                *cg_offset,
                point_sigma,
                DRAWS,
                seed,
            )
            .map_err(e)?;
            ("yaw", YAW_METHOD, serde_json::to_value(r).map_err(err)?)
        }
        CrashRequest::Momentum { vehicles } => {
            let r = crash::momentum(vehicles.clone(), DRAWS, seed).map_err(e)?;
            (
                "momentum",
                MOMENTUM_METHOD,
                serde_json::to_value(r).map_err(err)?,
            )
        }
        CrashRequest::Crush {
            label,
            a,
            b,
            stiffness_source,
            width,
            depths,
            pdof_deg,
            mass,
        } => {
            let r = crash::crush(
                label,
                *a,
                *b,
                stiffness_source,
                *width,
                depths.clone(),
                *pdof_deg,
                *mass,
                DRAWS,
                seed,
            )
            .map_err(e)?;
            ("crush", CRUSH_METHOD, serde_json::to_value(r).map_err(err)?)
        }
    })
}

/// Compute a crash reconstruction without storing it.
#[tauri::command]
pub async fn crash_preview(app: AppHandle, request: CrashRequest) -> CmdResult<serde_json::Value> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        let sigma = p.point_sigma().map_err(err)?;
        Ok(crash_run(&s.scene.read().unwrap(), sigma, &request)?.2)
    })
    .await
}

/// Compute a crash reconstruction and store it as an analysis record (audit-logged).
#[tauri::command]
pub async fn crash_save(
    app: AppHandle,
    name: String,
    request: CrashRequest,
    revises: Option<i64>,
) -> CmdResult<AnalysisRecord> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let sigma = p.point_sigma().map_err(err)?;
        let (tool, method, record) = crash_run(&s.scene.read().unwrap(), sigma, &request)?;
        p.add_analysis(tool, method, &name, &record, revises)
            .map_err(err)
    })
    .await
}
