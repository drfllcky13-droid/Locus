//! Point-cloud measurements: distance, angle, polygon area, height above a plane.
//! Method, assumptions and limitations: docs/methods/measurement.md.
//!
//! Points are in meters in the project frame. Each point is assumed to carry independent,
//! isotropic positional uncertainty `sigma_point` (1σ, meters). Uncertainty is propagated
//! to first order with a numerical Jacobian, so every result is `value ± sigma` (1σ).

use serde::{Deserialize, Serialize};

pub type P3 = [f64; 3];

/// A value with its 1σ standard uncertainty, in the value's SI unit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measured {
    pub value: f64,
    pub sigma: f64,
}

/// Least-squares plane through a set of points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    /// Centroid of the fitted points; the plane passes through it.
    pub point: P3,
    /// Unit normal, oriented so its z component is not negative.
    pub normal: P3,
    /// Root-mean-square distance of the fitted points from the plane, meters.
    pub rms: f64,
    /// Largest absolute distance of a fitted point from the plane, meters.
    pub max_abs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Area {
    /// Area of the polygon projected onto its best-fit plane, m².
    pub area: Measured,
    pub perimeter: f64,
    pub plane: Plane,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Height {
    /// Signed distance from the plane along its normal (positive above), meters.
    pub height: Measured,
    pub plane: Plane,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasureError {
    NeedPoints(usize),
    Degenerate(&'static str),
}

impl std::fmt::Display for MeasureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeasureError::NeedPoints(n) => write!(f, "needs at least {n} points"),
            MeasureError::Degenerate(why) => write!(f, "cannot measure: {why}"),
        }
    }
}

impl std::error::Error for MeasureError {}

pub(crate) fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn norm(a: P3) -> f64 {
    dot(a, a).sqrt()
}

/// First-order propagation of isotropic, independent point uncertainty through `f`,
/// using central differences scaled to the geometry.
pub fn propagate(points: &[P3], sigma_point: f64, f: &dyn Fn(&[P3]) -> f64) -> f64 {
    let scale = points
        .iter()
        .map(|p| norm(sub(*p, points[0])))
        .fold(0.0, f64::max)
        .max(1e-3);
    let h = scale * 1e-6;
    let mut var = 0.0;
    let mut work = points.to_vec();
    for i in 0..points.len() {
        for d in 0..3 {
            work[i][d] = points[i][d] + h;
            let up = f(&work);
            work[i][d] = points[i][d] - h;
            let down = f(&work);
            work[i][d] = points[i][d];
            let g = (up - down) / (2.0 * h);
            var += g * g;
        }
    }
    sigma_point * var.sqrt()
}

pub fn distance(a: P3, b: P3, sigma_point: f64) -> Measured {
    let f = |p: &[P3]| norm(sub(p[1], p[0]));
    let pts = [a, b];
    Measured {
        value: f(&pts),
        sigma: propagate(&pts, sigma_point, &f),
    }
}

/// Angle at `vertex` between the rays to `a` and `c`, radians in [0, π].
pub fn angle(a: P3, vertex: P3, c: P3, sigma_point: f64) -> Result<Measured, MeasureError> {
    let f = |p: &[P3]| {
        let (u, w) = (sub(p[0], p[1]), sub(p[2], p[1]));
        norm(cross(u, w)).atan2(dot(u, w))
    };
    if norm(sub(a, vertex)) == 0.0 || norm(sub(c, vertex)) == 0.0 {
        return Err(MeasureError::Degenerate(
            "an arm of the angle has zero length",
        ));
    }
    let pts = [a, vertex, c];
    Ok(Measured {
        value: f(&pts),
        sigma: propagate(&pts, sigma_point, &f),
    })
}

/// Eigen-decomposition of a symmetric 3×3 matrix by Jacobi rotation. Returns eigenvalues
/// and eigenvectors (columns of the returned matrix, as rows here: `vecs[k]`).
pub(crate) fn eigen_sym(mut a: [[f64; 3]; 3]) -> ([f64; 3], [P3; 3]) {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1].powi(2) + a[0][2].powi(2) + a[1][2].powi(2);
        if off < 1e-30 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in a.iter_mut() {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
            }
            let (rp, rq) = (a[p], a[q]);
            a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
            a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
            for row in v.iter_mut() {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    let vals = [a[0][0], a[1][1], a[2][2]];
    let vecs = std::array::from_fn(|k| [v[0][k], v[1][k], v[2][k]]);
    (vals, vecs)
}

/// Total-least-squares plane (minimizes perpendicular distances).
pub fn fit_plane(points: &[P3]) -> Result<Plane, MeasureError> {
    if points.len() < 3 {
        return Err(MeasureError::NeedPoints(3));
    }
    let n = points.len() as f64;
    let c: P3 = std::array::from_fn(|d| points.iter().map(|p| p[d]).sum::<f64>() / n);
    let mut m = [[0.0; 3]; 3];
    for p in points {
        let r = sub(*p, c);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += r[i] * r[j];
            }
        }
    }
    let (vals, vecs) = eigen_sym(m);
    let order = {
        let mut o = [0, 1, 2];
        o.sort_by(|&x, &y| vals[x].total_cmp(&vals[y]));
        o
    };
    // Collinear points: the two smallest spreads are both ~0.
    if vals[order[1]] <= 1e-18 * vals[order[2]].max(1e-300) {
        return Err(MeasureError::Degenerate(
            "the points are collinear or coincident",
        ));
    }
    let mut normal = vecs[order[0]];
    let len = norm(normal);
    normal = normal.map(|v| v / len);
    if normal[2] < 0.0
        || (normal[2] == 0.0 && (normal[1] < 0.0 || (normal[1] == 0.0 && normal[0] < 0.0)))
    {
        normal = normal.map(|v| -v);
    }
    let dists: Vec<f64> = points.iter().map(|p| dot(sub(*p, c), normal)).collect();
    Ok(Plane {
        point: c,
        normal,
        rms: (dists.iter().map(|d| d * d).sum::<f64>() / n).sqrt(),
        max_abs: dists.iter().map(|d| d.abs()).fold(0.0, f64::max),
    })
}

fn area_on_plane(pts: &[P3]) -> Result<(f64, Plane), MeasureError> {
    let plane = fit_plane(pts)?;
    let n = plane.normal;
    // Newell's method: area vector of the closed polygon; its component along the plane
    // normal is the area of the polygon projected onto the plane.
    let mut s = [0.0; 3];
    for i in 0..pts.len() {
        let c = cross(pts[i], pts[(i + 1) % pts.len()]);
        s = [s[0] + c[0], s[1] + c[1], s[2] + c[2]];
    }
    Ok((dot(s, n).abs() / 2.0, plane))
}

/// Area of a simple (non-self-intersecting) polygon given in order, projected onto its
/// best-fit plane. The plane's residuals show how far the polygon is from flat.
pub fn polygon_area(vertices: &[P3], sigma_point: f64) -> Result<Area, MeasureError> {
    let (value, plane) = area_on_plane(vertices)?;
    let f = |p: &[P3]| area_on_plane(p).map(|a| a.0).unwrap_or(f64::NAN);
    let perimeter = (0..vertices.len())
        .map(|i| norm(sub(vertices[(i + 1) % vertices.len()], vertices[i])))
        .sum();
    Ok(Area {
        area: Measured {
            value,
            sigma: propagate(vertices, sigma_point, &f),
        },
        perimeter,
        plane,
    })
}

/// Signed height of `p` above the plane fitted to `plane_points`. The uncertainty covers
/// both the point and the fitted plane (from the plane points' own uncertainty).
pub fn height_above_plane(
    plane_points: &[P3],
    p: P3,
    sigma_point: f64,
) -> Result<Height, MeasureError> {
    let plane = fit_plane(plane_points)?;
    let n = plane_points.len();
    let f = |q: &[P3]| {
        fit_plane(&q[..n])
            .map(|pl| dot(sub(q[n], pl.point), pl.normal))
            .unwrap_or(f64::NAN)
    };
    let mut all = plane_points.to_vec();
    all.push(p);
    Ok(Height {
        height: Measured {
            value: f(&all),
            sigma: propagate(&all, sigma_point, &f),
        },
        plane,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const S: f64 = 0.002;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn distance_hand_values() {
        // 3-4-12 gives 13; two independent points: σ = √2 σ_point.
        let d = distance([1.0, 1.0, 1.0], [4.0, 5.0, 13.0], S);
        assert!(close(d.value, 13.0, 1e-12));
        assert!(close(d.sigma, 2f64.sqrt() * S, 1e-9));
    }

    #[test]
    fn angle_hand_values() {
        // Right angle with unit arms: σ_θ² = σ²(2/L1² + 2/L2²) = 4σ², so σ_θ = 2σ.
        let a = angle([1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], S).unwrap();
        assert!(close(a.value, std::f64::consts::FRAC_PI_2, 1e-12));
        assert!(close(a.sigma, 2.0 * S, 1e-8));
        // Equilateral triangle: 60°.
        let e = angle([2.0, 0.0, 5.0], [0.0, 0.0, 5.0], [1.0, 3f64.sqrt(), 5.0], S).unwrap();
        assert!(close(e.value.to_degrees(), 60.0, 1e-10));
        assert!(angle([0.0; 3], [0.0; 3], [1.0, 0.0, 0.0], S).is_err());
    }

    #[test]
    fn area_hand_values() {
        let square = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let a = polygon_area(&square, S).unwrap();
        assert!(close(a.area.value, 1.0, 1e-12));
        assert!(close(a.perimeter, 4.0, 1e-12));
        assert!(a.plane.rms < 1e-12);
        let tri = polygon_area(&[[0.0, 0.0, 2.0], [3.0, 0.0, 2.0], [0.0, 4.0, 2.0]], S).unwrap();
        assert!(close(tri.area.value, 6.0, 1e-12));
        // Unit square standing on a wall (x = 7): still 1 m², normal along x.
        let wall = [
            [7.0, 0.0, 0.0],
            [7.0, 1.0, 0.0],
            [7.0, 1.0, 1.0],
            [7.0, 0.0, 1.0],
        ];
        let w = polygon_area(&wall, S).unwrap();
        assert!(close(w.area.value, 1.0, 1e-12));
        assert!(close(w.plane.normal[0].abs(), 1.0, 1e-12));
        // A warped quad reports its misfit instead of hiding it.
        let warped = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.1],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.1],
        ];
        assert!(polygon_area(&warped, S).unwrap().plane.rms > 0.04);
    }

    #[test]
    fn height_hand_values() {
        let floor = [
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ];
        let h = height_above_plane(&floor, [0.0, 0.0, 1.5], S).unwrap();
        assert!(close(h.height.value, 1.5, 1e-12));
        // Above the centroid the tilt terms vanish: σ² = σ²_point (1 + 1/n), n = 4.
        assert!(
            close(h.height.sigma, S * (1.25f64).sqrt(), 1e-8),
            "{}",
            h.height.sigma
        );
        let below = height_above_plane(&floor, [0.3, 0.2, -0.25], S).unwrap();
        assert!(close(below.height.value, -0.25, 1e-12));
    }

    #[test]
    fn plane_fit_rejects_degenerate_input() {
        assert_eq!(
            fit_plane(&[[0.0; 3], [1.0, 0.0, 0.0]]),
            Err(MeasureError::NeedPoints(3))
        );
        assert!(fit_plane(&[[0.0; 3], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]]).is_err());
    }

    #[test]
    fn far_from_origin_keeps_millimetres() {
        // UTM-sized coordinates: f64 keeps sub-micrometre precision here.
        let a = [500_000.123_4, 4_400_000.567_8, 312.25];
        let b = [500_003.123_4, 4_400_004.567_8, 312.25];
        assert!(close(distance(a, b, S).value, 5.0, 1e-7));
    }

    fn rotate(p: P3, yaw: f64, pitch: f64) -> P3 {
        let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
        let q = [cy * p[0] - sy * p[1], sy * p[0] + cy * p[1], p[2]];
        [q[0], cp * q[1] - sp * q[2], sp * q[1] + cp * q[2]]
    }

    proptest! {
        #[test]
        fn rigid_motion_changes_nothing(
            pts in prop::collection::vec(prop::array::uniform3(-50.0f64..50.0), 3..6),
            yaw in -3.0f64..3.0, pitch in -3.0f64..3.0, t in prop::array::uniform3(-1e5f64..1e5),
        ) {
            let moved: Vec<P3> = pts.iter().map(|p| { let r = rotate(*p, yaw, pitch); [r[0] + t[0], r[1] + t[1], r[2] + t[2]] }).collect();
            let d0 = distance(pts[0], pts[1], S).value;
            prop_assert!(close(distance(moved[0], moved[1], S).value, d0, 1e-6 * d0.max(1.0)));
            if let (Ok(a0), Ok(a1)) = (angle(pts[0], pts[1], pts[2], S), angle(moved[0], moved[1], moved[2], S)) {
                prop_assert!(close(a0.value, a1.value, 1e-6));
            }
            if let (Ok(p0), Ok(p1)) = (fit_plane(&pts), fit_plane(&moved)) {
                prop_assert!(close(p0.rms, p1.rms, 1e-6));
            }
        }

        #[test]
        fn coplanar_points_fit_exactly(
            uv in prop::collection::vec((-10.0f64..10.0, -10.0f64..10.0), 3..20),
            yaw in -3.0f64..3.0, pitch in -1.5f64..1.5,
        ) {
            let pts: Vec<P3> = uv.iter().map(|(u, v)| rotate([*u, *v, 0.0], yaw, pitch)).collect();
            if let Ok(p) = fit_plane(&pts) {
                prop_assert!(p.rms < 1e-9);
                prop_assert!(close(norm(p.normal), 1.0, 1e-12));
            }
        }

        #[test]
        fn angle_stays_in_range(a in prop::array::uniform3(-5.0f64..5.0), c in prop::array::uniform3(-5.0f64..5.0)) {
            if let Ok(m) = angle(a, [0.0; 3], c, S) {
                prop_assert!((0.0..=std::f64::consts::PI).contains(&m.value));
                prop_assert!(m.sigma >= 0.0);
            }
        }
    }
}
