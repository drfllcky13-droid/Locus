//! Registering an undamaged reference (an exemplar vehicle's scan) onto a damaged one for a
//! crush comparison: a rigid fit to picked pairs of undamaged features, then point-to-plane ICP
//! on the surfaces outside the damage region, so the damage doesn't pull the fit.

use crate::icp::{icp, IcpParams, IcpResult};
use crate::normals::{estimate, tree, voxel_downsample};
use crate::rigid::{fit, RigidFit};
use nalgebra::{Isometry3, Matrix6, Point3};

/// Registration uncertainty is scaled up as if each patch this size (m) were one independent
/// observation: neighbouring points share their errors, so the formal ICP covariance (every
/// pair independent) is too small.
pub const PATCH: f64 = 0.1;

#[derive(Debug, Clone)]
pub struct Aligned {
    /// Maps reference coordinates onto the damaged scan's.
    pub transform: Isometry3<f64>,
    /// The fit to the picked pairs (the starting pose).
    pub pairs: RigidFit,
    pub icp: IcpResult,
    /// The ICP covariance scaled by `inflation` (pairs per independent patch), about `about`.
    pub covariance: Matrix6<f64>,
    pub about: Point3<f64>,
    pub inflation: f64,
}

fn outside(p: &[f64; 3], lo: &[f64; 3], hi: &[f64; 3]) -> bool {
    (0..3).any(|k| p[k] < lo[k] || p[k] > hi[k])
}

/// Register `reference` onto `damaged` (both near the vehicle; the damaged scan's frame is the
/// result's). `pairs` are (reference, damaged) points on the same undamaged features, at least
/// 3; the box `lo`–`hi` (damaged frame) is the damage, left out of the ICP. `view` is the
/// damaged scan's scanner position (normals face it); `spacing` thins the reference for ICP.
pub fn align_exemplar(
    reference: &[[f64; 3]],
    damaged: &[[f64; 3]],
    view: [f64; 3],
    pairs: &[([f64; 3], [f64; 3])],
    lo: [f64; 3],
    hi: [f64; 3],
    spacing: f64,
) -> Result<Aligned, String> {
    let from: Vec<Point3<f64>> = pairs.iter().map(|p| p.0.into()).collect();
    let to: Vec<Point3<f64>> = pairs.iter().map(|p| p.1.into()).collect();
    let start = fit(&from, &to).map_err(|e| format!("the picked pairs: {e}"))?;
    let init = start.transform;
    let source: Vec<[f64; 3]> = voxel_downsample(reference, spacing)
        .into_iter()
        .filter(|p| outside(&(init * Point3::from(*p)).coords.into(), &lo, &hi))
        .collect();
    // The target at full density: thinning it moves the tangent planes between iterations and
    // the ICP stops converging.
    let target: Vec<[f64; 3]> = damaged
        .iter()
        .filter(|p| outside(p, &lo, &hi))
        .copied()
        .collect();
    if source.len() < 100 || target.len() < 100 {
        return Err(format!(
            "outside the damage region there are {} reference and {} damaged points to register on; at least 100 of each are needed",
            source.len(),
            target.len()
        ));
    }
    let t = tree(&target);
    let normals = estimate(&target, &t, 12, view);
    let params = IcpParams {
        // The pairs put it within a few centimetres; allow for poorly picked ones.
        max_distance: (3.0 * start.rms).clamp(0.05, 0.5),
        min_distance: (3.0 * spacing).max(0.01),
        pair_radius: 3.0 * spacing,
        ..IcpParams::default()
    };
    let r = icp(&source, &target, &normals, &t, init, &params)
        .ok_or("the ICP found too few overlapping points; check the pairs and the damage region")?;
    let patches = voxel_downsample(&source, PATCH).len().max(1);
    let inflation = (r.pairs as f64 / patches as f64).max(1.0);
    Ok(Aligned {
        transform: r.transform,
        covariance: r.covariance * inflation,
        about: r.about,
        inflation,
        pairs: start,
        icp: r,
    })
}

/// A vehicle's centre plane, fitted to picked pairs of symmetric features (left and right).
#[derive(Debug, Clone, PartialEq)]
pub struct CentrePlane {
    pub point: [f64; 3],
    /// Unit normal, from the first pair's first point toward its second.
    pub normal: [f64; 3],
    /// Each pair's midpoint distance from the plane (m), and the angle between its line and
    /// the normal (°): how well the pairs agree on the plane.
    pub midpoint_offsets: Vec<f64>,
    pub pair_angles_deg: Vec<f64>,
}

/// The plane halfway between symmetric pairs: its normal the mean of the pairs' directions,
/// through their midpoints' centroid. At least 2 pairs (3 or more to check them).
pub fn centre_plane(pairs: &[([f64; 3], [f64; 3])]) -> Result<CentrePlane, String> {
    if pairs.len() < 2 {
        return Err("pick at least 2 pairs of symmetric features (3 or more to check them)".into());
    }
    let v = |p: [f64; 3]| nalgebra::Vector3::from(p);
    let first = v(pairs[0].1) - v(pairs[0].0);
    let mut n = nalgebra::Vector3::zeros();
    for (a, b) in pairs {
        let d = v(*b) - v(*a);
        if d.norm() < 0.05 {
            return Err("a symmetric pair's points are less than 5 cm apart".into());
        }
        let d = d.normalize();
        n += if d.dot(&first) < 0.0 { -d } else { d };
    }
    let n = n.normalize();
    let mids: Vec<_> = pairs.iter().map(|(a, b)| (v(*a) + v(*b)) / 2.0).collect();
    let c = mids.iter().fold(nalgebra::Vector3::zeros(), |s, m| s + m) / mids.len() as f64;
    Ok(CentrePlane {
        point: c.into(),
        normal: n.into(),
        midpoint_offsets: mids.iter().map(|m| n.dot(&(m - c))).collect(),
        pair_angles_deg: pairs
            .iter()
            .map(|(a, b)| {
                let d = (v(*b) - v(*a)).normalize();
                d.dot(&n).abs().min(1.0).acos().to_degrees()
            })
            .collect(),
    })
}

/// `p` reflected across the plane.
pub fn reflect(p: [f64; 3], plane: &CentrePlane) -> [f64; 3] {
    let d: f64 = (0..3)
        .map(|k| (p[k] - plane.point[k]) * plane.normal[k])
        .sum();
    std::array::from_fn(|k| p[k] - 2.0 * d * plane.normal[k])
}

/// Patches this size (m) average the mirrored surface's offset from the original.
pub const SYMMETRY_PATCH: f64 = 0.05;

/// How far the reflected, registered surface stands off the original where both are
/// undamaged: each reflected point's distance along the original's normal to its nearest
/// original point (within 3 × `spacing`), averaged per 5 cm patch (at least 5 points), RMS
/// over the patches, less what the points' noise leaves in a patch mean.
#[allow(clippy::too_many_arguments)]
fn asymmetry(
    reference: &[[f64; 3]],
    points: &[[f64; 3]],
    view: [f64; 3],
    aligned: &Aligned,
    lo: [f64; 3],
    hi: [f64; 3],
    spacing: f64,
    point_sigma: f64,
) -> f64 {
    let target: Vec<[f64; 3]> = points
        .iter()
        .filter(|p| outside(p, &lo, &hi))
        .copied()
        .collect();
    let t = tree(&target);
    let normals = estimate(&target, &t, 12, view);
    let mut patches: std::collections::HashMap<[i64; 3], (f64, usize)> = Default::default();
    for r in voxel_downsample(reference, spacing / 2.0) {
        let m: [f64; 3] = (aligned.transform * Point3::from(r)).coords.into();
        if !outside(&m, &lo, &hi) {
            continue;
        }
        let (k, d2) = crate::normals::nearest(&t, &m);
        let Some(n) = normals[k] else { continue };
        if d2 > (3.0 * spacing).powi(2) {
            continue;
        }
        let q = target[k];
        let res: f64 = (0..3).map(|i| (m[i] - q[i]) * n[i]).sum();
        let e = patches
            .entry(m.map(|v| (v / SYMMETRY_PATCH).floor() as i64))
            .or_default();
        e.0 += res;
        e.1 += 1;
    }
    let means: Vec<(f64, usize)> = patches
        .values()
        .filter(|(_, n)| *n >= 5)
        .map(|(s, n)| (s / *n as f64, *n))
        .collect();
    if means.is_empty() {
        return 0.0;
    }
    let k = means.len() as f64;
    let ms = means.iter().map(|(m, _)| m * m).sum::<f64>() / k;
    let noise = means
        .iter()
        .map(|(_, n)| 2.0 * point_sigma * point_sigma / *n as f64)
        .sum::<f64>()
        / k;
    (ms - noise).max(0.0).sqrt()
}

#[derive(Debug, Clone)]
pub struct Mirrored {
    pub plane: CentrePlane,
    /// The vehicle's points outside the damage region, reflected (before `aligned.transform`).
    pub reference: Vec<[f64; 3]>,
    pub aligned: Aligned,
    /// 1σ of the asymmetry left on the undamaged surfaces after registration (m).
    pub symmetry_sigma: f64,
}

/// The damaged vehicle's own undamaged side as its reference: its points outside the damage
/// region reflected across the centre plane fitted to `pairs` (symmetric features, left and
/// right), then registered by ICP onto the undamaged surfaces as an exemplar would be. The
/// reflected damage never enters: points inside the region are left out before reflecting.
pub fn align_mirror(
    points: &[[f64; 3]],
    view: [f64; 3],
    pairs: &[([f64; 3], [f64; 3])],
    lo: [f64; 3],
    hi: [f64; 3],
    spacing: f64,
    point_sigma: f64,
) -> Result<Mirrored, String> {
    let plane = centre_plane(pairs)?;
    let reference: Vec<[f64; 3]> = points
        .iter()
        .filter(|p| outside(p, &lo, &hi))
        .map(|p| reflect(*p, &plane))
        .collect();
    // Each pair also gives two correspondences for the start: a reflected point lands on its
    // counterpart.
    let starts: Vec<_> = pairs
        .iter()
        .flat_map(|(a, b)| [(reflect(*a, &plane), *b), (reflect(*b, &plane), *a)])
        .collect();
    let aligned = align_exemplar(&reference, points, view, &starts, lo, hi, spacing)?;
    let symmetry_sigma = asymmetry(
        &reference,
        points,
        view,
        &aligned,
        lo,
        hi,
        spacing,
        point_sigma,
    );
    Ok(Mirrored {
        plane,
        reference,
        aligned,
        symmetry_sigma,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::{Translation3, UnitQuaternion, Vector3};

    /// A box's corner: front (x = 0), side (y = 0) and top (z = 1) faces, 1 m each way,
    /// sampled every 1 cm with 1 mm noise, and a dent in the front centred at (0, 0.5, 0.5).
    fn corner(dent: bool, seed: u64) -> Vec<[f64; 3]> {
        let mut s = seed;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 1_000_000) as f64 / 1e6 - 0.5
        };
        let mut out = vec![];
        for a in 0..100 {
            for b in 0..100 {
                let (p, q) = (a as f64 * 0.01, b as f64 * 0.01);
                let mut x = rnd() * 0.002;
                let r2 = (p - 0.5).powi(2) + (q - 0.5).powi(2);
                if dent && r2 < 0.04 {
                    x += 0.08 * (1.0 - r2 / 0.04);
                }
                out.push([x, p, q]);
                out.push([p, rnd() * 0.002, q]);
                out.push([p, q, 1.0 + rnd() * 0.002]);
            }
        }
        out
    }

    #[test]
    fn a_reference_is_registered_around_the_damage() {
        let truth = Isometry3::from_parts(
            Translation3::new(3.0, -2.0, 0.1),
            UnitQuaternion::from_euler_angles(0.01, -0.02, 0.6),
        );
        let reference = corner(false, 11);
        let damaged: Vec<[f64; 3]> = corner(true, 23)
            .iter()
            .map(|p| (truth * Point3::from(*p)).coords.into())
            .collect();
        // Four features picked on both, each off by up to ~5 mm.
        let feats = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 1.0],
        ];
        let pairs: Vec<_> = feats
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let off = Vector3::new(0.004, -0.003, 0.002) * (k as f64 - 1.5);
                (*f, (truth * (Point3::from(*f) + off)).coords.into())
            })
            .collect();
        let dent_box = |t: &Isometry3<f64>| {
            let c = t * Point3::new(0.0, 0.5, 0.5);
            (
                [c.x - 0.3, c.y - 0.3, c.z - 0.3],
                [c.x + 0.3, c.y + 0.3, c.z + 0.3],
            )
        };
        let (lo, hi) = dent_box(&truth);
        let view = (truth * Point3::new(-5.0, -5.0, 3.0)).coords.into();
        let a = align_exemplar(&reference, &damaged, view, &pairs, lo, hi, 0.02).unwrap();
        let d = a.transform * truth.inverse();
        assert!(d.translation.vector.norm() < 0.001, "{:?}", d.translation);
        assert!(d.rotation.angle() < 0.001, "{}", d.rotation.angle());
        assert!(a.icp.converged);
        assert!(a.inflation > 1.0);
    }

    #[test]
    fn the_centre_plane_comes_from_symmetric_pairs() {
        // Plane y = 2, pairs off by a few millimetres.
        let pairs = [
            ([1.0, 1.0, 0.5], [1.0, 3.002, 0.5]),
            ([0.0, 1.2, 1.0], [0.001, 2.8, 1.0]),
            ([2.0, 0.5, 0.2], [2.0, 3.5, 0.2]),
        ];
        let p = centre_plane(&pairs).unwrap();
        assert!((p.normal[1] - 1.0).abs() < 1e-3, "{:?}", p.normal);
        assert!((p.point[1] - 2.0).abs() < 2e-3, "{:?}", p.point);
        let r = reflect([5.0, 1.5, 3.0], &p);
        assert!((r[1] - 2.5).abs() < 5e-3, "{r:?}");
        assert!(p.midpoint_offsets.iter().all(|d| d.abs() < 2e-3));
    }
}
