//! Sphere target detection in a single scan.
//!
//! 1. Each point with a normal votes for a centre one nominal radius behind its surface
//!    (`p − r·n`, with `n` oriented toward the scanner). Points on a sphere of that radius
//!    vote for the same place; points on other surfaces spread their votes out.
//! 2. Vote peaks, strongest first, are verified: the points near the candidate are fitted
//!    with a geometric least-squares sphere, trimming outliers as the fit tightens.
//! 3. A candidate is accepted when the fit with free radius agrees with the nominal radius,
//!    the RMS residual is small, enough points support it, and they cover a real cap of the
//!    sphere, not a sliver. The reported centre is the fit with the radius held at the
//!    nominal value, which is better conditioned for a partial cap.

use crate::normals::{within, Tree};
use nalgebra::{Matrix3, Matrix4, Vector3, Vector4};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct SphereSearch {
    /// Nominal target radius (m).
    pub radius: f64,
    /// Largest accepted difference between the free-radius fit and `radius` (m).
    pub radius_tolerance: f64,
    /// Fewest supporting points.
    pub min_points: usize,
    /// Largest accepted RMS of the fit residuals (m).
    pub max_rms: f64,
}

impl SphereSearch {
    pub fn new(radius: f64) -> Self {
        SphereSearch {
            radius,
            radius_tolerance: 0.003,
            min_points: 30,
            max_rms: 0.005,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Sphere {
    /// Centre in the scan's frame (m), fitted with the radius held at nominal.
    pub centre: [f64; 3],
    /// Covariance of `centre` (m²), from the fit residuals.
    pub covariance: Matrix3<f64>,
    /// Radius from the free fit, used as a check (m).
    pub fitted_radius: f64,
    /// RMS of the residuals at the reported centre (m).
    pub rms: f64,
    /// Points supporting the fit.
    pub points: usize,
}

/// Find spheres of `search.radius` among `points` (with `normals` from
/// [`crate::normals::estimate`]). Strongest first.
pub fn detect(
    points: &[[f64; 3]],
    tree: &Tree,
    normals: &[Option<[f64; 3]>],
    search: &SphereSearch,
) -> Vec<Sphere> {
    let r = search.radius;
    let cell = r / 3.0;
    let key = |c: Vector3<f64>| (c / cell).map(|v| v.floor() as i64);
    let mut votes: HashMap<[i64; 3], (u32, Vector3<f64>)> = HashMap::new();
    for (p, n) in points.iter().zip(normals) {
        let Some(n) = n else { continue };
        let c = Vector3::from(*p) - Vector3::from(*n) * r;
        let k = key(c);
        let e = votes.entry([k.x, k.y, k.z]).or_default();
        e.0 += 1;
        e.1 += c;
    }
    let min_votes = (search.min_points / 4).max(5) as u32;
    let mut peaks: Vec<([i64; 3], u32)> = votes
        .iter()
        .filter(|(_, v)| v.0 >= min_votes)
        .map(|(k, v)| (*k, v.0))
        .collect();
    peaks.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let mut tried: Vec<Vector3<f64>> = vec![];
    let mut found: Vec<Sphere> = vec![];
    for (k, _) in peaks {
        // Start from the mean vote over the peak cell and its neighbours.
        let (mut n, mut sum) = (0u32, Vector3::zeros());
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = votes.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) {
                        n += v.0;
                        sum += v.1;
                    }
                }
            }
        }
        let c0 = sum / n as f64;
        if tried.iter().any(|t| (t - c0).norm() < r) {
            continue;
        }
        tried.push(c0);
        if let Some(s) = verify(points, tree, c0, search) {
            if !found
                .iter()
                .any(|f| (Vector3::from(f.centre) - Vector3::from(s.centre)).norm() < r)
            {
                found.push(s);
            }
        }
    }
    found
}

/// Gauss–Newton geometric sphere fit. With `radius = Some(r)` the radius is fixed.
/// Returns centre, radius and the normal matrix JᵀJ for the centre.
fn fit(
    pts: &[Vector3<f64>],
    mut c: Vector3<f64>,
    radius: Option<f64>,
    r0: f64,
) -> (Vector3<f64>, f64, Matrix3<f64>) {
    let mut r = radius.unwrap_or(r0);
    let mut jtj3 = Matrix3::zeros();
    for _ in 0..20 {
        let (mut jtj, mut jte) = (Matrix4::zeros(), Vector4::zeros());
        jtj3 = Matrix3::zeros();
        for p in pts {
            let d = c - p;
            let dist = d.norm();
            if dist < 1e-12 {
                continue;
            }
            let u = d / dist;
            let j = Vector4::new(u.x, u.y, u.z, -1.0);
            let e = dist - r;
            jtj += j * j.transpose();
            jte += j * e;
            jtj3 += u * u.transpose();
        }
        let step = if radius.is_some() {
            match jtj.fixed_view::<3, 3>(0, 0).into_owned().try_inverse() {
                Some(inv) => {
                    let s = -(inv * jte.fixed_rows::<3>(0));
                    Vector4::new(s.x, s.y, s.z, 0.0)
                }
                None => break,
            }
        } else {
            match jtj.try_inverse() {
                Some(inv) => -(inv * jte),
                None => break,
            }
        };
        c += step.fixed_rows::<3>(0);
        r += step.w;
        if step.norm() < 1e-10 {
            break;
        }
    }
    (c, r, jtj3)
}

fn verify(
    points: &[[f64; 3]],
    tree: &Tree,
    c0: Vector3<f64>,
    search: &SphereSearch,
) -> Option<Sphere> {
    let r = search.radius;
    let near: Vec<Vector3<f64>> = within(tree, &c0.into(), r * 1.5)
        .into_iter()
        .map(|i| Vector3::from(points[i]))
        .collect();
    let shell = |c: Vector3<f64>, gate: f64| -> Vec<Vector3<f64>> {
        near.iter()
            .copied()
            .filter(|p| ((p - c).norm() - r).abs() < gate)
            .collect()
    };
    let (mut c, mut gate) = (c0, r * 0.3);
    for _ in 0..6 {
        let inliers = shell(c, gate);
        if inliers.len() < search.min_points {
            return None;
        }
        c = fit(&inliers, c, Some(r), r).0;
        gate = (3.0 * rms(&inliers, c, r)).max(0.002).min(gate);
    }
    let inliers = shell(c, gate);
    if inliers.len() < search.min_points {
        return None;
    }
    let (c, _, jtj) = fit(&inliers, c, Some(r), r);
    let (_, fitted_radius, _) = fit(&inliers, c, None, r);
    let rms = rms(&inliers, c, r);
    // Cap coverage: the mean unit direction from the centre is (1 + cos α) / 2 for a cap of
    // half-angle α. Require α ≥ 30°.
    let mean = inliers
        .iter()
        .fold(Vector3::zeros(), |s, p| s + (p - c).normalize())
        / inliers.len() as f64;
    let cap_ok = mean.norm() <= (1.0 + 30f64.to_radians().cos()) / 2.0;
    if (fitted_radius - r).abs() > search.radius_tolerance || rms > search.max_rms || !cap_ok {
        return None;
    }
    let n = inliers.len();
    let s2 = inliers
        .iter()
        .map(|p| ((p - c).norm() - r).powi(2))
        .sum::<f64>()
        / (n - 3) as f64;
    Some(Sphere {
        centre: c.into(),
        covariance: jtj.try_inverse()? * s2,
        fitted_radius,
        rms,
        points: n,
    })
}

fn rms(pts: &[Vector3<f64>], c: Vector3<f64>, r: f64) -> f64 {
    (pts.iter()
        .map(|p| ((p - c).norm() - r).powi(2))
        .sum::<f64>()
        / pts.len() as f64)
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normals;
    use locus_synth::{scan_points, truth, Options, SPHERE_RADIUS};

    #[test]
    #[ignore = "heavy: synthetic scans of millions of points (see .config/nextest.toml)"]
    fn heavy_finds_the_synthetic_spheres_at_their_true_centres() {
        let opts = Options {
            scans: 2,
            points_per_scan: 6_000_000,
            seed: 11,
            ..Options::default()
        };
        let t = truth(&opts);
        for s in 0..opts.scans {
            let mut pts = vec![];
            scan_points(&opts, &t, s, &mut |p| {
                if let Some(p) = p {
                    pts.push(p.xyz)
                }
            });
            let tree = normals::tree(&pts);
            let nrm = normals::estimate(&pts, &tree, 12, [0.0; 3]);
            let found = detect(&pts, &tree, &nrm, &SphereSearch::new(SPHERE_RADIUS));
            let inv = t.scans[s].pose.inverse();
            let local: Vec<Vector3<f64>> = (0..t.spheres.len())
                .map(|k| Vector3::from(inv.apply(t.sphere_centre(k, s))))
                .collect();
            // No false detections: every sphere found is a real one, to within 1 mm.
            for f in &found {
                let err = local
                    .iter()
                    .map(|c| (c - Vector3::from(f.centre)).norm())
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    err < 0.001,
                    "scan {s}: detection {:?} is {err} m from any sphere",
                    f.centre
                );
                let predicted = f.covariance.trace().sqrt();
                assert!(predicted < 0.001, "centre σ {predicted}");
            }
            // Every sphere with 50 or more points on its surface (visible, not occluded) is found.
            let mut expected = 0;
            for c in &local {
                let on = pts
                    .iter()
                    .filter(|p| ((Vector3::from(**p) - c).norm() - SPHERE_RADIUS).abs() < 0.004)
                    .count();
                if on >= 50 {
                    expected += 1;
                    assert!(
                        found
                            .iter()
                            .any(|f| (c - Vector3::from(f.centre)).norm() < 0.001),
                        "scan {s}: sphere at {:.1} m with {on} points not found",
                        c.norm()
                    );
                }
            }
            assert!(
                expected > 0,
                "scan {s} sees no sphere well; the test proves nothing"
            );
        }
    }
}
