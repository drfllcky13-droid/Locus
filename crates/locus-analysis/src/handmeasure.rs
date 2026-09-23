//! Point positions from hand measurements taken at a scene with a tape: baseline/offset and
//! triangulation (trilateration). Method, assumptions and limitations:
//! docs/methods/hand-measurements.md.
//!
//! Positions are 2-D (plan view), metres, in the diagram's frame. Every input carries a 1σ
//! uncertainty: each known point is uncertain by `known_sigma` per axis, and each taped
//! distance by `tape.fixed + tape.per_metre × distance`. The solved position's covariance is
//! propagated to first order with a numerical Jacobian over all inputs.

use serde::{Deserialize, Serialize};

pub type P2 = [f64; 2];

/// Which side of the directed line from the first reference point to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

/// Taped-distance uncertainty, 1σ: `fixed + per_metre × distance` (m).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tape {
    pub fixed: f64,
    pub per_metre: f64,
}

impl Tape {
    pub fn sigma(&self, d: f64) -> f64 {
        self.fixed + self.per_metre * d.abs()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solved {
    pub position: P2,
    /// Covariance of `position` (m²).
    pub covariance: [[f64; 2]; 2],
    /// Distance from each reference to the solution minus the taped distance (m); zero for
    /// the exactly determined cases.
    pub residuals: Vec<f64>,
    /// With more references than needed: the largest residual in units of its own σ. Above
    /// 3 the tapes disagree with each other beyond their stated precision.
    pub worst_normalised: Option<f64>,
}

impl Solved {
    /// 1σ radius: square root of the covariance trace (m).
    pub fn sigma(&self) -> f64 {
        (self.covariance[0][0] + self.covariance[1][1]).sqrt()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HandError {
    /// Two reference points at the same place.
    CoincidentReferences,
    /// The distances don't reach: the circles don't meet (by this much, m).
    NoIntersection(f64),
    /// The references are in a line, so which side of it the point lies on is undetermined.
    SideNeeded,
    NeedReferences(usize),
}

impl std::fmt::Display for HandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandError::CoincidentReferences => {
                write!(f, "two reference points are at the same place")
            }
            HandError::NoIntersection(gap) => write!(
                f,
                "the distances can't all be right: the circles miss each other by {:.1} mm",
                gap * 1000.0
            ),
            HandError::SideNeeded => write!(
                f,
                "the reference points are in a line; say which side of it the point is on"
            ),
            HandError::NeedReferences(n) => write!(f, "needs at least {n} reference points"),
        }
    }
}

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn norm(a: P2) -> f64 {
    a[0].hypot(a[1])
}

/// Point `along` metres from `a` toward `b`, then `offset` metres square to the baseline on
/// `side`.
pub fn baseline_offset(
    a: P2,
    b: P2,
    along: f64,
    offset: f64,
    side: Side,
    known_sigma: f64,
    tape: Tape,
) -> Result<Solved, HandError> {
    let f = |v: &[f64]| -> Result<P2, HandError> {
        let (a, b) = ([v[0], v[1]], [v[2], v[3]]);
        let d = sub(b, a);
        let len = norm(d);
        if len < 1e-12 {
            return Err(HandError::CoincidentReferences);
        }
        let u = [d[0] / len, d[1] / len];
        let n = match side {
            Side::Left => [-u[1], u[0]],
            Side::Right => [u[1], -u[0]],
        };
        Ok([
            a[0] + u[0] * v[4] + n[0] * v[5],
            a[1] + u[1] * v[4] + n[1] * v[5],
        ])
    };
    let inputs = [a[0], a[1], b[0], b[1], along, offset];
    let sig = [
        known_sigma,
        known_sigma,
        known_sigma,
        known_sigma,
        tape.sigma(along),
        tape.sigma(offset),
    ];
    let position = f(&inputs)?;
    Ok(Solved {
        position,
        covariance: propagate(&inputs, &sig, &|v| f(v).unwrap_or(position)),
        residuals: vec![],
        worst_normalised: None,
    })
}

/// The point at the taped distances from the reference points. With two references the
/// circles meet in two points, and `side` (of the line from the first reference to the
/// second) picks one. With more, the position is the least-squares fit to all distances,
/// and `side` is only needed if the references are in a line.
pub fn triangulate(
    refs: &[(P2, f64)],
    side: Option<Side>,
    known_sigma: f64,
    tape: Tape,
) -> Result<Solved, HandError> {
    if refs.len() < 2 {
        return Err(HandError::NeedReferences(2));
    }
    let n = refs.len();
    let inputs: Vec<f64> = refs.iter().flat_map(|(p, d)| [p[0], p[1], *d]).collect();
    let sig: Vec<f64> = refs
        .iter()
        .flat_map(|(_, d)| [known_sigma, known_sigma, tape.sigma(*d)])
        .collect();
    let unpack = |v: &[f64]| -> Vec<(P2, f64)> {
        (0..n)
            .map(|i| ([v[3 * i], v[3 * i + 1]], v[3 * i + 2]))
            .collect()
    };
    let collinear = collinear(&refs.iter().map(|r| r.0).collect::<Vec<_>>());
    if collinear && side.is_none() {
        return Err(HandError::SideNeeded);
    }
    // For more than two references that aren't in a line, the side follows from the data:
    // try both starts and keep the better fit.
    let solve = |v: &[f64]| -> Result<P2, HandError> {
        let r = unpack(v);
        let sides = match side {
            Some(s) if collinear || n == 2 => vec![s],
            _ => vec![Side::Left, Side::Right],
        };
        let mut best: Option<(f64, P2)> = None;
        let mut last_err = None;
        for s in sides {
            match two_circles(r[0].0, r[0].1, r[1].0, r[1].1, s) {
                Ok(p0) => {
                    let p = if n == 2 { p0 } else { least_squares(&r, p0) };
                    let cost: f64 = r.iter().map(|(q, d)| (norm(sub(p, *q)) - d).powi(2)).sum();
                    if best.is_none_or(|b| cost < b.0) {
                        best = Some((cost, p));
                    }
                }
                Err(e) => last_err = Some(e),
            }
        }
        best.map(|b| b.1)
            .ok_or(last_err.unwrap_or(HandError::NeedReferences(2)))
    };
    let position = solve(&inputs)?;
    let residuals: Vec<f64> = refs
        .iter()
        .map(|(q, d)| norm(sub(position, *q)) - d)
        .collect();
    let worst_normalised = (n > 2).then(|| {
        refs.iter()
            .zip(&residuals)
            .map(|((_, d), r)| r.abs() / tape.sigma(*d).max(1e-12))
            .fold(0.0, f64::max)
    });
    Ok(Solved {
        position,
        covariance: propagate(&inputs, &sig, &|v| solve(v).unwrap_or(position)),
        residuals,
        worst_normalised,
    })
}

fn collinear(pts: &[P2]) -> bool {
    let a = pts[0];
    let Some(b) = pts
        .iter()
        .skip(1)
        .copied()
        .max_by(|p, q| norm(sub(*p, a)).total_cmp(&norm(sub(*q, a))))
    else {
        return true;
    };
    let d = sub(b, a);
    let len = norm(d);
    if len < 1e-12 {
        return true;
    }
    pts.iter()
        .all(|p| (d[0] * (p[1] - a[1]) - d[1] * (p[0] - a[0])).abs() / len < 1e-9 * len.max(1.0))
}

/// Where circles about `a` (radius `ra`) and `b` (radius `rb`) meet, on `side` of a→b.
fn two_circles(a: P2, ra: f64, b: P2, rb: f64, side: Side) -> Result<P2, HandError> {
    let ab = sub(b, a);
    let d = norm(ab);
    if d < 1e-12 {
        return Err(HandError::CoincidentReferences);
    }
    let x = (ra * ra - rb * rb + d * d) / (2.0 * d);
    let h2 = ra * ra - x * x;
    // Tape readings that just miss (within 0.1 mm) are treated as touching.
    if h2 < -(1e-4f64).powi(2) {
        // Too far apart for the two tapes, or one circle inside the other.
        let gap = if d > ra + rb {
            d - ra - rb
        } else {
            (ra - rb).abs() - d
        };
        return Err(HandError::NoIntersection(gap));
    }
    let h = h2.max(0.0).sqrt();
    let u = [ab[0] / d, ab[1] / d];
    let n = match side {
        Side::Left => [-u[1], u[0]],
        Side::Right => [u[1], -u[0]],
    };
    Ok([a[0] + u[0] * x + n[0] * h, a[1] + u[1] * x + n[1] * h])
}

/// Gauss–Newton on Σ (|p − q_i| − d_i)², from `p`.
fn least_squares(refs: &[(P2, f64)], mut p: P2) -> P2 {
    for _ in 0..50 {
        let (mut a11, mut a12, mut a22, mut b1, mut b2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (q, d) in refs {
            let v = sub(p, *q);
            let r = norm(v);
            if r < 1e-12 {
                continue;
            }
            let j = [v[0] / r, v[1] / r];
            let e = r - d;
            a11 += j[0] * j[0];
            a12 += j[0] * j[1];
            a22 += j[1] * j[1];
            b1 += j[0] * e;
            b2 += j[1] * e;
        }
        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-18 {
            break;
        }
        let dx = -(a22 * b1 - a12 * b2) / det;
        let dy = -(a11 * b2 - a12 * b1) / det;
        p = [p[0] + dx, p[1] + dy];
        if dx.hypot(dy) < 1e-13 {
            break;
        }
    }
    p
}

/// First-order covariance of `f(inputs)` for independent inputs with 1σ `sig`.
fn propagate(inputs: &[f64], sig: &[f64], f: &dyn Fn(&[f64]) -> P2) -> [[f64; 2]; 2] {
    let mut cov = [[0.0; 2]; 2];
    let mut v = inputs.to_vec();
    for i in 0..inputs.len() {
        let h = 1e-6 * inputs[i].abs().max(1.0);
        v[i] = inputs[i] + h;
        let up = f(&v);
        v[i] = inputs[i] - h;
        let down = f(&v);
        v[i] = inputs[i];
        let g = [(up[0] - down[0]) / (2.0 * h), (up[1] - down[1]) / (2.0 * h)];
        for r in 0..2 {
            for c in 0..2 {
                cov[r][c] += g[r] * g[c] * sig[i] * sig[i];
            }
        }
    }
    cov
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXACT: Tape = Tape {
        fixed: 0.0,
        per_metre: 0.0,
    };
    const TAPE: Tape = Tape {
        fixed: 0.002,
        per_metre: 0.001,
    };

    fn dist(a: P2, b: P2) -> f64 {
        norm(sub(a, b))
    }

    #[test]
    fn triangulation_reproduces_known_positions_exactly() {
        // Two references 7.3 m apart and points on both sides, near and far, including one
        // on the baseline's extension.
        let (a, b) = ([12.5, -3.25], [19.8, -3.25]);
        for p in [
            [15.0, 2.0],
            [14.0, -8.5],
            [30.25, 11.125],
            [-4.0, -3.0],
            [16.15, -0.001],
        ] {
            let side = if (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) > 0.0 {
                Side::Left
            } else {
                Side::Right
            };
            let s =
                triangulate(&[(a, dist(p, a)), (b, dist(p, b))], Some(side), 0.0, EXACT).unwrap();
            assert!(dist(s.position, p) < 1e-12, "{p:?} → {:?}", s.position);
        }
        // Three references, no side needed.
        let c = [14.0, 6.0];
        let p = [17.3, 1.9];
        let s = triangulate(
            &[(a, dist(p, a)), (b, dist(p, b)), (c, dist(p, c))],
            None,
            0.0,
            EXACT,
        )
        .unwrap();
        assert!(dist(s.position, p) < 1e-12);
        assert!(s.residuals.iter().all(|r| r.abs() < 1e-12));
    }

    #[test]
    fn baseline_offset_by_hand() {
        // Baseline along +x from (2, 1); 3 m along, 1.5 m to the left (+y) and right (−y).
        let (a, b) = ([2.0, 1.0], [12.0, 1.0]);
        let l = baseline_offset(a, b, 3.0, 1.5, Side::Left, 0.0, EXACT).unwrap();
        let r = baseline_offset(a, b, 3.0, 1.5, Side::Right, 0.0, EXACT).unwrap();
        assert!(dist(l.position, [5.0, 2.5]) < 1e-12);
        assert!(dist(r.position, [5.0, -0.5]) < 1e-12);
        // A rotated baseline: 45°, 2 along, 1 left.
        let s = baseline_offset([0.0, 0.0], [1.0, 1.0], 2.0, 1.0, Side::Left, 0.0, EXACT).unwrap();
        let k = std::f64::consts::FRAC_1_SQRT_2;
        assert!(dist(s.position, [2.0 * k - k, 2.0 * k + k]) < 1e-12);
    }

    #[test]
    fn uncertainty_for_a_baseline_offset_matches_the_tape() {
        // Along x with exact references: σx is the along tape's, σy the offset tape's.
        let s = baseline_offset([0.0, 0.0], [10.0, 0.0], 4.0, 2.0, Side::Left, 0.0, TAPE).unwrap();
        assert!((s.covariance[0][0].sqrt() - TAPE.sigma(4.0)).abs() < 1e-9);
        assert!((s.covariance[1][1].sqrt() - TAPE.sigma(2.0)).abs() < 1e-9);
    }

    #[test]
    fn triangulation_uncertainty_matches_monte_carlo() {
        let (a, b, p) = ([0.0, 0.0], [8.0, 0.0], [3.0, 5.0]);
        let (da, db) = (dist(p, a), dist(p, b));
        let s = triangulate(&[(a, da), (b, db)], Some(Side::Left), 0.003, TAPE).unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut unit = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut normal = || {
            let (u, v) = (unit().max(1e-300), unit());
            (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
        };
        let runs = 20_000;
        let (mut sx, mut sxx, mut sy, mut syy) = (0.0, 0.0, 0.0, 0.0);
        for _ in 0..runs {
            let a2 = [a[0] + 0.003 * normal(), a[1] + 0.003 * normal()];
            let b2 = [b[0] + 0.003 * normal(), b[1] + 0.003 * normal()];
            let r = triangulate(
                &[
                    (a2, da + TAPE.sigma(da) * normal()),
                    (b2, db + TAPE.sigma(db) * normal()),
                ],
                Some(Side::Left),
                0.0,
                EXACT,
            )
            .unwrap();
            sx += r.position[0];
            sxx += r.position[0] * r.position[0];
            sy += r.position[1];
            syy += r.position[1] * r.position[1];
        }
        let n = runs as f64;
        let vx = sxx / n - (sx / n).powi(2);
        let vy = syy / n - (sy / n).powi(2);
        assert!(
            (vx / s.covariance[0][0] - 1.0).abs() < 0.05,
            "x {vx} vs {}",
            s.covariance[0][0]
        );
        assert!(
            (vy / s.covariance[1][1] - 1.0).abs() < 0.05,
            "y {vy} vs {}",
            s.covariance[1][1]
        );
    }

    #[test]
    fn a_wrong_third_tape_is_flagged() {
        let (a, b, c, p) = ([0.0, 0.0], [8.0, 0.0], [4.0, 9.0], [3.0, 5.0]);
        let good = triangulate(
            &[(a, dist(p, a)), (b, dist(p, b)), (c, dist(p, c))],
            None,
            0.0,
            TAPE,
        )
        .unwrap();
        assert!(good.worst_normalised.unwrap() < 1e-6);
        // The third distance misread by 10 cm.
        let bad = triangulate(
            &[(a, dist(p, a)), (b, dist(p, b)), (c, dist(p, c) + 0.1)],
            None,
            0.0,
            TAPE,
        )
        .unwrap();
        assert!(
            bad.worst_normalised.unwrap() > 3.0,
            "{:?}",
            bad.worst_normalised
        );
    }

    #[test]
    fn impossible_or_ambiguous_inputs_are_refused() {
        let (a, b) = ([0.0, 0.0], [10.0, 0.0]);
        assert!(matches!(
            triangulate(&[(a, 3.0), (b, 3.0)], Some(Side::Left), 0.0, EXACT),
            Err(HandError::NoIntersection(_))
        ));
        assert_eq!(
            triangulate(&[(a, 6.0), (b, 6.0)], None, 0.0, EXACT),
            Err(HandError::SideNeeded)
        );
        // Three references in a line still need a side.
        assert_eq!(
            triangulate(
                &[(a, 6.0), (b, 6.0), ([5.0, 0.0], 3.3166247903554)],
                None,
                0.0,
                EXACT
            ),
            Err(HandError::SideNeeded)
        );
        assert_eq!(
            triangulate(&[(a, 6.0), (a, 6.0)], Some(Side::Left), 0.0, EXACT),
            Err(HandError::CoincidentReferences)
        );
        assert_eq!(
            baseline_offset(a, a, 1.0, 1.0, Side::Left, 0.0, EXACT),
            Err(HandError::CoincidentReferences)
        );
        // Tapes that touch: one solution on the baseline, accepted.
        let s = triangulate(&[(a, 4.0), (b, 6.0)], Some(Side::Left), 0.0, EXACT).unwrap();
        assert!(dist(s.position, [4.0, 0.0]) < 1e-12);
    }
}
