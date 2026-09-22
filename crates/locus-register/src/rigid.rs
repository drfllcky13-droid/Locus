//! Least-squares rigid transform between corresponding point sets (Kabsch / Umeyama without
//! scale), with residuals and the covariance of the result.

use nalgebra::{
    Isometry3, Matrix3, Matrix6, Point3, Rotation3, SymmetricEigen, Translation3, Vector3,
};

#[derive(Debug, Clone, PartialEq)]
pub enum FitError {
    /// Fewer than 3 correspondences.
    TooFew(usize),
    /// The points are (nearly) collinear, so the rotation about their line is undetermined.
    Degenerate,
}

impl std::fmt::Display for FitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FitError::TooFew(n) => write!(f, "a rigid fit needs at least 3 points, got {n}"),
            FitError::Degenerate => write!(f, "the points are collinear; rotation is undetermined"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RigidFit {
    /// Maps `from` onto `to`: `to[i] ≈ transform * from[i]`.
    pub transform: Isometry3<f64>,
    /// `|transform * from[i] − to[i]|` for each pair (m).
    pub residuals: Vec<f64>,
    pub rms: f64,
    /// A posteriori σ per coordinate, `sqrt(Σ|r|² / (3n − 6))`; `None` with exactly 3 points,
    /// which leave no redundancy to estimate it from.
    pub sigma: Option<f64>,
    /// Covariance of a small correction `(ω, τ)` (rotation vector in rad, translation in m)
    /// applied about `about`: `x' = x + ω × (x − about) + τ`. Scaled by `sigma²`, or by 1
    /// when `sigma` is `None` (multiply by the per-coordinate variance you assume).
    pub covariance: Matrix6<f64>,
    /// Centroid of the `to` points.
    pub about: Point3<f64>,
}

/// Rigid transform minimising `Σ |R·from[i] + t − to[i]|²`.
pub fn fit(from: &[Point3<f64>], to: &[Point3<f64>]) -> Result<RigidFit, FitError> {
    assert_eq!(from.len(), to.len(), "fit needs paired points");
    let n = from.len();
    if n < 3 {
        return Err(FitError::TooFew(n));
    }
    let centroid =
        |p: &[Point3<f64>]| p.iter().fold(Vector3::zeros(), |s, q| s + q.coords) / n as f64;
    let (ca, cb) = (centroid(from), centroid(to));
    let (mut h, mut scatter) = (Matrix3::zeros(), Matrix3::zeros());
    for (a, b) in from.iter().zip(to) {
        let (da, db) = (a.coords - ca, b.coords - cb);
        h += da * db.transpose();
        scatter += da * da.transpose();
    }
    // Collinear (or coincident) points: the second-largest spread is ~0.
    let mut ev = SymmetricEigen::new(scatter).eigenvalues;
    ev.as_mut_slice().sort_by(|a, b| b.total_cmp(a));
    if ev[1] <= ev[0] * 1e-12 {
        return Err(FitError::Degenerate);
    }
    let svd = h.svd(true, true);
    let (u, v_t) = (svd.u.expect("u"), svd.v_t.expect("v_t"));
    let mut d = Matrix3::identity();
    // Guard against a reflection.
    d[(2, 2)] = (v_t.transpose() * u.transpose()).determinant().signum();
    let r = Rotation3::from_matrix_unchecked(v_t.transpose() * d * u.transpose());
    let t = cb - r * ca;
    let transform = Isometry3::from_parts(Translation3::from(t), r.into());

    let residuals: Vec<f64> = from
        .iter()
        .zip(to)
        .map(|(a, b)| (transform * a - b).norm())
        .collect();
    let ss: f64 = residuals.iter().map(|r| r * r).sum();
    let rms = (ss / n as f64).sqrt();
    let sigma = (n > 3).then(|| (ss / (3 * n - 6) as f64).sqrt());

    // Normal matrix of the linearised problem about the centroid: J_i = [−[x_i − c]×, I].
    // The cross blocks are −Σ[x_i − c]×, which is zero about the centroid.
    let mut normal = Matrix6::zeros();
    let rot: Matrix3<f64> = to
        .iter()
        .map(|b| {
            let sk = (b.coords - cb).cross_matrix();
            sk.transpose() * sk
        })
        .sum();
    normal.fixed_view_mut::<3, 3>(0, 0).copy_from(&rot);
    normal
        .fixed_view_mut::<3, 3>(3, 3)
        .copy_from(&(Matrix3::identity() * n as f64));
    let covariance =
        normal.try_inverse().ok_or(FitError::Degenerate)? * sigma.map_or(1.0, |s| s * s);
    Ok(RigidFit {
        transform,
        residuals,
        rms,
        sigma,
        covariance,
        about: Point3::from(cb),
    })
}

/// Row-major 4 × 4 matrix, the layout scan poses use elsewhere in Locus.
pub fn to_row_major(t: &Isometry3<f64>) -> [f64; 16] {
    let m = t.to_homogeneous();
    std::array::from_fn(|i| m[(i / 4, i % 4)])
}

pub fn from_row_major(m: &[f64; 16]) -> Isometry3<f64> {
    let r = Matrix3::from_fn(|i, j| m[i * 4 + j]);
    Isometry3::from_parts(
        Translation3::new(m[3], m[7], m[11]),
        Rotation3::from_matrix(&r).into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::{UnitQuaternion, Vector3};

    fn known() -> Isometry3<f64> {
        Isometry3::from_parts(
            Translation3::new(12.5, -3.25, 0.8),
            UnitQuaternion::from_euler_angles(0.02, -0.01, 2.1),
        )
    }

    fn cloud() -> Vec<Point3<f64>> {
        [
            [0.0, 0.0, 0.0],
            [4.0, 0.3, 0.1],
            [1.0, 5.0, -0.4],
            [-3.0, 2.0, 1.5],
            [2.2, -4.1, 2.8],
            [-1.5, -2.5, 0.2],
        ]
        .map(|p| Point3::new(p[0], p[1], p[2]))
        .to_vec()
    }

    #[test]
    fn recovers_a_known_transform_exactly() {
        let from = cloud();
        let to: Vec<_> = from.iter().map(|p| known() * p).collect();
        let f = fit(&from, &to).unwrap();
        let d = f.transform.inverse() * known();
        assert!(d.translation.vector.norm() < 1e-12);
        assert!(d.rotation.angle() < 1e-12);
        assert!(f.rms < 1e-12);
        assert!(f.residuals.len() == 6);
    }

    #[test]
    fn works_far_from_the_origin() {
        // Georeferenced coordinates (UTM-like): precision must not suffer.
        let off = Vector3::new(500_000.0, 4_400_000.0, 250.0);
        let from: Vec<_> = cloud().iter().map(|p| p + off).collect();
        let to: Vec<_> = from.iter().map(|p| known() * p).collect();
        let f = fit(&from, &to).unwrap();
        assert!(f.rms < 1e-6, "rms {}", f.rms);
    }

    #[test]
    fn refuses_too_few_and_collinear_points() {
        let p = |x: f64| Point3::new(x, 2.0 * x, -x);
        assert_eq!(
            fit(&[p(0.0), p(1.0)], &[p(0.0), p(1.0)]).unwrap_err(),
            FitError::TooFew(2)
        );
        let line = [p(0.0), p(1.0), p(2.5), p(4.0)];
        assert_eq!(fit(&line, &line).unwrap_err(), FitError::Degenerate);
    }

    #[test]
    fn sigma_and_covariance_match_monte_carlo() {
        // Deterministic noise (xorshift + Box–Muller), σ = 2 mm on the target points.
        let mut s = 0x2545_f491_4f6c_dd1du64;
        let mut unit = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut normal = || {
            let (u, v) = (unit().max(1e-300), unit());
            (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
        };
        let sigma = 0.002;
        let from = cloud();
        let (mut sum_s, mut tx, mut tx2, mut predicted) = (0.0, 0.0, 0.0, 0.0);
        let runs = 4000;
        for _ in 0..runs {
            let to: Vec<_> = from
                .iter()
                .map(|p| known() * p + Vector3::new(normal(), normal(), normal()) * sigma)
                .collect();
            let f = fit(&from, &to).unwrap();
            sum_s += f.sigma.unwrap();
            // x-translation of the centroid, the τ_x the covariance describes.
            let c = f.transform
                * Point3::from(from.iter().fold(Vector3::zeros(), |a, p| a + p.coords) / 6.0);
            let truth = known()
                * Point3::from(from.iter().fold(Vector3::zeros(), |a, p| a + p.coords) / 6.0);
            let e = c.x - truth.x;
            tx += e;
            tx2 += e * e;
            predicted += f.covariance[(3, 3)] / (f.sigma.unwrap().powi(2)) * sigma * sigma;
        }
        let mean_sigma = sum_s / runs as f64;
        assert!((mean_sigma / sigma - 1.0).abs() < 0.05, "σ̂ {mean_sigma}");
        let var = tx2 / runs as f64 - (tx / runs as f64).powi(2);
        let pred = predicted / runs as f64;
        assert!(
            (var / pred - 1.0).abs() < 0.1,
            "var {var} vs predicted {pred}"
        );
    }

    #[test]
    fn row_major_round_trip() {
        let m = to_row_major(&known());
        let p = Point3::new(1.0, 2.0, 3.0);
        let q = known() * p;
        assert!((m[0] * p.x + m[1] * p.y + m[2] * p.z + m[3] - q.x).abs() < 1e-12);
        let back = from_row_major(&m);
        assert!((back * p - q).norm() < 1e-12);
    }
}
