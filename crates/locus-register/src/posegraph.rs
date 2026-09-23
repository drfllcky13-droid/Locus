//! Global registration: every scan's pose from all links at once (a pose graph).
//!
//! Every link is expressed as point pairs, so targets, survey control and cloud-to-cloud
//! results combine in one least-squares adjustment (the hybrid mode):
//! - a **target link** pairs each target seen in both scans, weighted by the two detections'
//!   covariances;
//! - a **control link** pairs a target in a scan with its surveyed coordinates;
//! - a **cloud link** (ICP) is represented by points spread over the overlap, mapped through
//!   the measured relative pose, each with the link's stated precision. This carries the
//!   relative pose together with its lever arms.
//!
//! The adjustment minimises `Σ rᵀ W r` over all pairs, `r = T_a p_a − T_b p_b` (or
//! `T_a p_a − c` for control), by Gauss–Newton on small rotations and translations of each
//! scan. Without control, the first scan is held fixed and defines the frame.
//!
//! Each link is then tested: its χ² (`Σ rᵀ W r` over its pairs) against the 99.9 % point of
//! χ² with 3 × pairs degrees of freedom. A link that fails is inconsistent with the rest: it
//! is flagged, set aside, and the adjustment repeated, worst link first, until every
//! remaining link passes. A link can only be tested where the graph is redundant (a loop, or
//! more than one link between the same scans); a link that is the only connection to a scan
//! always fits and is reported as untested.

use nalgebra::{
    DMatrix, DVector, Isometry3, Matrix3, Point3, Rotation3, Translation3, UnitQuaternion, Vector3,
};
use std::ops::AddAssign;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Target,
    Cloud,
    Control,
}

/// One observed correspondence. `b == None` means `pb` is a surveyed world coordinate.
#[derive(Debug, Clone)]
pub struct Pair {
    pub pa: [f64; 3],
    pub cov_a: Matrix3<f64>,
    pub pb: [f64; 3],
    pub cov_b: Matrix3<f64>,
}

#[derive(Debug, Clone)]
pub struct Link {
    pub kind: LinkKind,
    pub a: usize,
    /// The other scan; `None` for control (pairs are against world coordinates).
    pub b: Option<usize>,
    pub pairs: Vec<Pair>,
    /// Set by the examiner: never flag or set aside.
    pub forced: bool,
    /// A cloud link whose starting pose came from shape matching alone (no targets, no prior
    /// pose). In a symmetric scene every such link can agree on the same wrong answer, which
    /// no consistency test can catch, so the examiner must confirm it.
    pub shape_only: bool,
}

impl Link {
    /// Target link: each pair is one target as seen in scan `a` and in scan `b`.
    pub fn targets(a: usize, b: usize, pairs: Vec<Pair>) -> Link {
        Link {
            kind: LinkKind::Target,
            a,
            b: Some(b),
            pairs,
            forced: false,
            shape_only: false,
        }
    }

    /// Cloud link from a relative pose `b_to_a` (maps scan `b` coordinates into scan `a`),
    /// represented by `points` of scan `b` spread over the overlap, each with isotropic
    /// precision `sigma` (m).
    pub fn cloud(
        a: usize,
        b: usize,
        b_to_a: &Isometry3<f64>,
        points: &[[f64; 3]],
        sigma: f64,
    ) -> Link {
        let cov = Matrix3::identity() * (sigma * sigma / 2.0);
        Link {
            kind: LinkKind::Cloud,
            a,
            b: Some(b),
            pairs: points
                .iter()
                .map(|p| Pair {
                    pa: (b_to_a * Point3::from(*p)).coords.into(),
                    cov_a: cov,
                    pb: *p,
                    cov_b: cov,
                })
                .collect(),
            forced: false,
            shape_only: false,
        }
    }

    /// Control link: each pair is a target in scan `a` (`pa`) and its surveyed world
    /// coordinates (`pb`, with the survey's covariance).
    pub fn control(a: usize, pairs: Vec<Pair>) -> Link {
        Link {
            kind: LinkKind::Control,
            b: None,
            ..Link::targets(a, 0, pairs)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkStatus {
    /// Consistent with the rest of the graph.
    Ok,
    /// Inconsistent: set aside and not used in the final solution.
    Flagged,
    /// The only connection for part of the graph, so nothing can check it.
    Untested,
}

#[derive(Debug, Clone)]
pub struct LinkReport {
    pub status: LinkStatus,
    /// RMS of the pair residuals at the final solution (m), whether or not it was used.
    pub rms: f64,
    pub max: f64,
    /// χ² per degree of freedom at the final solution.
    pub chi2_per_dof: f64,
    /// The 99.9 % limit it was tested against, per degree of freedom.
    pub limit_per_dof: f64,
}

#[derive(Debug, Clone)]
pub struct Solution {
    /// Scan-to-frame poses: the world (control) frame if any control link is used, otherwise
    /// the first scan's frame.
    pub poses: Vec<Isometry3<f64>>,
    pub links: Vec<LinkReport>,
    pub iterations: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraphError {
    /// Scans not connected to the reference by any usable link.
    Disconnected(Vec<usize>),
    Singular,
}

/// χ² quantile (Wilson–Hilferty) for `k` degrees of freedom at standard normal `z`.
fn chi2_quantile(k: f64, z: f64) -> f64 {
    let a = 2.0 / (9.0 * k);
    k * (1.0 - a + z * a.sqrt()).powi(3)
}

/// Residual and weight of a pair under the current poses.
fn residual(link: &Link, pair: &Pair, poses: &[Isometry3<f64>]) -> (Vector3<f64>, Matrix3<f64>) {
    let ta = &poses[link.a];
    let ra = ta.rotation.to_rotation_matrix();
    let qa = ta * Point3::from(pair.pa);
    let (qb, cov_b) = match link.b {
        Some(b) => {
            let rb = poses[b].rotation.to_rotation_matrix();
            (
                poses[b] * Point3::from(pair.pb),
                rb * pair.cov_b * rb.transpose(),
            )
        }
        None => (Point3::from(pair.pb), pair.cov_b),
    };
    let cov = ra * pair.cov_a * ra.transpose() + cov_b;
    (qa - qb, cov.try_inverse().unwrap_or_else(Matrix3::zeros))
}

/// Scans reachable from the reference through `active` links.
fn reachable(n: usize, links: &[Link], active: &[bool], has_control: bool) -> Vec<bool> {
    let mut seen = vec![false; n];
    let mut stack = vec![];
    if has_control {
        for (l, &on) in links.iter().zip(active) {
            if on && l.b.is_none() && !seen[l.a] {
                seen[l.a] = true;
                stack.push(l.a);
            }
        }
    } else if n > 0 {
        seen[0] = true;
        stack.push(0);
    }
    while let Some(s) = stack.pop() {
        for (l, &on) in links.iter().zip(active) {
            let Some(b) = l.b else { continue };
            if !on {
                continue;
            }
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

/// Gauss–Newton over the free scans, starting from `poses`.
fn adjust(
    poses: &mut [Isometry3<f64>],
    links: &[Link],
    active: &[bool],
    fixed: Option<usize>,
) -> Result<usize, GraphError> {
    let n = poses.len();
    let index: Vec<Option<usize>> = {
        let mut k = 0;
        (0..n)
            .map(|s| {
                (Some(s) != fixed).then(|| {
                    k += 1;
                    k - 1
                })
            })
            .collect()
    };
    let dim = 6 * index.iter().flatten().count();
    // Linearise about a centre near the scans, so far-from-origin coordinates stay well
    // conditioned.
    let centre = poses
        .iter()
        .fold(Vector3::zeros(), |s, p| s + p.translation.vector)
        / n as f64;
    for it in 1..=50 {
        let mut h = DMatrix::<f64>::zeros(dim, dim);
        let mut g = DVector::<f64>::zeros(dim);
        for (l, &on) in links.iter().zip(active) {
            if !on {
                continue;
            }
            for pair in &l.pairs {
                let (r, w) = residual(l, pair, poses);
                // d(T p)/d(ω, τ) for a small motion about `centre`: [−[Tp − centre]×, I].
                let jac = |s: usize, p: &[f64; 3], sign: f64| {
                    let q = poses[s] * Point3::from(*p) - centre;
                    let mut j = nalgebra::Matrix3x6::zeros();
                    j.fixed_view_mut::<3, 3>(0, 0)
                        .copy_from(&(-q.coords.cross_matrix() * sign));
                    j.fixed_view_mut::<3, 3>(0, 3)
                        .copy_from(&(Matrix3::identity() * sign));
                    j
                };
                let mut blocks = vec![];
                if let Some(ia) = index[l.a] {
                    blocks.push((ia, jac(l.a, &pair.pa, 1.0)));
                }
                if let Some(b) = l.b {
                    if let Some(ib) = index[b] {
                        blocks.push((ib, jac(b, &pair.pb, -1.0)));
                    }
                }
                for (i, ji) in &blocks {
                    let gi = ji.transpose() * w * r;
                    g.rows_mut(6 * i, 6).add_assign(&gi);
                    for (k, jk) in &blocks {
                        let hik = ji.transpose() * w * jk;
                        h.view_mut((6 * i, 6 * k), (6, 6)).add_assign(&hik);
                    }
                }
            }
        }
        let step = h.cholesky().ok_or(GraphError::Singular)?.solve(&(-g));
        let mut biggest = 0f64;
        for s in 0..n {
            let Some(i) = index[s] else { continue };
            let d = step.rows(6 * i, 6);
            let (omega, tau) = (
                Vector3::new(d[0], d[1], d[2]),
                Vector3::new(d[3], d[4], d[5]),
            );
            let rot = Rotation3::from_scaled_axis(omega);
            let m = Isometry3::from_parts(
                Translation3::from(centre + tau - rot * centre),
                UnitQuaternion::from_rotation_matrix(&rot),
            );
            poses[s] = m * poses[s];
            biggest = biggest.max(tau.norm() + omega.norm() * 10.0);
        }
        if biggest < 1e-9 {
            return Ok(it);
        }
    }
    Ok(50)
}

/// Solve the graph for `n` scans from starting poses `init` (e.g. chained link poses).
pub fn solve(init: &[Isometry3<f64>], links: &[Link]) -> Result<Solution, GraphError> {
    let n = init.len();
    let mut active = vec![true; links.len()];
    let mut flagged = vec![false; links.len()];
    let z = 3.090_232; // standard normal at 99.9 %
    loop {
        let has_control = links
            .iter()
            .zip(&active)
            .any(|(l, &on)| on && l.b.is_none());
        let seen = reachable(n, links, &active, has_control);
        let lost: Vec<usize> = (0..n).filter(|&s| !seen[s]).collect();
        if !lost.is_empty() {
            return Err(GraphError::Disconnected(lost));
        }
        let mut poses = init.to_vec();
        let fixed = (!has_control).then_some(0);
        let iterations = adjust(&mut poses, links, &active, fixed)?;
        // Test each active, unforced link; set aside the worst failure and repeat.
        let tests: Vec<(f64, f64)> = links
            .iter()
            .map(|l| {
                let chi2: f64 = l
                    .pairs
                    .iter()
                    .map(|p| {
                        let (r, w) = residual(l, p, &poses);
                        r.dot(&(w * r))
                    })
                    .sum();
                let dof = (3 * l.pairs.len()) as f64;
                (chi2 / dof, chi2_quantile(dof, z) / dof)
            })
            .collect();
        let worst = (0..links.len())
            .filter(|&i| active[i] && !links[i].forced && tests[i].0 > tests[i].1)
            .max_by(|&x, &y| (tests[x].0 / tests[x].1).total_cmp(&(tests[y].0 / tests[y].1)));
        if let Some(w) = worst {
            // Only set it aside if the graph stays connected without it.
            let mut trial = active.clone();
            trial[w] = false;
            let has_control = links.iter().zip(&trial).any(|(l, &on)| on && l.b.is_none());
            if reachable(n, links, &trial, has_control).iter().all(|&s| s) {
                active = trial;
                flagged[w] = true;
                continue;
            }
        }
        // Redundancy: a link is testable if the graph stays connected without it.
        let reports = links
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let mut trial = active.clone();
                trial[i] = false;
                let has_control = links.iter().zip(&trial).any(|(l, &on)| on && l.b.is_none());
                let redundant =
                    flagged[i] || reachable(n, links, &trial, has_control).iter().all(|&s| s);
                let res: Vec<f64> = l
                    .pairs
                    .iter()
                    .map(|p| residual(l, p, &poses).0.norm())
                    .collect();
                let status = if flagged[i] {
                    LinkStatus::Flagged
                } else if !redundant {
                    LinkStatus::Untested
                } else {
                    LinkStatus::Ok
                };
                LinkReport {
                    status,
                    rms: (res.iter().map(|r| r * r).sum::<f64>() / res.len().max(1) as f64).sqrt(),
                    max: res.iter().copied().fold(0.0, f64::max),
                    chi2_per_dof: tests[i].0,
                    limit_per_dof: tests[i].1,
                }
            })
            .collect();
        return Ok(Solution {
            poses,
            links: reports,
            iterations,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use locus_synth::{truth, Options, Pose};

    fn iso(p: &Pose) -> Isometry3<f64> {
        let q = p.rotation;
        Isometry3::from_parts(
            Translation3::from(Vector3::from(p.translation)),
            UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(q[0], q[1], q[2], q[3])),
        )
    }

    /// Deterministic small noise.
    struct Noise(u64);
    impl Noise {
        fn next(&mut self, s: f64) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            ((self.0 >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 2.0 * s
        }
        fn v(&mut self, s: f64) -> [f64; 3] {
            [self.next(s), self.next(s), self.next(s)]
        }
    }

    fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }

    /// Target links between every pair of scans, from the true sphere and board centres in
    /// each scan's frame, with `noise` (m) on each detection.
    fn target_links(t: &locus_synth::Truth, noise: f64, rng: &mut Noise) -> Vec<Link> {
        let n = t.scans.len();
        let world: Vec<[f64; 3]> = t
            .spheres
            .iter()
            .map(|s| s.centre)
            .chain(t.boards.iter().map(|b| b.centre))
            .collect();
        let cov = Matrix3::identity() * (noise * noise / 3.0);
        let mut links = vec![];
        for a in 0..n {
            for b in a + 1..n {
                let (ia, ib) = (
                    iso(&t.scans[a].pose).inverse(),
                    iso(&t.scans[b].pose).inverse(),
                );
                let matched = world
                    .iter()
                    .map(|w| {
                        let pa = add((ia * Point3::from(*w)).coords.into(), rng.v(noise));
                        let pb = add((ib * Point3::from(*w)).coords.into(), rng.v(noise));
                        Pair {
                            pa,
                            cov_a: cov,
                            pb,
                            cov_b: cov,
                        }
                    })
                    .collect();
                links.push(Link::targets(a, b, matched));
            }
        }
        links
    }

    fn err(sol: &Solution, t: &locus_synth::Truth, s: usize) -> (f64, f64) {
        // Relative to scan 0, as the solution without control is.
        let tr = iso(&t.scans[0].pose).inverse() * iso(&t.scans[s].pose);
        let d = sol.poses[0].inverse() * sol.poses[s] * tr.inverse();
        let far = Point3::new(10.0, 0.0, 0.0);
        (
            (d * far - far).norm().max(d.translation.vector.norm()),
            d.rotation.angle().to_degrees(),
        )
    }

    #[test]
    fn target_and_cloud_links_recover_the_poses_and_flag_the_bad_link() {
        let t = truth(&Options {
            scans: 4,
            points_per_scan: 0,
            ..Options::default()
        });
        let mut rng = Noise(0x5eed_1234_abcd_ef01);
        let mut links = target_links(&t, 0.0005, &mut rng);
        // Cloud links around the ring 0-1-2-3-0 from the true relative poses, one of them 4 cm
        // and 0.2° wrong (a bad ICP result).
        let spread: Vec<[f64; 3]> = [
            [-8.0, -6.0, 0.0],
            [8.0, -6.0, 0.0],
            [8.0, 6.0, 0.0],
            [-8.0, 6.0, 0.0],
            [0.0, 0.0, 3.0],
            [5.0, 0.0, 4.5],
            [-5.0, 2.0, 0.5],
        ]
        .to_vec();
        for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
            let mut rel = iso(&t.scans[a].pose).inverse() * iso(&t.scans[b].pose);
            if (a, b) == (2, 3) {
                rel = Isometry3::new(
                    Vector3::new(0.04, 0.0, 0.0),
                    Vector3::new(0.0, 0.0, 0.2f64.to_radians()),
                ) * rel;
            }
            links.push(Link::cloud(a, b, &rel, &spread, 0.001));
        }
        let bad = links.len() - 2;
        // Start from the truth disturbed by centimetres.
        let init: Vec<Isometry3<f64>> = (0..4)
            .map(|s| {
                let rel = iso(&t.scans[0].pose).inverse() * iso(&t.scans[s].pose);
                if s == 0 {
                    rel
                } else {
                    Isometry3::new(
                        Vector3::new(0.03, -0.02, 0.01),
                        Vector3::new(0.0, 0.0, 0.003),
                    ) * rel
                }
            })
            .collect();
        let sol = solve(&init, &links).expect("solve");
        for s in 1..4 {
            let (d, a) = err(&sol, &t, s);
            assert!(
                d < 0.002 && a < 0.02,
                "scan {s}: {:.2} mm, {a:.4}°",
                d * 1e3
            );
        }
        for (i, r) in sol.links.iter().enumerate() {
            let expect = if i == bad {
                LinkStatus::Flagged
            } else {
                LinkStatus::Ok
            };
            assert_eq!(r.status, expect, "link {i}: {r:?}");
        }
        assert!(sol.links[bad].rms > 0.01);
    }

    #[test]
    fn survey_control_puts_the_scans_in_the_world_frame() {
        let t = truth(&Options {
            scans: 3,
            points_per_scan: 0,
            ..Options::default()
        });
        let mut rng = Noise(0x0dd_ba11);
        let mut links = target_links(&t, 0.0005, &mut rng);
        // Four surveyed spheres near the room's corners (control spread around the site, as
        // a surveyor would place it), 1 mm survey precision, matched in scan 1.
        let inv = iso(&t.scans[1].pose).inverse();
        let survey = Matrix3::identity() * (0.001f64.powi(2) / 3.0);
        let matched = [0, 4, 15, 19]
            .iter()
            .map(|&k| &t.spheres[k])
            .map(|s| {
                let pa = add((inv * Point3::from(s.centre)).coords.into(), rng.v(0.0005));
                Pair {
                    pa,
                    cov_a: Matrix3::identity() * (0.0005f64.powi(2) / 3.0),
                    pb: add(s.centre, rng.v(0.001)),
                    cov_b: survey,
                }
            })
            .collect();
        links.push(Link::control(1, matched));
        let init: Vec<Isometry3<f64>> = (0..3)
            .map(|s| {
                Isometry3::new(Vector3::new(0.05, 0.05, 0.0), Vector3::new(0.0, 0.0, 0.01))
                    * iso(&t.scans[s].pose)
            })
            .collect();
        let sol = solve(&init, &links).expect("solve");
        for s in 0..3 {
            let d = sol.poses[s].inverse() * iso(&t.scans[s].pose);
            assert!(
                d.translation.vector.norm() < 0.002,
                "scan {s} off by {:?}",
                d.translation.vector
            );
            assert!(d.rotation.angle().to_degrees() < 0.02);
        }
        assert!(
            sol.links.iter().all(|l| l.status == LinkStatus::Ok),
            "{:?}",
            sol.links
        );
    }

    #[test]
    fn a_lone_link_is_untested_and_a_missing_one_is_an_error() {
        let t = truth(&Options {
            scans: 3,
            points_per_scan: 0,
            ..Options::default()
        });
        let spread = [
            [-5.0, 0.0, 0.0],
            [5.0, 0.0, 0.0],
            [0.0, 5.0, 2.0],
            [0.0, -5.0, 4.0],
        ];
        let rel = |a: usize, b: usize| iso(&t.scans[a].pose).inverse() * iso(&t.scans[b].pose);
        let links = vec![
            Link::cloud(0, 1, &rel(0, 1), &spread, 0.001),
            Link::cloud(1, 2, &rel(1, 2), &spread, 0.001),
        ];
        let init = vec![Isometry3::identity(), rel(0, 1), rel(0, 2)];
        let sol = solve(&init, &links).unwrap();
        assert!(sol.links.iter().all(|l| l.status == LinkStatus::Untested));
        assert_eq!(
            solve(&init, &links[..1]).unwrap_err(),
            GraphError::Disconnected(vec![2])
        );
    }
}
