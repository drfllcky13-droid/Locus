//! Target correspondence between two scans and the target-based transform.
//!
//! Rigid motion preserves distances, so a target pair in one scan can only match a pair in
//! the other scan with the same separation (within `tolerance`) and the same kinds. Every
//! consistent triple of matches proposes a transform; the transform that brings the most
//! targets into agreement wins, and is refitted on all of them. If a clearly different
//! transform explains as many targets (a symmetric layout), the result is marked ambiguous
//! rather than guessed.

use crate::rigid::{self, RigidFit};
use nalgebra::Point3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sphere,
    Board,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub kind: Kind,
    /// Position in the scan's frame (m).
    pub position: [f64; 3],
}

#[derive(Debug, Clone)]
pub struct Correspondence {
    /// Matched (index in `from`, index in `to`) pairs.
    pub pairs: Vec<(usize, usize)>,
    /// Maps `from` positions onto `to` positions, fitted on all pairs.
    pub fit: RigidFit,
    /// Another transform, clearly different, matches as many targets.
    pub ambiguous: bool,
}

/// Inlier pairs, fit rms and the fit itself.
type Hypothesis = (Vec<(usize, usize)>, f64, RigidFit);

fn p(t: &Target) -> Point3<f64> {
    Point3::from(t.position)
}

/// Match targets of scan `from` to scan `to`. `tolerance` (m) bounds how much a distance
/// between two targets may differ between scans; it covers both scans' target precision.
/// `None` when fewer than three targets can be matched.
pub fn correspond(from: &[Target], to: &[Target], tolerance: f64) -> Option<Correspondence> {
    let d = |a: &Target, b: &Target| (p(a) - p(b)).norm();
    let same = |a: &Target, b: &Target| a.kind == b.kind;
    // Hypotheses: (inlier pairs, rms, transform).
    let mut hyps: Vec<Hypothesis> = vec![];
    let (n, m) = (from.len(), to.len());
    for i in 0..n {
        for j in i + 1..n {
            let dij = d(&from[i], &from[j]);
            for k in 0..m {
                if !same(&from[i], &to[k]) {
                    continue;
                }
                for l in 0..m {
                    if l == k
                        || !same(&from[j], &to[l])
                        || (d(&to[k], &to[l]) - dij).abs() > tolerance
                    {
                        continue;
                    }
                    for a in j + 1..n {
                        let (dia, dja) = (d(&from[i], &from[a]), d(&from[j], &from[a]));
                        for b in 0..m {
                            if b == k || b == l || !same(&from[a], &to[b]) {
                                continue;
                            }
                            if (d(&to[k], &to[b]) - dia).abs() > tolerance
                                || (d(&to[l], &to[b]) - dja).abs() > tolerance
                            {
                                continue;
                            }
                            let seed = [(i, k), (j, l), (a, b)];
                            if let Some(h) = hypothesis(from, to, &seed, tolerance) {
                                hyps.push(h);
                            }
                        }
                    }
                }
            }
        }
    }
    hyps.sort_by(|x, y| y.0.len().cmp(&x.0.len()).then(x.1.total_cmp(&y.1)));
    let (pairs, _, fit) = hyps.first()?.clone();
    let ambiguous = hyps
        .iter()
        .skip(1)
        .take_while(|h| h.0.len() == pairs.len())
        .any(|h| {
            let delta = h.2.transform.inverse() * fit.transform;
            delta.rotation.angle() > 1f64.to_radians() || delta.translation.vector.norm() > 0.05
        });
    Some(Correspondence {
        pairs,
        fit,
        ambiguous,
    })
}

/// From three seed matches: transform, then every mutual match within tolerance, refitted.
fn hypothesis(
    from: &[Target],
    to: &[Target],
    seed: &[(usize, usize)],
    tol: f64,
) -> Option<Hypothesis> {
    let fit = |pairs: &[(usize, usize)]| {
        let a: Vec<_> = pairs.iter().map(|&(i, _)| p(&from[i])).collect();
        let b: Vec<_> = pairs.iter().map(|&(_, k)| p(&to[k])).collect();
        rigid::fit(&a, &b).ok()
    };
    let mut f = fit(seed)?;
    let mut pairs = seed.to_vec();
    for _ in 0..2 {
        pairs = (0..from.len())
            .filter_map(|i| {
                let q = f.transform * p(&from[i]);
                let (k, dist) = to
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.kind == from[i].kind)
                    .map(|(k, t)| (k, (p(t) - q).norm()))
                    .min_by(|x, y| x.1.total_cmp(&y.1))?;
                // Mutual: no other `from` target lands closer to `to[k]`.
                let mutual = (0..from.len())
                    .filter(|&o| o != i && from[o].kind == from[i].kind)
                    .all(|o| (f.transform * p(&from[o]) - p(&to[k])).norm() > dist);
                (dist < tol && mutual).then_some((i, k))
            })
            .collect();
        if pairs.len() < 3 {
            return None;
        }
        f = fit(&pairs)?;
    }
    Some((pairs, f.rms, f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use locus_synth::{truth, MovedSphere, Options};
    use nalgebra::Vector3;

    /// Targets of scan `s` in its own frame, as detection would report them: every sphere and
    /// board, with `noise` (m) per coordinate, minus the indices in `hidden`.
    fn seen(t: &locus_synth::Truth, s: usize, noise: f64, hidden: &[usize]) -> Vec<Target> {
        let inv = t.scans[s].pose.inverse();
        let mut state = 0x1234_5678_9abc_def1u64 ^ s as u64;
        let mut jitter = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            ((state >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 2.0 * noise
        };
        let spheres = (0..t.spheres.len()).map(|k| (Kind::Sphere, t.sphere_centre(k, s)));
        let boards = t.boards.iter().map(|b| (Kind::Board, b.centre));
        spheres
            .chain(boards)
            .enumerate()
            .filter(|(i, _)| !hidden.contains(i))
            .map(|(_, (kind, w))| {
                let l = inv.apply(w);
                Target {
                    kind,
                    position: [l[0] + jitter(), l[1] + jitter(), l[2] + jitter()],
                }
            })
            .collect()
    }

    fn error(c: &Correspondence, t: &locus_synth::Truth, from: usize, to: usize) -> (f64, f64) {
        // True transform from scan `from`'s frame to scan `to`'s frame.
        let truth = t.scans[to].pose.inverse().compose(&t.scans[from].pose);
        let probe = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        let mut worst = 0f64;
        for q in probe {
            let est = c.fit.transform * Point3::from(q);
            let tru = truth.apply(q);
            worst = worst.max((est.coords - Vector3::from(tru)).norm());
        }
        let r = c.fit.transform.rotation;
        let tq = nalgebra::UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
            truth.rotation[0],
            truth.rotation[1],
            truth.rotation[2],
            truth.rotation[3],
        ));
        (worst, r.angle_to(&tq).to_degrees())
    }

    #[test]
    fn matches_targets_and_recovers_the_relative_pose() {
        let t = truth(&Options {
            scans: 3,
            points_per_scan: 0,
            ..Options::default()
        });
        let a = seen(&t, 0, 0.0005, &[2, 9]);
        let b = seen(&t, 1, 0.0005, &[4]);
        let c = correspond(&a, &b, 0.005).expect("matched");
        assert!(!c.ambiguous);
        assert!(c.pairs.len() >= 10, "{} pairs", c.pairs.len());
        let (dist, angle) = error(&c, &t, 0, 1);
        assert!(dist < 0.002, "{dist} m within 10 m");
        assert!(angle < 0.02, "{angle}°");
    }

    #[test]
    fn a_moved_sphere_is_left_out() {
        let t = truth(&Options {
            scans: 2,
            points_per_scan: 0,
            moved_sphere: Some(MovedSphere {
                sphere: 1,
                from_scan: 1,
                offset: [0.05, 0.0, 0.0],
            }),
            ..Options::default()
        });
        let a = seen(&t, 0, 0.0005, &[]);
        let b = seen(&t, 1, 0.0005, &[]);
        let c = correspond(&a, &b, 0.005).expect("matched");
        assert!(
            !c.pairs.iter().any(|&(i, _)| i == 1),
            "moved sphere matched: {:?}",
            c.pairs
        );
        assert_eq!(c.pairs.len(), a.len() - 1);
    }

    #[test]
    fn a_symmetric_layout_is_ambiguous() {
        // Four spheres on a square: a 90° turn explains them all equally well.
        let sq = [
            [0.0, 0.0, 1.0],
            [4.0, 0.0, 1.0],
            [4.0, 4.0, 1.0],
            [0.0, 4.0, 1.0],
        ];
        let a: Vec<Target> = sq
            .iter()
            .map(|&position| Target {
                kind: Kind::Sphere,
                position,
            })
            .collect();
        let c = correspond(&a, &a, 0.005).unwrap();
        assert!(c.ambiguous);
    }

    #[test]
    fn too_few_targets_give_nothing() {
        let a = [[0.0; 3], [3.0, 0.0, 0.0]].map(|position| Target {
            kind: Kind::Sphere,
            position,
        });
        assert!(correspond(&a, &a, 0.005).is_none());
    }
}
