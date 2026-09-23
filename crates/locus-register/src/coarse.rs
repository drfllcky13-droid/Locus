//! Coarse cloud-to-cloud alignment of two levelled scans with no starting pose.
//!
//! Terrestrial scanners level themselves (a tilt compensator keeps the scan's z axis within
//! a fraction of a degree of vertical), so between two scans the rotation is a heading plus
//! a small tilt. This module assumes that. Scans that are not levelled need a starting pose
//! from targets or from the file.
//!
//! 1. Both clouds are downsampled to `voxel` and given normals.
//! 2. Each point gets a Fast Point Feature Histogram (Rusu, Blodow & Beetz, 2009): angles
//!    between its normal and its neighbours' normals within `feature_radius`, binned into
//!    3 × 11 bins, then blended with the neighbours' own histograms. Only the most
//!    distinctive fraction (`keep`) is used; plane points all look alike.
//! 3. Each source point is paired with the target point whose feature is most similar,
//!    keeping only mutual best pairs. Most pairs are wrong; a few percent are right.
//! 4. Consensus by voting: for every heading in 0.5° steps, each pair votes for the
//!    translation it implies. Right pairs agree; wrong ones scatter or pile up on repeated
//!    structure. Up to 20 peaks per heading become candidates.
//! 5. Candidates are screened on a sample of points, the best 20 are tightened by ICP at
//!    voxel scale, and those are scored on all points: the share of wall-like surfaces that
//!    land on a matching surface, minus the share of points placed where the other scanner
//!    saw straight through empty space (checked both ways). If a clearly different candidate
//!    scores within 0.05 of the best, the result is marked ambiguous rather than trusted.
//!
//! The result is good to about a voxel; fine ICP on the full clouds refines it.

use crate::icp::{icp, IcpParams};
use crate::normals::{self, nearest, within, Tree};
use kiddo::{ImmutableKdTree, SquaredEuclidean};
use nalgebra::{Isometry3, Point3, Vector3};
use rayon::prelude::*;

const BINS: usize = 11;
pub const FEATURE_LEN: usize = 3 * BINS;
pub type Feature = [f64; FEATURE_LEN];

#[derive(Debug, Clone, PartialEq)]
pub struct CoarseParams {
    /// Downsampling voxel (m).
    pub voxel: f64,
    /// Neighbourhood for normals (m).
    pub normal_radius: f64,
    /// Neighbourhood for features (m).
    pub feature_radius: f64,
    /// Fraction of points, the most distinctive, whose features are matched.
    pub keep: f64,
    /// A rough pose (from the scanner's on-site registration, say): only headings and
    /// translations near it are searched, which also settles symmetric scenes.
    pub prior: Option<Prior>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prior {
    pub transform: Isometry3<f64>,
    /// Largest heading difference searched (°).
    pub degrees: f64,
    /// Largest translation difference searched (m).
    pub metres: f64,
}

impl Prior {
    fn heading(&self) -> f64 {
        let r = self.transform.rotation * Vector3::x();
        r.y.atan2(r.x)
    }
}

impl Default for CoarseParams {
    fn default() -> Self {
        CoarseParams {
            voxel: 0.1,
            normal_radius: 0.25,
            feature_radius: 1.0,
            keep: 0.2,
            prior: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Coarse {
    /// Maps source coordinates onto target coordinates.
    pub transform: Isometry3<f64>,
    /// Fraction of the source's downsampled points that land within two voxels of a target
    /// point facing the same way (for levelled scans, only non-horizontal surfaces count).
    pub overlap: f64,
    /// Feature pairs agreeing with the transform.
    pub inliers: usize,
    /// Fraction of the other scan's points that this transform puts where the scan saw
    /// through empty space (averaged both ways).
    pub conflicts: f64,
    /// A clearly different transform scores within 0.05 (repeated or symmetric structure).
    pub ambiguous: bool,
}

#[derive(Debug, Clone)]
struct Scored {
    score: f64,
    overlap: f64,
    conflicts: f64,
    inliers: usize,
    transform: Isometry3<f64>,
}

/// What a scanner saw: the nearest range in each 1° direction cell, in its own frame (the
/// scanner at the origin).
struct RangeImage(std::collections::HashMap<(i32, i32), f64>);

impl RangeImage {
    fn cell(p: &Vector3<f64>) -> (i32, i32) {
        let az = p.y.atan2(p.x).to_degrees().floor() as i32;
        let el = (p.z / p.norm())
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees()
            .floor() as i32;
        (az, el)
    }

    fn new(points: &[[f64; 3]]) -> Self {
        let mut m = std::collections::HashMap::new();
        for p in points {
            let v = Vector3::from(*p);
            let r = v.norm();
            if r > 1e-6 {
                let e = m.entry(Self::cell(&v)).or_insert(f64::INFINITY);
                *e = r.min(*e);
            }
        }
        RangeImage(m)
    }

    /// Fraction of `points` (already in this scanner's frame) lying clearly in front of the
    /// nearest surface this scanner saw in their direction, i.e. in space it saw through.
    fn conflicts(&self, points: impl Iterator<Item = Point3<f64>>, margin: f64) -> f64 {
        let (mut n, mut bad) = (0usize, 0usize);
        for q in points {
            let r = q.coords.norm();
            if r < 1e-6 {
                continue;
            }
            n += 1;
            if self
                .0
                .get(&Self::cell(&q.coords))
                .is_some_and(|&seen| r < seen - margin - 0.02 * r)
            {
                bad += 1;
            }
        }
        bad as f64 / n.max(1) as f64
    }
}

/// Normals from all neighbours within `radius`, oriented toward `viewpoint`.
fn radius_normals(
    points: &[[f64; 3]],
    tree: &Tree,
    radius: f64,
    viewpoint: [f64; 3],
) -> Vec<Option<Vector3<f64>>> {
    points
        .par_iter()
        .map(|p| {
            let nn = within(tree, p, radius);
            if nn.len() < 5 {
                return None;
            }
            let c = nn
                .iter()
                .fold(Vector3::zeros(), |s, &i| s + Vector3::from(points[i]))
                / nn.len() as f64;
            let cov = nn.iter().fold(nalgebra::Matrix3::zeros(), |s, &i| {
                let d = Vector3::from(points[i]) - c;
                s + d * d.transpose()
            });
            let e = nalgebra::SymmetricEigen::new(cov);
            let mut n: Vector3<f64> = e.eigenvectors.column(e.eigenvalues.imin()).into();
            if n.dot(&(Vector3::from(viewpoint) - Vector3::from(*p))) < 0.0 {
                n = -n;
            }
            Some(n)
        })
        .collect()
}

/// Darboux-frame angles between two oriented points (PCL's convention).
fn pair_feature(
    p1: Vector3<f64>,
    n1: Vector3<f64>,
    p2: Vector3<f64>,
    n2: Vector3<f64>,
) -> Option<(f64, f64, f64)> {
    let mut d = p2 - p1;
    let len = d.norm();
    if len < 1e-9 {
        return None;
    }
    d /= len;
    let (mut a, mut b) = (n1, n2);
    let (a1, a2) = (n1.dot(&d), n2.dot(&d));
    let f3 = if a1.abs().acos() > a2.abs().acos() {
        std::mem::swap(&mut a, &mut b);
        d = -d;
        -a2
    } else {
        a1
    };
    let v = d.cross(&a);
    let vn = v.norm();
    if vn < 1e-9 {
        return None;
    }
    let v = v / vn;
    let w = a.cross(&v);
    Some((w.dot(&b).atan2(a.dot(&b)), v.dot(&b), f3))
}

fn bin(v: f64, lo: f64, hi: f64) -> usize {
    (((v - lo) / (hi - lo) * BINS as f64).floor() as isize).clamp(0, BINS as isize - 1) as usize
}

/// FPFH of every point (None where it has no normal or too few neighbours).
pub fn fpfh(
    points: &[[f64; 3]],
    normals: &[Option<Vector3<f64>>],
    tree: &Tree,
    radius: f64,
) -> Vec<Option<Feature>> {
    let neighbours: Vec<Vec<usize>> = points.par_iter().map(|p| within(tree, p, radius)).collect();
    let spfh: Vec<Option<Feature>> = (0..points.len())
        .into_par_iter()
        .map(|i| {
            let ni = normals[i]?;
            let mut h = [0.0; FEATURE_LEN];
            let mut n = 0.0;
            for &j in &neighbours[i] {
                let Some(nj) = normals[j] else { continue };
                let Some((f1, f2, f3)) =
                    pair_feature(Vector3::from(points[i]), ni, Vector3::from(points[j]), nj)
                else {
                    continue;
                };
                h[bin(f1, -std::f64::consts::PI, std::f64::consts::PI)] += 1.0;
                h[BINS + bin(f2, -1.0, 1.0)] += 1.0;
                h[2 * BINS + bin(f3, -1.0, 1.0)] += 1.0;
                n += 1.0;
            }
            (n >= 3.0).then(|| h.map(|v| v * 100.0 / n))
        })
        .collect();
    (0..points.len())
        .into_par_iter()
        .map(|i| {
            let mut f = spfh[i]?;
            let (mut acc, mut k) = ([0.0; FEATURE_LEN], 0.0);
            for &j in &neighbours[i] {
                if j == i {
                    continue;
                }
                let Some(s) = spfh[j] else { continue };
                let w = 1.0
                    / (Vector3::from(points[i]) - Vector3::from(points[j]))
                        .norm()
                        .max(1e-6);
                for b in 0..FEATURE_LEN {
                    acc[b] += s[b] * w;
                }
                k += 1.0;
            }
            if k > 0.0 {
                for b in 0..FEATURE_LEN {
                    f[b] += acc[b] / k;
                }
            }
            // Renormalise each of the three sub-histograms to sum to 100.
            for block in f.chunks_mut(BINS) {
                let sum: f64 = block.iter().sum();
                if sum > 0.0 {
                    block.iter_mut().for_each(|v| *v *= 100.0 / sum);
                }
            }
            Some(f)
        })
        .collect()
}

struct Prepared {
    points: Vec<[f64; 3]>,
    normals: Vec<Option<Vector3<f64>>>,
    tree: Tree,
    features: Vec<Option<Feature>>,
}

fn prepare(cloud: &[[f64; 3]], p: &CoarseParams) -> Prepared {
    let points = normals::voxel_downsample(cloud, p.voxel);
    let tree = normals::tree(&points);
    let nrm = radius_normals(&points, &tree, p.normal_radius, [0.0; 3]);
    let mut features = fpfh(&points, &nrm, &tree, p.feature_radius);
    // Keep only distinctive points: those whose feature is farthest from the cloud's mean
    // feature. Plane points all look alike and would swamp the matching.
    let valid: Vec<&Feature> = features.iter().flatten().collect();
    if !valid.is_empty() {
        let mut mean = [0.0; FEATURE_LEN];
        for f in &valid {
            for b in 0..FEATURE_LEN {
                mean[b] += f[b] / valid.len() as f64;
            }
        }
        let dist = |f: &Feature| {
            f.iter()
                .zip(&mean)
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f64>()
        };
        let mut d: Vec<f64> = valid.iter().map(|f| dist(f)).collect();
        let k = (((d.len() as f64) * (1.0 - p.keep)) as usize).min(d.len() - 1);
        let cut = *d.select_nth_unstable_by(k, f64::total_cmp).1;
        for f in features.iter_mut() {
            if f.is_some_and(|f| dist(&f) < cut) {
                *f = None;
            }
        }
    }
    Prepared {
        points,
        normals: nrm,
        tree,
        features,
    }
}

/// Mutual nearest pairs in feature space: (source index, target index).
fn match_features(src: &Prepared, dst: &Prepared) -> Vec<(usize, usize)> {
    let index = |p: &Prepared| -> (Vec<usize>, Vec<Feature>) {
        p.features
            .iter()
            .enumerate()
            .filter_map(|(i, f)| Some((i, (*f)?)))
            .unzip()
    };
    let (si, sf) = index(src);
    let (di, df) = index(dst);
    if sf.is_empty() || df.is_empty() {
        return vec![];
    }
    let st: ImmutableKdTree<f64, FEATURE_LEN> =
        ImmutableKdTree::new_from_slice(&sf).expect("features");
    let dt: ImmutableKdTree<f64, FEATURE_LEN> =
        ImmutableKdTree::new_from_slice(&df).expect("features");
    let best = |t: &ImmutableKdTree<f64, FEATURE_LEN>, f: &Feature| {
        t.query(f)
            .nearest_one::<SquaredEuclidean<f64>>()
            .execute()
            .item as usize
    };
    (0..sf.len())
        .into_par_iter()
        .filter_map(|a| {
            let b = best(&dt, &sf[a]);
            (best(&st, &df[b]) == a).then_some((si[a], di[b]))
        })
        .collect()
}

/// Levelled scans: for every heading in 0.5° steps, each feature pair votes for the
/// translation it implies; correct pairs agree, wrong ones scatter. The best-supported
/// headings and translations are returned with their vote counts.
fn heading_votes(
    pairs: &[(usize, usize)],
    sp: &[[f64; 3]],
    dp: &[[f64; 3]],
    p: &CoarseParams,
) -> Vec<(usize, Isometry3<f64>)> {
    // Wide cells: residual tilt between two levelled scanners (up to ~1°) moves far points by
    // decimetres, spreading correct votes. Scoring and ICP take care of precision.
    let cell = 5.0 * p.voxel;
    let mut best: Vec<(usize, Isometry3<f64>)> = (0..720)
        .into_par_iter()
        .flat_map_iter(|k| {
            let heading = (k as f64 * 0.5).to_radians();
            if let Some(pr) = &p.prior {
                let d = (heading - pr.heading()).rem_euclid(std::f64::consts::TAU);
                if d.min(std::f64::consts::TAU - d) > pr.degrees.to_radians() {
                    return vec![];
                }
            }
            let rot = nalgebra::UnitQuaternion::from_axis_angle(&Vector3::z_axis(), heading);
            let mut votes: std::collections::HashMap<[i64; 3], (usize, Vector3<f64>)> =
                std::collections::HashMap::new();
            for &(i, j) in pairs {
                let t = Vector3::from(dp[j]) - rot * Vector3::from(sp[i]);
                let e = votes
                    .entry(t.map(|v| (v / cell).floor() as i64).into())
                    .or_default();
                e.0 += 1;
                e.1 += t;
            }
            // Sum each cell with its neighbours so a peak split across a boundary still counts,
            // then keep up to twenty peaks (3 votes or more) at least a metre apart: repeated
            // structure (a grid of pillars) gives many translations with similar support, and
            // the true one may not be the strongest.
            let mut peaks: Vec<(usize, Vector3<f64>)> = votes
                .keys()
                .map(|key| {
                    let (mut c, mut sum) = (0, Vector3::zeros());
                    for dx in -1..=1 {
                        for dy in -1..=1 {
                            for dz in -1..=1 {
                                if let Some(v) = votes.get(&[key[0] + dx, key[1] + dy, key[2] + dz])
                                {
                                    c += v.0;
                                    sum += v.1;
                                }
                            }
                        }
                    }
                    (c, sum / c as f64)
                })
                .collect();
            peaks.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.x.total_cmp(&b.1.x)));
            let mut kept: Vec<(usize, Isometry3<f64>)> = vec![];
            for (n, t) in peaks {
                if kept.len() == 20 || n < 3 {
                    break;
                }
                let near_prior = p
                    .prior
                    .as_ref()
                    .is_none_or(|pr| (pr.transform.translation.vector - t).norm() <= pr.metres);
                if near_prior
                    && kept
                        .iter()
                        .all(|(_, k)| (k.translation.vector - t).norm() > 1.0)
                {
                    kept.push((
                        n,
                        Isometry3::from_parts(nalgebra::Translation3::from(t), rot),
                    ));
                }
            }
            kept
        })
        .collect();
    // The prior itself is always a candidate, in case no feature pair near it is right.
    if let Some(pr) = &p.prior {
        best.push((0, pr.transform));
    }
    // Up to 2000 by support, never two within 1 m and 10° of each other: scoring, not
    // support, picks the winner.
    let mut all = std::mem::take(&mut best);
    all.sort_by_key(|h| std::cmp::Reverse(h.0));
    for h in all {
        if best.len() == 2000 {
            break;
        }
        let distinct = best.iter().all(|(_, k)| {
            let d = k.inverse() * h.1;
            d.translation.vector.norm() > 1.0 || d.rotation.angle() > 10f64.to_radians()
        });
        if distinct {
            best.push(h);
        }
    }
    best
}

/// Rough transform taking `source` onto `target` (both full clouds in their own frames).
pub fn coarse(source: &[[f64; 3]], target: &[[f64; 3]], p: &CoarseParams) -> Option<Coarse> {
    let (src, dst) = rayon::join(|| prepare(source, p), || prepare(target, p));
    let pairs = match_features(&src, &dst);
    if pairs.len() < 3 {
        return None;
    }
    let sp = |i: usize| Point3::from(src.points[pairs[i].0]);
    let dp = |i: usize| Point3::from(dst.points[pairs[i].1]);
    let inlier = 1.5 * p.voxel;
    let top = heading_votes(&pairs, &src.points, &dst.points, p);
    // Score each as proposed: overlap on structure, minus visibility conflicts both ways
    // (surfaces placed where the other scanner saw straight through empty space).
    let reach = 2.0 * p.voxel;
    let (src_view, dst_view) = (RangeImage::new(&src.points), RangeImage::new(&dst.points));
    let structure: Vec<(usize, Vector3<f64>)> = (0..src.points.len())
        .filter_map(|i| Some((i, src.normals[i]?)))
        .filter(|(_, n)| n.z.abs() < 0.7)
        .collect();
    let score = |t: Isometry3<f64>, stride: usize, reach: f64| -> Scored {
        let inliers = (0..pairs.len())
            .filter(|&i| (t * sp(i) - dp(i)).norm() < inlier)
            .count();
        // Floors and ceilings of levelled scans match at any horizontal shift, so only
        // non-horizontal surfaces count. A point lands when a target point is near and faces
        // the same way.
        let on = structure
            .iter()
            .step_by(stride)
            .filter(|(i, n)| {
                let (j, d2) = nearest(&dst.tree, &(t * Point3::from(src.points[*i])).coords.into());
                d2 < reach * reach && dst.normals[j].is_some_and(|m| m.dot(&(t.rotation * n)) > 0.8)
            })
            .count();
        let overlap = on as f64 / structure.len().div_ceil(stride).max(1) as f64;
        let conflicts = (dst_view.conflicts(
            src.points
                .iter()
                .step_by(stride)
                .map(|q| t * Point3::from(*q)),
            reach,
        ) + src_view.conflicts(
            dst.points
                .iter()
                .step_by(stride)
                .map(|q| t.inverse() * Point3::from(*q)),
            reach,
        )) / 2.0;
        Scored {
            score: overlap - conflicts,
            overlap,
            conflicts,
            inliers,
            transform: t,
        }
    };
    // Screen every candidate on about 2000 points, with a reach as coarse as the votes; tighten
    // the best 20 by ICP at voxel scale; score those on all points.
    let stride = (structure.len() / 2000).max(1);
    let mut screened: Vec<Scored> = top
        .into_par_iter()
        .map(|(_, t)| score(t, stride, 5.0 * p.voxel))
        .collect();
    screened.sort_by(|a, b| b.score.total_cmp(&a.score));
    screened.truncate(20);
    let dst_normals: Vec<Option<[f64; 3]>> =
        dst.normals.iter().map(|n| n.map(Into::into)).collect();
    let tighten = IcpParams {
        max_distance: 5.0 * p.voxel,
        min_distance: p.voxel,
        pair_radius: 3.0 * p.voxel,
        ..IcpParams::default()
    };
    let sample: Vec<[f64; 3]> = src
        .points
        .iter()
        .step_by((src.points.len() / 10_000).max(1))
        .copied()
        .collect();
    let mut scored: Vec<Scored> = screened
        .into_par_iter()
        .map(|c| {
            let t = icp(
                &sample,
                &dst.points,
                &dst_normals,
                &dst.tree,
                c.transform,
                &tighten,
            )
            .map_or(c.transform, |r| r.transform);
            score(t, 1, reach)
        })
        .collect();
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = scored.first()?.clone();
    let ambiguous = scored.iter().skip(1).any(|c| {
        let d = c.transform.inverse() * best.transform;
        c.score >= best.score - 0.05
            && (d.rotation.angle() > 5f64.to_radians() || d.translation.vector.norm() > 0.5)
    });
    let (transform, overlap, inliers) = (best.transform, best.overlap, best.inliers);
    Some(Coarse {
        transform,
        overlap,
        inliers,
        conflicts: best.conflicts,
        ambiguous,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use locus_synth::{scan_points, truth, Options, StoredPose};
    use nalgebra::{Quaternion, Translation3, UnitQuaternion};

    fn scan(opts: &Options, t: &locus_synth::Truth, s: usize) -> Vec<[f64; 3]> {
        let mut v = vec![];
        scan_points(opts, t, s, &mut |p| {
            if let Some(p) = p {
                v.push(p.xyz)
            }
        });
        v
    }

    #[test]
    #[ignore = "slow synthetic-scan test: run with --ignored (CI does, on Linux)"]
    fn aligns_two_scans_with_no_starting_pose_then_icp_reaches_the_truth() {
        // Two scene layouts; each pair shares a good part of the room from different stations.
        for (seed, from, to) in [(5, 1, 0), (9, 2, 0)] {
            let opts = Options {
                scans: 3,
                points_per_scan: 1_500_000,
                seed,
                stored_pose: StoredPose::None,
                ..Options::default()
            };
            let t = truth(&opts);
            let (src, dst) = (scan(&opts, &t, from), scan(&opts, &t, to));
            let tr = t.scans[to].pose.inverse().compose(&t.scans[from].pose);
            let q = tr.rotation;
            let truth = Isometry3::from_parts(
                Translation3::from(Vector3::from(tr.translation)),
                UnitQuaternion::from_quaternion(Quaternion::new(q[0], q[1], q[2], q[3])),
            );
            let c = coarse(&src, &dst, &CoarseParams::default()).expect("coarse");
            let d = c.transform.inverse() * truth;
            let far = Point3::new(10.0, 0.0, 0.0);
            eprintln!(
                "seed {seed}: coarse {:.1} cm at 10 m, overlap {:.2}, conflicts {:.3}, {} inliers, ambiguous {}",
                (d * far - far).norm() * 100.0,
                c.overlap,
                c.conflicts,
                c.inliers,
                c.ambiguous
            );
            assert!(
                (d * far - far).norm() < 0.1,
                "seed {seed}: coarse result is not near the truth"
            );
            assert!(!c.ambiguous);

            let tree = normals::tree(&dst);
            let nrm = normals::estimate(&dst, &tree, 12, [0.0; 3]);
            let sub = normals::voxel_downsample(&src, 0.05);
            let r = icp(&sub, &dst, &nrm, &tree, c.transform, &IcpParams::default()).expect("icp");
            let d = r.transform.inverse() * truth;
            let err = (d * far - far).norm();
            assert!(err < 0.002, "seed {seed}: {:.3} mm at 10 m", err * 1e3);
            assert!(d.rotation.angle().to_degrees() < 0.02);
        }
    }
}
