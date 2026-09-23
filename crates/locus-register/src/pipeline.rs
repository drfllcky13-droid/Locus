//! The whole registration of a set of scans: targets, cloud-to-cloud and survey control
//! combined in one pose graph. Pure: scans come in as points in their own frames.
//!
//! 1. Each scan: normals, sphere and checkerboard detection.
//! 2. Each pair of scans: target correspondence. Three or more unambiguous matches make a
//!    target link and a starting relative pose.
//! 3. Each pair of scans: ICP from the target pose, or else from the coarse search (levelled
//!    scans), restricted to near the rough poses when the scans come with them. A converged
//!    result with enough overlap and well-conditioned geometry makes a cloud link. Without
//!    targets or rough poses the link is marked shape-only, for the examiner to confirm.
//! 4. Survey control: each scan's targets matched to the control points make control links.
//! 5. Starting poses by chaining links outward from the first scan (or the control), then
//!    the pose graph adjusts everything and tests every link.

use crate::checker::{self, BoardSearch};
use crate::coarse::{coarse, CoarseParams, Prior};
use crate::icp::{icp, IcpParams};
use crate::normals::{self, nearest};
use crate::posegraph::{self, GraphError, Link, LinkStatus, Pair, Solution};
use crate::sphere::{self, SphereSearch};
use crate::targets::{correspond, Kind, Target};
use nalgebra::{Isometry3, Matrix3, Point3};
use rayon::prelude::*;

pub struct ScanInput {
    /// Points in the scan's own frame, scanner at the origin (m).
    pub points: Vec<[f64; 3]>,
    /// Intensity per point, any scale (used for checkerboards).
    pub intensity: Vec<f64>,
}

/// A surveyed target position in the world frame.
#[derive(Debug, Clone)]
pub struct ControlPoint {
    pub kind: Kind,
    pub position: [f64; 3],
    pub covariance: Matrix3<f64>,
}

#[derive(Debug, Clone)]
pub struct Params {
    /// Sphere target radius (m), if spheres were used.
    pub sphere_radius: Option<f64>,
    /// Checkerboard edge (m), if boards were used.
    pub board_size: Option<f64>,
    /// How much a target-to-target distance may differ between scans (m).
    pub target_tolerance: f64,
    /// Run cloud-to-cloud for scan pairs.
    pub cloud: bool,
    /// Precision assigned to each of a cloud link's points (m). ICP's formal precision is far
    /// smaller and unrealistic (see `icp`), so the examiner's figure is used.
    pub cloud_sigma: f64,
    /// Least ICP overlap for a cloud link.
    pub min_overlap: f64,
    /// Least ICP conditioning for a cloud link.
    pub min_conditioning: f64,
    /// How far from the rough poses the coarse search looks (°, m).
    pub prior_degrees: f64,
    pub prior_metres: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            sphere_radius: None,
            board_size: None,
            target_tolerance: 0.005,
            cloud: true,
            cloud_sigma: 0.002,
            min_overlap: 0.2,
            min_conditioning: 0.01,
            prior_degrees: 10.0,
            prior_metres: 2.0,
        }
    }
}

/// What was found and done, for the report.
#[derive(Debug, Clone)]
pub struct Registration {
    pub solution: Solution,
    pub links: Vec<Link>,
    /// Targets found in each scan.
    pub targets: Vec<Vec<(Target, Matrix3<f64>)>>,
    /// Per link: overlap from ICP (cloud links only).
    pub overlap: Vec<Option<f64>>,
    /// Per scan: tied to the reference (the first scan, or the control) by trusted links
    /// alone, i.e. not shape-only and not flagged. An unverified scan's placement rests on
    /// shape matching and must be confirmed by the examiner.
    pub verified: Vec<bool>,
}

/// Which scans trusted links connect to the reference.
pub fn verified(n: usize, links: &[Link], solution: &Solution) -> Vec<bool> {
    let trusted: Vec<&Link> = links
        .iter()
        .zip(&solution.links)
        .filter(|(l, r)| !l.shape_only && r.status != LinkStatus::Flagged)
        .map(|(l, _)| l)
        .collect();
    let mut seen = vec![false; n];
    let control: Vec<usize> = trusted
        .iter()
        .filter(|l| l.b.is_none())
        .map(|l| l.a)
        .collect();
    let used_control = links
        .iter()
        .zip(&solution.links)
        .any(|(l, r)| l.b.is_none() && r.status != LinkStatus::Flagged);
    let mut stack = if used_control { control } else { vec![0] };
    for &s in &stack {
        seen[s] = true;
    }
    while let Some(s) = stack.pop() {
        for l in &trusted {
            let Some(b) = l.b else { continue };
            for (x, y) in [(l.a, b), (b, l.a)] {
                if x == s && !seen[y] {
                    seen[y] = true;
                    stack.push(y);
                }
            }
        }
    }
    seen
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegisterError {
    Graph(GraphError),
}

/// A link found for a pair: the link, its relative pose (b → a), ICP overlap if a cloud link.
type Found = (Link, Isometry3<f64>, Option<f64>);

struct Prepared {
    tree: normals::Tree,
    normals: Vec<Option<[f64; 3]>>,
    targets: Vec<(Target, Matrix3<f64>)>,
}

fn prepare(scan: &ScanInput, p: &Params) -> Prepared {
    let tree = normals::tree(&scan.points);
    let nrm = normals::estimate(&scan.points, &tree, 12, [0.0; 3]);
    let mut targets = vec![];
    if let Some(r) = p.sphere_radius {
        for s in sphere::detect(&scan.points, &tree, &nrm, &SphereSearch::new(r)) {
            targets.push((
                Target {
                    kind: Kind::Sphere,
                    position: s.centre,
                    sigma: s.covariance.trace().sqrt(),
                },
                s.covariance,
            ));
        }
    }
    if let Some(size) = p.board_size {
        for b in checker::detect(
            &scan.points,
            &scan.intensity,
            &tree,
            &BoardSearch::new(size),
        ) {
            targets.push((
                Target {
                    kind: Kind::Board,
                    position: b.centre,
                    sigma: b.covariance.trace().sqrt(),
                },
                b.covariance,
            ));
        }
    }
    Prepared {
        tree,
        normals: nrm,
        targets,
    }
}

/// Up to `n` points spread over `points` (farthest-point sampling), for a cloud link.
fn spread(points: &[[f64; 3]], n: usize) -> Vec<[f64; 3]> {
    if points.is_empty() {
        return vec![];
    }
    let mut chosen = vec![points[0]];
    let mut dist: Vec<f64> = points.iter().map(|p| d2(p, &points[0])).collect();
    while chosen.len() < n.min(points.len()) {
        let (i, _) = dist
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("non-empty");
        chosen.push(points[i]);
        for (k, p) in points.iter().enumerate() {
            dist[k] = dist[k].min(d2(p, &points[i]));
        }
    }
    chosen
}

fn d2(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// Register `scans`. `rough` are scan-to-world poses known roughly (from the scanner's
/// on-site registration or the file), if any.
pub fn register(
    scans: &[ScanInput],
    rough: Option<&[Isometry3<f64>]>,
    control: &[ControlPoint],
    p: &Params,
) -> Result<Registration, RegisterError> {
    let n = scans.len();
    let prep: Vec<Prepared> = scans.par_iter().map(|s| prepare(s, p)).collect();
    let pairs: Vec<(usize, usize)> = (0..n)
        .flat_map(|a| (a + 1..n).map(move |b| (a, b)))
        .collect();

    let per_pair: Vec<Vec<Found>> = pairs
        .par_iter()
        .map(|&(a, b)| {
            let mut out = vec![];
            let ta: Vec<Target> = prep[a].targets.iter().map(|t| t.0).collect();
            let tb: Vec<Target> = prep[b].targets.iter().map(|t| t.0).collect();
            let mut start = None;
            if let Some(c) = correspond(&tb, &ta, p.target_tolerance).filter(|c| !c.ambiguous) {
                let link = Link::targets(
                    a,
                    b,
                    c.pairs
                        .iter()
                        .map(|&(ib, ia)| Pair {
                            pa: ta[ia].position,
                            cov_a: prep[a].targets[ia].1,
                            pb: tb[ib].position,
                            cov_b: prep[b].targets[ib].1,
                        })
                        .collect(),
                );
                start = Some(c.fit.transform);
                out.push((link, c.fit.transform, None));
            }
            if p.cloud {
                let prior = rough.map(|r| Prior {
                    transform: r[a].inverse() * r[b],
                    degrees: p.prior_degrees,
                    metres: p.prior_metres,
                });
                let shape_only = start.is_none() && prior.is_none();
                let start = start.or_else(|| {
                    let params = CoarseParams {
                        prior,
                        ..CoarseParams::default()
                    };
                    coarse(&scans[b].points, &scans[a].points, &params)
                        .filter(|c| !c.ambiguous)
                        .map(|c| c.transform)
                });
                if let Some(init) = start {
                    let sub = normals::voxel_downsample(&scans[b].points, 0.05);
                    let r = icp(
                        &sub,
                        &scans[a].points,
                        &prep[a].normals,
                        &prep[a].tree,
                        init,
                        &IcpParams::default(),
                    );
                    if let Some(r) = r.filter(|r| {
                        r.converged
                            && r.overlap >= p.min_overlap
                            && r.conditioning >= p.min_conditioning
                    }) {
                        // Represent the link by points of `b` that overlap `a`.
                        let overlapping: Vec<[f64; 3]> = sub
                            .iter()
                            .filter(|q| {
                                nearest(
                                    &prep[a].tree,
                                    &(r.transform * Point3::from(**q)).coords.into(),
                                )
                                .1 < 0.05 * 0.05
                            })
                            .copied()
                            .collect();
                        let pts = spread(&overlapping, 12);
                        let mut link = Link::cloud(a, b, &r.transform, &pts, p.cloud_sigma);
                        link.shape_only = shape_only;
                        out.push((link, r.transform, Some(r.overlap)));
                    }
                }
            }
            out
        })
        .collect();
    let mut links = vec![];
    let mut rel: Vec<(usize, usize, Isometry3<f64>)> = vec![];
    let mut overlap = vec![];
    for ((a, b), found) in pairs.iter().zip(per_pair) {
        for (l, t, o) in found {
            links.push(l);
            rel.push((*a, *b, t));
            overlap.push(o);
        }
    }

    // Control: each scan's targets against the surveyed points.
    let world: Vec<Target> = control
        .iter()
        .map(|c| Target {
            kind: c.kind,
            position: c.position,
            sigma: c.covariance.trace().sqrt(),
        })
        .collect();
    let mut to_world: Vec<Option<Isometry3<f64>>> = vec![None; n];
    if world.len() >= 3 {
        for (s, pr) in prep.iter().enumerate() {
            let ts: Vec<Target> = pr.targets.iter().map(|t| t.0).collect();
            if let Some(c) = correspond(&ts, &world, p.target_tolerance).filter(|c| !c.ambiguous) {
                links.push(Link::control(
                    s,
                    c.pairs
                        .iter()
                        .map(|&(i, k)| Pair {
                            pa: ts[i].position,
                            cov_a: pr.targets[i].1,
                            pb: world[k].position,
                            cov_b: control[k].covariance,
                        })
                        .collect(),
                ));
                overlap.push(None);
                to_world[s] = Some(c.fit.transform);
            }
        }
    }

    // Starting poses: breadth-first from the reference through the relative poses.
    let mut init: Vec<Option<Isometry3<f64>>> = if to_world.iter().any(Option::is_some) {
        to_world.clone()
    } else {
        let mut v = vec![None; n];
        if n > 0 {
            v[0] = Some(Isometry3::identity());
        }
        v
    };
    loop {
        let mut grew = false;
        for &(a, b, t) in &rel {
            match (init[a], init[b]) {
                (Some(pa), None) => {
                    init[b] = Some(pa * t);
                    grew = true;
                }
                (None, Some(pb)) => {
                    init[a] = Some(pb * t.inverse());
                    grew = true;
                }
                _ => {}
            }
        }
        if !grew {
            break;
        }
    }
    let lost: Vec<usize> = (0..n).filter(|&s| init[s].is_none()).collect();
    if !lost.is_empty() {
        return Err(RegisterError::Graph(GraphError::Disconnected(lost)));
    }
    let init: Vec<Isometry3<f64>> = init.into_iter().map(|t| t.expect("all placed")).collect();
    let solution = posegraph::solve(&init, &links).map_err(RegisterError::Graph)?;
    Ok(Registration {
        verified: verified(n, &links, &solution),
        solution,
        links,
        targets: prep.into_iter().map(|p| p.targets).collect(),
        overlap,
    })
}
