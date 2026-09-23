//! An animation: things moving along paths in segments, each segment with its source (an EDR
//! record, another analysis, measured evidence, or the examiner's assumption), a stated time
//! zero, and plausibility checks on the motion (accelerations against the stated friction,
//! jumps in speed or heading). Positions come from `motion`. See docs/methods/animation.md.

use crate::crash::G;
use crate::motion::{Path, Phase, Profile, Shape, P3};
use serde::{Deserialize, Serialize};

pub const ANIMATION_METHOD: &str = "animation/1";

/// Where a segment's motion (or a path, or a friction value) comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// An EDR analysis record in this project.
    Edr { analysis_id: i64, name: String },
    /// Another analysis record (skid, yaw, momentum, …).
    Analysis {
        analysis_id: i64,
        tool: String,
        name: String,
    },
    /// Measured on evidence (a scan, a photo, a video).
    Evidence {
        evidence_id: i64,
        name: String,
        note: String,
    },
    /// The examiner's assumption, with the reason.
    Assumption { note: String },
}

impl Source {
    pub fn assumed(&self) -> bool {
        matches!(self, Source::Assumption { .. })
    }
    pub fn describe(&self) -> String {
        match self {
            Source::Edr { analysis_id, name } => {
                format!("EDR record \"{name}\" (analysis {analysis_id})")
            }
            Source::Analysis {
                analysis_id,
                tool,
                name,
            } => format!("{tool} analysis \"{name}\" (analysis {analysis_id})"),
            Source::Evidence {
                evidence_id,
                name,
                note,
            } => format!(
                "measured on evidence {evidence_id} ({name}){}",
                if note.is_empty() {
                    String::new()
                } else {
                    format!(": {note}")
                }
            ),
            Source::Assumption { note } => format!("assumed: {note}"),
        }
    }
}

/// How a segment moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SegmentMotion {
    /// Constant speed (m/s), with its range (low, high) when its source gives one.
    Speed {
        speed: f64,
        #[serde(default)]
        range: Option<[f64; 2]>,
    },
    /// Constant acceleration (m/s², negative to brake) from `start_speed`, or from the previous
    /// segment's end speed when not given. Stops at zero.
    Accelerate {
        acceleration: f64,
        #[serde(default)]
        start_speed: Option<f64>,
        /// The start speed's range (low, high), when its source gives one.
        #[serde(default)]
        range: Option<[f64; 2]>,
    },
    /// Distances along the path (m, from the segment's start) at times (s, from its start):
    /// an EDR record's, or keyframes; with the speeds at those times when known (an EDR
    /// record's), which then set the interpolation's slopes.
    Table {
        times: Vec<f64>,
        distances: Vec<f64>,
        #[serde(default)]
        speeds: Option<Vec<f64>>,
        /// Per row, the range (low, high) of the distance and of the speed, when the source
        /// gives them (an EDR record's range method and speed tolerance).
        #[serde(default)]
        ranges: Option<Vec<[f64; 2]>>,
        #[serde(default)]
        speed_ranges: Option<Vec<[f64; 2]>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Seconds; `None` (only for the last) runs to the end of the timeline. A table's own
    /// duration is its last time.
    pub duration: Option<f64>,
    pub motion: SegmentMotion,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Friction {
    /// Coefficient and its tolerance (±), for the road surface.
    pub mu: f64,
    pub tolerance: f64,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MoverKind {
    /// Positioned by its rear axle's centre on the path; `wheelbase` (m) sets where the front is.
    Vehicle {
        wheelbase: f64,
    },
    Person,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mover {
    pub id: String,
    pub name: String,
    /// The scene object it moves, if any.
    #[serde(default)]
    pub object: Option<String>,
    pub kind: MoverKind,
    /// The path through picked points, and where the path comes from.
    pub path: Vec<P3>,
    pub shape: Shape,
    pub path_source: Source,
    /// Timeline time (s, from time zero) its first segment starts, and how far along the path.
    pub start: f64,
    #[serde(default)]
    pub offset: f64,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub friction: Option<Friction>,
}

/// What the timeline's zero is: an event, stated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeZero {
    /// "EDR trigger", "impact", "first frame of the video", …
    pub event: String,
    /// How it is known (the EDR record, a witness, the video's clock).
    pub basis: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lighting {
    Daylight,
    /// Dusk, dawn, night, or artificial light only.
    LowLight,
}

/// Horizontal field of view a driver or witness view defaults to (°): about what a person
/// attends to looking ahead, not the full extent of human vision (about 200°); wider views
/// distort distances and sizes and are warned.
pub const HUMAN_HFOV_DEG: f64 = 60.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewKind {
    /// From a vehicle: the eye's position in its frame (m: forward from the rear axle, left of
    /// centre, up from the ground).
    Driver { mover: String, eye: P3 },
    /// A witness standing at a floor point with a stated eye height, looking toward a target
    /// point, or toward a mover (tracked, at `LOOK_HEIGHT` above its path) when one is named.
    Witness {
        floor: P3,
        eye_height: f64,
        target: P3,
        #[serde(default)]
        target_mover: Option<String>,
    },
    /// A presentation camera circling a centre (m) at a radius and height above it, once per
    /// `period` seconds from the timeline's start. Nobody's point of view.
    Orbit {
        centre: P3,
        radius: f64,
        height: f64,
        period: f64,
    },
    /// A presentation camera following a mover: `offset` in its frame (m: forward, left, up;
    /// behind is negative), looking `look_ahead` m ahead of it. Nobody's point of view.
    Follow {
        mover: String,
        offset: P3,
        look_ahead: f64,
    },
}

impl ViewKind {
    /// A person's view (driver or witness): its field of view is held to a human-like one.
    pub fn human(&self) -> bool {
        matches!(self, ViewKind::Driver { .. } | ViewKind::Witness { .. })
    }
}

/// Height above a mover's path (m) that witness and follow cameras look at: about a car's
/// body or a person's chest.
pub const LOOK_HEIGHT: f64 = 1.0;

/// A camera's eye and the point it looks at, in the project frame (m).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub eye: P3,
    pub target: P3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub id: String,
    pub name: String,
    pub kind: ViewKind,
    pub hfov_deg: f64,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub time_zero: TimeZero,
    /// Timeline range (s from time zero).
    pub from: f64,
    pub to: f64,
    pub lighting: Lighting,
    pub movers: Vec<Mover>,
    #[serde(default)]
    pub views: Vec<View>,
}

/// A problem with the motion: when, what, how big, and the limit it passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    pub mover: String,
    /// "friction" (beyond (μ + tolerance) g), "friction_limit" (within the tolerance band),
    /// "speed_jump", "heading_jump", "no_friction".
    pub kind: String,
    pub from: f64,
    pub to: f64,
    pub peak: f64,
    pub limit: f64,
    pub message: String,
}

/// A segment the examiner assumed, for the timeline, the render and the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assumed {
    pub mover: String,
    pub segment: Option<usize>,
    pub from: f64,
    pub to: f64,
    pub note: String,
}

/// A mover at one time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub t: f64,
    pub position: P3,
    pub heading: f64,
    pub distance: f64,
    pub speed: f64,
    pub longitudinal: f64,
    pub lateral: f64,
    /// The segment it is in (`None` before the first and after the last), and whether that
    /// segment is assumed.
    pub segment: Option<usize>,
    pub assumed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimError(pub String);

impl std::fmt::Display for AnimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(m: impl Into<String>) -> Result<T, AnimError> {
    Err(AnimError(m.into()))
}

/// A mover made ready to evaluate: its path and each segment's profile, start and length.
pub struct Prepared<'a> {
    mover: &'a Mover,
    path: Path,
    /// Per segment: timeline start, duration (None: to the end), profile, distance at start.
    parts: Vec<(f64, Option<f64>, Profile, f64)>,
    /// Per segment: how its range is found, and the distance range at its start.
    spreads: Vec<(Spread, f64, f64)>,
    /// The distance range where the last timed segment ends.
    end: (f64, f64),
}

/// A segment's own range, from its source.
#[derive(Debug, Clone)]
enum Spread {
    /// None stated: the nominal motion.
    Exact,
    /// The motion at the low and at the high end of its speed range.
    Profiles(Profile, Profile),
    /// A table's per-row distance and speed ranges, interpolated linearly between rows.
    Rows {
        times: Vec<f64>,
        distances: Option<Vec<[f64; 2]>>,
        speeds: Option<Vec<[f64; 2]>>,
    },
}

/// Linear interpolation in a table of (low, high) rows at local time `u`.
fn lerp_rows(times: &[f64], rows: &[[f64; 2]], u: f64) -> [f64; 2] {
    let t0 = times[0];
    let k = times
        .windows(2)
        .position(|w| u + t0 < w[1])
        .unwrap_or(times.len() - 2);
    let f = ((u + t0 - times[k]) / (times[k + 1] - times[k])).clamp(0.0, 1.0);
    [0, 1].map(|j| rows[k][j] + (rows[k + 1][j] - rows[k][j]) * f)
}

fn profile(m: &SegmentMotion, entry_speed: f64) -> Profile {
    match m {
        SegmentMotion::Speed { speed, .. } => Profile::Constant { speed: *speed },
        SegmentMotion::Accelerate {
            acceleration,
            start_speed,
            ..
        } => Profile::Phases {
            speed: start_speed.unwrap_or(entry_speed),
            phases: vec![Phase {
                duration: 1e9,
                acceleration: *acceleration,
            }],
        },
        SegmentMotion::Table {
            times,
            distances,
            speeds,
            ..
        } => Profile::Table {
            times: times.iter().map(|t| t - times[0]).collect(),
            distances: distances.iter().map(|d| d - distances[0]).collect(),
            speeds: speeds.clone(),
        },
    }
}

/// A segment's own distance (from its start) and speed ranges at local time `u`:
/// [distance low, high, speed low, high].
fn local_bounds(spread: &Spread, nominal: &Profile, u: f64) -> [f64; 4] {
    let (ds, v, _) = nominal.at(u);
    match spread {
        Spread::Exact => [ds, ds, v, v],
        Spread::Profiles(a, b) => {
            let (da, va, _) = a.at(u);
            let (db, vb, _) = b.at(u);
            [da.min(db), da.max(db), va.min(vb), va.max(vb)]
        }
        Spread::Rows {
            times,
            distances,
            speeds,
        } => {
            let d = distances
                .as_ref()
                .map_or([ds, ds], |r| lerp_rows(times, r, u));
            let s = speeds.as_ref().map_or([v, v], |r| lerp_rows(times, r, u));
            [d[0], d[1], s[0], s[1]]
        }
    }
}

/// A profile's speed just after its start.
fn start_speed(p: &Profile) -> f64 {
    let h = 1e-6;
    p.at(h).0 / h
}

impl<'a> Prepared<'a> {
    pub fn new(m: &'a Mover) -> Result<Prepared<'a>, AnimError> {
        let path = Path::new(m.path.clone(), m.shape)
            .map_err(|e| AnimError(format!("{}: {}", m.name, e.0)))?;
        if m.segments.is_empty() {
            return err(format!("{}: give at least one motion segment", m.name));
        }
        let mut parts = vec![];
        let mut spreads = vec![];
        let (mut t, mut s, mut v) = (m.start, m.offset, 0.0);
        // The distance and speed ranges carried from one segment to the next.
        let (mut s_lo, mut s_hi, mut v_lo, mut v_hi) = (s, s, v, v);
        for (k, seg) in m.segments.iter().enumerate() {
            let p = profile(&seg.motion, v);
            p.check()
                .map_err(|e| AnimError(format!("{}, segment {}: {}", m.name, k + 1, e.0)))?;
            let bad = |what: &str| AnimError(format!("{}, segment {}: {what}", m.name, k + 1));
            let spread = match &seg.motion {
                SegmentMotion::Speed {
                    range: Some([lo, hi]),
                    speed,
                } => {
                    if !(*lo >= 0.0 && lo <= speed && speed <= hi) {
                        return Err(bad("the speed's range must hold the speed, from 0 up"));
                    }
                    Spread::Profiles(
                        Profile::Constant { speed: *lo },
                        Profile::Constant { speed: *hi },
                    )
                }
                SegmentMotion::Accelerate {
                    acceleration,
                    start_speed,
                    range,
                } => {
                    let r = match (range, start_speed) {
                        (Some(r), _) => Some(*r),
                        (None, None) if v_hi > v_lo => Some([v_lo, v_hi]),
                        _ => None,
                    };
                    match r {
                        Some([lo, hi]) if lo >= 0.0 && lo <= hi => {
                            let at = |speed| Profile::Phases {
                                speed,
                                phases: vec![Phase {
                                    duration: 1e9,
                                    acceleration: *acceleration,
                                }],
                            };
                            Spread::Profiles(at(lo), at(hi))
                        }
                        Some(_) => {
                            return Err(bad(
                                "the start speed's range must run from low to high, from 0 up",
                            ))
                        }
                        None => Spread::Exact,
                    }
                }
                SegmentMotion::Table {
                    times,
                    ranges,
                    speed_ranges,
                    ..
                } => {
                    for rows in [ranges, speed_ranges].into_iter().flatten() {
                        if rows.len() != times.len()
                            || rows
                                .iter()
                                .any(|r| r[0] > r[1] || r[0].is_nan() || r[1].is_nan())
                        {
                            return Err(bad("each row's range must run from low to high"));
                        }
                    }
                    if ranges.is_none() && speed_ranges.is_none() {
                        Spread::Exact
                    } else {
                        Spread::Rows {
                            times: times.clone(),
                            distances: ranges.clone(),
                            speeds: speed_ranges.clone(),
                        }
                    }
                }
                _ => Spread::Exact,
            };
            spreads.push((spread, s_lo, s_hi));
            let dur = match (&seg.motion, seg.duration) {
                (SegmentMotion::Table { times, .. }, _) => Some(times[times.len() - 1] - times[0]),
                (_, Some(d)) if d > 0.0 => Some(d),
                (_, None) if k + 1 == m.segments.len() => None,
                _ => return err(format!("{}, segment {}: give its duration", m.name, k + 1)),
            };
            parts.push((t, dur, p.clone(), s));
            if let Some(d) = dur {
                let (ds, ve, _) = p.at(d);
                let b = local_bounds(&spreads[k].0, &p, d);
                t += d;
                s += ds;
                v = ve;
                (s_lo, s_hi, v_lo, v_hi) = (s_lo + b[0], s_hi + b[1], b[2], b[3]);
            }
        }
        Ok(Prepared {
            mover: m,
            path,
            parts,
            spreads,
            end: (s_lo, s_hi),
        })
    }

    /// The range of the distance along the path and of the speed at time `t`: each segment's
    /// own range from its source, the distance's carried from segment to segment (the range
    /// method: the extremes of every stated range together). Clamped to the path.
    pub fn bounds(&self, t: f64) -> ([f64; 2], [f64; 2]) {
        let len = self.path.length();
        let clamp = |a: f64, b: f64| [a.clamp(0.0, len), b.clamp(0.0, len)];
        let (t0, ..) = self.parts[0];
        if t < t0 {
            let o = self.mover.offset;
            return (clamp(o, o), [0.0, 0.0]);
        }
        for (k, (ts, dur, p, _)) in self.parts.iter().enumerate() {
            if dur.is_none_or(|d| t < ts + d) {
                let (spread, lo, hi) = &self.spreads[k];
                let b = local_bounds(spread, p, t - ts);
                return (clamp(lo + b[0], hi + b[1]), [b[2], b[3]]);
            }
        }
        (clamp(self.end.0, self.end.1), [0.0, 0.0])
    }

    pub fn mover(&self) -> &Mover {
        self.mover
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Distance along the path, speed, longitudinal acceleration and segment at time `t`.
    fn along(&self, t: f64) -> (f64, f64, f64, Option<usize>) {
        let (t0, ..) = self.parts[0];
        if t < t0 {
            return (self.mover.offset, 0.0, 0.0, None);
        }
        for (k, (ts, dur, p, s0)) in self.parts.iter().enumerate() {
            if dur.is_none_or(|d| t < ts + d) {
                let (ds, v, a) = p.at(t - ts);
                return (s0 + ds, v, a, Some(k));
            }
        }
        // After the last timed segment: stopped where it ended.
        let (_, dur, p, s0) = self.parts.last().unwrap();
        (s0 + p.at(dur.unwrap()).0, 0.0, 0.0, None)
    }

    pub fn sample(&self, t: f64) -> Sample {
        let (raw, v, a, seg) = self.along(t);
        let len = self.path.length();
        let s = raw.clamp(0.0, len);
        let arrived = raw >= len && v > 0.0;
        let (p, d) = self.path.point_at(s);
        // Curvature from the direction's change over ±5 cm.
        let h = 0.05f64.min(len / 4.0);
        let (_, d1) = self.path.point_at((s - h).max(0.0));
        let (_, d2) = self.path.point_at((s + h).min(len));
        let span = ((s + h).min(len) - (s - h).max(0.0)).max(1e-9);
        let turn = (d2[1] * d1[0] - d2[0] * d1[1]).atan2(d2[0] * d1[0] + d2[1] * d1[1]);
        let kappa = turn / span;
        let v = if arrived { 0.0 } else { v };
        let mut heading = d[1].atan2(d[0]);
        // A vehicle's path is its rear axle's; the body points along the chord to the front
        // axle's point on the path, which is how a vehicle's heading follows a curve.
        if let MoverKind::Vehicle { wheelbase } = self.mover.kind {
            if wheelbase > 0.0 && s + wheelbase <= len {
                let (f, _) = self.path.point_at(s + wheelbase);
                heading = (f[1] - p[1]).atan2(f[0] - p[0]);
            }
        }
        Sample {
            t,
            position: p,
            heading,
            distance: s,
            speed: v,
            longitudinal: if arrived { 0.0 } else { a },
            lateral: v * v * kappa,
            segment: seg,
            assumed: seg.is_some_and(|k| self.mover.segments[k].source.assumed()),
        }
    }
}

/// Everything the timeline and the report need from an animation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evaluation {
    /// Per mover: its samples every `step` seconds over the timeline (for playback).
    pub step: f64,
    pub samples: Vec<(String, Vec<Sample>)>,
    pub flags: Vec<Flag>,
    pub assumed: Vec<Assumed>,
    pub warnings: Vec<String>,
    pub limitations: Vec<String>,
    /// Per view: its camera at the same times as the samples.
    #[serde(default)]
    pub cameras: Vec<(String, Vec<Camera>)>,
}

/// Where a view's camera is at time `t`, given its mover's sample then (for driver, follow
/// and a tracking witness).
pub fn camera(v: &ViewKind, t: f64, from: f64, mover: Option<&Sample>) -> Camera {
    let add = |p: P3, q: P3| [p[0] + q[0], p[1] + q[1], p[2] + q[2]];
    // A vector in a mover's frame (forward, left, up) to the project frame.
    let frame = |s: &Sample, q: P3| {
        let (c, n) = (s.heading.cos(), s.heading.sin());
        [c * q[0] - n * q[1], n * q[0] + c * q[1], q[2]]
    };
    match v {
        ViewKind::Driver { eye, .. } => {
            let s = mover.expect("a driver view has its vehicle");
            let e = add(s.position, frame(s, *eye));
            Camera {
                eye: e,
                target: add(e, frame(s, [10.0, 0.0, 0.0])),
            }
        }
        ViewKind::Witness {
            floor,
            eye_height,
            target,
            ..
        } => Camera {
            eye: add(*floor, [0.0, 0.0, *eye_height]),
            target: mover.map_or(*target, |s| add(s.position, [0.0, 0.0, LOOK_HEIGHT])),
        },
        ViewKind::Orbit {
            centre,
            radius,
            height,
            period,
        } => {
            let a = std::f64::consts::TAU * (t - from) / period;
            Camera {
                eye: add(*centre, [radius * a.cos(), radius * a.sin(), *height]),
                target: *centre,
            }
        }
        ViewKind::Follow {
            offset, look_ahead, ..
        } => {
            let s = mover.expect("a follow view has its mover");
            Camera {
                eye: add(s.position, frame(s, *offset)),
                target: add(s.position, frame(s, [*look_ahead, 0.0, LOOK_HEIGHT])),
            }
        }
    }
}

/// The mover a view is tied to, if any.
fn view_mover(v: &ViewKind) -> Option<&String> {
    match v {
        ViewKind::Driver { mover, .. } | ViewKind::Follow { mover, .. } => Some(mover),
        ViewKind::Witness { target_mover, .. } => target_mover.as_ref(),
        ViewKind::Orbit { .. } => None,
    }
}

/// Samples every `step` s for playback, and the checks at 100 Hz.
pub fn evaluate(a: &Animation, step: f64) -> Result<Evaluation, AnimError> {
    if a.to.is_nan() || a.from.is_nan() || a.to <= a.from {
        return err("the timeline must end after it starts");
    }
    if step.is_nan() || step <= 0.0 {
        return err("the sampling step must be over 0 s");
    }
    let mut samples = vec![];
    let mut flags = vec![];
    let mut assumed = vec![];
    let mut warnings = vec![];
    for m in &a.movers {
        let p = Prepared::new(m)?;
        let n = ((a.to - a.from) / step).ceil() as usize;
        samples.push((
            m.id.clone(),
            (0..=n)
                .map(|k| p.sample((a.from + k as f64 * step).min(a.to)))
                .collect(),
        ));
        flags.extend(check(&p, a.from, a.to));
        if m.path_source.assumed() {
            assumed.push(Assumed {
                mover: m.name.clone(),
                segment: None,
                from: a.from,
                to: a.to,
                note: format!("path {}", m.path_source.describe()),
            });
        }
        for (k, seg) in m.segments.iter().enumerate() {
            if let Source::Assumption { note } = &seg.source {
                let (ts, dur, ..) = p.parts[k];
                assumed.push(Assumed {
                    mover: m.name.clone(),
                    segment: Some(k),
                    from: ts,
                    to: dur.map_or(a.to, |d| ts + d),
                    note: note.clone(),
                });
            }
        }
    }
    let mut cameras = vec![];
    for v in &a.views {
        if !(v.hfov_deg > 1.0 && v.hfov_deg < 179.0) {
            return err(format!(
                "{}: the field of view must be between 1° and 179°",
                v.name
            ));
        }
        if let ViewKind::Orbit { radius, period, .. } = &v.kind {
            if !(*radius > 0.0 && *period > 0.0) {
                return err(format!(
                    "{}: the orbit needs a radius and a period over 0",
                    v.name
                ));
            }
        }
        let track = match view_mover(&v.kind) {
            Some(id) => {
                let Some(k) = a.movers.iter().position(|m| &m.id == id) else {
                    return err(format!("{}: its mover isn't in the animation", v.name));
                };
                if matches!(v.kind, ViewKind::Driver { .. })
                    && !matches!(a.movers[k].kind, MoverKind::Vehicle { .. })
                {
                    return err(format!("{}: a driver view needs a vehicle", v.name));
                }
                Some(&samples[k].1)
            }
            None => None,
        };
        let n = ((a.to - a.from) / step).ceil() as usize;
        cameras.push((
            v.id.clone(),
            (0..=n)
                .map(|k| {
                    let t = (a.from + k as f64 * step).min(a.to);
                    camera(&v.kind, t, a.from, track.map(|s: &Vec<Sample>| &s[k]))
                })
                .collect(),
        ));
        if v.kind.human() && v.hfov_deg > HUMAN_HFOV_DEG + 1e-9 {
            warnings.push(format!(
                "{}: a {:.0}° horizontal field of view is wider than the {:.0}° default for a human-like view; it makes things look farther away and smaller than a person there would see them.",
                v.name, v.hfov_deg, HUMAN_HFOV_DEG
            ));
        }
        if v.source.assumed() && v.kind.human() {
            assumed.push(Assumed {
                mover: v.name.clone(),
                segment: None,
                from: a.from,
                to: a.to,
                note: format!("view {}", v.source.describe()),
            });
        }
    }
    if a.time_zero.event.trim().is_empty() || a.time_zero.basis.trim().is_empty() {
        warnings.push(
            "Time zero isn't stated: give the event it stands for and how that is known.".into(),
        );
    }
    for m in &a.movers {
        let segs = m.segments.iter().map(|g| &g.source);
        for s in std::iter::once(&m.path_source).chain(segs) {
            if matches!(s, Source::Assumption { note } if note.trim().is_empty()) {
                warnings.push(format!(
                    "{}: an assumption without its reason; state why it is assumed.",
                    m.name
                ));
                break;
            }
        }
    }
    let mut limitations = vec![
        "Positions between the stated inputs are interpolated: a path is a smooth curve (or straight segments) through picked points, and motion within a segment follows its stated profile exactly; real motion between those inputs is not known.".to_string(),
        "Segments marked as assumed are the examiner's assumptions, not measurements; they are illustrative.".to_string(),
    ];
    if a.views
        .iter()
        .any(|v| matches!(v.kind, ViewKind::Driver { .. }))
    {
        limitations.push("A driver view is drawn without the driver's own vehicle: its pillars, mirrors, dashboard and tint are not modelled, so the view shows none of the obstruction they cause.".into());
    }
    if a.lighting == Lighting::LowLight {
        limitations.push("This is a night or low-light scene. The brightness and contrast of rendered images do not represent what a person could see: human visibility depends on adaptation, glare, headlamp patterns and contrast that a render does not reproduce. No conclusion about visibility is drawn from the renders.".into());
    }
    Ok(Evaluation {
        step,
        samples,
        flags,
        assumed,
        warnings,
        limitations,
        cameras,
    })
}

/// Plausibility at 100 Hz: accelerations against the stated friction (a friction circle,
/// longitudinal and lateral combined), and jumps in speed or heading.
fn check(p: &Prepared, from: f64, to: f64) -> Vec<Flag> {
    let m = p.mover;
    let mut flags = vec![];
    let dt = 0.01;
    let n = ((to - from) / dt).ceil() as usize;
    let states: Vec<Sample> = (0..=n)
        .map(|k| p.sample((from + k as f64 * dt).min(to)))
        .collect();
    // Friction: two bands, merged over consecutive samples.
    match &m.friction {
        Some(f) => {
            let (lo, hi) = ((f.mu - f.tolerance).max(0.0) * G, (f.mu + f.tolerance) * G);
            let mut run: Option<(f64, f64, f64, bool)> = None;
            let close = |run: &mut Option<(f64, f64, f64, bool)>, flags: &mut Vec<Flag>| {
                if let Some((a, b, peak, beyond)) = run.take() {
                    flags.push(Flag {
                        mover: m.name.clone(),
                        kind: if beyond { "friction" } else { "friction_limit" }.into(),
                        from: a,
                        to: b,
                        peak,
                        limit: if beyond { hi } else { lo },
                        message: if beyond {
                            format!("{}: from {a:.2} s to {b:.2} s the acceleration reaches {peak:.2} m/s² ({:.2} g), beyond what the stated friction allows ({:.2} ± {:.2}, at most {:.2} m/s²).", m.name, peak / G, f.mu, f.tolerance, hi)
                        } else {
                            format!("{}: from {a:.2} s to {b:.2} s the acceleration reaches {peak:.2} m/s² ({:.2} g), at the limit of the stated friction ({:.2} ± {:.2}).", m.name, peak / G, f.mu, f.tolerance)
                        },
                    });
                }
            };
            for s in &states {
                let total = s.longitudinal.hypot(s.lateral);
                let band = if total > hi + 1e-9 {
                    Some(true)
                } else if total > lo + 1e-9 {
                    Some(false)
                } else {
                    None
                };
                match (band, run.as_mut()) {
                    (Some(b), Some(r)) if r.3 == b => {
                        r.1 = s.t;
                        r.2 = r.2.max(total);
                    }
                    (Some(b), _) => {
                        close(&mut run, &mut flags);
                        run = Some((s.t, s.t, total, b));
                    }
                    (None, _) => close(&mut run, &mut flags),
                }
            }
            close(&mut run, &mut flags);
        }
        None if matches!(m.kind, MoverKind::Vehicle { .. }) => flags.push(Flag {
            mover: m.name.clone(),
            kind: "no_friction".into(),
            from,
            to,
            peak: 0.0,
            limit: 0.0,
            message: format!(
                "{}: no friction is stated, so its accelerations aren't checked against the road.",
                m.name
            ),
        }),
        None => {}
    }
    // Speed: a change between segments is instantaneous (infinite acceleration).
    for k in 1..p.parts.len() {
        let (ts, _, ref prof, _) = p.parts[k];
        let (pts, pdur, ref pprof, _) = p.parts[k - 1];
        let before = pprof.at(pdur.unwrap_or(ts - pts)).1;
        let after = start_speed(prof);
        if (after - before).abs() > 0.1 {
            flags.push(Flag {
                mover: m.name.clone(),
                kind: "speed_jump".into(),
                from: ts,
                to: ts,
                peak: after - before,
                limit: 0.1,
                message: format!("{}: at {ts:.2} s the speed jumps from {:.2} to {:.2} m/s between segments {k} and {}, an instantaneous change no vehicle or person makes.", m.name, before, after, k + 1),
            });
        }
    }
    // Heading: a turn of more than 2° within one 0.01 s step while moving, and more than three
    // times the steps either side: a corner in the path. A tight but smooth curve turns about
    // as much every step, and isn't flagged.
    let turn = |a: &Sample, b: &Sample| {
        (b.heading - a.heading + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI
    };
    let dh: Vec<f64> = states.windows(2).map(|w| turn(&w[0], &w[1])).collect();
    for k in 0..dh.len() {
        let near = [k.checked_sub(1), Some(k + 1)]
            .iter()
            .flatten()
            .filter_map(|&j| dh.get(j))
            .map(|d| d.abs())
            .fold(0.0, f64::max);
        let s = &states[k + 1];
        if s.speed > 0.1 && dh[k].abs().to_degrees() > 2.0 && dh[k].abs() > 3.0 * near {
            flags.push(Flag {
                mover: m.name.clone(),
                kind: "heading_jump".into(),
                from: s.t,
                to: s.t,
                peak: dh[k].to_degrees(),
                limit: 2.0,
                message: format!("{}: at {:.2} s the heading turns {:.1}° at once (a corner in its path) while moving at {:.1} m/s.", m.name, s.t, dh[k].to_degrees(), s.speed),
            });
        }
    }
    flags
}

/// A render, as logged: what was rendered, from which project and animation revision, with
/// which settings, and the file's hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderRecord {
    pub scene_id: i64,
    pub scene_revision: i64,
    /// The audit log's head hash when rendered: the project state it was made from.
    pub audit_head: String,
    pub view: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub from: f64,
    pub to: f64,
    pub frames: u64,
    pub overlays: Overlays,
    pub file: String,
    pub sha256: String,
}

/// Render overlays: all off by default.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Overlays {
    pub elapsed_time: bool,
    pub frame_number: bool,
    pub speeds: bool,
    pub scale_bar: bool,
    /// "Illustrative" on assumed segments.
    pub illustrative: bool,
    /// The time zero's event.
    pub time_zero: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assume(n: &str) -> Source {
        Source::Assumption { note: n.into() }
    }

    fn car(
        segments: Vec<Segment>,
        friction: Option<Friction>,
        path: Vec<P3>,
        shape: Shape,
    ) -> Mover {
        Mover {
            id: "v1".into(),
            name: "Car".into(),
            object: None,
            kind: MoverKind::Vehicle { wheelbase: 2.7 },
            path,
            shape,
            path_source: Source::Evidence {
                evidence_id: 1,
                name: "scan".into(),
                note: "tyre marks".into(),
            },
            start: -3.0,
            offset: 0.0,
            segments,
            friction,
        }
    }

    fn anim(m: Vec<Mover>, lighting: Lighting) -> Animation {
        Animation {
            time_zero: TimeZero {
                event: "impact".into(),
                basis: "EDR trigger".into(),
            },
            from: -4.0,
            to: 3.0,
            lighting,
            movers: m,
            views: vec![],
        }
    }

    #[test]
    fn segments_chain_and_keep_their_sources() {
        let edr = Source::Edr {
            analysis_id: 7,
            name: "Car EDR".into(),
        };
        // 2 s at 20 m/s (EDR), then brake at 6 m/s² (assumed) to a stop.
        let m = car(
            vec![
                Segment {
                    duration: Some(2.0),
                    motion: SegmentMotion::Speed {
                        speed: 20.0,
                        range: None,
                    },
                    source: edr,
                },
                Segment {
                    duration: None,
                    motion: SegmentMotion::Accelerate {
                        acceleration: -6.0,
                        start_speed: None,
                        range: None,
                    },
                    source: assume("braking after the EDR record ends"),
                },
            ],
            Some(Friction {
                mu: 0.75,
                tolerance: 0.05,
                source: assume("dry asphalt"),
            }),
            vec![[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]],
            Shape::Straight,
        );
        let a = anim(vec![m], Lighting::Daylight);
        let e = evaluate(&a, 0.1).unwrap();
        let s = &e.samples[0].1;
        // At −4 s it waits at the start; at −1 s (2 s in) it is 40 m along; it stops 400/12 m later.
        assert_eq!(s[0].position, [0.0, 0.0, 0.0]);
        let at = |t: f64| s.iter().find(|x| (x.t - t).abs() < 1e-9).unwrap();
        assert!((at(-1.0).position[0] - 40.0).abs() < 1e-9);
        assert!(!at(-1.5).assumed && at(0.0).assumed);
        let stop = 40.0 + 400.0 / 12.0;
        assert!((at(3.0).position[0] - stop).abs() < 1e-6, "{:?}", at(3.0));
        // Braking at 6 m/s² is within 0.70 g: no friction flag; one assumed segment listed.
        assert!(e.flags.is_empty(), "{:?}", e.flags);
        assert_eq!(e.assumed.len(), 1);
        assert!((e.assumed[0].from - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn implausible_motion_is_flagged() {
        // 25 m/s round a 30 m radius (20.8 m/s², 2.1 g) on μ 0.7 ± 0.05; then a jump to 10 m/s.
        let arc: Vec<P3> = (0..=12)
            .map(|k| {
                let a = std::f64::consts::PI * k as f64 / 12.0;
                [30.0 * a.sin(), 30.0 - 30.0 * a.cos(), 0.0]
            })
            .collect();
        let m = car(
            vec![
                Segment {
                    duration: Some(2.0),
                    motion: SegmentMotion::Speed {
                        speed: 25.0,
                        range: None,
                    },
                    source: assume("a"),
                },
                Segment {
                    duration: None,
                    motion: SegmentMotion::Speed {
                        speed: 10.0,
                        range: None,
                    },
                    source: assume("b"),
                },
            ],
            Some(Friction {
                mu: 0.7,
                tolerance: 0.05,
                source: assume("wet"),
            }),
            arc,
            Shape::Smooth,
        );
        let e = evaluate(&anim(vec![m], Lighting::LowLight), 0.1).unwrap();
        let fr = e
            .flags
            .iter()
            .find(|f| f.kind == "friction")
            .expect("friction flag");
        // The peak is the curve's own: a spline through 13 points on the circle ripples a few percent
        // in curvature around it. Mid-arc the lateral acceleration is v²/r within 3 %.
        assert!(
            (fr.peak / (25.0 * 25.0 / 30.0) - 1.0).abs() < 0.12,
            "{fr:?}"
        );
        let mid = e.samples[0]
            .1
            .iter()
            .find(|x| (x.t - (-2.0)).abs() < 1e-9)
            .unwrap();
        assert!(
            (mid.lateral / (25.0 * 25.0 / 30.0) - 1.0).abs() < 0.03,
            "{mid:?}"
        );
        assert!((fr.from - (-3.0)).abs() < 0.02, "{fr:?}");
        let j = e
            .flags
            .iter()
            .find(|f| f.kind == "speed_jump")
            .expect("speed jump");
        assert!((j.from - (-1.0)).abs() < 1e-9 && (j.peak + 15.0).abs() < 1e-6);
        assert!(e.limitations.iter().any(|l| l.contains("low-light")));
        // A corner in a straight path: a heading jump; no friction stated: flagged as unchecked.
        let m = car(
            vec![Segment {
                duration: None,
                motion: SegmentMotion::Speed {
                    speed: 5.0,
                    range: None,
                },
                source: assume("c"),
            }],
            None,
            vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
            Shape::Straight,
        );
        let e = evaluate(&anim(vec![m.clone()], Lighting::Daylight), 0.1).unwrap();
        assert!(e.flags.iter().any(|f| f.kind == "no_friction"));
        // A vehicle's chord heading turns through a corner over a wheelbase, not at once; a
        // person's tangent heading jumps there.
        assert!(!e.flags.iter().any(|f| f.kind == "heading_jump"));
        let person = Mover {
            kind: MoverKind::Person,
            ..m
        };
        let e = evaluate(&anim(vec![person], Lighting::Daylight), 0.1).unwrap();
        let j: Vec<_> = e
            .flags
            .iter()
            .filter(|f| f.kind == "heading_jump")
            .collect();
        assert_eq!(j.len(), 1, "{j:?}");
        assert!(
            (j[0].peak - 90.0).abs() < 1e-6 && (j[0].from - (-1.0)).abs() < 0.011,
            "{j:?}"
        );
        // A tight but smooth curve turns steadily, and isn't a jump.
        let mut curve = car(
            vec![Segment {
                duration: None,
                motion: SegmentMotion::Speed {
                    speed: 10.0,
                    range: None,
                },
                source: assume("c"),
            }],
            None,
            vec![[1.0, 3.0, 0.0], [4.0, 3.5, 0.0], [6.0, 2.0, 0.0]],
            Shape::Smooth,
        );
        curve.kind = MoverKind::Person;
        let e = evaluate(&anim(vec![curve], Lighting::Daylight), 0.1).unwrap();
        assert!(
            !e.flags.iter().any(|f| f.kind == "heading_jump"),
            "{:?}",
            e.flags
        );
    }

    #[test]
    fn a_wide_view_is_warned() {
        let mut a = anim(vec![], Lighting::Daylight);
        a.views.push(View {
            id: "w".into(),
            name: "Witness 1".into(),
            kind: ViewKind::Witness {
                floor: [0.0; 3],
                eye_height: 1.6,
                target: [10.0, 0.0, 1.0],
                target_mover: None,
            },
            hfov_deg: 90.0,
            source: assume("stated by the witness"),
        });
        let e = evaluate(&a, 0.1).unwrap();
        assert!(e.warnings[0].contains("90°"));
        a.views[0].hfov_deg = HUMAN_HFOV_DEG;
        assert!(evaluate(&a, 0.1).unwrap().warnings.is_empty());
        // A presentation camera isn't a person's view: no human field-of-view warning.
        a.views[0].kind = ViewKind::Orbit {
            centre: [0.0; 3],
            radius: 10.0,
            height: 5.0,
            period: 8.0,
        };
        a.views[0].hfov_deg = 90.0;
        assert!(evaluate(&a, 0.1).unwrap().warnings.is_empty());
    }

    #[test]
    fn cameras_follow_their_movers() {
        // A car at 10 m/s along +y from the origin, starting at t = -3 (timeline -4 to 3).
        let m = car(
            vec![Segment {
                duration: None,
                motion: SegmentMotion::Speed {
                    speed: 10.0,
                    range: None,
                },
                source: assume("c"),
            }],
            None,
            vec![[0.0, 0.0, 0.0], [0.0, 100.0, 0.0]],
            Shape::Straight,
        );
        let id = m.id.clone();
        let mut a = anim(vec![m], Lighting::Daylight);
        let view = |name: &str, kind| View {
            id: name.into(),
            name: name.into(),
            kind,
            hfov_deg: HUMAN_HFOV_DEG,
            source: assume("default"),
        };
        a.views = vec![
            view(
                "driver",
                ViewKind::Driver {
                    mover: id.clone(),
                    eye: [1.2, 0.35, 1.2],
                },
            ),
            view(
                "witness",
                ViewKind::Witness {
                    floor: [5.0, 20.0, 0.0],
                    eye_height: 1.6,
                    target: [0.0; 3],
                    target_mover: Some(id.clone()),
                },
            ),
            view(
                "orbit",
                ViewKind::Orbit {
                    centre: [0.0, 10.0, 0.0],
                    radius: 20.0,
                    height: 8.0,
                    period: 7.0,
                },
            ),
            view(
                "follow",
                ViewKind::Follow {
                    mover: id,
                    offset: [-8.0, 0.0, 3.0],
                    look_ahead: 5.0,
                },
            ),
        ];
        let e = evaluate(&a, 0.5).unwrap();
        let at = |v: usize, k: usize| e.cameras[v].1[k];
        let close = |p: P3, q: P3| (0..3).all(|i| (p[i] - q[i]).abs() < 1e-9);
        // k = 4 is t = -2: the car's rear axle is 10 m along, heading +y (left is -x).
        let c = at(0, 4);
        assert!(close(c.eye, [-0.35, 11.2, 1.2]), "{c:?}");
        assert!(close(c.target, [-0.35, 21.2, 1.2]), "{c:?}");
        let c = at(1, 4);
        assert!(close(c.eye, [5.0, 20.0, 1.6]), "{c:?}");
        assert!(close(c.target, [0.0, 10.0, LOOK_HEIGHT]), "{c:?}");
        // k = 7 is half a period in: the far side of the centre.
        let c = at(2, 7);
        assert!(close(c.eye, [-20.0, 10.0, 8.0]), "{c:?}");
        let c = at(3, 4);
        assert!(close(c.eye, [0.0, 2.0, 3.0]), "{c:?}");
        assert!(close(c.target, [0.0, 15.0, LOOK_HEIGHT]), "{c:?}");
        // Assumed driver and witness views are listed; a driver view of a person is refused.
        let views = e.assumed.iter().filter(|x| x.note.starts_with("view"));
        assert_eq!(views.count(), 2);
        a.movers[0].kind = MoverKind::Person;
        assert!(evaluate(&a, 0.5).is_err());
    }
}
