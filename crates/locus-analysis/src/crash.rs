//! Crash reconstruction: speed from skid marks, critical speed from yaw marks, two-vehicle
//! linear momentum, and crush energy (Campbell / CRASH3). Method, assumptions and
//! limitations: docs/methods/crash.md.
//!
//! Every input is a value with a range. Each result is given three ways: the value from the
//! inputs' values; the range method's extremes (every corner of the inputs' ranges); and a
//! Monte Carlo 95 % interval (uniform over each range, or normal with the range as ±2σ where
//! the input says so), seeded so a run repeats exactly. SI units throughout: m, s, kg, N, J.

use crate::measure::{fit_plane, P3};
use crate::trajectory::PointSource;
use serde::{Deserialize, Serialize};

/// Standard gravity (m/s²), CGPM 1901.
pub const G: f64 = 9.806_65;

pub const SKID_METHOD: &str = "skid/1";
pub const YAW_METHOD: &str = "yaw/1";
pub const MOMENTUM_METHOD: &str = "momentum/1";
pub const CRUSH_METHOD: &str = "crush/1";

#[derive(Debug, Clone, PartialEq)]
pub struct CrashError(pub String);

impl std::fmt::Display for CrashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn err<T>(m: impl Into<String>) -> Result<T, CrashError> {
    Err(CrashError(m.into()))
}

/// An input: its value and the range it could be in. Uniform over the range, or, with
/// `normal`, normal with the range as ±2σ (a measured value with its 95 % bounds).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub value: f64,
    pub low: f64,
    pub high: f64,
    #[serde(default)]
    pub normal: bool,
}

impl Input {
    /// A value with no range.
    pub fn exact(v: f64) -> Input {
        Input {
            value: v,
            low: v,
            high: v,
            normal: false,
        }
    }
    /// A value with a uniform range.
    pub fn range(value: f64, low: f64, high: f64) -> Input {
        Input {
            value,
            low,
            high,
            normal: false,
        }
    }
    pub(crate) fn check(&self, name: &str) -> Result<(), CrashError> {
        if !(self.value.is_finite() && self.low.is_finite() && self.high.is_finite()) {
            return err(format!("{name}: give a value"));
        }
        if !(self.low <= self.value && self.value <= self.high) {
            return err(format!(
                "{name}: the value must lie within its range ({} to {})",
                self.low, self.high
            ));
        }
        Ok(())
    }
    fn draw(&self, rng: &mut Rng) -> f64 {
        if self.low == self.high {
            self.value
        } else if self.normal {
            self.value + (self.high - self.low) / 4.0 * rng.gauss()
        } else {
            self.low + (self.high - self.low) * rng.uniform()
        }
    }
}

/// A result: from the inputs' values, the range method's extremes, and the Monte Carlo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spread {
    pub value: f64,
    /// Smallest and largest over every corner of the inputs' ranges.
    pub low: f64,
    pub high: f64,
    /// Monte Carlo mean, standard deviation and 95 % interval.
    pub mean: f64,
    pub sd: f64,
    pub interval95: [f64; 2],
    pub draws: usize,
    /// Draws with no answer (the inputs gave no real solution), left out.
    pub failed: usize,
}

/// Deterministic uniform and normal draws (xorshift64*, Box–Muller).
pub(crate) struct Rng(pub(crate) u64);
impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11
    }
    pub(crate) fn uniform(&mut self) -> f64 {
        (self.next() as f64 + 0.5) / (1u64 << 53) as f64
    }
    pub(crate) fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// Monte Carlo draws, unless the caller says otherwise.
pub const DRAWS: usize = 20_000;

/// Evaluate `f` over the inputs: at their values, at every corner of their ranges (up to 16
/// ranged inputs; beyond that the Monte Carlo's extremes stand in), and by Monte Carlo.
pub(crate) fn spread(
    inputs: &[Input],
    f: &dyn Fn(&[f64]) -> Option<f64>,
    draws: usize,
    seed: u64,
) -> Result<Spread, CrashError> {
    let values: Vec<f64> = inputs.iter().map(|i| i.value).collect();
    let Some(value) = f(&values) else {
        return err("the inputs' values give no real answer");
    };
    let ranged: Vec<usize> = (0..inputs.len())
        .filter(|&k| inputs[k].low < inputs[k].high)
        .collect();
    let (mut low, mut high) = (value, value);
    if ranged.len() <= 16 {
        for mask in 0..(1u32 << ranged.len()) {
            let mut x = values.clone();
            for (b, &k) in ranged.iter().enumerate() {
                x[k] = if mask >> b & 1 == 1 {
                    inputs[k].high
                } else {
                    inputs[k].low
                };
            }
            if let Some(v) = f(&x) {
                low = low.min(v);
                high = high.max(v);
            }
        }
    }
    let mut rng = Rng(seed.max(1).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut out = Vec::with_capacity(draws);
    let mut failed = 0;
    let mut x = values.clone();
    for _ in 0..draws {
        for (k, i) in inputs.iter().enumerate() {
            x[k] = i.draw(&mut rng);
        }
        match f(&x) {
            Some(v) if v.is_finite() => out.push(v),
            _ => failed += 1,
        }
    }
    if ranged.len() > 16 {
        low = out.iter().cloned().fold(value, f64::min);
        high = out.iter().cloned().fold(value, f64::max);
    }
    let n = out.len().max(1) as f64;
    let mean = out.iter().sum::<f64>() / n;
    let sd = (out.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt();
    out.sort_by(f64::total_cmp);
    let q = |p: f64| {
        if out.is_empty() {
            value
        } else {
            out[((p * (out.len() as f64 - 1.0)).round() as usize).min(out.len() - 1)]
        }
    };
    Ok(Spread {
        value,
        low,
        high,
        mean,
        sd,
        interval95: [q(0.025), q(0.975)],
        draws: out.len(),
        failed,
    })
}

// ---------------------------------------------------------------------------------------
// Speed from skid marks
// ---------------------------------------------------------------------------------------

/// One stretch of skid on one surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkidSegment {
    pub label: String,
    /// Length of the mark (m).
    pub distance: Input,
    /// Drag factor (friction coefficient) of the surface.
    pub drag: Input,
    /// Braking efficiency: the share of the full drag factor the braked wheels give (1 for
    /// all four locked; less for a braking fault or a motorcycle's one wheel).
    pub braking: Input,
    /// Grade: rise over run, positive uphill in the direction of travel.
    pub grade: Input,
    /// The mark as picked on the cloud (a polyline), when measured there.
    #[serde(default)]
    pub path: Vec<P3>,
    #[serde(default)]
    pub sources: Vec<PointSource>,
}

/// The effective drag factor on a grade: μ n cos θ + sin θ, θ = atan(grade).
pub fn effective_drag(drag: f64, braking: f64, grade: f64) -> f64 {
    let t = grade.atan();
    drag * braking * t.cos() + t.sin()
}

/// The speed at the start of the skid: √(v_end² + Σ 2 g f_i d_i).
pub fn skid_speed(end_speed: f64, segments: &[(f64, f64)]) -> Option<f64> {
    let v2 = end_speed * end_speed + segments.iter().map(|(f, d)| 2.0 * G * f * d).sum::<f64>();
    (v2 >= 0.0).then(|| v2.sqrt())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkidSegmentResult {
    pub label: String,
    pub effective_drag: f64,
    /// The speed this stretch alone takes off from a stop, √(2 g f d) (m/s).
    pub speed_alone: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkidRun {
    pub method: String,
    pub segments: Vec<SkidSegment>,
    /// Speed at the end of the marks (0 when the vehicle stopped there), m/s.
    pub end_speed: Input,
    pub results: Vec<SkidSegmentResult>,
    /// Speed at the start of the marks (m/s).
    pub speed: Spread,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub const SKID_ASSUMPTIONS: &[&str] = &[
    "The vehicle decelerated over the whole length of the marks at the stated drag factor, adjusted for grade and braking efficiency.",
    "Each drag factor applies to its surface along its stretch of the marks; the stretches add as √(v_end² + Σ 2 g f d).",
    "The marks' length is the distance the vehicle skidded (from where the tyres began to lock to where they stopped marking, less any wheelbase adjustment the examiner has applied).",
];

pub const SKID_LIMITATIONS: &[&str] = &[
    "Braking before the marks began (the time to lock the wheels) is not counted, so the result is a lower bound on the speed when braking began.",
    "A vehicle with anti-lock brakes may leave faint or no marks; their length then understates the braking distance.",
    "The drag factor is the most uncertain input; test skids with a similar vehicle on the same surface are the best source.",
];

pub fn skid(
    segments: Vec<SkidSegment>,
    end_speed: Input,
    draws: usize,
    seed: u64,
) -> Result<SkidRun, CrashError> {
    if segments.is_empty() {
        return err("add at least one stretch of skid");
    }
    end_speed.check("Speed at the end")?;
    if end_speed.low < 0.0 {
        return err("the speed at the end can't be negative");
    }
    let mut inputs = vec![end_speed];
    let mut results = vec![];
    for s in &segments {
        for (i, n) in [
            (&s.distance, "distance"),
            (&s.drag, "drag factor"),
            (&s.braking, "braking efficiency"),
            (&s.grade, "grade"),
        ] {
            i.check(&format!("{}: {n}", s.label))?;
        }
        if s.distance.low <= 0.0 {
            return err(format!("{}: the distance must be positive", s.label));
        }
        if s.drag.low <= 0.0 || s.drag.high > 1.5 {
            return err(format!(
                "{}: the drag factor must be between 0 and 1.5",
                s.label
            ));
        }
        if s.braking.low <= 0.0 || s.braking.high > 1.0 {
            return err(format!(
                "{}: the braking efficiency must be between 0 and 1",
                s.label
            ));
        }
        let f = effective_drag(s.drag.value, s.braking.value, s.grade.value);
        if f <= 0.0 {
            return err(format!(
                "{}: downhill steeper than the braking could hold; the effective drag factor is not positive",
                s.label
            ));
        }
        results.push(SkidSegmentResult {
            label: s.label.clone(),
            effective_drag: f,
            speed_alone: (2.0 * G * f * s.distance.value).sqrt(),
        });
        inputs.extend([s.distance, s.drag, s.braking, s.grade]);
    }
    let n = segments.len();
    let f = |x: &[f64]| {
        let segs: Vec<(f64, f64)> = (0..n)
            .map(|k| {
                let b = 1 + 4 * k;
                (effective_drag(x[b + 1], x[b + 2], x[b + 3]), x[b])
            })
            .collect();
        if segs.iter().any(|(f, _)| *f <= 0.0) {
            return None;
        }
        skid_speed(x[0], &segs)
    };
    let speed = spread(&inputs, &f, draws, seed)?;
    let summary = format!(
        "Speed at the start of the skid {:.1} m/s ({:.0} km/h); range {:.1}–{:.1} m/s; 95 % {:.1}–{:.1} m/s",
        speed.value,
        speed.value * 3.6,
        speed.low,
        speed.high,
        speed.interval95[0],
        speed.interval95[1]
    );
    Ok(SkidRun {
        method: SKID_METHOD.into(),
        segments,
        end_speed,
        results,
        speed,
        summary,
        assumptions: SKID_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: SKID_LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    })
}

// ---------------------------------------------------------------------------------------
// Critical speed from yaw marks
// ---------------------------------------------------------------------------------------

/// Radius from a chord and its middle ordinate: R = C²/(8M) + M/2.
pub fn radius_from_chord(chord: f64, ordinate: f64) -> Option<f64> {
    (chord > 0.0 && ordinate > 0.0).then(|| chord * chord / (8.0 * ordinate) + ordinate / 2.0)
}

/// Critical speed on radius R: √(g R (μ + e) / (1 − μ e)) for superelevation e (rise over
/// run, positive banked toward the centre); √(μ g R) on the level.
pub fn critical_speed(radius: f64, drag: f64, superelevation: f64) -> Option<f64> {
    let den = 1.0 - drag * superelevation;
    let v2 = G * radius * (drag + superelevation) / den;
    (radius > 0.0 && den > 0.0 && v2 >= 0.0).then(|| v2.sqrt())
}

/// A circle fitted to points along a yaw mark: in the plane fitted to them, by least
/// squares on the points' distances from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CircleFit {
    pub centre: P3,
    pub normal: P3,
    pub radius: f64,
    /// 1σ of the radius (m), from the fit's covariance and residuals.
    pub radius_sigma: f64,
    /// RMS distance of the points from the circle (m).
    pub rms: f64,
    pub points: usize,
    /// The arc the points span (degrees).
    pub arc_deg: f64,
}

/// Fit a circle to points (the algebraic fit started, then Gauss–Newton on the geometric
/// distances in the points' plane).
pub fn fit_circle(points: &[P3], point_sigma: f64) -> Result<CircleFit, CrashError> {
    if points.len() < 4 {
        return err("pick at least 4 points along the mark");
    }
    let plane = fit_plane(points).map_err(|e| CrashError(e.to_string()))?;
    let n = plane.normal;
    let t = if n[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let cross = |a: P3, b: P3| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let unit = |a: P3| {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        a.map(|v| v / l)
    };
    let e1 = unit(cross(n, t));
    let e2 = cross(n, e1);
    let uv: Vec<[f64; 2]> = points
        .iter()
        .map(|p| {
            let d = [0, 1, 2].map(|k| p[k] - plane.point[k]);
            [
                d[0] * e1[0] + d[1] * e1[1] + d[2] * e1[2],
                d[0] * e2[0] + d[1] * e2[1] + d[2] * e2[2],
            ]
        })
        .collect();
    // Algebraic (Kåsa) start: x² + y² + D x + E y + F = 0.
    let mut a = nalgebra::DMatrix::<f64>::zeros(uv.len(), 3);
    let mut b = nalgebra::DVector::<f64>::zeros(uv.len());
    for (i, p) in uv.iter().enumerate() {
        a[(i, 0)] = p[0];
        a[(i, 1)] = p[1];
        a[(i, 2)] = 1.0;
        b[i] = -(p[0] * p[0] + p[1] * p[1]);
    }
    let Some(sol) = (a.transpose() * &a)
        .try_inverse()
        .map(|m| m * a.transpose() * b)
    else {
        return err("the points lie on a straight line: no radius");
    };
    let mut c = [-sol[0] / 2.0, -sol[1] / 2.0];
    let mut r = (c[0] * c[0] + c[1] * c[1] - sol[2]).max(0.0).sqrt();
    // Gauss–Newton on d_i = |p_i − c| − r.
    let mut inv = nalgebra::Matrix3::<f64>::zeros();
    for _ in 0..50 {
        let mut jtj = nalgebra::Matrix3::<f64>::zeros();
        let mut g = nalgebra::Vector3::<f64>::zeros();
        for p in &uv {
            let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
            let dist = dx.hypot(dy).max(1e-12);
            let res = dist - r;
            let j = nalgebra::Vector3::new(-dx / dist, -dy / dist, -1.0);
            jtj += j * j.transpose();
            g -= j * res;
        }
        let Some(i) = jtj.try_inverse() else {
            return err("the points don't determine a circle");
        };
        inv = i;
        let step = i * g;
        c[0] += step[0];
        c[1] += step[1];
        r += step[2];
        if step.norm() < 1e-12 {
            break;
        }
    }
    let res: Vec<f64> = uv
        .iter()
        .map(|p| (p[0] - c[0]).hypot(p[1] - c[1]) - r)
        .collect();
    let dof = (uv.len() as f64 - 3.0).max(1.0);
    let s2 = (res.iter().map(|v| v * v).sum::<f64>() / dof).max(point_sigma * point_sigma);
    let ang: Vec<f64> = uv
        .iter()
        .map(|p| (p[1] - c[1]).atan2(p[0] - c[0]))
        .collect();
    // The arc spanned: the angles' range after unwrapping around the first.
    let rel: Vec<f64> = ang
        .iter()
        .map(|a| (a - ang[0] + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU))
        .collect();
    let arc = rel.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - rel.iter().cloned().fold(f64::INFINITY, f64::min);
    Ok(CircleFit {
        centre: [0, 1, 2].map(|k| plane.point[k] + c[0] * e1[k] + c[1] * e2[k]),
        normal: n,
        radius: r,
        radius_sigma: (s2 * inv[(2, 2)]).max(0.0).sqrt(),
        rms: (res.iter().map(|v| v * v).sum::<f64>() / uv.len() as f64).sqrt(),
        points: uv.len(),
        arc_deg: arc.to_degrees(),
    })
}

/// Points picked over less arc than this (degrees) leave the radius poorly determined; the
/// report warns.
pub const MIN_ARC_DEG: f64 = 20.0;

/// How the yaw mark's radius was found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum YawRadius {
    /// A chord across the mark and the middle ordinate from its midpoint to the mark (m).
    Chord { chord: Input, ordinate: Input },
    /// Points picked along the mark on the cloud.
    Points {
        points: Vec<P3>,
        #[serde(default)]
        sources: Vec<PointSource>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YawRun {
    pub method: String,
    pub radius_from: YawRadius,
    pub drag: Input,
    pub superelevation: Input,
    /// Subtracted from the mark's radius to reach the centre of mass's path (m): half the
    /// track, for the outside front tyre's mark.
    pub cg_offset: f64,
    #[serde(default)]
    pub circle: Option<CircleFit>,
    /// The radius used (of the centre of mass's path, m) and the critical speed (m/s).
    pub radius: Spread,
    pub speed: Spread,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub const YAW_ASSUMPTIONS: &[&str] = &[
    "The mark was made by a tyre of a vehicle turning at the limit of adhesion (sliding sideways), not braking hard or accelerating.",
    "The radius is measured early in the mark, where it is still the path of a tyre on the limit; the centre of mass's path is inside the outside front tyre's mark by half the track.",
    "The drag factor is the lateral friction available; superelevation is the road's cross slope toward the centre of the curve.",
];

pub const YAW_LIMITATIONS: &[&str] = &[
    "A chord and middle ordinate taken over a short chord make the radius very sensitive to the ordinate; the stated range shows it. A longer chord, or a circle fitted to many points along the mark, is better.",
    "Braking or acceleration during the yaw, a tyre that is not the outside front, or a radius measured late in the mark (where the vehicle has slowed and rotated) all make the result inaccurate.",
    "Critical speed is the speed at which the vehicle could just follow that radius; the vehicle's actual speed was at least that.",
];

pub fn yaw(
    radius_from: YawRadius,
    drag: Input,
    superelevation: Input,
    cg_offset: f64,
    point_sigma: f64,
    draws: usize,
    seed: u64,
) -> Result<YawRun, CrashError> {
    drag.check("Drag factor")?;
    superelevation.check("Superelevation")?;
    if drag.low <= 0.0 || drag.high > 1.5 {
        return err("the drag factor must be between 0 and 1.5");
    }
    if cg_offset.is_nan() || cg_offset < 0.0 {
        return err("the offset to the centre of mass's path can't be negative");
    }
    let (circle, radius_input, chord): (Option<CircleFit>, Input, Option<(Input, Input)>) =
        match &radius_from {
            YawRadius::Chord { chord, ordinate } => {
                chord.check("Chord")?;
                ordinate.check("Middle ordinate")?;
                if chord.low <= 0.0 || ordinate.low <= 0.0 {
                    return err("the chord and middle ordinate must be positive");
                }
                (None, Input::exact(0.0), Some((*chord, *ordinate)))
            }
            YawRadius::Points { points, .. } => {
                let c = fit_circle(points, point_sigma)?;
                let r = Input {
                    value: c.radius,
                    low: c.radius - 2.0 * c.radius_sigma,
                    high: c.radius + 2.0 * c.radius_sigma,
                    normal: true,
                };
                (Some(c), r, None)
            }
        };
    // Inputs: [radius or chord, ordinate, drag, superelevation].
    let inputs = match chord {
        Some((c, m)) => vec![c, m, drag, superelevation],
        None => vec![radius_input, Input::exact(0.0), drag, superelevation],
    };
    let is_chord = chord.is_some();
    let r_of = move |x: &[f64]| -> Option<f64> {
        let r = if is_chord {
            radius_from_chord(x[0], x[1])?
        } else {
            x[0]
        };
        let r = r - cg_offset;
        (r > 0.0).then_some(r)
    };
    let radius = spread(&inputs, &r_of, draws, seed)?;
    let v = |x: &[f64]| critical_speed(r_of(x)?, x[2], x[3]);
    let speed = spread(&inputs, &v, draws, seed + 1)?;
    let summary = format!(
        "Critical speed {:.1} m/s ({:.0} km/h) on a radius of {:.1} m; range {:.1}–{:.1} m/s; 95 % {:.1}–{:.1} m/s",
        speed.value,
        speed.value * 3.6,
        radius.value,
        speed.low,
        speed.high,
        speed.interval95[0],
        speed.interval95[1]
    );
    Ok(YawRun {
        method: YAW_METHOD.into(),
        radius_from,
        drag,
        superelevation,
        cg_offset,
        circle,
        radius,
        speed,
        summary,
        assumptions: YAW_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: YAW_LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    })
}

// ---------------------------------------------------------------------------------------
// Linear momentum, two vehicles
// ---------------------------------------------------------------------------------------

/// One vehicle in a two-vehicle collision. Headings are directions of travel, clockwise from
/// project north (+y), in degrees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MomentumVehicle {
    pub label: String,
    /// Mass with occupants and load (kg).
    pub mass: Input,
    pub approach_deg: Input,
    pub departure_deg: Input,
    /// Speed just after the impact (m/s), from the post-impact travel.
    pub departure_speed: Input,
}

fn dir(deg: f64) -> [f64; 2] {
    let t = deg.to_radians();
    [t.sin(), t.cos()]
}

/// Impact speeds of both vehicles from conservation of linear momentum in the plane:
/// m1 v1 d(θ1) + m2 v2 d(θ2) = m1 u1 d(φ1) + m2 u2 d(φ2), solved for v1 and v2.
/// Inputs: [m1, θ1, φ1, u1, m2, θ2, φ2, u2]. None when the approaches are parallel.
pub fn momentum_speeds(x: &[f64]) -> Option<[f64; 2]> {
    let (m1, t1, p1, u1, m2, t2, p2, u2) = (x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);
    let (a1, a2, d1, d2) = (dir(t1), dir(t2), dir(p1), dir(p2));
    let px = m1 * u1 * d1[0] + m2 * u2 * d2[0];
    let py = m1 * u1 * d1[1] + m2 * u2 * d2[1];
    // Solve [m1 a1 | m2 a2] [v1 v2]ᵀ = [px py]ᵀ.
    let det = m1 * m2 * (a1[0] * a2[1] - a1[1] * a2[0]);
    if det.abs() < 1e-9 * m1 * m2 {
        return None;
    }
    let v1 = m2 * (px * a2[1] - py * a2[0]) / det;
    let v2 = m1 * (a1[0] * py - a1[1] * px) / det;
    Some([v1, v2])
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensitivityRow {
    /// Which input was moved to its low and high ends (the others at their values).
    pub input: String,
    pub low_input: f64,
    pub high_input: f64,
    /// Both vehicles' impact speeds with it at its low end and at its high end (m/s).
    pub at_low: [f64; 2],
    pub at_high: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MomentumRun {
    pub method: String,
    pub vehicles: [MomentumVehicle; 2],
    /// Impact speeds (m/s) and each vehicle's speed change (delta-V, m/s).
    pub speeds: [Spread; 2],
    pub delta_v: [Spread; 2],
    /// One-at-a-time sensitivity: each input at its low and high ends.
    pub sensitivity: Vec<SensitivityRow>,
    /// The angle between the two approach directions (degrees): momentum can't separate the
    /// speeds when it is small.
    pub approach_angle_deg: f64,
    pub warnings: Vec<String>,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub const MOMENTUM_ASSUMPTIONS: &[&str] = &[
    "Linear momentum is conserved over the impact: external forces (tyre friction) are small compared with the collision forces during the short contact.",
    "The masses include occupants and load; the departure speeds and directions are those just after separation, from the post-impact travel to rest.",
    "The motion is in the horizontal plane.",
];

pub const MOMENTUM_LIMITATIONS: &[&str] = &[
    "When the approach directions are nearly parallel (or opposite), momentum can't separate the two speeds, and small input errors give large speed errors; the approach angle and the sensitivity table show it.",
    "Departure speeds are usually the least certain inputs; their ranges dominate the result.",
    "Angular momentum and energy are not used here: they are separate checks.",
];

pub fn momentum(
    vehicles: [MomentumVehicle; 2],
    draws: usize,
    seed: u64,
) -> Result<MomentumRun, CrashError> {
    let mut inputs = vec![];
    for v in &vehicles {
        for (i, n) in [
            (&v.mass, "mass"),
            (&v.approach_deg, "approach direction"),
            (&v.departure_deg, "departure direction"),
            (&v.departure_speed, "departure speed"),
        ] {
            i.check(&format!("{}: {n}", v.label))?;
        }
        if v.mass.low <= 0.0 {
            return err(format!("{}: the mass must be positive", v.label));
        }
        if v.departure_speed.low < 0.0 {
            return err(format!(
                "{}: the departure speed can't be negative",
                v.label
            ));
        }
        inputs.extend([v.mass, v.approach_deg, v.departure_deg, v.departure_speed]);
    }
    let angle = {
        let d = (vehicles[1].approach_deg.value - vehicles[0].approach_deg.value).rem_euclid(360.0);
        d.min(360.0 - d)
    };
    let mut warnings = vec![];
    if !(20.0..=160.0).contains(&angle) {
        warnings.push(format!(
            "The approach directions are {angle:.0}° apart: momentum barely separates the two speeds (best near 90°), and small errors in the inputs make large errors in them. Read the sensitivity table."
        ));
    }
    let speed = |k: usize| move |x: &[f64]| momentum_speeds(x).map(|v| v[k]);
    let dv = |k: usize| {
        move |x: &[f64]| {
            let v = momentum_speeds(x)?;
            let b = 4 * k;
            let (a, d) = (dir(x[b + 1]), dir(x[b + 2]));
            let u = x[b + 3];
            Some((v[k] * a[0] - u * d[0]).hypot(v[k] * a[1] - u * d[1]))
        }
    };
    let speeds = [
        spread(&inputs, &speed(0), draws, seed)?,
        spread(&inputs, &speed(1), draws, seed)?,
    ];
    let delta_v = [
        spread(&inputs, &dv(0), draws, seed)?,
        spread(&inputs, &dv(1), draws, seed)?,
    ];
    for (k, s) in speeds.iter().enumerate() {
        if s.value < 0.0 {
            warnings.push(format!(
                "{}: the impact speed comes out negative ({:.1} m/s): the directions or departure speeds are inconsistent.",
                vehicles[k].label, s.value
            ));
        }
    }
    let names = [
        "mass",
        "approach direction",
        "departure direction",
        "departure speed",
    ];
    let values: Vec<f64> = inputs.iter().map(|i| i.value).collect();
    let mut sensitivity = vec![];
    for (k, i) in inputs.iter().enumerate() {
        if i.low == i.high {
            continue;
        }
        let at = |v: f64| {
            let mut x = values.clone();
            x[k] = v;
            momentum_speeds(&x).unwrap_or([f64::NAN; 2])
        };
        sensitivity.push(SensitivityRow {
            input: format!("{}: {}", vehicles[k / 4].label, names[k % 4]),
            low_input: i.low,
            high_input: i.high,
            at_low: at(i.low),
            at_high: at(i.high),
        });
    }
    let summary = format!(
        "Impact speeds: {} {:.1} m/s ({:.0} km/h), {} {:.1} m/s ({:.0} km/h); 95 % {:.1}–{:.1} and {:.1}–{:.1} m/s",
        vehicles[0].label,
        speeds[0].value,
        speeds[0].value * 3.6,
        vehicles[1].label,
        speeds[1].value,
        speeds[1].value * 3.6,
        speeds[0].interval95[0],
        speeds[0].interval95[1],
        speeds[1].interval95[0],
        speeds[1].interval95[1]
    );
    Ok(MomentumRun {
        method: MOMENTUM_METHOD.into(),
        vehicles,
        speeds,
        delta_v,
        sensitivity,
        approach_angle_deg: angle,
        warnings,
        summary,
        assumptions: MOMENTUM_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: MOMENTUM_LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    })
}

// ---------------------------------------------------------------------------------------
// Crush profile measured on the scan
// ---------------------------------------------------------------------------------------

/// One measuring station across the damage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Station {
    /// The station on the undamaged face line, and the damaged surface found behind it (m).
    pub at: P3,
    pub surface: P3,
    /// Residual crush: the surface's distance behind the face line (m), and its 1σ.
    pub depth: f64,
    pub sigma: f64,
    /// Scan points in the station's strip.
    pub points: usize,
}

/// A crush profile measured on a damaged vehicle's scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrushProfile {
    /// The ends of the damage on the undamaged face line, as picked (m), and the horizontal
    /// direction into the vehicle.
    pub start: P3,
    pub end: P3,
    pub inward: P3,
    /// The measuring height (the ends' mean) and the half-height of the band of points used.
    pub height: f64,
    pub band: f64,
    /// The damage width: the ends' horizontal distance (m).
    pub width: f64,
    pub stations: Vec<Station>,
    #[serde(default)]
    pub sources: Vec<PointSource>,
}

/// Points behind the face line nearer than this are taken as the damaged surface: the depth
/// is this low percentile of the strip's depths, so a few stray points don't set it.
const SURFACE_PERCENTILE: f64 = 0.05;
/// Points more than this far in front of the face line (m) aren't the vehicle.
const PROUD: f64 = 0.10;

/// Measure a crush profile: `n` equally spaced stations between `start` and `end` (the
/// undamaged face line at the ends of the damage), each a strip across the line (half as wide
/// as the stations' spacing, at most 50 mm and at least 20 mm) within `band` of the ends' mean
/// height; its depth is where the surviving surface begins, the 5th percentile of the strip's
/// distances behind the line. `inside` is any point within the vehicle, for the inward side.
pub fn crush_profile(
    start: P3,
    end: P3,
    inside: P3,
    points: &[P3],
    n: usize,
    band: f64,
    point_sigma: f64,
) -> Result<CrushProfile, CrashError> {
    if !(2..=10).contains(&n) {
        return err("use 2 to 10 stations (2, 4 or 6 in the usual protocols)");
    }
    if !(band > 0.0 && band <= 0.5) {
        return err("the height band must be between 0 and 0.5 m");
    }
    let d = [end[0] - start[0], end[1] - start[1]];
    let width = d[0].hypot(d[1]);
    if width < 0.1 {
        return err("the ends of the damage are less than 0.1 m apart");
    }
    let u = [d[0] / width, d[1] / width, 0.0];
    let mut w = [-u[1], u[0], 0.0];
    if (inside[0] - start[0]) * w[0] + (inside[1] - start[1]) * w[1] < 0.0 {
        w = [-w[0], -w[1], 0.0];
    }
    let height = (start[2] + end[2]) / 2.0;
    let half = (width / (n - 1) as f64 / 4.0).clamp(0.02, 0.05);
    let mut stations = vec![];
    for k in 0..n {
        let s = width * k as f64 / (n - 1) as f64;
        let at = [start[0] + u[0] * s, start[1] + u[1] * s, height];
        let mut depths: Vec<(f64, P3)> = points
            .iter()
            .filter(|p| (p[2] - height).abs() <= band)
            .filter_map(|p| {
                let r = [p[0] - at[0], p[1] - at[1]];
                let along = r[0] * u[0] + r[1] * u[1];
                let into = r[0] * w[0] + r[1] * w[1];
                (along.abs() <= half && (-PROUD..=2.0).contains(&into)).then_some((into, *p))
            })
            .collect();
        if depths.len() < 5 {
            return err(format!(
                "station C{}: fewer than 5 scan points in its strip; widen the height band or check the ends",
                k + 1
            ));
        }
        depths.sort_by(|a, b| a.0.total_cmp(&b.0));
        let q = |f: f64| {
            depths[((f * (depths.len() - 1) as f64).round() as usize).min(depths.len() - 1)]
        };
        let (d5, surface) = q(SURFACE_PERCENTILE);
        let d15 = q(0.15).0;
        // 1σ: the scan points', the face line's (the ends are picks), and how sharply the
        // surface begins (the spread between the 5th and 15th percentiles).
        let sigma = (2.0 * point_sigma * point_sigma + (d15 - d5).powi(2)).sqrt();
        stations.push(Station {
            at,
            surface,
            depth: d5.max(0.0),
            sigma,
            points: depths.len(),
        });
    }
    Ok(CrushProfile {
        start,
        end,
        inward: w,
        height,
        band,
        width,
        stations,
        sources: vec![],
    })
}

// ---------------------------------------------------------------------------------------
// Crush energy (Campbell / CRASH3)
// ---------------------------------------------------------------------------------------

/// Energy absorbed by a crush profile under the CRASH3 model: over the damage width, each
/// strip absorbs A c + B c²/2 + G per unit width (G = A²/(2B)), the depths linear between
/// equally spaced measurements; times (1 + tan² α) for a principal direction of force α off
/// the face's normal.
pub fn crush_energy(a: f64, b: f64, width: f64, depths: &[f64], pdof_deg: f64) -> Option<f64> {
    crush_energy_g(a, b, a * a / (2.0 * b), width, depths, pdof_deg)
}

/// As `crush_energy`, with G given (CRASH3's own tables list G beside A and B, rounded
/// separately from A²/2B).
pub fn crush_energy_g(
    a: f64,
    b: f64,
    g: f64,
    width: f64,
    depths: &[f64],
    pdof_deg: f64,
) -> Option<f64> {
    if depths.len() < 2 || !(a > 0.0 && b > 0.0 && width > 0.0) {
        return None;
    }
    let step = width / (depths.len() - 1) as f64;
    let e: f64 = depths
        .windows(2)
        .map(|w| {
            let (c1, c2) = (w[0].max(0.0), w[1].max(0.0));
            step * (a * (c1 + c2) / 2.0 + b * (c1 * c1 + c1 * c2 + c2 * c2) / 6.0 + g)
        })
        .sum();
    Some(e * (1.0 + pdof_deg.to_radians().tan().powi(2)))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrushRun {
    pub method: String,
    pub label: String,
    /// Stiffness coefficients A (N/m) and B (N/m²), and where they came from.
    pub a: Input,
    pub b: Input,
    pub stiffness_source: String,
    /// The bundled table's entry A and B came from (its NHTSA tests), when not entered by the
    /// examiner.
    #[serde(default)]
    pub table_entry: Option<crate::stiffness::Entry>,
    /// The profile as measured on the scan, when the width and depths came from it.
    #[serde(default)]
    pub profile: Option<CrushProfile>,
    /// Width of the damage (m) and the crush depths, equally spaced across it (m).
    pub width: Input,
    pub depths: Vec<Input>,
    /// Principal direction of force, degrees off the damaged face's normal (|α| ≤ 45°).
    pub pdof_deg: Input,
    pub mass: Input,
    /// Crush energy (J) and the equivalent barrier speed √(2E/m) (m/s).
    pub energy: Spread,
    pub ebs: Spread,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub const CRUSH_ASSUMPTIONS: &[&str] = &[
    "The vehicle's structure behaves as the CRASH3 model says: force per unit width linear in crush depth (A + B c), with the stiffness coefficients stated for this vehicle and face.",
    "The crush depths are residual crush, measured from the undamaged outline, equally spaced across the damage width; the depth varies linearly between them.",
    "The principal direction of force is within 45° of the damaged face's normal.",
];

pub const CRUSH_LIMITATIONS: &[&str] = &[
    "Stiffness coefficients from crash tests at one speed and one face are extrapolated to this impact; their source and range matter more than any other input.",
    "Residual crush understates the maximum crush (restitution); the model's coefficients are built on residual crush, so the two are consistent, but restitution is not reported.",
    "The equivalent barrier speed is the speed into a rigid barrier that would absorb the same energy; it is not the impact speed, and it is not a delta-V without the other vehicle's energy and the collision's geometry.",
];

#[allow(clippy::too_many_arguments)]
pub fn crush(
    label: &str,
    a: Input,
    b: Input,
    stiffness_source: &str,
    width: Input,
    depths: Vec<Input>,
    pdof_deg: Input,
    mass: Input,
    draws: usize,
    seed: u64,
) -> Result<CrushRun, CrashError> {
    for (i, n) in [
        (&a, "A"),
        (&b, "B"),
        (&width, "damage width"),
        (&pdof_deg, "principal direction of force"),
        (&mass, "mass"),
    ] {
        i.check(n)?;
    }
    if a.low <= 0.0 || b.low <= 0.0 || width.low <= 0.0 || mass.low <= 0.0 {
        return err("A, B, the damage width and the mass must be positive");
    }
    if pdof_deg.low < -45.0 || pdof_deg.high > 45.0 {
        return err("the principal direction of force must be within 45° of the face's normal");
    }
    if !(2..=10).contains(&depths.len()) {
        return err("give between 2 and 10 crush depths (2, 4 or 6 in the usual protocols)");
    }
    for (k, d) in depths.iter().enumerate() {
        d.check(&format!("Crush depth C{}", k + 1))?;
        if d.low < 0.0 {
            return err("crush depths can't be negative");
        }
    }
    if stiffness_source.trim().is_empty() {
        return err("say where the stiffness coefficients come from");
    }
    let n = depths.len();
    let mut inputs = vec![a, b, width, pdof_deg, mass];
    inputs.extend(depths.iter().copied());
    let e = move |x: &[f64]| crush_energy(x[0], x[1], x[2], &x[5..5 + n], x[3]);
    let energy = spread(&inputs, &e, draws, seed)?;
    let ebs = spread(
        &inputs,
        &move |x: &[f64]| Some((2.0 * e(x)? / x[4]).sqrt()),
        draws,
        seed,
    )?;
    let summary = format!(
        "{label}: crush energy {:.0} kJ, equivalent barrier speed {:.1} m/s ({:.0} km/h); 95 % {:.1}–{:.1} m/s",
        energy.value / 1000.0,
        ebs.value,
        ebs.value * 3.6,
        ebs.interval95[0],
        ebs.interval95[1]
    );
    Ok(CrushRun {
        method: CRUSH_METHOD.into(),
        label: label.into(),
        a,
        b,
        stiffness_source: stiffness_source.trim().into(),
        table_entry: None,
        profile: None,
        width,
        depths,
        pdof_deg,
        mass,
        energy,
        ebs,
        summary,
        assumptions: CRUSH_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: CRUSH_LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(d: f64, mu: f64, n: f64, grade: f64) -> SkidSegment {
        SkidSegment {
            label: "s".into(),
            distance: Input::exact(d),
            drag: Input::exact(mu),
            braking: Input::exact(n),
            grade: Input::exact(grade),
            path: vec![],
            sources: vec![],
        }
    }

    // Hand-worked, g = 9.80665 m/s²:
    //   30 m, μ 0.7, level:   v = √(2 · 9.80665 · 0.7 · 30) = √411.879 = 20.2948 m/s
    //   + 5 % uphill:         f = 0.7 cos(2.862°) + sin(2.862°) = 0.749064; v = 20.9940 m/s
    //   braking 0.8:          v = √(2 g · 0.56 · 30) = 18.1522 m/s
    //   20 m at 0.7 + 10 m at 0.4: v = √(2 g (14 + 4)) = 18.7893 m/s
    #[test]
    fn skid_speeds_match_the_hand_worked_examples() {
        let v = |segs: Vec<SkidSegment>| skid(segs, Input::exact(0.0), 10, 1).unwrap().speed.value;
        assert!((v(vec![seg(30.0, 0.7, 1.0, 0.0)]) - 20.294_810).abs() < 1e-5);
        assert!((v(vec![seg(30.0, 0.7, 1.0, 0.05)]) - 20.994_015).abs() < 1e-5);
        assert!((v(vec![seg(30.0, 0.7, 0.8, 0.0)]) - 18.152_230).abs() < 1e-5);
        assert!(
            (v(vec![seg(20.0, 0.7, 1.0, 0.0), seg(10.0, 0.4, 1.0, 0.0)]) - 18.789_343).abs() < 1e-5
        );
        // Still moving at 10 m/s at the end: √(10² + 411.879) = 22.6247 m/s.
        let r = skid(vec![seg(30.0, 0.7, 1.0, 0.0)], Input::exact(10.0), 10, 1).unwrap();
        assert!((r.speed.value - (100.0f64 + 411.879_4).sqrt()).abs() < 1e-3);
        // Downhill steeper than the brakes can hold is refused.
        assert!(skid(vec![seg(30.0, 0.3, 1.0, -0.5)], Input::exact(0.0), 10, 1).is_err());
    }

    #[test]
    fn a_range_gives_the_extremes_and_an_interval_inside_them() {
        // μ 0.6–0.8 over 30 m: √(2 g 0.6 · 30) = 18.7893 to √(2 g 0.8 · 30) = 21.6961.
        let s = SkidSegment {
            drag: Input::range(0.7, 0.6, 0.8),
            ..seg(30.0, 0.7, 1.0, 0.0)
        };
        let r = skid(vec![s], Input::exact(0.0), 20_000, 3).unwrap();
        assert!((r.speed.low - 18.789_343).abs() < 1e-5, "{:?}", r.speed);
        assert!((r.speed.high - 21.696_064).abs() < 1e-5);
        assert!(r.speed.interval95[0] > r.speed.low && r.speed.interval95[1] < r.speed.high);
        // Uniform μ: the speed's median is at μ's median, 20.2948.
        assert!((r.speed.mean - 20.26).abs() < 0.05, "{}", r.speed.mean);
    }

    // R = 30²/(8 · 1.5) + 1.5/2 = 75.75 m; v = √(0.7 · g · 75.75) = 22.8035 m/s; with
    // superelevation 0.05: √(g R (0.75)/(1 − 0.035)) = 24.0281 m/s.
    #[test]
    fn yaw_speeds_match_the_hand_worked_examples() {
        assert!((radius_from_chord(30.0, 1.5).unwrap() - 75.75).abs() < 1e-12);
        assert!((critical_speed(75.75, 0.7, 0.0).unwrap() - 22.803_456).abs() < 1e-5);
        assert!((critical_speed(75.75, 0.7, 0.05).unwrap() - 24.028_056).abs() < 1e-5);
        let r = yaw(
            YawRadius::Chord {
                chord: Input::exact(30.0),
                ordinate: Input::exact(1.5),
            },
            Input::exact(0.7),
            Input::exact(0.0),
            0.0,
            0.0,
            10,
            1,
        )
        .unwrap();
        assert!((r.speed.value - 22.803_456).abs() < 1e-5);
        // Half a 1.55 m track off the radius: √(0.7 g (75.75 − 0.775)) = 22.6863 m/s.
        let r = yaw(
            YawRadius::Chord {
                chord: Input::exact(30.0),
                ordinate: Input::exact(1.5),
            },
            Input::exact(0.7),
            Input::exact(0.0),
            0.775,
            0.0,
            10,
            1,
        )
        .unwrap();
        assert!((r.speed.value - (0.7 * G * 74.975f64).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn a_circle_fitted_to_points_on_a_mark_gives_its_radius() {
        // 12 points over 40° of a 60 m circle on a road sloping 2 %, with 5 mm noise.
        let mut rng = Rng(5);
        let pts: Vec<P3> = (0..12)
            .map(|k| {
                let t = (k as f64 / 11.0 * 40.0).to_radians();
                let (x, y) = (
                    60.0 * t.cos() + 0.005 * rng.gauss(),
                    60.0 * t.sin() + 0.005 * rng.gauss(),
                );
                [x, y, 0.02 * x]
            })
            .collect();
        let c = fit_circle(&pts, 0.005).unwrap();
        // On the 2 % slope the mark is stretched by at most √(1 + 0.02²): 60.012 m.
        assert!((c.radius - 60.0).abs() < 0.2, "{c:?}");
        assert!(c.radius_sigma > 0.0 && c.radius_sigma < 0.5);
        assert!((c.arc_deg - 40.0).abs() < 1.0);
        let r = yaw(
            YawRadius::Points {
                points: pts,
                sources: vec![],
            },
            Input::exact(0.7),
            Input::exact(0.0),
            0.0,
            0.005,
            2000,
            1,
        )
        .unwrap();
        assert!(r.speed.interval95[0] < r.speed.value && r.speed.value < r.speed.interval95[1]);
        assert!(fit_circle(
            &[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
            0.0
        )
        .is_err());
    }

    #[test]
    fn a_short_mark_on_a_small_circle_fits_exactly() {
        let pts: Vec<P3> = (0..6)
            .map(|k| {
                let t = (-60.0 + 20.0 * k as f64).to_radians();
                [4.0 + 3.0 * t.cos(), 3.0 + 3.0 * t.sin(), 0.0]
            })
            .collect();
        let c = fit_circle(&pts, 0.001).unwrap();
        assert!((c.radius - 3.0).abs() < 1e-9 && c.rms < 1e-9, "{c:?}");
        assert!((c.arc_deg - 100.0).abs() < 1e-6, "{c:?}");
    }

    fn vehicle(label: &str, m: f64, approach: f64, departure: f64, u: f64) -> MomentumVehicle {
        MomentumVehicle {
            label: label.into(),
            mass: Input::exact(m),
            approach_deg: Input::exact(approach),
            departure_deg: Input::exact(departure),
            departure_speed: Input::exact(u),
        }
    }

    // A 1,500 kg car going north at 20 m/s and a 1,200 kg car going east at 15 m/s lock
    // together: momentum (18,000, 30,000) kg·m/s, departing at atan2(18,000, 30,000) =
    // 30.964° from north at 34,985.7 / 2,700 = 12.9577 m/s. Momentum gives back 20 and 15.
    #[test]
    fn momentum_recovers_the_hand_worked_collision() {
        let dep = 18_000f64.atan2(30_000.0).to_degrees();
        let u = 18_000f64.hypot(30_000.0) / 2_700.0;
        let r = momentum(
            [
                vehicle("A", 1500.0, 0.0, dep, u),
                vehicle("B", 1200.0, 90.0, dep, u),
            ],
            10,
            1,
        )
        .unwrap();
        assert!((r.speeds[0].value - 20.0).abs() < 1e-9, "{:?}", r.speeds);
        assert!((r.speeds[1].value - 15.0).abs() < 1e-9);
        // A's delta-V: |20 m/s north − 12.9577 m/s at 30.964°|.
        let dv_a = (u * dep.to_radians().sin()).hypot(20.0 - u * dep.to_radians().cos());
        assert!((r.delta_v[0].value - dv_a).abs() < 1e-9);
        assert!(r.warnings.is_empty());
        // Nearly parallel approaches are warned about.
        let r = momentum(
            [
                vehicle("A", 1500.0, 0.0, 5.0, 12.0),
                vehicle("B", 1200.0, 10.0, 5.0, 12.0),
            ],
            10,
            1,
        )
        .unwrap();
        assert!(!r.warnings.is_empty());
    }

    // One-at-a-time sensitivity, worked by hand: in the collision above, with B's mass at
    // 1,100 kg and the departure (both at 12.9577 m/s, 30.964° from north) held, the momentum
    // is P = 2,600 · 12.9577 = 33,690 kg·m/s; v_A = P cos φ / 1,500 = 19.2593 m/s and
    // v_B = P sin φ / 1,100 = 15.7576 m/s.
    #[test]
    fn the_sensitivity_table_moves_one_input_at_a_time() {
        let dep = 18_000f64.atan2(30_000.0).to_degrees();
        let u = 18_000f64.hypot(30_000.0) / 2_700.0;
        let mut b = vehicle("B", 1200.0, 90.0, dep, u);
        b.mass = Input::range(1200.0, 1100.0, 1300.0);
        let r = momentum([vehicle("A", 1500.0, 0.0, dep, u), b], 2000, 1).unwrap();
        assert_eq!(r.sensitivity.len(), 1);
        let row = &r.sensitivity[0];
        let (s, c) = (dep.to_radians().sin(), dep.to_radians().cos());
        let at = |mb: f64| {
            let p = (1500.0 + mb) * u;
            [p * c / 1500.0, p * s / mb]
        };
        for k in 0..2 {
            assert!((row.at_low[k] - at(1100.0)[k]).abs() < 1e-9, "{row:?}");
            assert!((row.at_high[k] - at(1300.0)[k]).abs() < 1e-9);
        }
        assert!((row.at_low[0] - 19.259_259).abs() < 1e-5);
        assert!((row.at_low[1] - 15.757_576).abs() < 1e-5);
    }

    // The CRASH3 User's Guide and Technical Manual (NHTSA, 1979), §5 sample run, vehicle 1
    // (category 4 front: A 356 lb/in, B 34 lb/in², G 1874 lb, Table 8-2): L 73.0 in, C1 2.7,
    // C2 3.6 in (two points), no oblique correction in the printout; printed energy
    // 19,245.3 ft-lb. Vehicle 2 (category 4 side: A 143, B 50, G 203): L 84.5 in,
    // C1–C6 6.2, 8.3, 9.2, 5.9, 4.4, 0.8 in, 45° (× 2); printed 31,220.8 ft-lb. The printout
    // gives the inputs to 0.1 in, so each printed energy must lie within the range of the
    // formula over the inputs ± 0.05 in (and the manual's equations (2)–(4) are the integral
    // used here).
    #[test]
    fn the_crash3_manuals_sample_energies_are_reproduced() {
        let inch = 0.0254;
        let lbf = 4.448_221_615_260_5;
        let ftlb = 1.355_817_948_331_4;
        let a_unit = lbf / inch; // lb/in → N/m
        let b_unit = lbf / (inch * inch); // lb/in² → N/m²
        let check = |a: f64, b: f64, g: f64, l: f64, c: &[f64], ang: f64, printed: f64| {
            let e = |l: f64, c: &[f64]| {
                crush_energy_g(
                    a * a_unit,
                    b * b_unit,
                    g * lbf,
                    l * inch,
                    &c.iter().map(|v| v * inch).collect::<Vec<_>>(),
                    ang,
                )
                .unwrap()
                    / ftlb
            };
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for mask in 0..(1u32 << (c.len() + 1)) {
                let s = |k: usize| if mask >> k & 1 == 1 { 0.05 } else { -0.05 };
                let cc: Vec<f64> = c
                    .iter()
                    .enumerate()
                    .map(|(k, v)| (v + s(k + 1)).max(0.0))
                    .collect();
                let v = e(l + s(0), &cc);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let nominal = e(l, c);
            assert!(
                lo <= printed && printed <= hi,
                "{printed} outside {lo}–{hi} (nominal {nominal})"
            );
            nominal
        };
        let v1 = check(356.0, 34.0, 1874.0, 73.0, &[2.7, 3.6], 0.0, 19_245.3);
        assert!((v1 - 19_245.3).abs() / 19_245.3 < 0.001, "{v1}");
        let v2 = check(
            143.0,
            50.0,
            203.0,
            84.5,
            &[6.2, 8.3, 9.2, 5.9, 4.4, 0.8],
            45.0,
            31_220.8,
        );
        assert!((v2 - 31_220.8).abs() / 31_220.8 < 0.005, "{v2}");
        // Equation (2) in closed form equals the span-by-span integral.
        let c = [0.1, 0.25, 0.31, 0.2, 0.12, 0.05];
        let (a, b, g, l) = (40_000.0, 500_000.0, 1_600.0, 1.6);
        let closed = l / 5.0
            * (a / 2.0 * (c[0] + 2.0 * (c[1] + c[2] + c[3] + c[4]) + c[5])
                + b / 6.0
                    * (c[0] * c[0]
                        + 2.0 * (c[1] * c[1] + c[2] * c[2] + c[3] * c[3] + c[4] * c[4])
                        + c[5] * c[5]
                        + c[0] * c[1]
                        + c[1] * c[2]
                        + c[2] * c[3]
                        + c[3] * c[4]
                        + c[4] * c[5])
                + 5.0 * g);
        assert!((crush_energy_g(a, b, g, l, &c, 0.0).unwrap() - closed).abs() < 1e-6);
    }

    /// A damaged front: the undamaged face along y = 0 (the vehicle at y > 0), dented over
    /// x 0–2 m to 0.3 (1 − (x − 1)²) m, with points behind the surface (the engine bay) and the
    /// bumper band 0.3–0.7 m high.
    #[test]
    fn a_crush_profile_is_measured_from_the_damaged_surface() {
        let mut pts = vec![];
        let mut rng = Rng(3);
        for i in 0..=300 {
            let x = -0.3 + 2.6 * i as f64 / 300.0;
            let dent = if (0.0..=2.0).contains(&x) {
                0.3 * (1.0 - (x - 1.0).powi(2))
            } else {
                0.0
            };
            for j in 0..=20 {
                let z = 0.3 + 0.4 * j as f64 / 20.0;
                pts.push([x, dent + 0.002 * rng.gauss(), z]);
                // Parts behind the surface.
                pts.push([x, dent + 0.2 + 0.5 * rng.uniform(), z]);
            }
        }
        let p = crush_profile(
            [0.0, 0.0, 0.5],
            [2.0, 0.0, 0.5],
            [1.0, 1.0, 0.5],
            &pts,
            6,
            0.15,
            0.002,
        )
        .unwrap();
        assert!((p.width - 2.0).abs() < 1e-12 && p.inward[1] > 0.99);
        for (k, st) in p.stations.iter().enumerate() {
            let x = 2.0 * k as f64 / 5.0;
            let truth = 0.3 * (1.0 - (x - 1.0f64).powi(2));
            // The strip is ±50 mm wide, and the curve's lowest point in it sets the depth.
            assert!(
                (st.depth - truth).abs() < 0.03,
                "C{}: {} vs {truth}",
                k + 1,
                st.depth
            );
            assert!(st.sigma > 0.0 && st.points > 20);
        }
        // The inside point on the other side flips the direction, and nothing is found.
        assert!(crush_profile(
            [0.0, 0.0, 0.5],
            [2.0, 0.0, 0.5],
            [1.0, -1.0, 0.5],
            &pts,
            6,
            0.15,
            0.002
        )
        .map(|p| p.stations.iter().all(|s| s.depth < 0.01))
        .unwrap_or(true));
    }

    // Uniform crush 0.3 m over 1.5 m, A = 50,000 N/m, B = 1,000,000 N/m²: G = 1,250 N;
    // E = 1.5 (50,000 · 0.3 + 1,000,000 · 0.09 / 2 + 1,250) = 91,875 J; for 1,500 kg the
    // equivalent barrier speed is √(2 · 91,875 / 1,500) = 11.0680 m/s.
    #[test]
    fn crush_energy_matches_the_hand_worked_example() {
        let e = crush_energy(50_000.0, 1_000_000.0, 1.5, &[0.3, 0.3], 0.0).unwrap();
        assert!((e - 91_875.0).abs() < 1e-6);
        // Six points give the same for uniform crush; a triangle (0 to 0.3 m) integrates to
        // 1.5 (50,000 · 0.15 + 1,000,000 · 0.09 / 6 + 1,250) = 35,625 J.
        let e6 = crush_energy(50_000.0, 1_000_000.0, 1.5, &[0.3; 6], 0.0).unwrap();
        assert!((e6 - 91_875.0).abs() < 1e-6);
        let tri = crush_energy(50_000.0, 1_000_000.0, 1.5, &[0.0, 0.3], 0.0).unwrap();
        assert!((tri - 35_625.0).abs() < 1e-6);
        // A force 30° off the normal: × (1 + tan² 30°) = × 4/3.
        let e30 = crush_energy(50_000.0, 1_000_000.0, 1.5, &[0.3, 0.3], 30.0).unwrap();
        assert!((e30 - 91_875.0 * 4.0 / 3.0).abs() < 1e-6);
        let r = crush(
            "front",
            Input::exact(50_000.0),
            Input::exact(1_000_000.0),
            "test",
            Input::exact(1.5),
            vec![Input::exact(0.3), Input::exact(0.3)],
            Input::exact(0.0),
            Input::exact(1500.0),
            10,
            1,
        )
        .unwrap();
        assert!((r.ebs.value - 11.067_972).abs() < 1e-5);
        assert!(crush(
            "f",
            Input::exact(1.0),
            Input::exact(1.0),
            " ",
            Input::exact(1.0),
            vec![Input::exact(0.1); 2],
            Input::exact(0.0),
            Input::exact(1.0),
            10,
            1
        )
        .is_err());
    }
}
