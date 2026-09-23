//! Crash reconstruction commands: skid, yaw, momentum and crush energy. Marks measured on the
//! cloud are resolved again here from stored data, so the stored run says exactly what it was
//! built from.

use crate::commands::{blocking, err, CmdResult};
use crate::scene_cmds::{resolve, Pick};
use locus_analysis::crash::{
    self, Input, MomentumVehicle, SkidSegment, YawRadius, CRUSH_METHOD, DRAWS, MOMENTUM_METHOD,
    SKID_METHOD, YAW_METHOD,
};
use locus_analysis::crush_volume;
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
        /// A and B from the bundled NHTSA table (this vehicle), else as entered with a source.
        #[serde(default)]
        table: Option<TableKey>,
        #[serde(default = "no_input")]
        a: Input,
        #[serde(default = "no_input")]
        b: Input,
        #[serde(default)]
        stiffness_source: String,
        /// The width and depths measured on the scan instead of entered.
        #[serde(default)]
        profile: Option<ProfileRequest>,
        #[serde(default = "no_input")]
        width: Input,
        #[serde(default)]
        depths: Vec<Input>,
        pdof_deg: Input,
        mass: Input,
    },
    /// Volumetric crush: an undamaged reference scan registered onto the damaged one by picked
    /// pairs (reference, damaged) and ICP around the damage region `lo`–`hi`.
    Volume {
        label: String,
        damaged: String,
        reference: String,
        /// The reference is the damaged vehicle's own opposite side, mirrored across its centre
        /// plane; `pairs` are then symmetric features (left, right) on the damaged scan.
        #[serde(default)]
        mirror: bool,
        pairs: Vec<[Pick; 2]>,
        lo: P3,
        hi: P3,
        cell: f64,
    },
    /// EDR pre-crash data as CSV (imported or built from the form), its source, the speed's
    /// tolerance (a fraction and m/s) and, optionally, a path picked in the scene.
    Edr {
        label: String,
        source: String,
        csv: String,
        speed_unit: locus_analysis::edr::SpeedUnit,
        scale_tolerance: f64,
        offset_tolerance: f64,
        /// Required when the tolerance is wider than the recording accuracy.
        #[serde(default)]
        tolerance_reason: String,
        #[serde(default)]
        end_time: Option<f64>,
        #[serde(default)]
        path: Vec<Pick>,
    },
}

/// Points of the vehicle this far around the pairs and the damage region are used (m).
const VOLUME_MARGIN: f64 = 1.0;
/// The reference is thinned to this spacing for the ICP (m).
const VOLUME_SPACING: f64 = 0.02;
const VOLUME_DRAWS: usize = 200;

#[allow(clippy::too_many_arguments)]
fn crush_volume_run(
    scene: &Scene,
    point_sigma: f64,
    label: &str,
    damaged: &str,
    reference: &str,
    mirror: bool,
    pairs: &[[Pick; 2]],
    lo: P3,
    hi: P3,
    cell: f64,
) -> CmdResult<crush_volume::VolumeRun> {
    use locus_octree::scene::{apply, ScanKey};
    use locus_register::exemplar;
    use nalgebra::Point3;
    let key = |s: &str| ScanKey::parse(s).ok_or_else(|| format!("No scan {s}."));
    let dk = key(damaged)?;
    let rk = if mirror { dk } else { key(reference)? };
    if !mirror && dk == rk {
        return Err("Choose two different scans: the damaged vehicle and the reference.".into());
    }
    if pairs.len() < 3 {
        return Err(if mirror {
            "Pick at least 3 pairs of symmetric features: each on the left, then its counterpart on the right."
        } else {
            "Pick at least 3 pairs: the same undamaged feature on the reference, then on the damaged vehicle."
        }
        .into());
    }
    if !(0..3).all(|k| hi[k] > lo[k]) {
        return Err("Set the clip box around the damage first.".into());
    }
    let (first, second) = if mirror {
        (damaged, damaged)
    } else {
        (reference, damaged)
    };
    let mut rows = vec![];
    for (k, [r, d]) in pairs.iter().enumerate() {
        if r.scan != first || d.scan != second {
            return Err(format!(
                "Pair {}: {}",
                k + 1,
                if mirror {
                    "pick both symmetric features on the damaged vehicle's scan."
                } else {
                    "pick the reference's point on the reference scan, then the damaged vehicle's on the damaged scan."
                }
            ));
        }
        let (rp, rs) = resolve_all(scene, std::slice::from_ref(r))?;
        let (dp, ds) = resolve_all(scene, std::slice::from_ref(d))?;
        rows.push(crush_volume::PairRow {
            reference: rp[0],
            damaged: dp[0],
            residual: f64::NAN,
            reference_source: rs[0].clone(),
            damaged_source: ds[0].clone(),
        });
    }
    let grow = |pts: &mut dyn Iterator<Item = P3>| {
        let (mut a, mut b) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in pts {
            for k in 0..3 {
                a[k] = a[k].min(p[k]);
                b[k] = b[k].max(p[k]);
            }
        }
        (a.map(|v| v - VOLUME_MARGIN), b.map(|v| v + VOLUME_MARGIN))
    };
    let view = apply(&scene.scans[&dk].pose, [0.0; 3]);
    let pair_pts: Vec<(P3, P3)> = rows.iter().map(|r| (r.reference, r.damaged)).collect();
    let (da, db) = grow(
        &mut rows
            .iter()
            .flat_map(|r| [r.damaged, r.reference])
            .filter(|_| mirror)
            .chain(rows.iter().map(|r| r.damaged))
            .chain([lo, hi]),
    );
    let dam = scene.scan_points_in(dk, da, db).map_err(err)?;
    // The reference's points and how a reference point maps onto the damaged scan.
    let (refs, a, mirrored) = if mirror {
        let m = exemplar::align_mirror(&dam, view, &pair_pts, lo, hi, VOLUME_SPACING, point_sigma)
            .map_err(|e| capital(&e))?;
        let info = crush_volume::MirrorInfo {
            plane_point: m.plane.point,
            normal: m.plane.normal,
            midpoint_offsets: m.plane.midpoint_offsets.clone(),
            pair_angles_deg: m.plane.pair_angles_deg.clone(),
            symmetry_sigma: m.symmetry_sigma,
        };
        let plane = m.plane.clone();
        for r in &mut rows {
            let t = m.aligned.transform * Point3::from(exemplar::reflect(r.reference, &plane));
            r.residual = (t.coords - nalgebra::Vector3::from(r.damaged)).norm();
        }
        (m.reference, m.aligned, Some(info))
    } else {
        let (ra, rb) = grow(&mut rows.iter().map(|r| r.reference));
        let refs = scene.scan_points_in(rk, ra, rb).map_err(err)?;
        let a = exemplar::align_exemplar(&refs, &dam, view, &pair_pts, lo, hi, VOLUME_SPACING)
            .map_err(|e| capital(&e))?;
        for r in &mut rows {
            let t = a.transform * Point3::from(r.reference);
            r.residual = (t.coords - nalgebra::Vector3::from(r.damaged)).norm();
        }
        (refs, a, None)
    };
    let moved: Vec<P3> = refs
        .iter()
        .map(|p| (a.transform * Point3::from(*p)).coords.into())
        .collect();
    let result = crush_volume::crush_volume(
        &moved,
        &dam,
        lo,
        hi,
        view,
        cell,
        point_sigma,
        Some(crush_volume::PoseUncertainty {
            covariance: a.covariance,
            about: a.about.coords.into(),
            surface: mirrored.as_ref().map_or(0.0, |m| m.symmetry_sigma),
        }),
        VOLUME_DRAWS,
        1,
    )
    .map_err(|e| capital(&e.0))?;
    let h = a.transform.to_homogeneous();
    let c = &a.covariance;
    let registration = crush_volume::Registration {
        pairs: rows,
        pairs_rms: a.pairs.rms,
        icp_rms: a.icp.rms,
        icp_pairs: a.icp.pairs,
        overlap: a.icp.overlap,
        conditioning: a.icp.conditioning,
        iterations: a.icp.iterations,
        converged: a.icp.converged,
        inflation: a.inflation,
        transform: std::array::from_fn(|k| h[(k / 4, k % 4)]),
        sigma_translation: (c[(3, 3)] + c[(4, 4)] + c[(5, 5)]).max(0.0).sqrt(),
        sigma_rotation_deg: (c[(0, 0)] + c[(1, 1)] + c[(2, 2)])
            .max(0.0)
            .sqrt()
            .to_degrees(),
    };
    Ok(crush_volume::volume_run(
        label,
        damaged,
        if mirror { damaged } else { reference },
        lo,
        hi,
        registration,
        result,
        mirrored,
    ))
}

/// A crush profile to measure on the scan: the damage's two ends on the undamaged face line
/// and a point inside the vehicle, the number of stations and the height band (m).
#[derive(Deserialize)]
pub struct ProfileRequest {
    pub picks: Vec<Pick>,
    pub stations: usize,
    pub band: f64,
}

/// Measure a crush profile on the scan (the command behind the crush tool's "Measure on the
/// scan").
fn measure_profile(
    scene: &Scene,
    req: &ProfileRequest,
    point_sigma: f64,
) -> CmdResult<crash::CrushProfile> {
    if req.picks.len() != 3 {
        return Err("Pick the damage's two ends on the undamaged face line, then a point inside the vehicle.".into());
    }
    let (pts, sources) = resolve_all(scene, &req.picks)?;
    let (a, b, inside) = (pts[0], pts[1], pts[2]);
    let mid = [
        (a[0] + b[0]) / 2.0,
        (a[1] + b[1]) / 2.0,
        (a[2] + b[2]) / 2.0,
    ];
    let half = ((b[0] - a[0]).hypot(b[1] - a[1])) / 2.0;
    // Every point within reach of the face line's stations and 2 m behind it.
    let reach = (half + 0.1).hypot(2.1).hypot(req.band);
    let near = scene.points_within(mid, reach).map_err(err)?;
    let mut p = crash::crush_profile(a, b, inside, &near, req.stations, req.band, point_sigma)
        .map_err(|x| capital(&x.to_string()))?;
    p.sources = sources;
    Ok(p)
}

fn no_input() -> Input {
    Input::exact(f64::NAN)
}

/// A vehicle in the bundled stiffness table.
#[derive(Deserialize, Clone)]
pub struct TableKey {
    pub make: String,
    pub model: String,
    pub model_year: u32,
}

/// An entry's coefficients as inputs: normal, with ±2σ as the range.
fn table_input(v: f64, sigma: f64) -> Input {
    Input {
        value: v,
        low: v - 2.0 * sigma,
        high: v + 2.0 * sigma,
        normal: true,
    }
}

/// A measured crush depth as an input: normal ±2σ, or, when that would reach below 0 (crush
/// can't be negative), uniform from 0 to d + 2σ.
fn depth_input(d: f64, sigma: f64) -> Input {
    if d - 2.0 * sigma >= 0.0 {
        table_input(d, sigma)
    } else {
        Input {
            value: d,
            low: 0.0,
            high: d + 2.0 * sigma,
            normal: false,
        }
    }
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
            table,
            a,
            b,
            stiffness_source,
            profile,
            width,
            depths,
            pdof_deg,
            mass,
        } => {
            let measured = match profile {
                Some(pr) => Some(measure_profile(scene, pr, point_sigma)?),
                None => None,
            };
            let (width, depths) = match &measured {
                Some(p) => {
                    let w_sigma = std::f64::consts::SQRT_2 * point_sigma;
                    (
                        table_input(p.width, w_sigma),
                        p.stations
                            .iter()
                            .map(|s| depth_input(s.depth, s.sigma))
                            .collect::<Vec<_>>(),
                    )
                }
                None => (*width, depths.clone()),
            };
            let entry = match table {
                Some(k) => Some(
                    locus_analysis::stiffness::table()
                        .iter()
                        .find(|e| {
                            e.make == k.make && e.model == k.model && e.model_year == k.model_year
                        })
                        .ok_or_else(|| {
                            format!(
                                "{} {} {} is not in the stiffness table.",
                                k.model_year, k.make, k.model
                            )
                        })?
                        .clone(),
                ),
                None => None,
            };
            let (a, b, source) = match &entry {
                Some(e) => (
                    table_input(e.a, e.a_sigma),
                    table_input(e.b, e.b_sigma),
                    format!(
                        "NHTSA Vehicle Crash Test Database, {} {} {} (frontal, full-width rigid barrier): test{} {}; Campbell's method, b0 = 8 ± 3.2 km/h (docs/methods/crash-stiffness.md)",
                        e.model_year,
                        e.make,
                        e.model,
                        if e.tests.len() > 1 { "s" } else { "" },
                        e.tests
                            .iter()
                            .map(|t| t.test_no.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
                None => {
                    if a.value.is_nan() || b.value.is_nan() {
                        return Err("Choose the vehicle from the stiffness table, or enter A and B with their source.".into());
                    }
                    (*a, *b, stiffness_source.clone())
                }
            };
            let mut r = crash::crush(
                label, a, b, &source, width, depths, *pdof_deg, *mass, DRAWS, seed,
            )
            .map_err(e)?;
            r.table_entry = entry;
            r.profile = measured;
            ("crush", CRUSH_METHOD, serde_json::to_value(r).map_err(err)?)
        }
        CrashRequest::Edr {
            label,
            source,
            csv,
            speed_unit,
            scale_tolerance,
            offset_tolerance,
            tolerance_reason,
            end_time,
            path,
        } => {
            use locus_analysis::edr;
            let (samples, columns) = edr::parse_csv(csv, *speed_unit).map_err(e)?;
            let (points, sources) = resolve_all(scene, path)?;
            let mut r = edr::edr(
                label,
                source,
                csv,
                columns,
                samples,
                *scale_tolerance,
                *offset_tolerance,
                tolerance_reason,
                *end_time,
                points,
                sources,
                DRAWS,
                seed,
            )
            .map_err(e)?;
            r.csv_sha256 = locus_core::hash::sha256_reader(csv.as_bytes(), &mut |_| {})
                .map_err(err)?
                .0;
            (
                "edr",
                edr::EDR_METHOD,
                serde_json::to_value(r).map_err(err)?,
            )
        }
        CrashRequest::Volume {
            label,
            damaged,
            reference,
            mirror,
            pairs,
            lo,
            hi,
            cell,
        } => {
            let r = crush_volume_run(
                scene,
                point_sigma,
                label,
                damaged,
                reference,
                *mirror,
                pairs,
                *lo,
                *hi,
                *cell,
            )?;
            (
                "crush_volume",
                crush_volume::VOLUME_METHOD,
                serde_json::to_value(r).map_err(err)?,
            )
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

/// Vehicles in the bundled CRASH3 stiffness table matching a make, a model (part of the name)
/// and a model-year range; at most 100.
#[tauri::command]
pub async fn stiffness_lookup(
    make: String,
    model: String,
    year_from: Option<u32>,
    year_to: Option<u32>,
) -> CmdResult<Vec<locus_analysis::stiffness::Entry>> {
    let years = match (year_from, year_to) {
        (Some(a), Some(b)) => Some((a, b)),
        (Some(a), None) => Some((a, a)),
        (None, Some(b)) => Some((b, b)),
        _ => None,
    };
    Ok(locus_analysis::stiffness::lookup(&make, &model, years)
        .into_iter()
        .take(100)
        .cloned()
        .collect())
}

/// The makes in the bundled stiffness table.
#[tauri::command]
pub async fn stiffness_makes() -> CmdResult<Vec<String>> {
    let mut m: Vec<String> = locus_analysis::stiffness::table()
        .iter()
        .map(|e| e.make.clone())
        .collect();
    m.dedup();
    Ok(m)
}

/// Measure a crush profile on the scan without computing energy (for the form).
#[tauri::command]
pub async fn crash_crush_profile(
    app: AppHandle,
    request: ProfileRequest,
) -> CmdResult<crash::CrushProfile> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        let sigma = p.point_sigma().map_err(err)?;
        measure_profile(&s.scene.read().unwrap(), &request, sigma)
    })
    .await
}
