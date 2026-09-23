//! Scaling and placing a reconstruction: from the photos' GPS/RTK positions, from known
//! distances between points clicked in the photos, or from ground control points clicked in the
//! photos with their coordinates. Every clicked target is triangulated from at least two
//! registered photos with the reconstruction's own cameras.

use crate::camera;
use crate::exif::GpsTags;
use crate::model::{Model, P3};
use crate::scale::{fit_gcps, scale_from_distances, KnownDistance, Similarity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A target clicked in one photo (image name as COLMAP has it; pixel x right, y down, pixel
/// (i, j) spanning [i, i + 1) as COLMAP counts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Click {
    pub image: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Triangulated {
    pub point: P3,
    /// The widest angle between its rays (°) and its reprojection error in each photo (px).
    pub angle_deg: f64,
    pub residuals_px: Vec<f64>,
}

/// A target's position in the model from its clicks (at least 2 photos).
pub fn triangulate_clicks(m: &Model, clicks: &[Click]) -> Result<Triangulated, String> {
    if clicks.len() < 2 {
        return Err("click the target in at least 2 photos".into());
    }
    let mut rays = vec![];
    let mut used = vec![];
    for c in clicks {
        let im = m
            .images
            .values()
            .find(|i| i.name == c.image)
            .ok_or_else(|| format!("{} isn't in the reconstruction", c.image))?;
        let cam = &m.cameras[&im.camera];
        rays.push(camera::ray(cam, im, [c.x, c.y])?);
        used.push((cam, im, [c.x, c.y]));
    }
    let (point, angle_deg) = camera::triangulate(&rays).ok_or("the rays are parallel")?;
    let residuals_px = used
        .iter()
        .map(|(cam, im, px)| {
            camera::project_world(cam, im, point)
                .ok()
                .flatten()
                .map_or(f64::INFINITY, |q| {
                    ((q[0] - px[0]).powi(2) + (q[1] - px[1]).powi(2)).sqrt()
                })
        })
        .collect();
    Ok(Triangulated {
        point,
        angle_deg,
        residuals_px,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistanceItem {
    pub label: String,
    pub a: Vec<Click>,
    pub b: Vec<Click>,
    /// True length and its 1σ (m).
    pub length: f64,
    pub sigma: f64,
    /// Held out of the scaling and reported as a check.
    #[serde(default)]
    pub check: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcpItem {
    pub label: String,
    pub clicks: Vec<Click>,
    /// The point's coordinates in the project (m): surveyed, or picked on a scan.
    pub world: P3,
    /// Held out of the fit and reported as a check.
    #[serde(default)]
    pub check: bool,
    /// The coordinates' 1σ per axis (m): a survey's stated accuracy, or a scan's.
    #[serde(default = "gcp_sigma")]
    pub sigma: f64,
}

fn gcp_sigma() -> f64 {
    0.005
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ScaleRequest {
    Gps,
    Distances { items: Vec<DistanceItem> },
    Gcps { items: Vec<GcpItem> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleRow {
    pub label: String,
    /// Where it is in the model, where it should be (or its length), and the residual (m).
    pub model: P3,
    pub target: String,
    /// What the scaled reconstruction measures there (a length, or coordinates).
    #[serde(default)]
    pub measured: String,
    pub residual: f64,
    #[serde(default)]
    pub check: bool,
    #[serde(default)]
    pub angle_deg: f64,
    /// A check's 95 % limit (m), and whether its residual passes it.
    #[serde(default)]
    pub limit: f64,
    #[serde(default)]
    pub exceeds: bool,
}

/// Every measurement made on a photogrammetric point cloud takes the uncertainty
/// max(percent × length, floor) (1σ): the percentage from the benchmark or the case's own checks
/// (whichever is larger) with the scaling's own uncertainty, and an absolute floor so short
/// distances don't get unrealistically tight bounds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeasurementModel {
    /// Relative 1σ (a fraction) and the floor (m, 1σ).
    pub percent: f64,
    pub floor: f64,
    /// Whether the case's checks set either term (else the benchmark did).
    pub from_checks: bool,
}

impl Default for MeasurementModel {
    fn default() -> Self {
        MeasurementModel {
            percent: BENCH_PERCENT,
            floor: BENCH_FLOOR,
            from_checks: false,
        }
    }
}

impl MeasurementModel {
    /// A length's 1σ (m).
    pub fn sigma(&self, length: f64) -> f64 {
        (self.percent * length.abs()).max(self.floor)
    }
    /// An angle's 1σ (rad) from the floor: each arm's far end displaced by it across the arm.
    pub fn angle_sigma(&self, arm_a: f64, arm_b: f64) -> f64 {
        self.floor * (arm_a.max(1e-9).powi(-2) + arm_b.max(1e-9).powi(-2)).sqrt()
    }
    /// An area's 1σ (m²): twice the percentage (area goes as length squared), or the floor
    /// over half the perimeter, whichever is larger.
    pub fn area_sigma(&self, area: f64, perimeter: f64) -> f64 {
        (2.0 * self.percent * area.abs()).max(self.floor * perimeter / 2.0)
    }
}

/// From ETH3D's pipes benchmark (docs/methods/photogrammetry.md): the RMS relative error of
/// distances between scene points was 0.23–0.33 % over four runs, and a point's error 3.0–3.8 mm
/// per axis; a distance's floor is √2 × 3.8 mm, rounded up.
pub const BENCH_PERCENT: f64 = 0.0035;
pub const BENCH_FLOOR: f64 = 0.006;

/// A length's 1σ from the benchmark alone, with the scaling's relative uncertainty.
fn benchmark_sigma(length: f64, scale_rel: f64) -> f64 {
    (BENCH_PERCENT.hypot(scale_rel) * length).max(BENCH_FLOOR)
}

/// The model for a case: the benchmark's terms, raised by the checks' (relative RMS, absolute
/// RMS) when they are larger, the percentage combined with the scaling's own uncertainty.
fn model_from(scale_rel: f64, checks: Option<(f64, f64)>) -> MeasurementModel {
    let (p, f) = checks.unwrap_or((0.0, 0.0));
    MeasurementModel {
        percent: BENCH_PERCENT.max(p).hypot(scale_rel),
        floor: BENCH_FLOOR.max(f),
        from_checks: p > BENCH_PERCENT || f > BENCH_FLOOR,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleRecord {
    pub method: String,
    /// Model → project, and the relative 1σ of its scale.
    pub transform: Similarity,
    pub scale_sigma_rel: f64,
    pub rows: Vec<ScaleRow>,
    /// The uncertainty every measurement on the resulting cloud takes (runs stored before it
    /// existed read as the benchmark's terms).
    #[serde(default)]
    pub uncertainty: MeasurementModel,
    /// RMS of the fit's residuals (m).
    pub rms: f64,
    /// The local east-north-up frame's origin (latitude °, longitude °, altitude m) when
    /// georeferenced by GPS.
    #[serde(default)]
    pub enu_origin: Option<[f64; 3]>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

/// A rotation taking unit vector `a` onto unit vector `b` (rows).
fn rotation_between(a: [f64; 3], b: [f64; 3]) -> [[f64; 3]; 3] {
    let (a, b) = (nalgebra::Vector3::from(a), nalgebra::Vector3::from(b));
    let r = nalgebra::Rotation3::rotation_between(&a, &b).unwrap_or_else(|| {
        nalgebra::Rotation3::from_axis_angle(&nalgebra::Vector3::x_axis(), std::f64::consts::PI)
    });
    std::array::from_fn(|i| std::array::from_fn(|k| r[(i, k)]))
}

/// The photos' mean "up" in the model: each camera's −y axis (COLMAP cameras look along +z
/// with +y down), averaged. Photos held roughly level make this close to the true vertical.
pub fn photos_up(m: &Model) -> [f64; 3] {
    let mut u = nalgebra::Vector3::zeros();
    for im in m.images.values() {
        let r = im.rotation();
        u -= nalgebra::Vector3::new(r[1][0], r[1][1], r[1][2]);
    }
    u.normalize().into()
}

fn centroid(ps: impl Iterator<Item = P3>) -> P3 {
    let (mut s, mut n) = ([0.0; 3], 0.0f64);
    for p in ps {
        for k in 0..3 {
            s[k] += p[k];
        }
        n += 1.0;
    }
    s.map(|v| v / n.max(1.0))
}

fn dist(a: P3, b: P3) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
}

/// A similarity fit's relative scale uncertainty, from its residual RMS over the points'
/// spread: σ_s/s ≈ RMS / √(Σ |x − x̄|²).
fn fit_scale_sigma(world: &[P3], rms: f64) -> f64 {
    let c = centroid(world.iter().copied());
    let spread: f64 = world.iter().map(|p| dist(*p, c).powi(2)).sum();
    rms / spread.sqrt().max(1e-9)
}

/// Solve the scaling. `gps` holds each photo's tags by image name (for `Gps`).
pub fn solve(
    m: &Model,
    req: &ScaleRequest,
    gps: &BTreeMap<String, GpsTags>,
) -> Result<ScaleRecord, String> {
    let mut warnings = vec![];
    let mut notes = vec![];
    match req {
        ScaleRequest::Gps => {
            let with: Vec<_> = m
                .images
                .values()
                .filter_map(|im| {
                    let g = gps.get(&im.name)?;
                    Some((im, g.latitude?, g.longitude?, g.altitude.unwrap_or(0.0), g))
                })
                .collect();
            if with.len() < 3 {
                return Err(format!(
                    "only {} registered photos carry a GPS position; at least 3 are needed",
                    with.len()
                ));
            }
            let origin = (with[0].1, with[0].2, with[0].3);
            let model: Vec<P3> = with.iter().map(|w| w.0.centre()).collect();
            let world: Vec<P3> = with
                .iter()
                .map(|w| crate::geo::enu(origin, (w.1, w.2, w.3)))
                .collect();
            let f = fit_gcps(&model, &world, &[])?;
            let rtk = with.iter().filter(|w| w.4.rtk_flag == Some(50)).count();
            notes.push(format!(
                "Georeferenced by the GPS positions of {} photos ({} with an RTK fixed solution), into a local east-north-up frame from the first of them. Altitudes are as the photos record them (usually above sea level).",
                with.len(),
                rtk
            ));
            if rtk < with.len() {
                warnings.push("Not every photo has an RTK fixed position. Ordinary GPS in photos is usually good to several metres, which over a small scene leaves the scale and orientation poorly determined: check the residuals, or scale by known distances or control points instead.".into());
            }
            let rows = with
                .iter()
                .zip(&f.residuals)
                .zip(&world)
                .map(|((w, r), e)| ScaleRow {
                    label: w.0.name.clone(),
                    model: w.0.centre(),
                    target: format!("E {:.3}, N {:.3}, U {:.3} m", e[0], e[1], e[2]),
                    measured: String::new(),
                    residual: *r,
                    check: false,
                    angle_deg: 0.0,
                    limit: 0.0,
                    exceeds: false,
                })
                .collect();
            let scale_rel = fit_scale_sigma(&world, f.rms);
            Ok(ScaleRecord {
                method: "gps".into(),
                uncertainty: model_from(scale_rel, None),
                scale_sigma_rel: scale_rel,
                transform: f.transform,
                rows,
                rms: f.rms,
                enu_origin: Some([origin.0, origin.1, origin.2]),
                notes,
                warnings,
            })
        }
        ScaleRequest::Distances { items } => {
            if !items.iter().any(|i| !i.check) {
                return Err(
                    "give at least one known distance to scale by (besides check distances)".into(),
                );
            }
            let mut known = vec![];
            let mut ends = vec![];
            for it in items {
                let a = triangulate_clicks(m, &it.a)
                    .map_err(|e| format!("{}, first end: {e}", it.label))?;
                let b = triangulate_clicks(m, &it.b)
                    .map_err(|e| format!("{}, second end: {e}", it.label))?;
                known.push(KnownDistance {
                    a: a.point,
                    b: b.point,
                    length: it.length,
                    sigma: it.sigma,
                });
                ends.push((a, b));
            }
            let fit: Vec<KnownDistance> = items
                .iter()
                .zip(&known)
                .filter(|(it, _)| !it.check)
                .map(|(_, k)| *k)
                .collect();
            let s = scale_from_distances(&fit)?;
            let scale_rel = s.sigma / s.scale;
            // Level by the photos' mean up, and centre on the sparse points.
            let r = rotation_between(photos_up(m), [0.0, 0.0, 1.0]);
            let c = centroid(m.points.iter().map(|p| p.xyz));
            let rc: P3 = std::array::from_fn(|i| (0..3).map(|k| r[i][k] * c[k]).sum::<f64>());
            let transform = Similarity {
                scale: s.scale,
                rotation: r,
                translation: rc.map(|v| -s.scale * v),
            };
            let nchecks = items.iter().filter(|i| i.check).count();
            notes.push(format!(
                "Scaled by {} known distance{}{}; levelled by the photos' mean up direction (approximate: photos held tilted tilt it) and centred on the reconstruction. Its position and heading in the project are arbitrary: register it to a scan or place it by control points to relate it to other evidence.",
                fit.len(),
                if fit.len() == 1 { "" } else { "s" },
                if nchecks > 0 {
                    format!(", with {nchecks} check distance{} held out", if nchecks == 1 { "" } else { "s" })
                } else {
                    String::new()
                }
            ));
            let rows: Vec<ScaleRow> = items
                .iter()
                .zip(&known)
                .zip(&ends)
                .map(|((it, k), (a, b))| {
                    let measured = s.scale * dist(k.a, k.b);
                    let residual = measured - it.length;
                    // A check's 95 % limit: its stated tolerance and the measurement model's
                    // (benchmark) uncertainty at that length.
                    let limit = 2.0 * it.sigma.hypot(benchmark_sigma(it.length, scale_rel));
                    ScaleRow {
                        label: it.label.clone(),
                        model: [0.0; 3],
                        target: format!("{:.4} ± {:.4} m", it.length, it.sigma),
                        measured: format!("{measured:.4} m"),
                        residual,
                        check: it.check,
                        angle_deg: a.angle_deg.min(b.angle_deg),
                        limit,
                        exceeds: it.check && residual.abs() > limit,
                    }
                })
                .collect();
            if s.deviations_pct.iter().any(|d| d.abs() > 1.0) {
                warnings.push("The known distances disagree with each other by more than 1 %: check the clicks and the lengths.".into());
            }
            for row in &rows {
                if row.angle_deg < 5.0 {
                    warnings.push(format!("{}: an end is seen from photos less than 5° apart, so its position along the line of sight is weak.", row.label));
                }
            }
            check_warnings(&rows, &mut warnings);
            if nchecks == 0 {
                warnings.push("No check distances: nothing independent tests the scale. Mark one or more known distances as check only.".into());
            }
            let fitted: Vec<&ScaleRow> = rows.iter().filter(|r| !r.check).collect();
            let rms = (fitted.iter().map(|r| r.residual.powi(2)).sum::<f64>()
                / fitted.len() as f64)
                .sqrt();
            // The case's own checks, where there are any, may raise the model's terms.
            let checks: Vec<(f64, f64)> = items
                .iter()
                .zip(&rows)
                .filter(|(it, _)| it.check)
                .map(|(it, r)| (r.residual, it.length))
                .collect();
            let case = (!checks.is_empty()).then(|| {
                let n = checks.len() as f64;
                (
                    (checks.iter().map(|(e, l)| (e / l).powi(2)).sum::<f64>() / n).sqrt(),
                    (checks.iter().map(|(e, _)| e * e).sum::<f64>() / n).sqrt(),
                )
            });
            Ok(ScaleRecord {
                method: "distances".into(),
                scale_sigma_rel: scale_rel,
                uncertainty: model_from(scale_rel, case),
                transform,
                rows,
                rms,
                enu_origin: None,
                notes,
                warnings,
            })
        }
        ScaleRequest::Gcps { items } => {
            let mut fit = vec![];
            let mut checks = vec![];
            let mut tri = vec![];
            for it in items {
                let t =
                    triangulate_clicks(m, &it.clicks).map_err(|e| format!("{}: {e}", it.label))?;
                if it.check {
                    checks.push((t.point, it.world));
                } else {
                    fit.push((t.point, it.world));
                }
                tri.push(t);
            }
            if fit.len() < 3 {
                return Err("give at least 3 control points (besides check points)".into());
            }
            let (mm, ww): (Vec<P3>, Vec<P3>) = fit.iter().copied().unzip();
            let f = fit_gcps(&mm, &ww, &checks)?;
            let scale_rel = fit_scale_sigma(&ww, f.rms);
            let centre = centroid(ww.iter().copied());
            let (mut fi, mut ci) = (0, 0);
            let rows: Vec<ScaleRow> = items
                .iter()
                .zip(&tri)
                .map(|(it, t)| {
                    let residual = if it.check {
                        ci += 1;
                        f.check_errors[ci - 1]
                    } else {
                        fi += 1;
                        f.residuals[fi - 1]
                    };
                    let placed = f.transform.apply(t.point);
                    // A check point's 95 % limit (3-D, χ² with 3 degrees of freedom, 2.80σ):
                    // its stated coordinates' σ, a point's benchmark σ per axis, and the scale's
                    // uncertainty at its distance from the control points' centre.
                    let axis = it
                        .sigma
                        .hypot(BENCH_FLOOR / 2f64.sqrt())
                        .hypot(scale_rel.max(BENCH_PERCENT) * dist(it.world, centre));
                    let limit = 2.80 * axis;
                    ScaleRow {
                        label: it.label.clone(),
                        model: t.point,
                        target: format!(
                            "{:.3}, {:.3}, {:.3} m (±{:.3})",
                            it.world[0], it.world[1], it.world[2], it.sigma
                        ),
                        measured: format!("{:.3}, {:.3}, {:.3} m", placed[0], placed[1], placed[2]),
                        residual,
                        check: it.check,
                        angle_deg: t.angle_deg,
                        limit,
                        exceeds: it.check && residual > limit,
                    }
                })
                .collect();
            notes.push(format!(
                "Placed and scaled by {} control points (a similarity: scale, rotation, translation){}.",
                fit.len(),
                if checks.is_empty() {
                    String::new()
                } else {
                    format!(", with {} check points held out", checks.len())
                }
            ));
            if checks.is_empty() {
                warnings.push("No check points: the fit's residuals can't show an error the control points share. Hold one or more out as checks.".into());
            }
            check_warnings(&rows, &mut warnings);
            // A check point's 3-D error, as a distance's: about √(2/3) of it.
            let case = (!checks.is_empty()).then(|| {
                let n = f.check_errors.len() as f64;
                let rms = (f.check_errors.iter().map(|e| e * e).sum::<f64>() / n).sqrt();
                (0.0, rms * (2.0f64 / 3.0).sqrt())
            });
            Ok(ScaleRecord {
                method: "gcps".into(),
                scale_sigma_rel: scale_rel,
                uncertainty: model_from(scale_rel, case),
                transform: f.transform,
                rows,
                rms: f.rms,
                enu_origin: None,
                notes,
                warnings,
            })
        }
    }
}

/// A warning for each check whose residual passes its 95 % limit.
fn check_warnings(rows: &[ScaleRow], warnings: &mut Vec<String>) {
    for r in rows.iter().filter(|r| r.exceeds) {
        warnings.push(format!(
            "Check {}: the residual, {:.1} mm, is more than its 95 % limit of {:.1} mm (its stated uncertainty with the measurement's). Check the clicks and the stated value; the reconstruction may be less accurate than the benchmark here.",
            r.label,
            r.residual.abs() * 1000.0,
            r.limit * 1000.0
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Camera, Image};

    /// A small synthetic model: 4 pinhole cameras on a line looking along +z at points on a
    /// plane 5 m away, in a frame scaled by 1/3 and turned.
    fn model() -> (Model, Vec<P3>) {
        let mut m = Model::default();
        m.cameras.insert(
            1,
            Camera {
                id: 1,
                model: "PINHOLE".into(),
                width: 4000,
                height: 3000,
                params: vec![3000.0, 3000.0, 2000.0, 1500.0],
            },
        );
        for k in 0..4 {
            // Centre (k − 1.5, 0, 0): t = −R C with R = identity.
            m.images.insert(
                k + 1,
                Image {
                    id: k + 1,
                    camera: 1,
                    name: format!("IMG_{k}.JPG"),
                    q: [1.0, 0.0, 0.0, 0.0],
                    t: [-(k as f64 - 1.5), 0.0, 0.0],
                    observations: 0,
                    keypoints: vec![],
                },
            );
        }
        let pts: Vec<P3> = vec![
            [-1.0, -0.5, 5.0],
            [1.2, 0.3, 5.5],
            [0.2, 1.0, 4.8],
            [-0.4, 0.2, 6.0],
            [0.9, -0.8, 5.2],
        ];
        (m, pts)
    }

    fn clicks(m: &Model, x: P3) -> Vec<Click> {
        m.images
            .values()
            .map(|im| {
                let q = camera::project_world(&m.cameras[&1], im, x)
                    .unwrap()
                    .unwrap();
                Click {
                    image: im.name.clone(),
                    x: q[0],
                    y: q[1],
                }
            })
            .collect()
    }

    #[test]
    fn measurements_take_the_larger_term() {
        let m = MeasurementModel {
            percent: 0.0035,
            floor: 0.006,
            from_checks: false,
        };
        // 1 m: the floor (3.5 mm < 6 mm); 10 m: the percentage (35 mm).
        assert_eq!(m.sigma(1.0), 0.006);
        assert!((m.sigma(10.0) - 0.035).abs() < 1e-12);
        // Arms of 1 m and 2 m: 6 mm × √(1 + 1/4) rad.
        assert!((m.angle_sigma(1.0, 2.0) - 0.006 * 1.25f64.sqrt()).abs() < 1e-12);
        // 4 m² with a 8 m perimeter: max(2.8e-2, 2.4e-2).
        assert!((m.area_sigma(4.0, 8.0) - 0.028).abs() < 1e-12);
    }

    #[test]
    fn a_clicked_target_is_triangulated() {
        let (m, pts) = model();
        let t = triangulate_clicks(&m, &clicks(&m, pts[1])).unwrap();
        assert!((0..3).all(|k| (t.point[k] - pts[1][k]).abs() < 1e-9));
        assert!(t.residuals_px.iter().all(|r| *r < 1e-6));
        assert!(triangulate_clicks(&m, &clicks(&m, pts[1])[..1]).is_err());
    }

    #[test]
    fn distances_and_control_points_scale_the_model() {
        let (m, pts) = model();
        // The model is at 1/3 scale: a model distance d is 3d in the world.
        let d = |a: P3, b: P3| dist(a, b);
        let item = |label: &str, a: usize, b: usize, length: f64, check: bool| DistanceItem {
            label: label.into(),
            a: clicks(&m, pts[a]),
            b: clicks(&m, pts[b]),
            length,
            sigma: 0.002,
            check,
        };
        let req = ScaleRequest::Distances {
            items: vec![item("tape", 0, 1, 3.0 * d(pts[0], pts[1]), false)],
        };
        let r = solve(&m, &req, &BTreeMap::new()).unwrap();
        assert!((r.transform.scale - 3.0).abs() < 1e-9);
        assert!(r.rows[0].residual.abs() < 1e-9);
        assert!(r.warnings.iter().any(|w| w.contains("No check distances")));
        // No checks: the benchmark's terms, with the scaling's own σ (2 mm over 6.8 m).
        assert!(!r.uncertainty.from_checks);
        assert_eq!(r.uncertainty.floor, BENCH_FLOOR);
        assert!(r.uncertainty.percent > BENCH_PERCENT && r.uncertainty.percent < 0.004);
        assert_eq!(r.uncertainty.sigma(0.5), BENCH_FLOOR);
        // Photos level: the cameras' −y (image up) is the model's −y, levelled onto +z.
        let up = r.transform.apply([0.0, -1.0, 0.0]);
        let o = r.transform.apply([0.0, 0.0, 0.0]);
        assert!((up[2] - o[2] - 3.0).abs() < 1e-9, "{up:?} {o:?}");
        // With a check distance: right, it passes and doesn't scale; 5 cm wrong, it's flagged
        // and raises the model's floor.
        let good = 3.0 * d(pts[2], pts[3]);
        let r = solve(
            &m,
            &ScaleRequest::Distances {
                items: vec![
                    item("tape", 0, 1, 3.0 * d(pts[0], pts[1]), false),
                    item("check", 2, 3, good, true),
                ],
            },
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(r.rows[1].check && !r.rows[1].exceeds && r.rows[1].residual.abs() < 1e-9);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let r = solve(
            &m,
            &ScaleRequest::Distances {
                items: vec![
                    item("tape", 0, 1, 3.0 * d(pts[0], pts[1]), false),
                    item("check", 2, 3, good + 0.05, true),
                ],
            },
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(
            (r.transform.scale - 3.0).abs() < 1e-9,
            "a check doesn't scale"
        );
        assert!(r.rows[1].exceeds);
        assert!(r.warnings.iter().any(|w| w.contains("Check check")));
        assert!(r.uncertainty.from_checks && (r.uncertainty.floor - 0.05).abs() < 1e-9);
        // Only checks: refused.
        assert!(solve(
            &m,
            &ScaleRequest::Distances {
                items: vec![item("c", 0, 1, 1.0, true)]
            },
            &BTreeMap::new()
        )
        .is_err());
        // Control points: a known similarity, with one check point.
        let t = Similarity {
            scale: 3.0,
            rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [100.0, 50.0, 2.0],
        };
        let gcps = |shift: f64| -> Vec<GcpItem> {
            pts.iter()
                .enumerate()
                .map(|(k, p)| {
                    let mut w = t.apply(*p);
                    if k == 4 {
                        w[0] += shift;
                    }
                    GcpItem {
                        label: format!("G{k}"),
                        clicks: clicks(&m, *p),
                        world: w,
                        check: k == 4,
                        sigma: 0.005,
                    }
                })
                .collect()
        };
        let r = solve(
            &m,
            &ScaleRequest::Gcps { items: gcps(0.0) },
            &BTreeMap::new(),
        )
        .unwrap();
        assert!((r.transform.scale - 3.0).abs() < 1e-9);
        assert!(r.rows[4].check && r.rows[4].residual < 1e-9 && !r.rows[4].exceeds);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        // A check point 10 cm off: flagged.
        let r = solve(
            &m,
            &ScaleRequest::Gcps { items: gcps(0.1) },
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(r.rows[4].exceeds && (r.rows[4].residual - 0.1).abs() < 1e-9);
        assert!(r.warnings.iter().any(|w| w.contains("Check G4")));
        // GPS: too few photos with positions.
        assert!(solve(&m, &ScaleRequest::Gps, &BTreeMap::new()).is_err());
    }
}
