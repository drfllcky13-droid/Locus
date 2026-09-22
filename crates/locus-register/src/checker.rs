//! Checkerboard target detection (2 × 2 squares) in a single scan, from intensity.
//!
//! 1. Candidates: cells half a board wide whose points span a high intensity contrast
//!    (a dark and a bright material side by side).
//! 2. Around each candidate, a plane is fitted robustly (RANSAC, then least squares on the
//!    inliers) and its points are labelled dark or bright at the midpoint of the 10th and
//!    90th intensity percentiles.
//! 3. A coarse search finds the centre and rotation of the saddle pattern that best explains
//!    the labels inside a circle inscribed in the board.
//! 4. Refinement: every pair of neighbouring points with different labels gives an edge
//!    sample at their midpoint. The samples are split between the two edge lines, each line
//!    is fitted by least squares, and the centre is their intersection. Repeated three times.
//! 5. Accepted when the labels agree with the pattern, both lines are well supported and
//!    close to perpendicular. Centre precision is limited by point spacing: each edge sample
//!    is uncertain by up to half a spacing, averaged over the samples on each line.

use crate::normals::{knn, within, Tree};
use nalgebra::{Matrix2, Matrix3, SymmetricEigen, Vector2, Vector3};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct BoardSearch {
    /// Board edge length (m); the pattern is 2 × 2 squares of half this.
    pub size: f64,
    /// Fewest points inside the inscribed circle.
    pub min_points: usize,
    /// Fewest points agreeing with the fitted pattern, as a fraction.
    pub min_agreement: f64,
    /// Smallest ratio of bright to dark intensity (90th to 10th percentile).
    pub min_contrast: f64,
}

impl BoardSearch {
    pub fn new(size: f64) -> Self {
        BoardSearch {
            size,
            min_points: 40,
            min_agreement: 0.9,
            min_contrast: 4.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Board {
    /// Where the four squares meet, on the board surface, in the scan's frame (m).
    pub centre: [f64; 3],
    /// Unit normal, toward the scanner.
    pub normal: [f64; 3],
    /// Covariance of `centre` (m²).
    pub covariance: Matrix3<f64>,
    /// Fraction of points whose label matches the fitted pattern.
    pub agreement: f64,
    /// Points inside the inscribed circle.
    pub points: usize,
}

struct Patch {
    idx: Vec<usize>,
    uv: Vec<Vector2<f64>>,
    bright: Vec<bool>,
    origin: Vector3<f64>,
    u: Vector3<f64>,
    v: Vector3<f64>,
    n: Vector3<f64>,
    plane_rms: f64,
}

fn percentile(v: &mut [f64], q: f64) -> f64 {
    let k = ((v.len() - 1) as f64 * q).round() as usize;
    *v.select_nth_unstable_by(k, f64::total_cmp).1
}

/// Find checkerboards among `points` with per-point `intensity`. Strongest first.
pub fn detect(
    points: &[[f64; 3]],
    intensity: &[f64],
    tree: &Tree,
    search: &BoardSearch,
) -> Vec<Board> {
    let cell = search.size / 2.0;
    let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        cells
            .entry(p.map(|v| (v / cell).floor() as i64))
            .or_default()
            .push(i);
    }
    let mut candidates: Vec<(usize, Vector3<f64>)> = cells
        .values()
        .filter(|ix| ix.len() >= search.min_points / 4)
        .filter_map(|ix| {
            let mut v: Vec<f64> = ix.iter().map(|&i| intensity[i]).collect();
            let (lo, hi) = (percentile(&mut v, 0.1), percentile(&mut v, 0.9));
            (hi / lo.max(1.0) >= search.min_contrast).then(|| {
                let c = ix
                    .iter()
                    .fold(Vector3::zeros(), |s, &i| s + Vector3::from(points[i]));
                (ix.len(), c / ix.len() as f64)
            })
        })
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.x.total_cmp(&b.1.x)));

    let mut tried: Vec<Vector3<f64>> = vec![];
    let mut found: Vec<Board> = vec![];
    for (_, c) in candidates {
        if tried.iter().any(|t| (t - c).norm() < cell / 2.0) {
            continue;
        }
        tried.push(c);
        let Some(b) = verify(points, intensity, tree, c, search) else {
            continue;
        };
        tried.push(Vector3::from(b.centre));
        if !found
            .iter()
            .any(|f| (Vector3::from(f.centre) - Vector3::from(b.centre)).norm() < cell)
        {
            found.push(b);
        }
    }
    found
}

/// Plane through the neighbourhood of `c`: RANSAC for inliers, then least squares.
fn patch(
    points: &[[f64; 3]],
    intensity: &[f64],
    tree: &Tree,
    c: Vector3<f64>,
    size: f64,
) -> Option<Patch> {
    let near = within(tree, &c.into(), size * 0.75);
    if near.len() < 10 {
        return None;
    }
    let p = |i: usize| Vector3::from(points[near[i]]);
    let tol = 0.005;
    let mut best: (usize, Vector3<f64>, Vector3<f64>) = (0, Vector3::zeros(), Vector3::z());
    let mut seed = 0x9e37_79b9_7f4a_7c15u64 ^ near.len() as u64;
    let mut pick = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % near.len() as u64) as usize
    };
    for _ in 0..200 {
        let (a, b, d) = (p(pick()), p(pick()), p(pick()));
        let n = (b - a).cross(&(d - a));
        if n.norm() < 1e-9 {
            continue;
        }
        let n = n.normalize();
        let count = (0..near.len())
            .filter(|&i| (p(i) - a).dot(&n).abs() < tol)
            .count();
        if count > best.0 {
            best = (count, a, n);
        }
    }
    let inl: Vec<usize> = (0..near.len())
        .filter(|&i| (p(i) - best.1).dot(&best.2).abs() < tol)
        .collect();
    if inl.len() < 10 {
        return None;
    }
    let origin = inl.iter().fold(Vector3::zeros(), |s, &i| s + p(i)) / inl.len() as f64;
    let cov = inl.iter().fold(Matrix3::zeros(), |s, &i| {
        let d = p(i) - origin;
        s + d * d.transpose()
    });
    let e = SymmetricEigen::new(cov);
    let mut n: Vector3<f64> = e.eigenvectors.column(e.eigenvalues.imin()).into();
    if n.dot(&-origin) < 0.0 {
        n = -n; // toward the scanner at the scan origin
    }
    let u = n
        .cross(&Vector3::z())
        .try_normalize(1e-6)
        .unwrap_or_else(|| n.cross(&Vector3::x()).normalize());
    let v = n.cross(&u);
    let plane_rms = (inl
        .iter()
        .map(|&i| (p(i) - origin).dot(&n).powi(2))
        .sum::<f64>()
        / inl.len() as f64)
        .sqrt();
    let mut level: Vec<f64> = inl.iter().map(|&i| intensity[near[i]]).collect();
    let (lo, hi) = (percentile(&mut level, 0.1), percentile(&mut level, 0.9));
    let mid = (lo + hi) / 2.0;
    Some(Patch {
        idx: inl.iter().map(|&i| near[i]).collect(),
        uv: inl
            .iter()
            .map(|&i| {
                let d = p(i) - origin;
                Vector2::new(d.dot(&u), d.dot(&v))
            })
            .collect(),
        bright: inl.iter().map(|&i| intensity[near[i]] > mid).collect(),
        origin,
        u,
        v,
        n,
        plane_rms,
    })
}

/// Agreement of the labels inside radius `r` of `c` with a saddle at angle `theta`, taking
/// whichever polarity fits better. Returns (agreement, points counted).
fn agreement(pa: &Patch, c: Vector2<f64>, theta: f64, r: f64) -> (f64, usize) {
    let (s, co) = theta.sin_cos();
    let (mut agree, mut n) = (0usize, 0usize);
    for (uv, &b) in pa.uv.iter().zip(&pa.bright) {
        let d = uv - c;
        if d.norm_squared() > r * r {
            continue;
        }
        let (x, y) = (d.x * co + d.y * s, -d.x * s + d.y * co);
        n += 1;
        if ((x >= 0.0) == (y >= 0.0)) == b {
            agree += 1;
        }
    }
    if n == 0 {
        return (0.0, 0);
    }
    let a = agree as f64 / n as f64;
    (a.max(1.0 - a), n)
}

/// Total-least-squares line through 2-D samples: (point on line, unit direction, rms).
fn fit_line(s: &[Vector2<f64>]) -> (Vector2<f64>, Vector2<f64>, f64) {
    let m = s.iter().sum::<Vector2<f64>>() / s.len() as f64;
    let cov = s
        .iter()
        .fold(Matrix2::zeros(), |a, p| a + (p - m) * (p - m).transpose());
    let e = SymmetricEigen::new(cov);
    let dir: Vector2<f64> = e.eigenvectors.column(e.eigenvalues.imax()).into();
    let nrm = Vector2::new(-dir.y, dir.x);
    let rms = (s.iter().map(|p| (p - m).dot(&nrm).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
    (m, dir, rms)
}

fn verify(
    points: &[[f64; 3]],
    intensity: &[f64],
    tree: &Tree,
    c: Vector3<f64>,
    search: &BoardSearch,
) -> Option<Board> {
    let pa = patch(points, intensity, tree, c, search.size)?;
    let mut level: Vec<f64> = pa.idx.iter().map(|&i| intensity[i]).collect();
    if percentile(&mut level, 0.9) / percentile(&mut level, 0.1).max(1.0) < search.min_contrast {
        return None;
    }
    let r = search.size * 0.43; // inside the board whatever its rotation
                                // Coarse search: centre on a grid across the patch, rotation in 5° steps.
    let step = search.size / 15.0;
    let span = (search.size / 2.0 / step).ceil() as i64;
    let mut best = (0.0, Vector2::zeros(), 0.0);
    for i in -span..=span {
        for j in -span..=span {
            let cc = Vector2::new(i as f64 * step, j as f64 * step);
            for k in 0..18 {
                let th = (k as f64 * 5.0).to_radians();
                let (a, n) = agreement(&pa, cc, th, r);
                if n >= search.min_points && a > best.0 {
                    best = (a, cc, th);
                }
            }
        }
    }
    let (_, mut centre, mut theta) = best;
    if best.0 < search.min_agreement {
        return None;
    }
    // Edge samples: each point paired with its nearest neighbour of the other label; the
    // midpoint of each distinct pair is a sample, and the pair's length is the local spacing.
    let slot: HashMap<usize, usize> = pa.idx.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let mut pairs = std::collections::BTreeSet::new();
    for (k, &i) in pa.idx.iter().enumerate() {
        let other = knn(tree, &points[i], 7)
            .into_iter()
            .filter_map(|j| slot.get(&j).copied())
            .find(|&kj| pa.bright[kj] != pa.bright[k]);
        if let Some(kj) = other {
            pairs.insert((k.min(kj), k.max(kj)));
        }
    }
    let edges: Vec<Vector2<f64>> = pairs
        .iter()
        .map(|&(a, b)| (pa.uv[a] + pa.uv[b]) / 2.0)
        .collect();
    let mut gaps: Vec<f64> = pairs
        .iter()
        .map(|&(a, b)| (pa.uv[a] - pa.uv[b]).norm())
        .collect();
    if gaps.is_empty() {
        return None;
    }
    let spacing = percentile(&mut gaps, 0.5);
    let mut lines = None;
    for _ in 0..3 {
        let (s, co) = theta.sin_cos();
        let (da, db) = (Vector2::new(co, s), Vector2::new(-s, co));
        let (mut a, mut b) = (vec![], vec![]);
        for e in &edges {
            let d = e - centre;
            let (along_a, along_b) = (d.dot(&da), d.dot(&db));
            // Skip samples near the crossing (ambiguous) or outside the inscribed circle.
            if d.norm() > r || d.norm() < search.size * 0.05 {
                continue;
            }
            if along_b.abs() < along_a.abs() {
                a.push(*e) // near line A (direction da)
            } else {
                b.push(*e)
            }
        }
        // Each line must be seen on both sides of the crossing; a saddle-like pattern at the
        // board's outer edge has one half missing.
        let halves = |v: &[Vector2<f64>], dir: Vector2<f64>| {
            let pos = v.iter().filter(|e| (*e - centre).dot(&dir) > 0.0).count();
            pos.min(v.len() - pos)
        };
        if halves(&a, da) < 3 || halves(&b, db) < 3 {
            return None;
        }
        let (la, lb) = (fit_line(&a), fit_line(&b));
        // Intersect p_a + s·d_a = p_b + t·d_b.
        let m = Matrix2::from_columns(&[la.1, -lb.1]);
        let st = m.try_inverse()? * (lb.0 - la.0);
        centre = la.0 + la.1 * st.x;
        theta = la.1.y.atan2(la.1.x);
        lines = Some((la, lb, a.len(), b.len()));
    }
    let (la, lb, na, nb) = lines?;
    let perpendicular = la.1.dot(&lb.1).abs() < 5f64.to_radians().sin();
    let (agree, n) = agreement(&pa, centre, theta, r);
    if !perpendicular || agree < search.min_agreement || n < search.min_points {
        return None;
    }
    // Centre uncertainty: a sample can be anywhere between its two points, so its error
    // across the line is at least spacing/√12 whatever the fit residual says; samples share
    // points, so only half of them count as independent. Along the normal: the plane fit.
    let (pa_n, pb_n) = (Vector2::new(-la.1.y, la.1.x), Vector2::new(-lb.1.y, lb.1.x));
    let m = Matrix2::from_rows(&[pa_n.transpose(), pb_n.transpose()]);
    let minv = m.try_inverse()?;
    let floor = spacing / 12f64.sqrt();
    let var = |rms: f64, n: usize| rms.max(floor).powi(2) / (n as f64 / 2.0).max(1.0);
    let d = Matrix2::from_diagonal(&Vector2::new(var(la.2, na), var(lb.2, nb)));
    let cov2 = minv * d * minv.transpose();
    let basis = Matrix3::from_columns(&[pa.u, pa.v, pa.n]);
    let mut cov_local = Matrix3::zeros();
    cov_local.fixed_view_mut::<2, 2>(0, 0).copy_from(&cov2);
    cov_local[(2, 2)] = pa.plane_rms.powi(2) / pa.idx.len() as f64;
    let c3 = pa.origin + pa.u * centre.x + pa.v * centre.y;
    Some(Board {
        centre: c3.into(),
        normal: pa.n.into(),
        covariance: basis * cov_local * basis.transpose(),
        agreement: agree,
        points: n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normals;
    use locus_synth::{scan_points, truth, Options, BOARD_SIZE};

    #[test]
    fn finds_the_synthetic_checkerboards_at_their_true_centres() {
        let opts = Options {
            scans: 2,
            points_per_scan: 6_000_000,
            seed: 11,
            ..Options::default()
        };
        let t = truth(&opts);
        for s in 0..opts.scans {
            let (mut pts, mut inten) = (vec![], vec![]);
            scan_points(&opts, &t, s, &mut |p| {
                if let Some(p) = p {
                    pts.push(p.xyz);
                    inten.push(p.intensity as f64);
                }
            });
            let tree = normals::tree(&pts);
            let found = detect(&pts, &inten, &tree, &BoardSearch::new(BOARD_SIZE));
            let inv = t.scans[s].pose.inverse();
            let local: Vec<(Vector3<f64>, Vector3<f64>)> = t
                .boards
                .iter()
                .map(|b| {
                    (
                        Vector3::from(inv.apply(b.centre)),
                        Vector3::from(inv.rotate(b.normal)),
                    )
                })
                .collect();
            // Every detection is a real board, within 3σ of its own reported uncertainty.
            let sigma = |f: &Board| f.covariance.trace().sqrt();
            let near =
                |f: &Board, c: &Vector3<f64>| (c - Vector3::from(f.centre)).norm() < 3.0 * sigma(f);
            for f in &found {
                assert!(
                    local.iter().any(|(c, _)| near(f, c)),
                    "scan {s}: board {:?} (σ {:.1} mm) matches no true board",
                    f.centre,
                    sigma(f) * 1e3
                );
                assert!(sigma(f) < 0.01);
            }
            // Every board with 60 or more points inside its inscribed circle is found.
            let mut expected = 0;
            for (c, n) in &local {
                let on = pts
                    .iter()
                    .filter(|p| {
                        let d = Vector3::from(**p) - c;
                        d.dot(n).abs() < 0.004 && (d - n * d.dot(n)).norm() < BOARD_SIZE * 0.43
                    })
                    .count();
                if on >= 60 {
                    expected += 1;
                    assert!(
                        found.iter().any(|f| near(f, c)),
                        "scan {s}: board at {:.1} m with {on} points not found",
                        c.norm()
                    );
                }
            }
            assert!(
                expected > 0,
                "scan {s} sees no board well; the test proves nothing"
            );
        }
    }
}
