//! Point-to-plane ICP: the fine step of cloud-to-cloud registration.
//!
//! Each iteration pairs every source point (transformed by the current estimate) with its
//! nearest target point within a distance threshold, and solves for the small rigid motion
//! that minimises the sum of squared distances from the source points to the target points'
//! tangent planes, `Σ w_i (n_i · (q_i − m_i))²`. Weights are Huber on the residual, with the
//! scale from the median absolute residual, so a few wrong pairs (edges, clutter that moved)
//! don't pull the solution. The threshold shrinks toward `min_distance` as the fit tightens.
//!
//! The reported covariance is the formal one, `σ² (JᵀWJ)⁻¹`. It treats every pair as an
//! independent observation, so it is a lower bound: dense neighbouring points are not
//! independent, and systematic effects (range bias, incidence angle) are not in it.

use crate::normals::{nearest, Tree};
use nalgebra::{
    Isometry3, Matrix6, Point3, Rotation3, Translation3, UnitQuaternion, Vector3, Vector6,
};

#[derive(Debug, Clone, PartialEq)]
pub struct IcpParams {
    /// Starting pairing threshold (m): larger than the initial misalignment.
    pub max_distance: f64,
    /// The threshold never shrinks below this (m); a few times the range noise. It gates the
    /// point-to-plane distance.
    pub min_distance: f64,
    /// A pair's points may be this far apart along the surface (m), so sparse areas far from
    /// the scanner still pair; a few times the coarsest point spacing.
    pub pair_radius: f64,
    pub max_iterations: usize,
    /// Stop when an update moves points by less than this (m, at 10 m from the centroid).
    pub tolerance: f64,
}

impl Default for IcpParams {
    fn default() -> Self {
        IcpParams {
            max_distance: 0.5,
            min_distance: 0.02,
            pair_radius: 0.15,
            max_iterations: 60,
            tolerance: 1e-6,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IcpResult {
    /// Maps source coordinates onto target coordinates.
    pub transform: Isometry3<f64>,
    /// RMS point-to-plane distance of the final pairs (m).
    pub rms: f64,
    /// Pairs used in the final iteration.
    pub pairs: usize,
    /// Fraction of source points paired in the final iteration (within `pair_radius` and the
    /// final point-to-plane threshold): the share of the source that overlaps the target.
    pub overlap: f64,
    /// Formal covariance of a small correction `(ω, τ)` about `about` (see module docs).
    pub covariance: Matrix6<f64>,
    pub about: Point3<f64>,
    /// How well the weakest direction is constrained: the smallest eigenvalue of the normal
    /// matrix, with rotation expressed as motion at the paired points' RMS radius, divided by
    /// the number of pairs. 1/3 would mean surfaces facing every way equally; near zero means
    /// a motion the geometry barely resists, like sliding along a corridor.
    pub conditioning: f64,
    pub iterations: usize,
    pub converged: bool,
}

pub fn icp(
    source: &[[f64; 3]],
    target: &[[f64; 3]],
    target_normals: &[Option<[f64; 3]>],
    tree: &Tree,
    init: Isometry3<f64>,
    params: &IcpParams,
) -> Option<IcpResult> {
    let mut t = init;
    let mut threshold = params.max_distance;
    let mut last = None;
    for it in 1..=params.max_iterations {
        // Pair and linearise about the centroid of the transformed source.
        let moved: Vec<Point3<f64>> = source.iter().map(|p| t * Point3::from(*p)).collect();
        let c =
            moved.iter().fold(Vector3::zeros(), |s, q| s + q.coords) / moved.len().max(1) as f64;
        let pairs: Vec<(Vector3<f64>, Vector3<f64>, f64)> = moved
            .iter()
            .filter_map(|q| {
                let (i, d2) = nearest(tree, &q.coords.into());
                let reach = threshold.max(params.pair_radius);
                if d2 > reach * reach {
                    return None;
                }
                let n = Vector3::from(target_normals[i]?);
                let r = n.dot(&(q.coords - Vector3::from(target[i])));
                (r.abs() <= threshold).then_some((q.coords - c, n, r))
            })
            .collect();
        if pairs.len() < 6 {
            return None;
        }
        let mut abs: Vec<f64> = pairs.iter().map(|p| p.2.abs()).collect();
        let mid = abs.len() / 2;
        let mad = *abs.select_nth_unstable_by(mid, f64::total_cmp).1;
        let scale = (1.4826 * mad).max(1e-4);
        let k = 1.345 * scale;
        let (mut a, mut b) = (Matrix6::zeros(), Vector6::zeros());
        let (mut wsum, mut wr2) = (0.0, 0.0);
        for (x, n, r) in &pairs {
            let xn = x.cross(n);
            let j = Vector6::new(xn.x, xn.y, xn.z, n.x, n.y, n.z);
            let w = if r.abs() <= k { 1.0 } else { k / r.abs() };
            a += j * j.transpose() * w;
            b += j * (w * r);
            wsum += w;
            wr2 += w * r * r;
        }
        let dx = a.cholesky()?.solve(&(-b));
        let (omega, tau) = (
            dx.fixed_rows::<3>(0).into_owned(),
            dx.fixed_rows::<3>(3).into_owned(),
        );
        let rot = Rotation3::from_scaled_axis(omega);
        let step = Isometry3::from_parts(
            Translation3::from(c + tau - rot * c),
            UnitQuaternion::from_rotation_matrix(&rot),
        );
        t = step * t;
        let rms = (wr2 / wsum).sqrt();
        let moved_by = tau.norm() + omega.norm() * 10.0;
        let overlap = pairs.len() as f64 / source.len() as f64;
        let radius =
            (pairs.iter().map(|p| p.0.norm_squared()).sum::<f64>() / pairs.len() as f64).sqrt();
        last = Some(Fit {
            rms,
            pairs: pairs.len(),
            overlap,
            normal: a,
            about: c,
            variance: wr2 / (wsum - 6.0).max(1.0),
            radius,
        });
        threshold = threshold
            .min((6.0 * scale).max(params.min_distance))
            .max(params.min_distance);
        if moved_by < params.tolerance && threshold <= params.min_distance {
            return Some(result(t, last?, it, true));
        }
    }
    Some(result(t, last?, params.max_iterations, false))
}

/// The last iteration's fit statistics.
struct Fit {
    rms: f64,
    pairs: usize,
    overlap: f64,
    normal: Matrix6<f64>,
    about: Vector3<f64>,
    variance: f64,
    /// RMS distance of the paired points from `about`.
    radius: f64,
}

fn result(t: Isometry3<f64>, f: Fit, iterations: usize, converged: bool) -> IcpResult {
    // Rotation rows scaled so a rotation is measured as motion at the RMS radius.
    let mut s = Matrix6::identity();
    for i in 0..3 {
        s[(i, i)] = 1.0 / f.radius.max(1e-6);
    }
    let ev = (s * f.normal * s).symmetric_eigenvalues();
    IcpResult {
        transform: t,
        rms: f.rms,
        pairs: f.pairs,
        overlap: f.overlap,
        covariance: f.normal.try_inverse().unwrap_or_else(Matrix6::zeros) * f.variance,
        about: Point3::from(f.about),
        conditioning: ev.min() / f.pairs as f64,
        iterations,
        converged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normals;
    use locus_synth::{scan_points, truth, Options, StoredPose};

    pub(crate) fn scan(opts: &Options, t: &locus_synth::Truth, s: usize) -> Vec<[f64; 3]> {
        let mut v = vec![];
        scan_points(opts, t, s, &mut |p| {
            if let Some(p) = p {
                v.push(p.xyz)
            }
        });
        v
    }

    #[test]
    fn refines_a_perturbed_pose_to_the_truth() {
        let opts = Options {
            scans: 2,
            points_per_scan: 1_500_000,
            seed: 5,
            stored_pose: StoredPose::None,
            ..Options::default()
        };
        let t = truth(&opts);
        let (src, dst) = (scan(&opts, &t, 1), scan(&opts, &t, 0));
        let tree = normals::tree(&dst);
        let nrm = normals::estimate(&dst, &tree, 12, [0.0; 3]);
        // True source → target transform, then disturb it by 5 cm and 1°.
        let tr = t.scans[0].pose.inverse().compose(&t.scans[1].pose);
        let q = tr.rotation;
        let truth = Isometry3::from_parts(
            Translation3::from(Vector3::from(tr.translation)),
            UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(q[0], q[1], q[2], q[3])),
        );
        let off = Isometry3::new(
            Vector3::new(0.03, -0.04, 0.01),
            Vector3::new(0.0, 0.01, 0.015),
        );
        let sub = normals::voxel_downsample(&src, 0.05);
        let r = icp(&sub, &dst, &nrm, &tree, off * truth, &IcpParams::default()).expect("icp");
        let d = r.transform.inverse() * truth;
        // Error of a point 10 m out, and the rotation.
        let err = (d * Point3::new(10.0, 0.0, 0.0) - Point3::new(10.0, 0.0, 0.0)).norm();
        assert!(r.converged, "{} iterations", r.iterations);
        assert!(err < 0.002, "{:.3} mm at 10 m", err * 1e3);
        assert!(d.rotation.angle().to_degrees() < 0.02);
        assert!(r.overlap > 0.5, "overlap {}", r.overlap);
        eprintln!(
            "conditioning {:.4}, overlap {:.2}",
            r.conditioning, r.overlap
        );
        assert!(r.conditioning > 0.01, "conditioning {}", r.conditioning);
    }

    #[test]
    fn a_corridor_reads_as_poorly_constrained() {
        // Floor and two walls running along x, 30 m long: sliding along x is unconstrained.
        let mut pts = vec![];
        for i in 0..600 {
            for j in 0..40 {
                let (x, u) = (i as f64 * 0.05, j as f64 * 0.05);
                pts.push([x, u, 0.0]);
                pts.push([x, 0.0, u]);
                pts.push([x, 2.0, u]);
            }
        }
        let tree = normals::tree(&pts);
        let nrm = normals::estimate(&pts, &tree, 12, [15.0, 1.0, 1.0]);
        let sub = normals::voxel_downsample(&pts, 0.2);
        let init = Isometry3::translation(0.01, 0.01, 0.01);
        let r = icp(&sub, &pts, &nrm, &tree, init, &IcpParams::default()).expect("icp");
        assert!(r.conditioning < 0.01, "conditioning {}", r.conditioning);
    }
}
