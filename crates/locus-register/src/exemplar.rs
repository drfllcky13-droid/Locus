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
}
