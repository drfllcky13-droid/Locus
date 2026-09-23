//! Volumetric crush: a damaged vehicle's scan against an undamaged reference already
//! registered to it, over a damage region.
//!
//! The damaged face is taken as a height field over its plane: the plane fitted to the
//! reference's points in the region (normal pointing outward, toward the reference scanner),
//! gridded into square cells. In each cell the reference and damaged surfaces are the median
//! height of their points above the plane, and the crush depth is the reference's minus the
//! damaged's (positive inward). A cell counts as crushed when its neighbours' mean depth is
//! positive, and the crush volume is the sum of those cells' signed depths × cell area; the
//! others are summed as material pushed outward. Deciding from the neighbours, not the cell's
//! own depth, keeps noise from biasing the volume: a cell's noise can't choose which sum it
//! joins, so it averages out in either (Σ max(depth, 0) would add noise on every undamaged
//! cell). Its uncertainty is a seeded Monte Carlo over the registration
//! (the reference moved by draws from the pose covariance, and the cells rebuilt) and each
//! cell's median noise. See docs/methods/crash-volume.md.

use crate::crash::Rng;
use crate::measure::P3;
use crate::trajectory::PointSource;
use nalgebra::{Matrix3, Matrix6, SymmetricEigen, Vector3, Vector6};
use serde::{Deserialize, Serialize};

pub const VOLUME_METHOD: &str = "crush-volume/1";

/// Fewer points than this in a cell and its surface isn't estimated.
const MIN_POINTS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeError(pub String);

impl std::fmt::Display for VolumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(m: impl Into<String>) -> Result<T, VolumeError> {
    Err(VolumeError(m.into()))
}

/// One grid cell with both surfaces measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// Cell indices along the plane's u and v axes.
    pub i: i64,
    pub j: i64,
    /// Crush depth (m, positive inward) and its 1σ from the two medians.
    pub depth: f64,
    pub sigma: f64,
    pub reference_points: usize,
    pub damaged_points: usize,
}

/// A Monte Carlo result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub value: f64,
    pub mean: f64,
    pub sd: f64,
    pub interval95: [f64; 2],
    pub draws: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrushVolume {
    /// The face's plane: a point on it, its in-plane axes and outward normal.
    pub origin: P3,
    pub u: P3,
    pub v: P3,
    pub normal: P3,
    /// Cell size (m).
    pub cell: f64,
    pub cells: Vec<Cell>,
    /// Cells where the reference has a surface but the damaged scan has too few points.
    pub uncovered: usize,
    /// Crush volume (m³): inward, from the cells as measured, with the Monte Carlo.
    pub inward: Estimate,
    /// Material displaced outward (m³), from the cells as measured.
    pub outward: f64,
    /// Deepest cell (m) and the area of cells deeper than 3σ (m²).
    pub max_depth: f64,
    pub crushed_area: f64,
}

/// How the reference may be wrong: its registration, the covariance of a small correction
/// (ω, τ), x' = x + ω × (x − about) + τ, as locus-register reports it; and `surface`, the 1σ
/// of a systematic offset of the whole reference surface along the normal (a mirrored side's
/// asymmetry; 0 for an exemplar), drawn once per Monte Carlo draw for every cell.
#[derive(Debug, Clone, Copy)]
pub struct PoseUncertainty {
    pub covariance: Matrix6<f64>,
    pub about: P3,
    pub surface: f64,
}

fn inside(p: &P3, lo: &P3, hi: &P3) -> bool {
    (0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k])
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Median height per cell and its 1σ (1.2533 × spread / √n; the spread at least the scan's σ).
fn surface(
    pts: &[(i64, i64, f64)],
    point_sigma: f64,
) -> std::collections::BTreeMap<(i64, i64), (f64, f64, usize)> {
    let mut by: std::collections::BTreeMap<(i64, i64), Vec<f64>> = Default::default();
    for &(i, j, h) in pts {
        by.entry((i, j)).or_default().push(h);
    }
    by.into_iter()
        .filter(|(_, h)| h.len() >= MIN_POINTS)
        .map(|(k, mut h)| {
            let n = h.len() as f64;
            let mean = h.iter().sum::<f64>() / n;
            let sd = (h.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
            let m = median(&mut h);
            (k, (m, 1.2533 * sd.max(point_sigma) / n.sqrt(), h.len()))
        })
        .collect()
}

/// Crush volume of `damaged` against `reference` (already registered into the same frame)
/// inside the box `lo`–`hi`. `outward` is any point outside the vehicle (the reference's
/// scanner position), which sets the normal's sign. `pose`, when given, is the registration's
/// uncertainty, drawn in the Monte Carlo.
#[allow(clippy::too_many_arguments)]
pub fn crush_volume(
    reference: &[P3],
    damaged: &[P3],
    lo: P3,
    hi: P3,
    outward: P3,
    cell: f64,
    point_sigma: f64,
    pose: Option<PoseUncertainty>,
    draws: usize,
    seed: u64,
) -> Result<CrushVolume, VolumeError> {
    if !(0.005..=0.2).contains(&cell) {
        return err("the cell size must be between 5 and 200 mm");
    }
    let r: Vec<P3> = reference
        .iter()
        .filter(|p| inside(p, &lo, &hi))
        .copied()
        .collect();
    let d: Vec<P3> = damaged
        .iter()
        .filter(|p| inside(p, &lo, &hi))
        .copied()
        .collect();
    if r.len() < 100 || d.len() < 100 {
        return err(format!(
            "the damage region holds {} reference and {} damaged points; at least 100 of each are needed",
            r.len(),
            d.len()
        ));
    }
    // The face's plane from the reference's points.
    let n = r.len() as f64;
    let c = r
        .iter()
        .fold(Vector3::zeros(), |s, p| s + Vector3::from(*p))
        / n;
    let mut m = Matrix3::zeros();
    for p in &r {
        let q = Vector3::from(*p) - c;
        m += q * q.transpose();
    }
    let e = SymmetricEigen::new(m / n);
    let mut order = [0, 1, 2];
    order.sort_by(|a, b| e.eigenvalues[*b].total_cmp(&e.eigenvalues[*a]));
    let u = e.eigenvectors.column(order[0]).into_owned();
    let mut normal = e.eigenvectors.column(order[2]).into_owned();
    if normal.dot(&(Vector3::from(outward) - c)) < 0.0 {
        normal = -normal;
    }
    let v = normal.cross(&u);
    let bin = |p: Vector3<f64>| -> (i64, i64, f64) {
        let q = p - c;
        (
            (q.dot(&u) / cell).floor() as i64,
            (q.dot(&v) / cell).floor() as i64,
            q.dot(&normal),
        )
    };
    let d_bins: Vec<_> = d.iter().map(|p| bin(Vector3::from(*p))).collect();
    let dam = surface(&d_bins, point_sigma);
    let area = cell * cell;

    // The cells from a set of reference bins.
    let cells_of = |r_bins: &[(i64, i64, f64)]| -> (Vec<Cell>, usize) {
        let refs = surface(r_bins, point_sigma);
        let mut cells = vec![];
        let mut uncovered = 0;
        for (k, (hr, sr, nr)) in &refs {
            match dam.get(k) {
                Some((hd, sd, nd)) => cells.push(Cell {
                    i: k.0,
                    j: k.1,
                    depth: hr - hd,
                    sigma: sr.hypot(*sd),
                    reference_points: *nr,
                    damaged_points: *nd,
                }),
                None => uncovered += 1,
            }
        }
        (cells, uncovered)
    };
    let (cells, uncovered) =
        cells_of(&r.iter().map(|p| bin(Vector3::from(*p))).collect::<Vec<_>>());
    if cells.is_empty() {
        return err("no cell has both surfaces; check the region and the registration");
    }
    // Crushed (inward) and pushed-out volumes, each cell classed by its neighbours' mean depth
    // (its own when none of the 8 has both surfaces).
    let volumes = |cells: &[Cell], depth: &dyn Fn(usize) -> f64| -> (f64, f64) {
        let at: std::collections::HashMap<(i64, i64), usize> = cells
            .iter()
            .enumerate()
            .map(|(k, c)| ((c.i, c.j), k))
            .collect();
        let (mut inward, mut out) = (0.0, 0.0);
        for (k, c) in cells.iter().enumerate() {
            let (mut sum, mut n) = (0.0, 0);
            for di in -1..=1 {
                for dj in -1..=1 {
                    if (di, dj) != (0, 0) {
                        if let Some(&m) = at.get(&(c.i + di, c.j + dj)) {
                            sum += depth(m);
                            n += 1;
                        }
                    }
                }
            }
            let side = if n > 0 { sum } else { depth(k) };
            if side > 0.0 {
                inward += depth(k);
            } else {
                out -= depth(k);
            }
        }
        (inward * area, out * area)
    };

    // Monte Carlo: the reference moved by a pose draw (cells rebuilt), then each cell's depth
    // drawn about its value.
    let chol = pose.map(|p| {
        let cov = p.covariance + Matrix6::identity() * 1e-18;
        (
            cov.cholesky().map(|c| c.l()).unwrap_or_else(|| {
                // Not positive definite: its diagonal alone.
                Matrix6::from_diagonal(&cov.diagonal().map(|x| x.max(0.0).sqrt()))
            }),
            Vector3::from(p.about),
        )
    });
    let surface = pose.map_or(0.0, |p| p.surface);
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut vols = Vec::with_capacity(draws);
    for _ in 0..draws {
        let offset = surface * rng.gauss();
        let drawn = match &chol {
            Some((l, about)) => {
                let x = l * Vector6::from_fn(|_, _| rng.gauss());
                let (w, t) = (x.fixed_rows::<3>(0), x.fixed_rows::<3>(3));
                let bins: Vec<_> = r
                    .iter()
                    .map(|p| {
                        let p = Vector3::from(*p);
                        bin(p + w.cross(&(p - about)) + t)
                    })
                    .collect();
                cells_of(&bins).0
            }
            None => cells.clone(),
        };
        let noisy: Vec<f64> = drawn
            .iter()
            .map(|c| c.depth + offset + c.sigma * rng.gauss())
            .collect();
        vols.push(volumes(&drawn, &|k| noisy[k]).0);
    }
    let estimate = {
        let k = vols.len().max(1) as f64;
        let mean = vols.iter().sum::<f64>() / k;
        let sd = (vols.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (k - 1.0).max(1.0)).sqrt();
        vols.sort_by(|a, b| a.total_cmp(b));
        let q = |f: f64| {
            if vols.is_empty() {
                f64::NAN
            } else {
                vols[((f * (vols.len() - 1) as f64).round() as usize).min(vols.len() - 1)]
            }
        };
        Estimate {
            value: volumes(&cells, &|k| cells[k].depth).0,
            mean,
            sd,
            interval95: [q(0.025), q(0.975)],
            draws,
        }
    };
    Ok(CrushVolume {
        origin: c.into(),
        u: u.into(),
        v: v.into(),
        normal: normal.into(),
        cell,
        outward: volumes(&cells, &|k| cells[k].depth).1,
        max_depth: cells
            .iter()
            .map(|c| c.depth)
            .fold(f64::NEG_INFINITY, f64::max),
        crushed_area: cells.iter().filter(|c| c.depth > 3.0 * c.sigma).count() as f64 * area,
        inward: estimate,
        uncovered,
        cells,
    })
}

/// A picked pair of the same undamaged feature on both scans, and its residual after the
/// registration (m).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairRow {
    pub reference: P3,
    pub damaged: P3,
    pub residual: f64,
    pub reference_source: PointSource,
    pub damaged_source: PointSource,
}

/// How the reference was registered onto the damaged scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registration {
    pub pairs: Vec<PairRow>,
    /// RMS of the pairs after the fit to them alone (m).
    pub pairs_rms: f64,
    /// Point-to-plane ICP on the surfaces outside the damage region.
    pub icp_rms: f64,
    pub icp_pairs: usize,
    pub overlap: f64,
    pub conditioning: f64,
    pub iterations: usize,
    pub converged: bool,
    /// The factor the formal covariance was scaled by (pairs per 10 cm patch).
    pub inflation: f64,
    /// Reference → scene, row-major 4×4.
    pub transform: [f64; 16],
    /// 1σ of the registration: translation (m) and rotation (°), from the scaled covariance.
    pub sigma_translation: f64,
    pub sigma_rotation_deg: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeRun {
    pub method: String,
    pub label: String,
    pub damaged_scan: String,
    pub reference_scan: String,
    /// The damage region (scene frame).
    pub lo: P3,
    pub hi: P3,
    pub registration: Registration,
    /// When the reference is the vehicle's own opposite side, mirrored: its centre plane.
    #[serde(default)]
    pub mirror: Option<MirrorInfo>,
    pub result: CrushVolume,
    pub summary: String,
    pub warnings: Vec<String>,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

/// The centre plane a mirrored reference was reflected across, fitted to symmetric pairs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirrorInfo {
    pub plane_point: P3,
    pub normal: P3,
    /// Each pair's midpoint distance from the plane (m) and its line's angle to the normal (°).
    pub midpoint_offsets: Vec<f64>,
    pub pair_angles_deg: Vec<f64>,
    /// 1σ of the asymmetry measured on the undamaged surfaces after mirroring (m), drawn in the
    /// Monte Carlo as one offset of the whole reference surface.
    pub symmetry_sigma: f64,
}

pub const VOLUME_ASSUMPTIONS: &[&str] = &[
    "The reference is the same make and model (and body) as the damaged vehicle, undamaged, and its surfaces outside the damage region match the damaged vehicle's: they are what the registration is fitted to.",
    "Within the damage region the damaged face is a height field over the reference's plane: each line along the plane's normal meets each surface once.",
    "The crush depth in each cell is the difference of the two surfaces' median heights; where the surface was torn open, the damaged scan may see into the vehicle and read deeper than the surviving skin.",
];

pub const VOLUME_LIMITATIONS: &[&str] = &[
    "The volume is the space between the undamaged and damaged outer surfaces, not the volume of material displaced inside the vehicle.",
    "Cells the damaged scan didn't reach are left out (listed as uncovered), so occlusion understates the volume.",
    "The registration's uncertainty is the ICP's formal covariance scaled as if each 10 cm patch were one independent observation; systematic differences between the two vehicles (tyre pressure, load, ride height, trim) are not in it.",
    "Crush volume is not an energy: no validated relation to the CRASH3 coefficients is applied.",
];

pub const MIRROR_LIMITATION: &str = "A mirrored reference assumes the vehicle was symmetric before the collision. Its asymmetry is measured only where both sides are undamaged; any difference inside the damage region (a side-specific part, trim, an earlier repair) reads as crush or as material pushed outward.";

/// Assemble the stored run from the registration and the volume, with its warnings.
#[allow(clippy::too_many_arguments)]
pub fn volume_run(
    label: &str,
    damaged_scan: &str,
    reference_scan: &str,
    lo: P3,
    hi: P3,
    registration: Registration,
    result: CrushVolume,
    mirror: Option<MirrorInfo>,
) -> VolumeRun {
    let mut warnings = vec![];
    let is_mirror = mirror.is_some();
    if let Some(m) = &mirror {
        warnings.push(format!(
            "The reference is this vehicle's own opposite side, mirrored across its centre plane. Real vehicles are not exactly symmetric (manufacturing tolerance, earlier repairs, damage elsewhere, load and suspension); the asymmetry measured on the undamaged surfaces, {:.1} mm (1σ), is in the interval as an offset of the whole reference surface, but asymmetry inside the damage region can't be measured.",
            m.symmetry_sigma * 1000.0
        ));
        if m.symmetry_sigma > 0.005 {
            warnings.push("The mirrored surfaces differ from the originals by more than 5 mm where both are undamaged; check the symmetric pairs, or use an exemplar vehicle.".into());
        }
        let off = m
            .midpoint_offsets
            .iter()
            .fold(0.0f64, |a, b| a.max(b.abs()));
        let ang = m.pair_angles_deg.iter().fold(0.0f64, |a, b| a.max(*b));
        if off > 0.02 || ang > 5.0 {
            warnings.push(format!("The symmetric pairs disagree on the centre plane (midpoints up to {:.0} mm off it, lines up to {:.1}° from its normal); a pair may not be symmetric.", off * 1000.0, ang));
        }
    }
    let total = result.cells.len() + result.uncovered;
    if result.uncovered * 10 > total {
        warnings.push(format!(
            "{} of the reference's {} cells in the region ({:.0} %) have no surface in the damaged scan (occlusion, or missing panels); their crush isn't counted.",
            result.uncovered,
            total,
            100.0 * result.uncovered as f64 / total as f64
        ));
    }
    let r = &registration;
    if !r.converged {
        warnings.push("The ICP didn't converge; check the pairs and the damage region.".into());
    }
    if r.overlap < 0.5 {
        warnings.push(format!(
            "Only {:.0} % of the reference's points outside the damage region matched the damaged scan; the reference may not be the same shape there, or the region may be too small to register on.",
            r.overlap * 100.0
        ));
    }
    if r.pairs_rms > 0.01 {
        warnings.push(format!(
            "The picked pairs fit to {:.0} mm RMS; a pair may be on different features.",
            r.pairs_rms * 1000.0
        ));
    }
    if r.conditioning < 0.01 {
        warnings.push("The surfaces outside the damage region barely constrain one direction of the registration (like a flat panel sliding along itself); include more of the vehicle's shape.".into());
    }
    if result.inward.value <= 0.0 {
        warnings.push("No net inward crush was found in the region.".into());
    }
    let summary = format!(
        "{label}: crush volume {:.1} L (95 % {:.1}–{:.1} L); deepest {:.0} mm; {:.2} m² crushed",
        result.inward.value * 1000.0,
        result.inward.interval95[0] * 1000.0,
        result.inward.interval95[1] * 1000.0,
        result.max_depth * 1000.0,
        result.crushed_area
    );
    VolumeRun {
        method: VOLUME_METHOD.into(),
        label: label.into(),
        damaged_scan: damaged_scan.into(),
        reference_scan: reference_scan.into(),
        lo,
        hi,
        registration,
        mirror,
        result,
        summary,
        warnings,
        assumptions: VOLUME_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: VOLUME_LIMITATIONS
            .iter()
            .map(|s| s.to_string())
            .chain(is_mirror.then(|| MIRROR_LIMITATION.to_string()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1.6 × 0.6 m vertical face (x = 0, outward −x) sampled every 8 mm with 1 mm noise, a
    /// paraboloid dent of radius R and depth D (volume π R² D / 2) in the damaged copy.
    fn face(dent: Option<(f64, f64)>, seed: u64) -> Vec<P3> {
        let mut rng = Rng(seed);
        let mut out = vec![];
        let step = 0.008;
        for a in 0..=(1.6 / step) as usize {
            for b in 0..=(0.6 / step) as usize {
                let (y, z) = (
                    a as f64 * step + rng.uniform() * 0.004,
                    0.3 + b as f64 * step + rng.uniform() * 0.004,
                );
                let mut x = 0.001 * rng.gauss();
                if let Some((rad, depth)) = dent {
                    let r2 = (y - 0.8).powi(2) + (z - 0.6).powi(2);
                    if r2 < rad * rad {
                        x += depth * (1.0 - r2 / (rad * rad));
                    }
                }
                out.push([x, y, z]);
            }
        }
        out
    }

    #[test]
    fn a_paraboloid_dent_has_its_volume() {
        let (rad, depth) = (0.25, 0.1);
        let truth = std::f64::consts::PI * rad * rad * depth / 2.0;
        let pose = PoseUncertainty {
            covariance: Matrix6::from_diagonal(&Vector6::new(1e-6, 1e-6, 1e-6, 1e-6, 1e-6, 1e-6)),
            about: [0.0, 0.8, 0.6],
            surface: 0.0,
        };
        let v = crush_volume(
            &face(None, 1),
            &face(Some((rad, depth)), 2),
            [-0.3, -0.05, 0.25],
            [0.3, 1.65, 0.95],
            [-5.0, 0.8, 0.6],
            0.02,
            0.001,
            Some(pose),
            200,
            7,
        )
        .unwrap();
        assert!((v.normal[0] + 1.0).abs() < 1e-3, "{:?}", v.normal);
        let rel = (v.inward.value - truth) / truth;
        eprintln!(
            "dent: {:.5} m³ (truth {truth:.5}), 95 % {:?}, max {:.4} m, outward {:.6}, uncovered {}",
            v.inward.value, v.inward.interval95, v.max_depth, v.outward, v.uncovered
        );
        assert!(rel.abs() < 0.03, "{} vs {truth} ({rel})", v.inward.value);
        assert!(
            v.inward.interval95[0] < truth * 1.03 && v.inward.interval95[1] > truth * 0.97,
            "{:?} vs {truth}",
            v.inward.interval95
        );
        assert!((v.max_depth - depth).abs() < 0.005, "{}", v.max_depth);
        assert!(v.outward < 0.1 * truth);
        // Only ragged edge cells, where the jittered samples leave fewer than 3 points.
        assert!(v.uncovered < v.cells.len() / 50, "{}", v.uncovered);
    }

    #[test]
    fn an_undamaged_face_reads_only_its_noise() {
        let v = crush_volume(
            &face(None, 1),
            &face(None, 2),
            [-0.3, -0.05, 0.25],
            [0.3, 1.65, 0.95],
            [-5.0, 0.8, 0.6],
            0.02,
            0.001,
            None,
            100,
            7,
        )
        .unwrap();
        // Noise alone: the volume is near zero either way (1 mm noise, 0.96 m² of cells).
        assert!(v.inward.value.abs() < 5e-5, "{}", v.inward.value);
        assert!(
            v.inward.interval95[0] < 0.0 && v.inward.interval95[1] > 0.0
                || v.inward.value.abs() < 2e-5,
            "{:?}",
            v.inward.interval95
        );
        assert_eq!(v.crushed_area, 0.0);
    }

    #[test]
    fn a_hole_in_the_damaged_scan_is_counted_as_uncovered() {
        let dam: Vec<P3> = face(None, 2)
            .into_iter()
            .filter(|p| (p[1] - 0.4).abs() > 0.1)
            .collect();
        let v = crush_volume(
            &face(None, 1),
            &dam,
            [-0.3, -0.05, 0.25],
            [0.3, 1.65, 0.95],
            [-5.0, 0.8, 0.6],
            0.02,
            0.001,
            None,
            10,
            7,
        )
        .unwrap();
        // 0.2 m of 1.6 m: about 10 columns of 30 cells.
        assert!((250..=380).contains(&v.uncovered), "{}", v.uncovered);
    }
}
