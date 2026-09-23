//! The centre of a bullet hole in a scanned surface, from the points around its rim.
//! Method, assumptions and limitations: docs/methods/trajectory.md ("Hole centres").
//!
//! A hole has no points, so its centre can't be picked directly. From a click on or near the
//! hole: the points of the clicked face are projected onto their fitted plane; the hole is
//! found as the largest empty circle near the click; the nearest point in each direction
//! around it (sectors about one point spacing wide at the edge) is taken as a rim point; and an ellipse is fitted to the rim points
//! (Gauss–Newton on the ellipse equation). The centre's uncertainty comes from the fit, and
//! the ellipse's axes, corrected for the point spacing, give an impact angle asin(b / a)
//! that is independent of the trajectory fit.

use crate::measure::{cross, dot, fit_plane, norm, sub, Measured, P3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefectFit {
    /// Centre of the hole on the face's plane (m, project frame), and its 1σ in the plane
    /// (the larger of the two axes).
    pub centre: P3,
    pub centre_sigma: f64,
    /// Unit normal of the face.
    pub normal: P3,
    /// Semi-axes (long, short), corrected for the point spacing, with 1σ (m).
    pub semi_axes: [Measured; 2],
    /// Direction of the long axis within the face (unit).
    pub long_axis: P3,
    /// Impact angle from the ellipse, asin(short / long), degrees (1σ).
    pub impact: Measured,
    pub rim_points: usize,
    /// Median distance between neighbouring points on the face (m).
    pub spacing: f64,
    /// RMS distance of the rim points from the fitted ellipse, before the spacing
    /// correction (m).
    pub rms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefectError {
    TooFewPoints,
    NoHole,
    OpenHole,
    FitFailed,
}

impl std::fmt::Display for DefectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DefectError::TooFewPoints => "too few scan points around the click",
            DefectError::NoHole => {
                "no hole found near the click (no gap wider than the point spacing)"
            }
            DefectError::OpenHole => {
                "the gap isn't closed all round (at the edge of the scan or the surface)"
            }
            DefectError::FitFailed => "the rim doesn't fit an ellipse",
        })
    }
}

type P2 = [f64; 2];

fn unit(a: P3) -> P3 {
    let n = norm(a);
    [a[0] / n, a[1] / n, a[2] / n]
}

/// Ellipse parameters: centre x, y, semi-axes a, b, angle of a from +x.
type Ell = [f64; 5];

fn residuals(e: &Ell, pts: &[P2]) -> Vec<f64> {
    let (c, s) = (e[4].cos(), e[4].sin());
    pts.iter()
        .map(|p| {
            let (dx, dy) = (p[0] - e[0], p[1] - e[1]);
            let (u, v) = (c * dx + s * dy, -s * dx + c * dy);
            // Scaled to metres near the ellipse: (√(u²/a² + v²/b²) − 1) × the mean semi-axis.
            ((u / e[2]).powi(2) + (v / e[3]).powi(2)).sqrt() * ((e[2] + e[3]) / 2.0)
                - (e[2] + e[3]) / 2.0
        })
        .collect()
}

/// Solve the 5 × 5 system `m x = r` (Gaussian elimination with partial pivoting).
fn solve5(mut m: [[f64; 5]; 5], mut r: [f64; 5]) -> Option<[f64; 5]> {
    for i in 0..5 {
        let p = (i..5).max_by(|&a, &b| m[a][i].abs().total_cmp(&m[b][i].abs()))?;
        if m[p][i].abs() < 1e-300 {
            return None;
        }
        m.swap(i, p);
        r.swap(i, p);
        for k in i + 1..5 {
            let f = m[k][i] / m[i][i];
            let pivot = m[i];
            for (mkj, mij) in m[k].iter_mut().zip(pivot).skip(i) {
                *mkj -= f * mij;
            }
            r[k] -= f * r[i];
        }
    }
    let mut x = [0.0; 5];
    for i in (0..5).rev() {
        x[i] = (r[i] - (i + 1..5).map(|j| m[i][j] * x[j]).sum::<f64>()) / m[i][i];
    }
    Some(x)
}

fn jacobian(e: &Ell, pts: &[P2]) -> Vec<[f64; 5]> {
    let r0 = residuals(e, pts);
    let mut j = vec![[0.0; 5]; pts.len()];
    for k in 0..5 {
        let h = if k == 4 { 1e-7 } else { 1e-9 };
        let mut e2 = *e;
        e2[k] += h;
        for (i, r) in residuals(&e2, pts).iter().enumerate() {
            j[i][k] = (r - r0[i]) / h;
        }
    }
    j
}

/// Levenberg–Marquardt on the ellipse residuals. Returns the fit and (JᵀJ)⁻¹.
fn fit_ellipse(pts: &[P2]) -> Option<(Ell, [[f64; 5]; 5])> {
    let n = pts.len() as f64;
    let c = [
        pts.iter().map(|p| p[0]).sum::<f64>() / n,
        pts.iter().map(|p| p[1]).sum::<f64>() / n,
    ];
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for p in pts {
        let (x, y) = (p[0] - c[0], p[1] - c[1]);
        sxx += x * x;
        syy += y * y;
        sxy += x * y;
    }
    let (sxx, syy, sxy) = (sxx / n, syy / n, sxy / n);
    // Points spread round an ellipse have variance a²/2 and b²/2 along its axes.
    let tr = sxx + syy;
    let det = sxx * syy - sxy * sxy;
    let l1 = tr / 2.0 + (tr * tr / 4.0 - det).max(0.0).sqrt();
    let l2 = (tr - l1).max(1e-12);
    let mut e: Ell = [
        c[0],
        c[1],
        (2.0 * l1).sqrt(),
        (2.0 * l2).sqrt(),
        0.5 * (2.0 * sxy).atan2(sxx - syy),
    ];
    let cost = |e: &Ell| residuals(e, pts).iter().map(|r| r * r).sum::<f64>();
    let mut lambda = 1e-3;
    for _ in 0..100 {
        let r = residuals(&e, pts);
        let j = jacobian(&e, pts);
        let mut a = [[0.0; 5]; 5];
        let mut g = [0.0; 5];
        for (row, ri) in j.iter().zip(&r) {
            for p in 0..5 {
                g[p] -= row[p] * ri;
                for q in 0..5 {
                    a[p][q] += row[p] * row[q];
                }
            }
        }
        let c0 = cost(&e);
        let mut stepped = false;
        for _ in 0..10 {
            let mut ad = a;
            for (p, row) in ad.iter_mut().enumerate() {
                row[p] *= 1.0 + lambda;
            }
            let Some(dx) = solve5(ad, g) else { break };
            let mut e2 = e;
            for k in 0..5 {
                e2[k] += dx[k];
            }
            if e2[2] > 0.0 && e2[3] > 0.0 && cost(&e2) < c0 {
                e = e2;
                lambda = (lambda / 3.0).max(1e-9);
                stepped = true;
                break;
            }
            lambda *= 10.0;
        }
        if !stepped {
            break;
        }
    }
    // a is the long axis.
    if e[3] > e[2] {
        e.swap(2, 3);
        e[4] += std::f64::consts::FRAC_PI_2;
    }
    let j = jacobian(&e, pts);
    let mut a = [[0.0; 5]; 5];
    for row in &j {
        for p in 0..5 {
            for q in 0..5 {
                a[p][q] += row[p] * row[q];
            }
        }
    }
    let mut inv = [[0.0; 5]; 5];
    for k in 0..5 {
        let mut ek = [0.0; 5];
        ek[k] = 1.0;
        let col = solve5(a, ek)?;
        for i in 0..5 {
            inv[i][k] = col[i];
        }
    }
    Some((e, inv))
}

/// Find and fit the hole near `click` from the scan points around it (within a radius a few
/// times the hole's size; both faces of a thin surface may be included).
pub fn fit_defect(click: P3, neighbours: &[P3]) -> Result<DefectFit, DefectError> {
    if neighbours.len() < 20 {
        return Err(DefectError::TooFewPoints);
    }
    // The clicked face: fit a plane, keep the layer of points at the click's depth, refit.
    let plane0 = fit_plane(neighbours).map_err(|_| DefectError::TooFewPoints)?;
    let depth = |p: P3, pl: &crate::measure::Plane| dot(sub(p, pl.point), pl.normal);
    let d_click = depth(click, &plane0);
    let face: Vec<P3> = neighbours
        .iter()
        .copied()
        .filter(|p| {
            (depth(*p, &plane0) - d_click).abs() < 0.003f64.max(3.0 * plane0.rms.min(0.001))
        })
        .collect();
    if face.len() < 20 {
        return Err(DefectError::TooFewPoints);
    }
    let plane = fit_plane(&face).map_err(|_| DefectError::TooFewPoints)?;
    let n = unit(plane.normal);
    let t = if n[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = unit(cross(n, t));
    let v = cross(n, u);
    let origin = sub(
        click,
        [
            n[0] * depth(click, &plane),
            n[1] * depth(click, &plane),
            n[2] * depth(click, &plane),
        ],
    );
    let pts: Vec<P2> = face
        .iter()
        .map(|p| {
            let r = sub(*p, origin);
            [dot(r, u), dot(r, v)]
        })
        .collect();
    // Point spacing: the median distance to each point's 4th nearest neighbour. On a grid
    // that is the spacing; it stays so when a thin surface's two faces project onto each
    // other (each point then has a twin almost on top of it, which a nearest-neighbour
    // median would report instead).
    let mut nn: Vec<f64> = pts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut best = [f64::INFINITY; 4];
            for (j, q) in pts.iter().enumerate() {
                if j == i {
                    continue;
                }
                let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                if d < best[3] {
                    best[3] = d;
                    best.sort_by(f64::total_cmp);
                }
            }
            best[3]
        })
        .collect();
    nn.sort_by(f64::total_cmp);
    let spacing = nn[nn.len() / 2];
    let clear = |c: P2| {
        pts.iter()
            .map(|p| ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt())
            .fold(f64::INFINITY, f64::min)
    };
    // The hole: the largest empty circle centred within reach of the click.
    let reach = pts
        .iter()
        .map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt())
        .fold(0.0, f64::max)
        / 2.0;
    let step = (spacing / 2.0).max(reach / 60.0);
    let k = (reach / step).ceil() as i64;
    let mut best = ([0.0, 0.0], clear([0.0, 0.0]));
    for i in -k..=k {
        for j in -k..=k {
            let c = [i as f64 * step, j as f64 * step];
            if c[0] * c[0] + c[1] * c[1] > reach * reach {
                continue;
            }
            let r = clear(c);
            if r > best.1 {
                best = (c, r);
            }
        }
    }
    if best.1 < 1.5 * spacing {
        return Err(DefectError::NoHole);
    }
    // Rim: the nearest point in each direction around the current centre; refine the centre
    // from the fitted ellipse and repeat.
    let mut centre = best.0;
    let mut fit: Option<(Ell, [[f64; 5]; 5], Vec<P2>)> = None;
    for _ in 0..4 {
        // Sectors about one point spacing wide at the hole's edge, so each holds a rim point.
        let sectors = ((std::f64::consts::TAU * best.1 / spacing).round() as usize).clamp(12, 72);
        let mut rim: Vec<Option<(f64, P2)>> = vec![None; sectors];
        for p in &pts {
            let (dx, dy) = (p[0] - centre[0], p[1] - centre[1]);
            let r = (dx * dx + dy * dy).sqrt();
            let s = (((dy.atan2(dx) / std::f64::consts::TAU).rem_euclid(1.0)) * sectors as f64)
                as usize
                % sectors;
            if rim[s].is_none_or(|(rr, _)| r < rr) {
                rim[s] = Some((r, *p));
            }
        }
        // Closed all round: no run of empty sectors wider than 45°, and rim points no farther
        // than a few times the empty circle (else the "hole" opens onto the scan's edge).
        let limit = 4.0 * best.1;
        let filled: Vec<bool> = rim
            .iter()
            .map(|x| x.is_some_and(|(r, _)| r < limit))
            .collect();
        let mut run = 0;
        let mut worst = 0;
        for i in 0..2 * sectors {
            if filled[i % sectors] {
                run = 0;
            } else {
                run += 1;
                worst = worst.max(run);
            }
        }
        if worst * 360 / sectors > 45 {
            return Err(DefectError::OpenHole);
        }
        let rim: Vec<P2> = rim
            .into_iter()
            .flatten()
            .filter(|(r, _)| *r < limit)
            .map(|(_, p)| p)
            .collect();
        let Some((e, inv)) = fit_ellipse(&rim) else {
            return Err(DefectError::FitFailed);
        };
        let moved = ((e[0] - centre[0]).powi(2) + (e[1] - centre[1]).powi(2)).sqrt();
        centre = [e[0], e[1]];
        fit = Some((e, inv, rim));
        if moved < spacing / 20.0 {
            break;
        }
    }
    let (e, inv, rim) = fit.ok_or(DefectError::FitFailed)?;
    let r = residuals(&e, &rim);
    let dof = (rim.len() as f64 - 5.0).max(1.0);
    let s2 = r.iter().map(|x| x * x).sum::<f64>() / dof;
    let rms = (r.iter().map(|x| x * x).sum::<f64>() / rim.len() as f64).sqrt();
    if !(e[2].is_finite() && e[3].is_finite()) || e[2] > 1.0 {
        return Err(DefectError::FitFailed);
    }
    // Rim points lie outside the true edge by up to one spacing: on average half of one.
    let q = spacing / 12f64.sqrt();
    let (a, b) = (e[2] - spacing / 2.0, e[3] - spacing / 2.0);
    if b <= 0.0 {
        return Err(DefectError::NoHole);
    }
    let sa = (s2 * inv[2][2] + q * q).sqrt();
    let sb = (s2 * inv[3][3] + q * q).sqrt();
    let cov_ab = s2 * inv[2][3];
    // asin(b/a): ∂/∂a = −b/(a²√(1−r²)), ∂/∂b = 1/(a√(1−r²)), r = b/a.
    let ratio = (b / a).min(1.0);
    let k = (1.0 - ratio * ratio).max(1e-12).sqrt();
    let (ga, gb) = (-b / (a * a * k), 1.0 / (a * k));
    let var_imp = ga * ga * sa * sa + gb * gb * sb * sb + 2.0 * ga * gb * cov_ab;
    // The fit's own σ, plus the rim's sampling (one spacing, uniform): the fit statistics
    // alone understate it when few rim points are available or two faces overlap.
    let centre_sigma = ((s2 * inv[0][0]).max(s2 * inv[1][1]) + q * q).sqrt();
    let c3 = [
        origin[0] + e[0] * u[0] + e[1] * v[0],
        origin[1] + e[0] * u[1] + e[1] * v[1],
        origin[2] + e[0] * u[2] + e[1] * v[2],
    ];
    let long = unit([
        e[4].cos() * u[0] + e[4].sin() * v[0],
        e[4].cos() * u[1] + e[4].sin() * v[1],
        e[4].cos() * u[2] + e[4].sin() * v[2],
    ]);
    Ok(DefectFit {
        centre: c3,
        centre_sigma,
        normal: n,
        semi_axes: [
            Measured {
                value: a,
                sigma: sa,
            },
            Measured {
                value: b,
                sigma: sb,
            },
        ],
        long_axis: long,
        impact: Measured {
            value: ratio.asin().to_degrees(),
            // Near 90° (a nearly round hole) asin is ill-conditioned; past 90° the σ says
            // only "undetermined".
            sigma: var_imp.max(0.0).sqrt().to_degrees().min(90.0),
        },
        rim_points: rim.len(),
        spacing,
        rms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat face on a jittered grid (spacing s) with an elliptical hole (semi-axes a, b,
    /// long axis at angle th) centred at `c`, in the plane z = 0.
    fn face(s: f64, c: P2, a: f64, b: f64, th: f64, half: f64) -> Vec<P3> {
        let mut out = vec![];
        let n = (half / s) as i64;
        let mut seed = 0x9e37_79b9_u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for i in -n..=n {
            for j in -n..=n {
                let p = [
                    i as f64 * s + rnd() * s * 0.3,
                    j as f64 * s + rnd() * s * 0.3,
                ];
                let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                let (x, y) = (
                    th.cos() * dx + th.sin() * dy,
                    -th.sin() * dx + th.cos() * dy,
                );
                if (x / a).powi(2) + (y / b).powi(2) > 1.0 {
                    out.push([p[0], p[1], rnd() * 0.0004]);
                }
            }
        }
        out
    }

    #[test]
    fn finds_the_centre_of_an_oblique_hole_and_its_angle() {
        // A 9 mm bullet at 40° to the surface: 7.0 mm long semi-axis, 4.5 mm short.
        let (a, b) = (0.0045 / 40f64.to_radians().sin(), 0.0045);
        let c = [0.0012, -0.0007];
        let pts = face(0.0015, c, a, b, 0.6, 0.03);
        // Clicked on the rim, not in the hole.
        let f = fit_defect(
            [c[0] + a * 0.6_f64.cos(), c[1] + a * 0.6_f64.sin(), 0.0],
            &pts,
        )
        .unwrap();
        let err = ((f.centre[0] - c[0]).powi(2) + (f.centre[1] - c[1]).powi(2)).sqrt();
        assert!(err < 0.0005, "centre off by {err}");
        assert!(
            err < 3.0 * f.centre_sigma.max(1e-4),
            "{err} vs σ {}",
            f.centre_sigma
        );
        assert!(
            (f.impact.value - 40.0).abs() < 3.0 * f.impact.sigma + 2.0,
            "{:?}",
            f.impact
        );
        assert!(
            (f.semi_axes[0].value - a).abs() < 0.001,
            "{:?}",
            f.semi_axes
        );
        // The long axis points along the ellipse's (either way round).
        let along = (f.long_axis[0] * 0.6f64.cos() + f.long_axis[1] * 0.6f64.sin()).abs();
        assert!(along > 0.99, "{along}");
    }

    #[test]
    fn a_round_hole_is_ninety_degrees() {
        let pts = face(0.0015, [0.0, 0.0], 0.0045, 0.0045, 0.0, 0.03);
        let f = fit_defect([0.0, 0.0, 0.0], &pts).unwrap();
        // Near 90° the angle is poorly determined: it must be consistent with 90°, and say so.
        assert!(
            90.0 - f.impact.value <= 2.0 * f.impact.sigma,
            "{:?}",
            f.impact
        );
        assert!(f.impact.sigma > 10.0);
    }

    #[test]
    fn no_hole_or_an_open_one_is_refused() {
        let solid = face(0.0015, [1.0, 1.0], 0.001, 0.001, 0.0, 0.02);
        assert_eq!(
            fit_defect([0.0; 3], &solid).unwrap_err(),
            DefectError::NoHole
        );
        // A "hole" that runs off the edge of the scanned patch.
        let edge: Vec<P3> = solid
            .into_iter()
            .filter(|p| p[0] < 0.0 || p[1].abs() > 0.006)
            .collect();
        assert!(fit_defect([0.004, 0.0, 0.0], &edge).is_err());
        assert_eq!(
            fit_defect([0.0; 3], &[[0.0; 3]; 5]).unwrap_err(),
            DefectError::TooFewPoints
        );
    }
}
