//! Bloodstain area of origin. Method, assumptions and limitations: docs/methods/bloodstain.md.
//!
//! Each stain photo is aligned to its surface in the scan by point pairs (a similarity in the
//! surface's plane). The stain's edge, clicked or found automatically, is fitted with an
//! ellipse. Its impact angle is asin(width / length), and its direction of travel is along
//! the long axis toward the tail the examiner marked. That gives a ray from the stain back
//! along the droplet's path. The origin is the least-squares point nearest all the rays
//! used, with each stain's residual and a 95 % ellipsoid from bootstrap resampling.

use crate::defect::{fit_ellipse, residuals, P2};
use crate::measure::{cross, dot, eigen_sym, norm, sub, Measured, P3};
use crate::trajectory::PhotoRef;
use serde::{Deserialize, Serialize};

pub const METHOD: &str = "bloodstain/1";

/// ln Γ(x) for x > 0 (Lanczos, g = 7; relative error below 1e-13).
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    let x = x - 1.0;
    let t = x + 7.5;
    let a = C[1..]
        .iter()
        .enumerate()
        .fold(C[0], |a, (i, c)| a + c / (x + i as f64 + 1.0));
    0.5 * std::f64::consts::TAU.ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// The regularised incomplete beta function I_x(a, b) (continued fraction, Lentz).
fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    if x > (a + 1.0) / (a + b + 2.0) {
        return 1.0 - inc_beta(b, a, 1.0 - x);
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp() / a;
    let tiny = 1e-300;
    let (mut c, mut d) = (1.0, 1.0 - (a + b) * x / (a + 1.0));
    d = 1.0 / if d.abs() < tiny { tiny } else { d };
    let mut f = d;
    for m in 1..300 {
        let m = m as f64;
        for num in [
            m * (b - m) * x / ((a + 2.0 * m - 1.0) * (a + 2.0 * m)),
            -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 2.0 * m + 1.0)),
        ] {
            d = 1.0 + num * d;
            d = 1.0 / if d.abs() < tiny { tiny } else { d };
            c = 1.0 + num / c;
            c = if c.abs() < tiny { tiny } else { c };
            f *= c * d;
        }
        if (c * d - 1.0).abs() < 1e-15 {
            break;
        }
    }
    front * f
}

/// The p quantile of the F distribution with (d1, d2) degrees of freedom (bisection).
fn f_quantile(d1: f64, d2: f64, p: f64) -> f64 {
    let cdf = |f: f64| inc_beta(d1 / 2.0, d2 / 2.0, d1 * f / (d1 * f + d2));
    let (mut lo, mut hi) = (0.0, 1.0);
    while cdf(hi) < p {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// The squared Mahalanobis radius of a 95 % region for a point estimated from `n` stains,
/// with its covariance also estimated from them (Hotelling's T²): p n / (n − p) ·
/// F(p, n − p) for p = 3. It tends to χ²₃ (7.81) for many stains and grows for few.
fn radius2_95(n: usize) -> f64 {
    let (p, n) = (3.0, n as f64);
    p * n / (n - p) * f_quantile(p, n - p, 0.95)
}

pub const ASSUMPTIONS: &[&str] = &[
    "Each droplet travelled in a straight line from the origin to its stain. Gravity and air drag are ignored (see the limitations).",
    "Each stain is the ellipse of a spherical droplet striking a flat, smooth surface: width over length is the sine of the impact angle, and the long axis lies along the direction of travel, toward the tail.",
    "The droplets came from one origin, at about the same time.",
    "Each photo is flat on the stain's surface and taken square on to it, so the alignment is a similarity (scale, rotation and shift) in the surface's plane.",
    "Only wall stains clearly moving upward at impact are used (upward by more than twice the direction's 1σ), unless the examiner has included the others with a stated reason.",
];

pub const LIMITATIONS: &[&str] = &[
    "Straight-line paths ignore gravity and drag. Real droplets fall along curved paths, so the straight-line origin is usually too high. The height is the least reliable coordinate, and the estimate is best read as an upper bound on it.",
    "Width over length is sensitive to measurement for nearly round stains (impact angles above about 70°): a small error in either axis moves the angle a lot.",
    "Rough, absorbent or textured surfaces, satellite spatter, and stains that ran, dried unevenly or overlap distort the ellipse.",
    "The ellipsoid is from resampling the stains used. It does not include any bias from the straight-line model, the photo alignment, or stains chosen from one side of the pattern.",
    "The origin is where the rays pass closest together. It does not say what caused the pattern, or how many events there were.",
];

#[derive(Debug, Clone, PartialEq)]
pub enum BloodstainError {
    Alignment(&'static str),
    Edges(&'static str),
    Fit(&'static str),
    Origin(String),
}

impl std::fmt::Display for BloodstainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BloodstainError::Alignment(m) => write!(f, "photo alignment: {m}"),
            BloodstainError::Edges(m) => write!(f, "stain edge: {m}"),
            BloodstainError::Fit(m) => write!(f, "ellipse fit: {m}"),
            BloodstainError::Origin(m) => write!(f, "{m}"),
        }
    }
}

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: P3, s: f64) -> P3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn unit(a: P3) -> P3 {
    scale(a, 1.0 / norm(a))
}

/// A surface's in-plane axes: to the right and up, seen facing the surface from the side its
/// normal points to; on a floor or ceiling, along +x and n × x.
pub fn surface_axes(n: P3) -> (P3, P3) {
    let e1 = if n[2].abs() > 0.9 {
        unit(sub([1.0, 0.0, 0.0], scale(n, n[0])))
    } else {
        unit(cross([0.0, 0.0, 1.0], n))
    };
    (e1, cross(n, e1))
}

// ---------------------------------------------------------------------------------------
// Photo alignment
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignPair {
    /// Pixel in the photo (x right, y down).
    pub px: P2,
    /// The same point in the scan (m, project frame).
    pub world: P3,
}

/// A photo placed on its surface: pixel (x, y) is at `origin + x · x_step + y · y_step`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    pub pairs: Vec<AlignPair>,
    /// The surface's plane (from the scan around the stain).
    pub plane_point: P3,
    pub plane_normal: P3,
    pub origin: P3,
    /// One pixel's step along the image's x and y (m, project frame).
    pub x_step: P3,
    pub y_step: P3,
    pub pixels_per_metre: f64,
    /// Each pair's distance from where the alignment puts its pixel (m).
    pub residuals: Vec<f64>,
    /// RMS of the residuals; none with two pairs (they fix the alignment exactly).
    pub rms: Option<f64>,
}

impl Alignment {
    pub fn to_world(&self, px: P2) -> P3 {
        add(
            self.origin,
            add(scale(self.x_step, px[0]), scale(self.y_step, px[1])),
        )
    }

    /// A pixel's position in the surface's own axes (m from the plane point).
    fn to_plane(&self, px: P2) -> P2 {
        let (e1, e2) = surface_axes(self.plane_normal);
        let r = sub(self.to_world(px), self.plane_point);
        [dot(r, e1), dot(r, e2)]
    }
}

/// Align a photo to its surface from two or more pixel–scan point pairs, as a similarity
/// in the surface's plane (no mirror image, no perspective).
pub fn align_photo(
    pairs: &[AlignPair],
    plane_point: P3,
    plane_normal: P3,
) -> Result<Alignment, BloodstainError> {
    if pairs.len() < 2 {
        return Err(BloodstainError::Alignment(
            "give at least two point pairs (three to check the fit)",
        ));
    }
    let n = unit(plane_normal);
    let (e1, e2) = surface_axes(n);
    let p: Vec<P2> = pairs
        .iter()
        .map(|a| {
            let r = sub(a.world, plane_point);
            [dot(r, e1), dot(r, e2)]
        })
        .collect();
    // Image y runs down; flip it so both frames are right-handed.
    let q: Vec<P2> = pairs.iter().map(|a| [a.px[0], -a.px[1]]).collect();
    let k = pairs.len() as f64;
    let mean = |v: &[P2]| {
        [
            v.iter().map(|a| a[0]).sum::<f64>() / k,
            v.iter().map(|a| a[1]).sum::<f64>() / k,
        ]
    };
    let (pm, qm) = (mean(&p), mean(&q));
    let (mut sqq, mut sa, mut sb) = (0.0, 0.0, 0.0);
    for (pi, qi) in p.iter().zip(&q) {
        let (dp, dq) = (
            [pi[0] - pm[0], pi[1] - pm[1]],
            [qi[0] - qm[0], qi[1] - qm[1]],
        );
        sqq += dq[0] * dq[0] + dq[1] * dq[1];
        sa += dq[0] * dp[0] + dq[1] * dp[1];
        sb += dq[0] * dp[1] - dq[1] * dp[0];
    }
    if sqq < 1.0 {
        return Err(BloodstainError::Alignment(
            "the photo points are on top of each other",
        ));
    }
    // p = M q + t with M = [[a, −b], [b, a]].
    let (a, b) = (sa / sqq, sb / sqq);
    let s = a.hypot(b);
    if s.is_nan() || s <= 0.0 {
        return Err(BloodstainError::Alignment(
            "the scan points are on top of each other",
        ));
    }
    let t = [
        pm[0] - (a * qm[0] - b * qm[1]),
        pm[1] - (b * qm[0] + a * qm[1]),
    ];
    let on = |v: P2| add(scale(e1, v[0]), scale(e2, v[1]));
    let origin = add(plane_point, on(t));
    let x_step = on([a, b]);
    let y_step = on([b, -a]);
    let residuals: Vec<f64> = pairs
        .iter()
        .map(|pr| {
            let w = add(
                origin,
                add(scale(x_step, pr.px[0]), scale(y_step, pr.px[1])),
            );
            norm(sub(w, pr.world))
        })
        .collect();
    let rms = (pairs.len() > 2).then(|| (residuals.iter().map(|r| r * r).sum::<f64>() / k).sqrt());
    Ok(Alignment {
        pairs: pairs.to_vec(),
        plane_point,
        plane_normal: n,
        origin,
        x_step,
        y_step,
        pixels_per_metre: 1.0 / s,
        residuals,
        rms,
    })
}

// ---------------------------------------------------------------------------------------
// Stain edges and ellipse
// ---------------------------------------------------------------------------------------

/// The edge of the dark region around `seed` in a greyscale image (row-major), where the
/// brightness crosses `threshold`, to a fraction of a pixel (linear between pixel centres).
/// Pixel (x, y)'s centre is at (x + 0.5, y + 0.5).
pub fn stain_edges(
    luma: &[u8],
    width: usize,
    height: usize,
    seed: P2,
    threshold: u8,
) -> Result<Vec<P2>, BloodstainError> {
    if luma.len() != width * height {
        return Err(BloodstainError::Edges(
            "the image size doesn't match its pixels",
        ));
    }
    let (sx, sy) = (seed[0].floor() as i64, seed[1].floor() as i64);
    if sx < 0 || sy < 0 || sx >= width as i64 || sy >= height as i64 {
        return Err(BloodstainError::Edges("the seed is outside the photo"));
    }
    let at = |x: usize, y: usize| luma[y * width + x];
    if at(sx as usize, sy as usize) >= threshold {
        return Err(BloodstainError::Edges(
            "the seed isn't in the stain (it's brighter than the threshold)",
        ));
    }
    let mut inside = vec![false; width * height];
    let mut stack = vec![(sx as usize, sy as usize)];
    inside[sy as usize * width + sx as usize] = true;
    while let Some((x, y)) = stack.pop() {
        if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
            return Err(BloodstainError::Edges(
                "the stain runs off the photo (or the threshold is too high)",
            ));
        }
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            let i = ny * width + nx;
            if !inside[i] && at(nx, ny) < threshold {
                inside[i] = true;
                stack.push((nx, ny));
            }
        }
    }
    let thr = threshold as f64;
    let mut edges = vec![];
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            if !inside[y * width + x] {
                continue;
            }
            let li = at(x, y) as f64;
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = ((x as i64 + dx) as usize, (y as i64 + dy) as usize);
                if inside[ny * width + nx] {
                    continue;
                }
                let lo = at(nx, ny) as f64;
                let f = ((thr - li) / (lo - li)).clamp(0.0, 1.0);
                edges.push([
                    x as f64 + 0.5 + dx as f64 * f,
                    y as f64 + 0.5 + dy as f64 * f,
                ]);
            }
        }
    }
    Ok(edges)
}

/// A stain's fitted ellipse, on its surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StainFit {
    /// Edge points given, and those left out as not on the ellipse (the tail, satellites).
    pub edge_points: usize,
    pub trimmed: usize,
    /// RMS distance of the kept points from the ellipse (m).
    pub rms: f64,
    pub centre: P3,
    /// Full width and length (m), 1σ: the fit's own, and the edge method's systematic
    /// error in quadrature (`EDGE_SYSTEMATIC`).
    pub width: Measured,
    pub length: Measured,
    /// The long axis in the surface (unit; its sign is set by the tail, not the fit).
    pub long_axis: P3,
    pub axis_sigma_deg: f64,
}

/// The edge method's systematic error, measured on the generator's stain photos
/// (crates/locus-validate/tests/bloodstain.rs): 0.2 % on each axis (the tail's base and the
/// threshold) and 0.1° on the long axis's direction. The fit's covariance alone sees only
/// the scatter of the edge points: without this the origin's χ² was 7 × its degrees of
/// freedom and the origin 2.5 mm low; with it, χ² is about half and the error under 1 mm.
pub const EDGE_SYSTEMATIC: (f64, f64) = (0.002, 0.1);

/// Fit an ellipse to a stain's edge points (pixels in its aligned photo). Points well off
/// the ellipse (the tail, a satellite touching the stain) are left out, repeatedly, by a
/// robust cut: more than 3 × 1.4826 × the median absolute residual, and more than a pixel.
pub fn fit_stain(edges_px: &[P2], al: &Alignment) -> Result<StainFit, BloodstainError> {
    let all: Vec<P2> = edges_px.iter().map(|p| al.to_plane(*p)).collect();
    let pixel = 1.0 / al.pixels_per_metre;
    let mut kept = all.clone();
    let mut fit = None;
    for _ in 0..10 {
        if kept.len() < 8 {
            return Err(BloodstainError::Fit("fewer than 8 edge points"));
        }
        let Some((e, inv)) = fit_ellipse(&kept) else {
            return Err(BloodstainError::Fit("the edge points don't fit an ellipse"));
        };
        let r = residuals(&e, &all);
        let mut abs: Vec<f64> = residuals(&e, &kept).iter().map(|v| v.abs()).collect();
        abs.sort_by(f64::total_cmp);
        let cut = (3.0 * 1.4826 * abs[abs.len() / 2]).max(pixel);
        let next: Vec<P2> = all
            .iter()
            .zip(&r)
            .filter(|(_, v)| v.abs() <= cut)
            .map(|(p, _)| *p)
            .collect();
        let done = next.len() == kept.len();
        fit = Some((e, inv));
        if done {
            break;
        }
        kept = next;
    }
    let (e, inv) = fit.expect("at least one fit");
    let r = residuals(&e, &kept);
    let dof = (kept.len() as f64 - 5.0).max(1.0);
    let s2 = r.iter().map(|v| v * v).sum::<f64>() / dof;
    let sd = |k: usize| (s2 * inv[k][k]).max(0.0).sqrt();
    let (e1, e2) = surface_axes(al.plane_normal);
    let on = |u: f64, v: f64| add(scale(e1, u), scale(e2, v));
    Ok(StainFit {
        edge_points: all.len(),
        trimmed: all.len() - kept.len(),
        rms: (r.iter().map(|v| v * v).sum::<f64>() / kept.len() as f64).sqrt(),
        centre: add(al.plane_point, on(e[0], e[1])),
        width: Measured {
            value: 2.0 * e[3],
            sigma: (2.0 * sd(3)).hypot(2.0 * e[3] * EDGE_SYSTEMATIC.0),
        },
        length: Measured {
            value: 2.0 * e[2],
            sigma: (2.0 * sd(2)).hypot(2.0 * e[2] * EDGE_SYSTEMATIC.0),
        },
        long_axis: on(e[4].cos(), e[4].sin()),
        axis_sigma_deg: sd(4).to_degrees().hypot(EDGE_SYSTEMATIC.1),
    })
}

// ---------------------------------------------------------------------------------------
// The origin
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StainInput {
    pub label: String,
    pub surface: String,
    pub centre: P3,
    /// Unit normal of the surface, on the side the blood came from.
    pub normal: P3,
    /// Full width and length of the ellipse (m), 1σ.
    pub width: Measured,
    pub length: Measured,
    /// Direction of travel in the surface: along the long axis, toward the tail (unit).
    pub travel: P3,
    pub travel_sigma_deg: f64,
    /// Left out by the examiner, with the reason.
    #[serde(default)]
    pub excluded: Option<String>,
    #[serde(default)]
    pub fit: Option<StainFit>,
    #[serde(default)]
    pub alignment: Option<Alignment>,
    #[serde(default)]
    pub photo: Option<PhotoRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameters {
    /// Floor elevation (m), for the origin's height.
    pub floor_z: f64,
    /// Bootstrap resamples, and the seed (so a run can be repeated exactly).
    pub bootstrap: usize,
    pub seed: u64,
    /// Use stains that aren't clearly moving upward too (floor stains, downward or
    /// uncertain directions); the examiner's reason.
    #[serde(default)]
    pub include_not_upward: Option<String>,
    /// Floor stains' direction: clockwise from this axis, `reference_deg` clockwise from
    /// project +y.
    pub reference: String,
    pub reference_deg: f64,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            floor_z: 0.0,
            bootstrap: 2000,
            seed: 1,
            include_not_upward: None,
            reference: "project north (+y)".into(),
            reference_deg: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StainResult {
    /// asin(width / length), degrees.
    pub impact: Measured,
    /// Direction of travel in the surface, degrees: on a wall clockwise from straight up,
    /// seen facing the wall; on a floor clockwise from the reference axis.
    pub directionality: Measured,
    /// Unit direction from the stain back along the droplet's path.
    pub ray: P3,
    /// Moving upward at impact, and clearly so: a wall stain whose direction is within
    /// 90° − 2σ of straight up. A stain whose direction is too uncertain can't be relied
    /// on to be moving upward.
    pub upward: bool,
    pub clearly_upward: bool,
    pub used: bool,
    /// Why it wasn't used.
    pub not_used: Option<String>,
    /// Distance of the origin from this stain's ray (m), used or not, and the same in
    /// units of the ray's uncertainty at that distance (√χ², 2 degrees of freedom).
    pub residual: f64,
    pub residual_sigmas: f64,
    /// The origin is behind the stain along its ray (the ray points away from it).
    pub behind: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ellipsoid {
    /// 95 % semi-axes (m), largest first, and their directions (unit).
    pub semi_axes: [f64; 3],
    pub axes: [P3; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    pub point: P3,
    /// Height above the floor (m), 1σ from the bootstrap.
    pub height: Measured,
    /// 1σ in x, y, z from the bootstrap (m).
    pub sigma: P3,
    pub ellipsoid: Ellipsoid,
    /// RMS distance of the origin from the rays used (m).
    pub rms_residual: f64,
    /// Σ of the squared residuals in σ units, on 2n − 3 degrees of freedom: near the
    /// degrees of freedom when the stated uncertainties describe the scatter.
    pub chi2: f64,
    pub dof: usize,
    pub stains_used: usize,
    /// Resamples, and those that failed (rays nearly parallel) and were left out.
    pub bootstrap: usize,
    pub bootstrap_failed: usize,
    /// How well the rays pin the point down: the smallest eigenvalue of the mean of
    /// (I − r rᵀ), between 0 (all parallel) and 2/3 (spread evenly in every direction).
    pub conditioning: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub method: String,
    pub inputs: Vec<StainInput>,
    pub parameters: Parameters,
    pub stains: Vec<StainResult>,
    pub origin: Origin,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

fn clockwise_from(v: P2) -> f64 {
    v[0].atan2(v[1]).to_degrees().rem_euclid(360.0)
}

/// Impact angle and direction of one stain, and its ray back along the path.
pub fn stain(s: &StainInput, p: &Parameters) -> Result<StainResult, BloodstainError> {
    let (w, l) = (s.width, s.length);
    if !(w.value > 0.0 && l.value >= w.value) {
        return Err(BloodstainError::Origin(format!(
            "{}: the width must be positive and no more than the length",
            s.label
        )));
    }
    let n = unit(s.normal);
    let t = unit(sub(s.travel, scale(n, dot(s.travel, n))));
    let ratio = w.value / l.value;
    let alpha = ratio.asin();
    let ratio_sigma = ratio * ((w.sigma / w.value).powi(2) + (l.sigma / l.value).powi(2)).sqrt();
    let impact = Measured {
        value: alpha.to_degrees(),
        sigma: (ratio_sigma / alpha.cos().max(1e-9)).to_degrees().min(90.0),
    };
    let (e1, e2) = surface_axes(n);
    let directionality = if n[2].abs() > 0.9 {
        clockwise_from([dot(t, e1), dot(t, e2)]) - p.reference_deg
    } else {
        // Clockwise from up, seen facing the wall: e1 is right, e2 is up.
        clockwise_from([dot(t, e1), dot(t, e2)])
    };
    let v = sub(scale(t, alpha.cos()), scale(n, alpha.sin()));
    Ok(StainResult {
        impact,
        directionality: Measured {
            value: directionality.rem_euclid(360.0),
            sigma: s.travel_sigma_deg,
        },
        ray: scale(v, -1.0),
        upward: v[2] > 0.0,
        clearly_upward: n[2].abs() <= 0.9 && {
            let from_up = directionality.rem_euclid(360.0);
            from_up.min(360.0 - from_up) + 2.0 * s.travel_sigma_deg < 90.0
        },
        used: false,
        not_used: None,
        residual: 0.0,
        residual_sigmas: 0.0,
        behind: false,
    })
}

/// A stain as the origin fit sees it: where it is, its surface, and the two angles measured
/// from it with their 1σ (radians).
#[derive(Clone, Copy)]
struct Ray {
    c: P3,
    /// Unit ray back along the measured path (for the starting point and conditioning).
    r: P3,
    n: P3,
    e1: P3,
    e2: P3,
    alpha: f64,
    phi: f64,
    sa: f64,
    sp: f64,
}

/// The smallest angular 1σ the fit uses (0.01°), so an exact input can't take all the weight.
const MIN_SIGMA: f64 = 1.745e-4;

impl Ray {
    /// The impact angle and direction a droplet from `x` would have had at this stain,
    /// against those measured, in σ units.
    fn misfit(&self, x: P3) -> [f64; 2] {
        let u = sub(x, self.c);
        let d = norm(u);
        if d < 1e-9 {
            return [0.0, 0.0];
        }
        let u = scale(u, 1.0 / d);
        let along = dot(u, self.n);
        let alpha = along.clamp(-1.0, 1.0).asin();
        // Direction of travel in the surface: against the in-plane part of u.
        let w = sub(scale(self.n, along), u);
        let phi = dot(w, self.e1).atan2(dot(w, self.e2));
        let dphi = (phi - self.phi + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        [
            (alpha - self.alpha) / self.sa.max(MIN_SIGMA),
            dphi / self.sp.max(MIN_SIGMA),
        ]
    }
    fn chi2(&self, x: P3) -> f64 {
        let [a, b] = self.misfit(x);
        a * a + b * b
    }
}

/// The plain least-squares point nearest the rays (Σ (I − r rᵀ)(x − c) = 0): the starting
/// point for the fit.
fn nearest_lines(rays: &[Ray]) -> Option<P3> {
    let mut a = [[0.0; 3]; 3];
    let mut b = [0.0; 3];
    for ray in rays {
        for (i, (row, bi)) in a.iter_mut().zip(b.iter_mut()).enumerate() {
            for (j, aij) in row.iter_mut().enumerate() {
                let m = (i == j) as u8 as f64 - ray.r[i] * ray.r[j];
                *aij += m;
                *bi += m * ray.c[j];
            }
        }
    }
    solve3(a, b)
}

/// Solve a symmetric positive definite 3 × 3 system; none if it is nearly singular.
fn solve3(a: [[f64; 3]; 3], b: P3) -> Option<P3> {
    let (vals, vecs) = eigen_sym(a);
    let max = vals.iter().cloned().fold(0.0, f64::max);
    if !(max > 0.0 && vals.iter().all(|v| *v > max * 1e-12)) {
        return None;
    }
    Some(
        vals.iter()
            .zip(&vecs)
            .fold([0.0; 3], |x, (lam, v)| add(x, scale(*v, dot(*v, b) / lam))),
    )
}

/// The origin: the point whose paths to the stains best match the measured impact angles
/// and directions, each in units of its own 1σ (Levenberg–Marquardt from the plain
/// least-squares point).
fn nearest(rays: &[Ray]) -> Option<P3> {
    let mut x = nearest_lines(rays)?;
    let cost = |x: P3| rays.iter().map(|r| r.chi2(x)).sum::<f64>();
    let mut c0 = cost(x);
    let mut lambda = 1e-3;
    for _ in 0..100 {
        let mut jtj = [[0.0; 3]; 3];
        let mut g = [0.0; 3];
        for ray in rays {
            let r0 = ray.misfit(x);
            let h = 1e-6;
            let jac: [[f64; 2]; 3] = std::array::from_fn(|k| {
                let mut xh = x;
                xh[k] += h;
                let r1 = ray.misfit(xh);
                [(r1[0] - r0[0]) / h, (r1[1] - r0[1]) / h]
            });
            for p in 0..3 {
                for q in 0..2 {
                    g[p] -= jac[p][q] * r0[q];
                }
                for (k, row) in jac.iter().enumerate() {
                    jtj[p][k] += (0..2).map(|q| jac[p][q] * row[q]).sum::<f64>();
                }
            }
        }
        let mut stepped = false;
        for _ in 0..12 {
            let mut a = jtj;
            for (k, row) in a.iter_mut().enumerate() {
                row[k] *= 1.0 + lambda;
            }
            let Some(dx) = solve3(a, g) else {
                lambda *= 10.0;
                continue;
            };
            let x2 = add(x, dx);
            let c2 = cost(x2);
            if c2 < c0 {
                let small = norm(dx) < 1e-7;
                (x, c0) = (x2, c2);
                lambda = (lambda / 3.0).max(1e-9);
                stepped = !small;
                break;
            }
            lambda *= 10.0;
        }
        if !stepped {
            break;
        }
    }
    Some(x)
}

/// The smallest eigenvalue of the mean of (I − r rᵀ): 0 when the rays are all parallel.
fn conditioning(rays: &[Ray]) -> f64 {
    let a: [[f64; 3]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            rays.iter()
                .map(|r| (i == j) as u8 as f64 - r.r[i] * r.r[j])
                .sum::<f64>()
                / rays.len() as f64
        })
    });
    eigen_sym(a).0.iter().cloned().fold(f64::INFINITY, f64::min)
}

fn distance_to_ray(x: P3, c: P3, r: P3) -> (f64, bool) {
    let d = sub(x, c);
    let along = dot(d, r);
    (norm(sub(d, scale(r, along))), along < 0.0)
}

/// Deterministic uniform stream (xorshift64*), for the bootstrap.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as usize % n
    }
}

pub fn run(inputs: Vec<StainInput>, parameters: Parameters) -> Result<Run, BloodstainError> {
    let p = &parameters;
    let mut stains = inputs
        .iter()
        .map(|s| stain(s, p))
        .collect::<Result<Vec<_>, _>>()?;
    for (s, i) in stains.iter_mut().zip(&inputs) {
        s.not_used = if let Some(why) = &i.excluded {
            Some(format!("excluded by the examiner: {why}"))
        } else if !s.clearly_upward && p.include_not_upward.is_none() {
            Some(if !s.upward {
                "moving downward at impact".into()
            } else {
                format!(
                    "not clearly moving upward (direction {:.0}° ± {:.0}°)",
                    s.directionality.value, s.directionality.sigma
                )
            })
        } else {
            None
        };
        s.used = s.not_used.is_none();
    }
    let ray_of = |s: &StainResult, i: &StainInput| {
        let n = unit(i.normal);
        let (e1, e2) = surface_axes(n);
        let t = sub(scale(n, dot(s.ray, n)), s.ray);
        Ray {
            c: i.centre,
            r: s.ray,
            n,
            e1,
            e2,
            alpha: s.impact.value.to_radians(),
            phi: dot(t, e1).atan2(dot(t, e2)),
            sa: s.impact.sigma.to_radians(),
            sp: s.directionality.sigma.to_radians(),
        }
    };
    let all: Vec<Ray> = stains
        .iter()
        .zip(&inputs)
        .map(|(s, i)| ray_of(s, i))
        .collect();
    let rays: Vec<Ray> = all
        .iter()
        .zip(&stains)
        .filter(|(_, s)| s.used)
        .map(|(r, _)| *r)
        .collect();
    if rays.len() < 4 {
        return Err(BloodstainError::Origin(format!(
            "{} stains can be used; at least 4 are needed (3 fix a point, and more are needed to say how well){}",
            rays.len(),
            if p.include_not_upward.is_none() && stains.iter().any(|s| !s.clearly_upward) {
                " (stains not clearly moving upward are left out unless included with a reason)"
            } else {
                ""
            }
        )));
    }
    let conditioning = conditioning(&rays);
    let found = (conditioning > 1e-6).then(|| nearest(&rays)).flatten();
    let Some(x) = found else {
        return Err(BloodstainError::Origin(
            "the stains' paths are nearly parallel, so they don't cross near one point".into(),
        ));
    };
    for ((s, i), ray) in stains.iter_mut().zip(&inputs).zip(&all) {
        (s.residual, s.behind) = distance_to_ray(x, i.centre, s.ray);
        s.residual_sigmas = ray.chi2(x).sqrt();
    }
    let chi2 = rays.iter().map(|r| r.chi2(x)).sum::<f64>();
    let rms_residual = (stains
        .iter()
        .filter(|s| s.used)
        .map(|s| s.residual * s.residual)
        .sum::<f64>()
        / rays.len() as f64)
        .sqrt();

    // Bootstrap: resample the stains used, with replacement.
    let mut rng = Rng(p.seed.max(1).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut xs = vec![];
    let mut failed = 0;
    let mut sample = Vec::with_capacity(rays.len());
    for _ in 0..p.bootstrap {
        sample.clear();
        sample.extend((0..rays.len()).map(|_| rays[rng.below(rays.len())]));
        match nearest(&sample) {
            Some(b) => xs.push(b),
            None => failed += 1,
        }
    }
    let mut cov = [[0.0; 3]; 3];
    let m = xs.len().max(2) as f64 - 1.0;
    let mean = xs
        .iter()
        .fold([0.0; 3], |a, b| add(a, *b))
        .map(|v| v / xs.len().max(1) as f64);
    for b in &xs {
        let d = sub(*b, mean);
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j] / m;
            }
        }
    }
    let (vals, vecs) = eigen_sym(cov);
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| vals[b].total_cmp(&vals[a]));
    let q = radius2_95(rays.len());
    let ellipsoid = Ellipsoid {
        semi_axes: order.map(|k| (q * vals[k].max(0.0)).sqrt()),
        axes: order.map(|k| vecs[k]),
    };
    let sigma = [0, 1, 2].map(|k| cov[k][k].max(0.0).sqrt());
    let height = Measured {
        value: x[2] - p.floor_z,
        sigma: sigma[2],
    };
    let summary = format!(
        "Origin at ({:.3}, {:.3}, {:.3}) m, {:.2} m above the floor (95 % ellipsoid {:.2} × {:.2} × {:.2} m), from {} of {} stains",
        x[0],
        x[1],
        x[2],
        height.value,
        2.0 * ellipsoid.semi_axes[0],
        2.0 * ellipsoid.semi_axes[1],
        2.0 * ellipsoid.semi_axes[2],
        rays.len(),
        stains.len()
    );
    Ok(Run {
        method: METHOD.into(),
        inputs,
        parameters,
        origin: Origin {
            point: x,
            height,
            sigma,
            ellipsoid,
            rms_residual,
            chi2,
            dof: 2 * rays.len() - 3,
            stains_used: rays.len(),
            bootstrap: xs.len(),
            bootstrap_failed: failed,
            conditioning,
        },
        stains,
        summary,
        assumptions: ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(v: f64, s: f64) -> Measured {
        Measured { value: v, sigma: s }
    }

    /// A stain on a surface (point, normal) hit by a straight path from `origin`.
    fn stain_from(origin: P3, centre: P3, normal: P3, label: &str) -> StainInput {
        let v = unit(sub(centre, origin));
        let sin_a = -dot(v, normal);
        let travel = unit(add(v, scale(normal, sin_a)));
        let width = 0.004;
        StainInput {
            label: label.into(),
            surface: "s".into(),
            centre,
            normal,
            width: m(width, 0.0001),
            length: m(width / sin_a, 0.0001),
            travel,
            travel_sigma_deg: 1.0,
            ..Default::default()
        }
    }

    fn wall_stains(origin: P3) -> Vec<StainInput> {
        // A wall at x = 0 (normal +x), stains above the origin's height.
        let mut v = vec![];
        for (k, (y, z)) in [
            (0.5, 1.4),
            (1.0, 1.6),
            (1.5, 1.3),
            (2.0, 1.9),
            (2.6, 1.5),
            (1.2, 2.2),
        ]
        .iter()
        .enumerate()
        {
            v.push(stain_from(
                origin,
                [0.0, *y, *z],
                [1.0, 0.0, 0.0],
                &format!("w{k}"),
            ));
        }
        v
    }

    #[test]
    fn exact_stains_give_the_exact_origin() {
        let o = [1.4, 1.5, 1.1];
        let r = run(wall_stains(o), Parameters::default()).unwrap();
        for (p, q) in r.origin.point.iter().zip(o) {
            assert!((p - q).abs() < 1e-9, "{:?}", r.origin.point);
        }
        assert!(r
            .stains
            .iter()
            .all(|s| s.used && s.upward && s.residual < 1e-9 && !s.behind));
        assert!((r.origin.height.value - 1.1).abs() < 1e-9);
        // No scatter: the bootstrap has nothing to spread.
        assert!(r.origin.ellipsoid.semi_axes[0] < 1e-9);
    }

    #[test]
    fn downward_stains_are_left_out_unless_included_with_a_reason() {
        let o = [1.4, 1.5, 1.1];
        let mut s = wall_stains(o);
        s.push(stain_from(o, [0.0, 1.0, 0.6], [1.0, 0.0, 0.0], "low"));
        s.push(stain_from(o, [0.8, 0.7, 0.0], [0.0, 0.0, 1.0], "floor"));
        s[0].excluded = Some("overlaps another stain".into());
        let r = run(s.clone(), Parameters::default()).unwrap();
        let used: Vec<bool> = r.stains.iter().map(|s| s.used).collect();
        assert_eq!(used, [false, true, true, true, true, true, false, false]);
        assert!(r.stains[6].not_used.as_deref() == Some("moving downward at impact"));
        assert!(r.stains[0].not_used.as_ref().unwrap().contains("overlaps"));
        let r = run(
            s,
            Parameters {
                include_not_upward: Some("straight-flight test".into()),
                ..Parameters::default()
            },
        )
        .unwrap();
        assert_eq!(r.origin.stains_used, 7);
        assert!(r.stains[7].residual < 1e-9);
    }

    #[test]
    fn directionality_follows_the_conventions() {
        let p = Parameters::default();
        let at = |travel: P3, normal: P3| {
            stain(
                &StainInput {
                    label: "t".into(),
                    normal,
                    width: m(0.004, 0.0),
                    length: m(0.008, 0.0),
                    travel,
                    ..Default::default()
                },
                &p,
            )
            .unwrap()
        };
        // Wall x = 0, facing it from +x: right is +y.
        let up = at([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        assert!(up.directionality.value.abs() < 1e-9 && up.upward);
        assert!((up.impact.value - 30.0).abs() < 1e-9);
        assert!((at([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]).directionality.value - 90.0).abs() < 1e-9);
        let down = at([0.0, 0.0, -1.0], [1.0, 0.0, 0.0]);
        assert!((down.directionality.value - 180.0).abs() < 1e-9 && !down.upward);
        // Floor: clockwise from +y.
        assert!((at([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]).directionality.value - 90.0).abs() < 1e-9);
        // The ray leaves the surface, against the tail.
        let r = up.ray;
        assert!(r[0] > 0.0 && r[2] < 0.0);
    }

    #[test]
    fn too_few_or_parallel_paths_are_refused() {
        let o = [1.4, 1.5, 1.1];
        let s = wall_stains(o);
        assert!(run(s[..3].to_vec(), Parameters::default()).is_err());
        let mut par = s.clone();
        for x in par.iter_mut() {
            x.travel = [0.0, 0.0, 1.0];
            x.length = m(0.008, 0.0001);
        }
        let e = run(par, Parameters::default()).unwrap_err();
        assert!(e.to_string().contains("parallel"), "{e}");
    }

    #[test]
    fn the_bootstrap_ellipsoid_covers_the_truth_about_95_percent_of_the_time() {
        // Stains on two walls and noisy measurements; how often is the truth inside?
        let o = [1.6, 1.4, 1.0];
        let mut rng = 7u64;
        let mut normal = || {
            let mut u = || {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                (rng >> 11) as f64 / (1u64 << 53) as f64
            };
            let (a, b) = (u().max(1e-300), u());
            (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
        };
        let mut inside = 0;
        let runs = 200;
        for k in 0..runs {
            let mut s = vec![];
            for i in 0..30 {
                let f = i as f64 / 30.0;
                let (c, n) = if i % 2 == 0 {
                    (
                        [0.0, 0.3 + 2.5 * f, 1.2 + 0.9 * ((i * 7) % 5) as f64 / 5.0],
                        [1.0, 0.0, 0.0],
                    )
                } else {
                    (
                        [0.3 + 2.8 * f, 0.0, 1.2 + 0.9 * ((i * 3) % 5) as f64 / 5.0],
                        [0.0, 1.0, 0.0],
                    )
                };
                let mut st = stain_from(o, c, n, "s");
                // 3 % noise on each axis, 3° on the direction.
                st.width.value *= 1.0 + 0.03 * normal();
                st.length.value = (st.length.value * (1.0 + 0.03 * normal())).max(st.width.value);
                let th = 3f64.to_radians() * normal();
                let across = cross(n, st.travel);
                st.travel = unit(add(scale(st.travel, th.cos()), scale(across, th.sin())));
                s.push(st);
            }
            let r = run(
                s,
                Parameters {
                    bootstrap: 400,
                    seed: k + 1,
                    ..Parameters::default()
                },
            )
            .unwrap();
            let e = &r.origin.ellipsoid;
            let d = sub(o, r.origin.point);
            let q: f64 = (0..3)
                .map(|j| (dot(d, e.axes[j]) / e.semi_axes[j]).powi(2))
                .sum();
            if q <= 1.0 {
                inside += 1;
            }
        }
        let cover = inside as f64 / runs as f64;
        eprintln!("bootstrap 95 % ellipsoid coverage: {cover}");
        assert!((0.91..=0.99).contains(&cover), "coverage {cover}");
    }

    #[test]
    fn f_quantiles_match_the_tables() {
        // NIST/SEMATECH e-Handbook, 1.3.6.7.3: upper 5 % points of F.
        for (d2, f) in [(7.0, 4.347), (10.0, 3.708), (27.0, 2.960), (100.0, 2.696)] {
            assert!((f_quantile(3.0, d2, 0.95) - f).abs() < 1e-3, "{d2}");
        }
        // Many stains: the χ²₃ radius, 7.815.
        assert!((radius2_95(100_000) - 7.8147).abs() < 1e-3);
    }

    #[test]
    fn a_photo_aligns_from_point_pairs() {
        // Wall x = 0: pixel (x, y) → world (0, 1.0 + x / 5000, 1.5 − y / 5000), rotated 10°.
        let (c, s) = (10f64.to_radians().cos(), 10f64.to_radians().sin());
        let world = |px: P2| {
            let (u, v) = (px[0] / 5000.0, -px[1] / 5000.0);
            [0.0, 1.0 + c * u - s * v, 1.5 + s * u + c * v]
        };
        let pairs: Vec<AlignPair> = [[10.0, 20.0], [900.0, 40.0], [300.0, 700.0]]
            .iter()
            .map(|p| AlignPair {
                px: *p,
                world: world(*p),
            })
            .collect();
        let a = align_photo(&pairs, [0.0, 1.0, 1.5], [1.0, 0.0, 0.0]).unwrap();
        assert!((a.pixels_per_metre - 5000.0).abs() < 1e-6);
        assert!(a.rms.unwrap() < 1e-12);
        let w = a.to_world([512.0, 384.0]);
        assert!(norm(sub(w, world([512.0, 384.0]))) < 1e-12);
        // Two pairs fix it exactly: no residual to report.
        assert!(align_photo(&pairs[..2], [0.0, 1.0, 1.5], [1.0, 0.0, 0.0])
            .unwrap()
            .rms
            .is_none());
        // A mirrored photo can't be aligned without error.
        let mirrored: Vec<AlignPair> = pairs
            .iter()
            .map(|p| AlignPair {
                px: [1000.0 - p.px[0], p.px[1]],
                world: p.world,
            })
            .collect();
        let a = align_photo(&mirrored, [0.0, 1.0, 1.5], [1.0, 0.0, 0.0]).unwrap();
        assert!(a.rms.unwrap() > 0.01);
    }

    /// A stain drawn like the generator's: an ellipse with a tail, antialiased.
    fn draw(w: usize, a: f64, b: f64, theta: f64) -> Vec<u8> {
        let (c, s) = (theta.cos(), theta.sin());
        let half = w as f64 / 2.0;
        let mut img = vec![0u8; w * w];
        for y in 0..w {
            for x in 0..w {
                let mut ink = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f64 + (sx as f64 + 0.5) / 4.0 - half;
                        let py = y as f64 + (sy as f64 + 0.5) / 4.0 - half;
                        let (l, q) = (px * c + py * s, -px * s + py * c);
                        let tail =
                            l > a && l < a * 1.35 && q.abs() < b * 0.25 * (1.35 - l / a) / 0.35;
                        if (l / a).powi(2) + (q / b).powi(2) <= 1.0 || tail {
                            ink += 1;
                        }
                    }
                }
                let f = ink as f64 / 16.0;
                img[y * w + x] = (210.0 * (1.0 - f) + 30.0 * f).round() as u8;
            }
        }
        img
    }

    #[test]
    fn automatic_edges_fit_the_ellipse_and_leave_out_the_tail() {
        let (w, a, b, theta) = (240, 80.0, 40.0, 0.4f64);
        let img = draw(w, a, b, theta);
        let edges = stain_edges(&img, w, w, [120.0, 120.0], 120).unwrap();
        // 1 px = 0.1 mm, on a floor.
        let pairs = [[0.0, 0.0], [240.0, 0.0], [0.0, 240.0]].map(|p: P2| AlignPair {
            px: p,
            world: [p[0] * 1e-4, -p[1] * 1e-4, 0.0],
        });
        let al = align_photo(&pairs, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        let f = fit_stain(&edges, &al).unwrap();
        assert!(f.trimmed > 0, "the tail should be left out");
        assert!(
            (f.length.value - 2.0 * a * 1e-4).abs() < 0.3e-4,
            "{:?}",
            f.length
        );
        assert!(
            (f.width.value - 2.0 * b * 1e-4).abs() < 0.3e-4,
            "{:?}",
            f.width
        );
        // The long axis, in world terms: image y is −world y.
        let ax = f.long_axis;
        let ang = (-ax[1]).atan2(ax[0]);
        let d = (ang - theta).rem_euclid(std::f64::consts::PI);
        assert!(
            d.min(std::f64::consts::PI - d) < 0.5f64.to_radians(),
            "{ang}"
        );
        assert!(f.width.sigma > 0.0 && f.width.sigma < 0.6e-4);
        // A seed outside the stain, or a stain off the edge, is refused.
        assert!(stain_edges(&img, w, w, [5.0, 5.0], 120).is_err());
        assert!(stain_edges(&img, w, w, [120.0, 120.0], 250).is_err());
    }
}
