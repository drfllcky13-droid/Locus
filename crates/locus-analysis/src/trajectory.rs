//! Bullet trajectory from defects on one or more surfaces, or from a probe rod.
//! Method, assumptions and limitations: docs/methods/trajectory.md.
//!
//! The path is a straight line fitted to the defect centres by weighted total least
//! squares (each centre with its own isotropic 1σ). Its direction's uncertainty is
//! propagated to first order from every input coordinate (numerical Jacobian), checked
//! against the fit's residuals (χ²), and reported as a 95 % cone. Angles are given in the
//! project frame (bearing clockwise from +y, elevation up from horizontal) and relative to
//! each impacted surface.

use crate::measure::{cross, dot, eigen_sym, norm, sub, Measured, P3};
use serde::{Deserialize, Serialize};

/// χ² with 2 degrees of freedom at 95 %: a 2-D normal's 95 % ellipse is `sqrt(5.991)` σ.
pub const CHI2_2DOF_95: f64 = 5.991_464_547_107_979;

/// A defect centre (or rod point) as picked, in travel order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathPoint {
    pub point: P3,
    /// 1σ, isotropic (m).
    pub sigma: f64,
}

/// The surface a defect is on (from a plane fit around it): a point and unit normal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    pub point: P3,
    pub normal: P3,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cone {
    /// 95 % half-angles along the cone's two principal directions (degrees).
    pub major_deg: f64,
    pub minor_deg: f64,
    /// Unit direction (square to the path) of the major axis.
    pub major_axis: P3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// A point on the line (the weighted centroid) and the unit direction of travel.
    pub point: P3,
    pub direction: P3,
    /// Covariance of the direction (3 × 3, rank 2, square to it), after any χ² inflation.
    pub covariance: [[f64; 3]; 3],
    /// Bearing clockwise from +y and elevation up from horizontal (degrees, 1σ).
    pub bearing: Measured,
    pub elevation: Measured,
    /// 95 % uncertainty cone of the direction.
    pub cone: Cone,
    /// Perpendicular distance of each input point from the line (m).
    pub residuals: Vec<f64>,
    /// χ² of the residuals against the stated σ, its degrees of freedom (2n − 4), and the
    /// factor the covariance was inflated by (√(χ²/dof) when above 1; 1 otherwise).
    pub chi2: f64,
    pub dof: usize,
    pub inflation: f64,
}

/// Angles of the path relative to one surface.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SurfaceAngles {
    /// Between the path and the surface: 90° is square on (degrees, 1σ).
    pub impact: Measured,
    /// In the surface's frame, from its normal: horizontal (+ to the right as seen facing
    /// the surface from the shooter's side) and vertical (+ upward), degrees (1σ). For a
    /// horizontal surface, "horizontal" is measured toward +x and "vertical" toward +y.
    pub horizontal: Measured,
    pub vertical: Measured,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrajectoryError {
    NeedPoints,
    Coincident,
    BadSigma,
}

impl std::fmt::Display for TrajectoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrajectoryError::NeedPoints => write!(f, "a trajectory needs at least two points"),
            TrajectoryError::Coincident => {
                write!(
                    f,
                    "the points are at the same place: no direction can be fitted"
                )
            }
            TrajectoryError::BadSigma => write!(f, "every point needs a positive uncertainty"),
        }
    }
}

fn scale(a: P3, s: f64) -> P3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn unit(a: P3) -> P3 {
    scale(a, 1.0 / norm(a))
}

/// Direction of the weighted TLS line through `pts`, oriented from the first point toward
/// the last (the order of travel). Returns (centroid, direction).
fn fit(pts: &[P3], w: &[f64]) -> Result<(P3, P3), TrajectoryError> {
    let sw: f64 = w.iter().sum();
    let c = std::array::from_fn(|k| pts.iter().zip(w).map(|(p, w)| p[k] * w).sum::<f64>() / sw);
    let mut m = [[0.0; 3]; 3];
    for (p, w) in pts.iter().zip(w) {
        let r = sub(*p, c);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += w * r[i] * r[j];
            }
        }
    }
    let (vals, vecs) = eigen_sym(m);
    let k = (0..3).max_by(|&a, &b| vals[a].total_cmp(&vals[b])).unwrap();
    if vals[k] <= 0.0 {
        return Err(TrajectoryError::Coincident);
    }
    let mut d = unit(vecs[k]);
    if dot(d, sub(pts[pts.len() - 1], pts[0])) < 0.0 {
        d = scale(d, -1.0);
    }
    Ok((c, d))
}

/// Bearing (clockwise from +y) and elevation of a direction, degrees.
pub fn angles(d: P3) -> (f64, f64) {
    let bearing = d[0].atan2(d[1]).to_degrees().rem_euclid(360.0);
    let elevation = d[2].clamp(-1.0, 1.0).asin().to_degrees();
    (bearing, elevation)
}

/// Direction of travel from a bearing and elevation, degrees.
pub fn direction(bearing: f64, elevation: f64) -> P3 {
    let (b, e) = (bearing.to_radians(), elevation.to_radians());
    [b.sin() * e.cos(), b.cos() * e.cos(), e.sin()]
}

/// σ of a scalar function of the direction, from the direction's covariance.
fn sigma_of(d: P3, cov: &[[f64; 3]; 3], f: &dyn Fn(P3) -> f64) -> f64 {
    let h = 1e-7;
    let f0 = f(d);
    let g: [f64; 3] = std::array::from_fn(|k| {
        let mut dd = d;
        dd[k] += h;
        // Wrap angular differences into (−180°, 180°].
        let mut df = f(unit(dd)) - f0;
        if df > 180.0 {
            df -= 360.0;
        } else if df < -180.0 {
            df += 360.0;
        }
        df / h
    });
    let mut v = 0.0;
    for i in 0..3 {
        for j in 0..3 {
            v += g[i] * cov[i][j] * g[j];
        }
    }
    v.max(0.0).sqrt()
}

/// Fit the path to points in travel order (defect centres, entry and exit on each surface;
/// or a rod's two ends). `extra_deg`, if positive, is added in quadrature to the direction's
/// uncertainty on both axes (a probe rod's play in its hole).
pub fn fit_line(points: &[PathPoint], extra_deg: f64) -> Result<Line, TrajectoryError> {
    if points.len() < 2 {
        return Err(TrajectoryError::NeedPoints);
    }
    if points.iter().any(|p| !p.sigma.is_finite() || p.sigma <= 0.0) {
        return Err(TrajectoryError::BadSigma);
    }
    let pts: Vec<P3> = points.iter().map(|p| p.point).collect();
    let w: Vec<f64> = points.iter().map(|p| 1.0 / (p.sigma * p.sigma)).collect();
    let (c, d) = fit(&pts, &w)?;
    // Residuals and χ².
    let residuals: Vec<f64> = pts
        .iter()
        .map(|p| {
            let r = sub(*p, c);
            norm(sub(r, scale(d, dot(r, d))))
        })
        .collect();
    let chi2: f64 = residuals
        .iter()
        .zip(points)
        .map(|(r, p)| (r / p.sigma).powi(2))
        .sum();
    let dof = (2 * pts.len()).saturating_sub(4);
    let inflation = if dof > 0 {
        (chi2 / dof as f64).sqrt().max(1.0)
    } else {
        1.0
    };
    // Covariance of d: numerical Jacobian over every input coordinate.
    let mut cov = [[0.0; 3]; 3];
    let h = 1e-7;
    for (i, p) in points.iter().enumerate() {
        for k in 0..3 {
            let mut q = pts.clone();
            q[i][k] += h;
            let (_, dq) = fit(&q, &w)?;
            // Keep the orientation consistent before differencing.
            let dq = if dot(dq, d) < 0.0 {
                scale(dq, -1.0)
            } else {
                dq
            };
            let j = scale(sub(dq, d), 1.0 / h);
            for a in 0..3 {
                for b in 0..3 {
                    cov[a][b] += j[a] * j[b] * p.sigma * p.sigma;
                }
            }
        }
    }
    for row in cov.iter_mut() {
        for v in row.iter_mut() {
            *v *= inflation * inflation;
        }
    }
    // Rod play (or any extra angular allowance): isotropic in the plane square to d.
    if extra_deg > 0.0 {
        let s2 = extra_deg.to_radians().powi(2);
        for a in 0..3 {
            for b in 0..3 {
                cov[a][b] += s2 * ((a == b) as u8 as f64 - d[a] * d[b]);
            }
        }
    }
    // 95 % cone: principal axes of the covariance square to d.
    let (vals, vecs) = eigen_sym(cov);
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| vals[b].total_cmp(&vals[a]));
    let half = |v: f64| (CHI2_2DOF_95 * v.max(0.0)).sqrt().to_degrees();
    let cone = Cone {
        major_deg: half(vals[order[0]]),
        minor_deg: half(vals[order[1]]),
        major_axis: unit(vecs[order[0]]),
    };
    let (b, e) = angles(d);
    Ok(Line {
        point: c,
        direction: d,
        bearing: Measured {
            value: b,
            sigma: sigma_of(d, &cov, &|x| angles(x).0),
        },
        elevation: Measured {
            value: e,
            sigma: sigma_of(d, &cov, &|x| angles(x).1),
        },
        covariance: cov,
        cone,
        residuals,
        chi2,
        dof,
        inflation,
    })
}

/// The path's angles relative to a surface (its normal as fitted; the normal's own small
/// uncertainty is not included).
pub fn surface_angles(line: &Line, s: &Surface) -> SurfaceAngles {
    // The normal facing the shooter (against the direction of travel).
    let n = if dot(s.normal, line.direction) > 0.0 {
        scale(s.normal, -1.0)
    } else {
        s.normal
    };
    let n = unit(n);
    // Surface axes: horizontal (to the right, seen from the shooter's side) and vertical.
    let (h, v) = if n[2].abs() < 0.99 {
        // n faces the shooter, so up × n points to the shooter's right.
        let h = unit(cross([0.0, 0.0, 1.0], n));
        (h, cross(n, h))
    } else {
        ([1.0, 0.0, 0.0], unit(cross(n, [1.0, 0.0, 0.0])))
    };
    let impact = |d: P3| (-dot(d, n)).clamp(-1.0, 1.0).asin().to_degrees();
    let horiz = |d: P3| dot(d, h).atan2(-dot(d, n)).to_degrees();
    let vert = |d: P3| dot(d, v).atan2(-dot(d, n)).to_degrees();
    let d = line.direction;
    let m = |f: &dyn Fn(P3) -> f64| Measured {
        value: f(d),
        sigma: sigma_of(d, &line.covariance, f),
    };
    SurfaceAngles {
        impact: m(&impact),
        horizontal: m(&horiz),
        vertical: m(&vert),
    }
}

/// Where a shooter could have been: the path traced back from `anchor` (the first defect)
/// through a height band above a floor, within `cone_deg` of it (a circular cone).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShooterBand {
    /// Along the path's centre line: distances back from the anchor where it enters and
    /// leaves the band (m), and the points there; None if it never passes through.
    pub centre: Option<([f64; 2], [P3; 2])>,
    /// Plan outline (x, y) of everywhere within the cone that is inside the band: the
    /// convex hull of where the cone's edges cross the band.
    pub footprint: Vec<[f64; 2]>,
}

/// `band` is [low, high] above `floor_z`; distances are capped at `max_range`.
pub fn shooter_band(
    line: &Line,
    anchor: P3,
    cone_deg: f64,
    floor_z: f64,
    band: [f64; 2],
    max_range: f64,
) -> ShooterBand {
    let (lo, hi) = (floor_z + band[0], floor_z + band[1]);
    // Back along a direction d (travel), from the anchor: z(t) = anchor.z − t d.z.
    let span = |d: P3| -> Option<[f64; 2]> {
        let dz = -d[2];
        let z0 = anchor[2];
        let (mut t0, mut t1) = if dz.abs() < 1e-12 {
            if (lo..=hi).contains(&z0) {
                (0.0, max_range)
            } else {
                return None;
            }
        } else {
            let a = (lo - z0) / dz;
            let b = (hi - z0) / dz;
            (a.min(b), a.max(b))
        };
        t0 = t0.max(0.0);
        t1 = t1.min(max_range);
        (t1 > t0).then_some([t0, t1])
    };
    let at = |d: P3, t: f64| add(anchor, scale(d, -t));
    let d = line.direction;
    let centre = span(d).map(|s| (s, [at(d, s[0]), at(d, s[1])]));
    // Cone edges: rotate d by cone_deg toward 72 directions around it.
    let t = if d[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = unit(cross(d, t));
    let w = cross(d, u);
    let c = cone_deg.to_radians();
    let mut pts: Vec<[f64; 2]> = vec![];
    for k in 0..72 {
        let a = std::f64::consts::TAU * k as f64 / 72.0;
        let side = add(scale(u, a.cos()), scale(w, a.sin()));
        let dk = unit(add(scale(d, c.cos()), scale(side, c.sin())));
        if let Some(s) = span(dk) {
            for t in s {
                let p = at(dk, t);
                pts.push([p[0], p[1]]);
            }
        }
    }
    if let Some((_, ps)) = centre {
        pts.extend(ps.iter().map(|p| [p[0], p[1]]));
    }
    ShooterBand {
        centre,
        footprint: hull(pts),
    }
}

/// Convex hull (Andrew's monotone chain), anticlockwise.
fn hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let turn = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut h: Vec<[f64; 2]> = vec![];
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &[f64; 2]>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in it {
            while h.len() >= start + 2 && turn(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(a: P3, b: P3) -> f64 {
        dot(unit(a), unit(b)).clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// Deterministic standard normals (Box–Muller on an xorshift stream).
    struct Rng(u64);
    impl Rng {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        fn normal(&mut self) -> f64 {
            let (u, v) = (self.unit().max(1e-300), self.unit());
            (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
        }
    }

    #[test]
    fn angles_follow_the_conventions() {
        let (b, e) = angles(direction(62.0, -4.0));
        assert!((b - 62.0).abs() < 1e-12 && (e + 4.0).abs() < 1e-12);
        assert_eq!(angles([0.0, 1.0, 0.0]), (0.0, 0.0)); // along +y: bearing 0
        assert!((angles([1.0, 0.0, 0.0]).0 - 90.0).abs() < 1e-12); // +x: 90°
        assert!((angles([-1.0, 0.0, 0.0]).0 - 270.0).abs() < 1e-12);
    }

    #[test]
    fn exact_points_give_the_exact_line_with_zero_residuals() {
        let d = direction(62.0, -4.0);
        let o = [431_200.0, 5_390_110.0, 212.0];
        let pts: Vec<PathPoint> = [1.8, 1.801, 4.6, 4.613, 6.1, 6.118]
            .iter()
            .map(|t| PathPoint {
                point: add(o, scale(d, *t)),
                sigma: 0.002,
            })
            .collect();
        let l = fit_line(&pts, 0.0).unwrap();
        assert!(deg(l.direction, d) < 1e-7);
        assert!((l.bearing.value - 62.0).abs() < 1e-6);
        assert!((l.elevation.value + 4.0).abs() < 1e-6);
        assert!(l.residuals.iter().all(|r| *r < 1e-8));
        assert_eq!(l.dof, 8);
        assert_eq!(l.inflation, 1.0);
        // Travel order is respected: reversed input reverses the direction.
        let rev: Vec<PathPoint> = pts.iter().rev().copied().collect();
        assert!(deg(fit_line(&rev, 0.0).unwrap().direction, scale(d, -1.0)) < 1e-7);
    }

    #[test]
    fn stated_uncertainty_agrees_with_monte_carlo() {
        // Three surfaces 4.3 m apart, 2 mm picking noise.
        let d = direction(30.0, 10.0);
        let ts = [0.0, 0.01, 2.1, 2.12, 4.3, 4.32];
        let truth: Vec<P3> = ts.iter().map(|t| scale(d, *t)).collect();
        let sigma = 0.002;
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let (mut sb, mut se) = (0.0, 0.0);
        let runs = 4000;
        let mut stated = (0.0, 0.0);
        for i in 0..runs {
            let pts: Vec<PathPoint> = truth
                .iter()
                .map(|p| PathPoint {
                    point: [
                        p[0] + rng.normal() * sigma,
                        p[1] + rng.normal() * sigma,
                        p[2] + rng.normal() * sigma,
                    ],
                    sigma,
                })
                .collect();
            let l = fit_line(&pts, 0.0).unwrap();
            let mut db = l.bearing.value - 30.0;
            if db > 180.0 {
                db -= 360.0;
            }
            sb += db * db;
            se += (l.elevation.value - 10.0).powi(2);
            if i == 0 {
                // Without χ² inflation, for comparing with the spread.
                stated = (
                    l.bearing.sigma / l.inflation,
                    l.elevation.sigma / l.inflation,
                );
            }
        }
        let (mb, me) = ((sb / runs as f64).sqrt(), (se / runs as f64).sqrt());
        assert!(
            (stated.0 / mb - 1.0).abs() < 0.1,
            "bearing σ {} vs {mb}",
            stated.0
        );
        assert!(
            (stated.1 / me - 1.0).abs() < 0.1,
            "elevation σ {} vs {me}",
            stated.1
        );
    }

    #[test]
    fn residuals_larger_than_stated_inflate_the_uncertainty() {
        let d = direction(0.0, 0.0);
        let mut pts: Vec<PathPoint> = [0.0, 1.0, 2.0, 3.0]
            .iter()
            .map(|t| PathPoint {
                point: scale(d, *t),
                sigma: 0.001,
            })
            .collect();
        pts[1].point[0] += 0.02; // a 2 cm misplaced pick, stated as 1 mm
        let l = fit_line(&pts, 0.0).unwrap();
        assert!(l.inflation > 5.0, "{}", l.inflation);
        assert!(l.residuals[1] > 0.01);
    }

    #[test]
    fn two_close_points_give_a_wide_cone_and_rod_play_widens_it() {
        let d = direction(90.0, 0.0);
        let pts = [
            PathPoint {
                point: [0.0; 3],
                sigma: 0.002,
            },
            PathPoint {
                point: scale(d, 0.0125),
                sigma: 0.002,
            },
        ];
        let l = fit_line(&pts, 0.0).unwrap();
        // σ ≈ √2·2 mm / 12.5 mm ≈ 13°: the entry and exit of one thin wall define little.
        assert!(l.cone.major_deg > 20.0, "{}", l.cone.major_deg);
        let rod = [
            PathPoint {
                point: [0.0; 3],
                sigma: 0.001,
            },
            PathPoint {
                point: scale(d, 0.5),
                sigma: 0.001,
            },
        ];
        let tight = fit_line(&rod, 0.0).unwrap();
        let loose = fit_line(&rod, 3.0).unwrap();
        assert!(tight.cone.major_deg < 1.0);
        assert!((loose.cone.major_deg - (CHI2_2DOF_95.sqrt() * 3.0)).abs() < 0.1);
    }

    #[test]
    fn surface_angles_for_known_geometry() {
        // A wall facing −y (normal toward the shooter at −y), path rising 10° and going 20°
        // to the right of square on.
        let d = direction(20.0, 10.0);
        let pts = [
            PathPoint {
                point: [0.0; 3],
                sigma: 0.001,
            },
            PathPoint {
                point: scale(d, 5.0),
                sigma: 0.001,
            },
        ];
        let l = fit_line(&pts, 0.0).unwrap();
        let s = surface_angles(
            &l,
            &Surface {
                point: [0.0, 5.0, 0.0],
                normal: [0.0, -1.0, 0.0],
            },
        );
        // The normal's sign doesn't matter.
        let s2 = surface_angles(
            &l,
            &Surface {
                point: [0.0, 5.0, 0.0],
                normal: [0.0, 1.0, 0.0],
            },
        );
        assert_eq!(s, s2);
        let expect_impact = (d[1]).asin().to_degrees();
        assert!((s.impact.value - expect_impact).abs() < 1e-9);
        assert!((s.horizontal.value - d[0].atan2(d[1]).to_degrees()).abs() < 1e-9);
        assert!((s.vertical.value - d[2].atan2(d[1]).to_degrees()).abs() < 1e-9);
        assert!(s.horizontal.value > 0.0 && s.vertical.value > 0.0);
    }

    #[test]
    fn shooter_band_on_a_downward_shot() {
        // Travelling −5° (downward), first defect at 1.0 m: traced back the path rises, and
        // is 1.2–1.8 m above the floor between (1.2−1.0)/tan5° and (1.8−1.0)/tan5° back.
        let d = direction(0.0, -5.0);
        let anchor = [0.0, 10.0, 1.0];
        let pts = [
            PathPoint {
                point: anchor,
                sigma: 0.001,
            },
            PathPoint {
                point: add(anchor, scale(d, 2.0)),
                sigma: 0.001,
            },
        ];
        let l = fit_line(&pts, 0.0).unwrap();
        let b = shooter_band(&l, anchor, 5.0, 0.0, [1.2, 1.8], 50.0);
        let (s, _) = b.centre.unwrap();
        let k = 5f64.to_radians().sin();
        assert!((s[0] - 0.2 / k).abs() < 1e-9 && (s[1] - 0.8 / k).abs() < 1e-9);
        // The footprint is behind the anchor (−y) and contains the centre segment.
        assert!(b.footprint.len() >= 3);
        assert!(b.footprint.iter().all(|p| p[1] < 10.0));
        // A level shot at 1.5 m: the whole range back is in the band.
        let lvl = fit_line(
            &[
                PathPoint {
                    point: [0.0, 0.0, 1.5],
                    sigma: 0.001,
                },
                PathPoint {
                    point: [0.0, 1.0, 1.5],
                    sigma: 0.001,
                },
            ],
            0.0,
        )
        .unwrap();
        let b = shooter_band(&lvl, [0.0, 0.0, 1.5], 0.0, 0.0, [1.2, 1.8], 30.0);
        assert_eq!(b.centre.unwrap().0, [0.0, 30.0]);
    }

    #[test]
    fn bad_inputs_are_refused() {
        let p = PathPoint {
            point: [0.0; 3],
            sigma: 0.001,
        };
        assert_eq!(
            fit_line(&[p], 0.0).unwrap_err(),
            TrajectoryError::NeedPoints
        );
        assert_eq!(
            fit_line(&[p, p], 0.0).unwrap_err(),
            TrajectoryError::Coincident
        );
        let q = PathPoint {
            point: [1.0, 0.0, 0.0],
            sigma: 0.0,
        };
        assert_eq!(
            fit_line(&[p, q], 0.0).unwrap_err(),
            TrajectoryError::BadSigma
        );
    }
}
