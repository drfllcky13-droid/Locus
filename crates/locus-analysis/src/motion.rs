//! Motion for the animation timeline: where a vehicle, person or camera is at any time. The
//! app's playback, renders and time/distance/speed reports all take positions from here, so the
//! tested math is the only math. See docs/methods/animation.md.
//!
//! A path is a curve through picked points (a centripetal Catmull–Rom spline, or straight
//! segments), parametrised by arc length. A speed profile says how far along it the object is
//! at each time: constant speed, phases of constant acceleration (stopping at zero, not
//! reversing), or a table of times and distances (an EDR record, or keyframes) interpolated
//! monotonically. A track is a path, a profile and a start time.

use serde::{Deserialize, Serialize};

pub type P3 = [f64; 3];

#[derive(Debug, Clone, PartialEq)]
pub struct MotionError(pub String);

impl std::fmt::Display for MotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(m: impl Into<String>) -> Result<T, MotionError> {
    Err(MotionError(m.into()))
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn norm(a: P3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn lerp(a: P3, b: P3, t: f64) -> P3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// A smooth curve through every point (centripetal Catmull–Rom: no cusps or loops).
    Smooth,
    /// Straight segments between the points.
    Straight,
}

/// A path through points, parametrised by arc length.
#[derive(Debug, Clone)]
pub struct Path {
    points: Vec<P3>,
    shape: Shape,
    /// Per segment: cumulative arc length at each of `SUB` + 1 equally spaced parameter values.
    table: Vec<Vec<f64>>,
    length: f64,
}

/// Parameter steps per segment in the arc-length table; each step's length comes from 5-point
/// Gauss–Legendre quadrature of the speed, so the table is exact to far below a millimetre.
const SUB: usize = 64;

const GL5: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

impl Path {
    pub fn new(points: Vec<P3>, shape: Shape) -> Result<Path, MotionError> {
        let mut pts: Vec<P3> = vec![];
        for p in points {
            if pts.last().is_none_or(|q| norm(sub(p, *q)) > 1e-9) {
                pts.push(p);
            }
        }
        if pts.len() < 2 {
            return err("a path needs at least two distinct points");
        }
        let mut path = Path {
            points: pts,
            shape,
            table: vec![],
            length: 0.0,
        };
        let mut total = 0.0;
        for seg in 0..path.points.len() - 1 {
            let mut row = vec![total];
            for k in 0..SUB {
                let (a, b) = (k as f64 / SUB as f64, (k + 1) as f64 / SUB as f64);
                let (mid, half) = ((a + b) / 2.0, (b - a) / 2.0);
                let len: f64 = GL5
                    .iter()
                    .map(|(x, w)| w * norm(path.derivative(seg, mid + half * x)))
                    .sum::<f64>()
                    * half;
                total += len;
                row.push(total);
            }
            path.table.push(row);
        }
        path.length = total;
        Ok(path)
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    /// Centripetal Catmull–Rom control points for segment `i` (P1 to P2).
    fn controls(&self, i: usize) -> [P3; 4] {
        let p = &self.points;
        let n = p.len();
        let p1 = p[i];
        let p2 = p[i + 1];
        // Beyond the ends, a phantom point that continues the curvature (quadratic
        // extrapolation from three points); with only two points, a straight continuation.
        let ext = |a: P3, b: P3, c: Option<P3>| -> P3 {
            match c {
                Some(c) => std::array::from_fn(|k| 3.0 * a[k] - 3.0 * b[k] + c[k]),
                None => lerp(b, a, 2.0),
            }
        };
        let p0 = if i > 0 {
            p[i - 1]
        } else {
            ext(p1, p2, p.get(i + 2).copied())
        };
        let p3 = if i + 2 < n {
            p[i + 2]
        } else {
            ext(p2, p1, i.checked_sub(1).map(|j| p[j]))
        };
        [p0, p1, p2, p3]
    }

    /// The point at parameter `u` ∈ [0, 1] of segment `seg`.
    fn at(&self, seg: usize, u: f64) -> P3 {
        match self.shape {
            Shape::Straight => lerp(self.points[seg], self.points[seg + 1], u),
            Shape::Smooth => {
                // Barry–Goldman pyramidal form with centripetal knots (α = 0.5).
                let [p0, p1, p2, p3] = self.controls(seg);
                let knot = |a: P3, b: P3| norm(sub(b, a)).sqrt().max(1e-9);
                let t0 = 0.0;
                let t1 = t0 + knot(p0, p1);
                let t2 = t1 + knot(p1, p2);
                let t3 = t2 + knot(p2, p3);
                let t = t1 + (t2 - t1) * u;
                let f = |a: P3, b: P3, ta: f64, tb: f64| lerp(a, b, (t - ta) / (tb - ta));
                let a1 = f(p0, p1, t0, t1);
                let a2 = f(p1, p2, t1, t2);
                let a3 = f(p2, p3, t2, t3);
                let b1 = f(a1, a2, t0, t2);
                let b2 = f(a2, a3, t1, t3);
                f(b1, b2, t1, t2)
            }
        }
    }

    /// d(point)/du on segment `seg`, by central differences (exact for the straight shape).
    fn derivative(&self, seg: usize, u: f64) -> P3 {
        if self.shape == Shape::Straight {
            return sub(self.points[seg + 1], self.points[seg]);
        }
        let h = 1e-6;
        let (a, b) = ((u - h).max(0.0), (u + h).min(1.0));
        let d = sub(self.at(seg, b), self.at(seg, a));
        d.map(|v| v / (b - a))
    }

    /// The point `s` metres along the path (clamped to its ends) and the unit direction of
    /// travel there.
    pub fn point_at(&self, s: f64) -> (P3, P3) {
        let s = s.clamp(0.0, self.length);
        // The segment, then the step, then Newton on the arc length within the step.
        let seg = self
            .table
            .iter()
            .position(|row| s <= row[SUB])
            .unwrap_or(self.table.len() - 1);
        let row = &self.table[seg];
        let k = row.windows(2).position(|w| s <= w[1]).unwrap_or(SUB - 1);
        let (a, b) = (k as f64 / SUB as f64, (k + 1) as f64 / SUB as f64);
        let mut u = a + (b - a) * ((s - row[k]) / (row[k + 1] - row[k]).max(1e-15));
        for _ in 0..6 {
            // Arc length from a to u by Gauss–Legendre, then a Newton step.
            let (mid, half) = ((a + u) / 2.0, (u - a) / 2.0);
            let len: f64 = row[k]
                + GL5
                    .iter()
                    .map(|(x, w)| w * norm(self.derivative(seg, mid + half * x)))
                    .sum::<f64>()
                    * half;
            let speed = norm(self.derivative(seg, u)).max(1e-12);
            let du = (s - len) / speed;
            u = (u + du).clamp(a, b);
            if du.abs() < 1e-13 {
                break;
            }
        }
        let d = self.derivative(seg, u);
        let n = norm(d).max(1e-15);
        (self.at(seg, u), d.map(|v| v / n))
    }
}

/// How far along the path the object is over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Profile {
    /// Constant speed (m/s).
    Constant { speed: f64 },
    /// From `speed` (m/s), phases of constant acceleration (m/s², negative to brake), each for
    /// its duration (s); after the last, the last speed holds. Speed never goes below zero: an
    /// object braking to a stop stays stopped.
    Phases { speed: f64, phases: Vec<Phase> },
    /// Distance along the path (m) at times (s), increasing: an EDR record's distances, or
    /// keyframes. Interpolated monotonically (Fritsch–Carlson), so it never runs backwards.
    Table {
        times: Vec<f64>,
        distances: Vec<f64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Phase {
    pub duration: f64,
    pub acceleration: f64,
}

impl Profile {
    pub fn check(&self) -> Result<(), MotionError> {
        match self {
            Profile::Constant { speed } if !(speed.is_finite() && *speed >= 0.0) => {
                err("the speed must be 0 or more")
            }
            Profile::Phases { speed, phases } => {
                if !(speed.is_finite() && *speed >= 0.0) {
                    return err("the starting speed must be 0 or more");
                }
                if phases.iter().any(|p| {
                    !(p.duration.is_finite() && p.duration > 0.0) || !p.acceleration.is_finite()
                }) {
                    return err("each phase needs a duration over 0 s and an acceleration");
                }
                Ok(())
            }
            Profile::Table { times, distances } => {
                if times.len() < 2 || times.len() != distances.len() {
                    return err("a time–distance table needs at least two rows");
                }
                if times.iter().any(|t| !t.is_finite()) || times.windows(2).any(|w| w[1] <= w[0]) {
                    return err("the table's times must increase");
                }
                if distances.windows(2).any(|w| w[1] < w[0]) {
                    return err("the table's distances must not decrease");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Distance (m), speed (m/s) and acceleration (m/s²) at `t` seconds from the profile's
    /// start (0 before it).
    pub fn at(&self, t: f64) -> (f64, f64, f64) {
        if t <= 0.0 {
            return match self {
                Profile::Table { distances, .. } => (distances[0], 0.0, 0.0),
                _ => (0.0, 0.0, 0.0),
            };
        }
        match self {
            Profile::Constant { speed } => (speed * t, *speed, 0.0),
            Profile::Phases { speed, phases } => {
                let (mut s, mut v, mut left) = (0.0, *speed, t);
                for p in phases {
                    let dt = p.duration.min(left);
                    let (ds, v1, a) = phase(v, p.acceleration, dt);
                    s += ds;
                    if left <= p.duration {
                        return (s, v1, a);
                    }
                    v = v1;
                    left -= p.duration;
                }
                (s + v * left, v, 0.0)
            }
            Profile::Table { times, distances } => pchip(times, distances, t),
        }
    }

    /// How long the profile runs (s): to its last phase or row; `None` for constant speed.
    pub fn duration(&self) -> Option<f64> {
        match self {
            Profile::Constant { .. } => None,
            Profile::Phases { phases, .. } => Some(phases.iter().map(|p| p.duration).sum()),
            Profile::Table { times, .. } => Some(times[times.len() - 1]),
        }
    }
}

/// Distance, end speed and acceleration of `dt` seconds from speed `v` at acceleration `a`,
/// stopping at zero.
fn phase(v: f64, a: f64, dt: f64) -> (f64, f64, f64) {
    if a < 0.0 && v + a * dt < 0.0 {
        let stop = v / -a;
        (v * stop / 2.0, 0.0, 0.0)
    } else {
        (v * dt + a * dt * dt / 2.0, v + a * dt, a)
    }
}

/// Monotone cubic interpolation (Fritsch & Carlson 1980) of s(t), clamped outside the table
/// (holding the last distance): distance, speed, acceleration.
fn pchip(t: &[f64], s: &[f64], x: f64) -> (f64, f64, f64) {
    let n = t.len();
    if x >= t[n - 1] {
        return (s[n - 1], 0.0, 0.0);
    }
    let h: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    let d: Vec<f64> = (0..n - 1).map(|k| (s[k + 1] - s[k]) / h[k]).collect();
    let mut m = vec![0.0; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        if d[k - 1] * d[k] <= 0.0 {
            m[k] = 0.0;
        } else {
            // Weighted harmonic mean (Fritsch–Butland), which keeps monotonicity.
            let (w1, w2) = (2.0 * h[k] + h[k - 1], h[k] + 2.0 * h[k - 1]);
            m[k] = (w1 + w2) / (w1 / d[k - 1] + w2 / d[k]);
        }
    }
    let k = t.windows(2).position(|w| x < w[1]).unwrap_or(n - 2);
    let (hk, u) = (h[k], (x - t[k]) / h[k]);
    let (u2, u3) = (u * u, u * u * u);
    let (h00, h10, h01, h11) = (
        2.0 * u3 - 3.0 * u2 + 1.0,
        u3 - 2.0 * u2 + u,
        -2.0 * u3 + 3.0 * u2,
        u3 - u2,
    );
    let pos = h00 * s[k] + h10 * hk * m[k] + h01 * s[k + 1] + h11 * hk * m[k + 1];
    let (d00, d10, d01, d11) = (
        6.0 * u2 - 6.0 * u,
        3.0 * u2 - 4.0 * u + 1.0,
        -6.0 * u2 + 6.0 * u,
        3.0 * u2 - 2.0 * u,
    );
    let vel = (d00 * s[k] + d01 * s[k + 1]) / hk + d10 * m[k] + d11 * m[k + 1];
    let (a00, a10, a01, a11) = (
        12.0 * u - 6.0,
        6.0 * u - 4.0,
        -12.0 * u + 6.0,
        6.0 * u - 2.0,
    );
    let acc = (a00 * s[k] + a01 * s[k + 1]) / (hk * hk) + (a10 * m[k] + a11 * m[k + 1]) / hk;
    (pos, vel, acc)
}

/// Something moving along a path: its profile starts at `start` (s on the timeline), `offset`
/// metres along the path.
#[derive(Debug, Clone)]
pub struct Track {
    pub path: Path,
    pub profile: Profile,
    pub start: f64,
    pub offset: f64,
}

/// Where a tracked object is at a time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub t: f64,
    pub position: P3,
    /// Unit direction of travel, and heading in the horizontal plane (rad from +x toward +y).
    pub direction: P3,
    pub heading: f64,
    /// Distance along the path (m), speed (m/s) and acceleration along it (m/s²).
    pub distance: f64,
    pub speed: f64,
    pub acceleration: f64,
    /// Whether it has reached the path's end (and stopped there).
    pub arrived: bool,
}

impl Track {
    pub fn new(
        path: Path,
        profile: Profile,
        start: f64,
        offset: f64,
    ) -> Result<Track, MotionError> {
        profile.check()?;
        if !(offset >= 0.0 && offset <= path.length()) {
            return err("the starting point must be on the path");
        }
        Ok(Track {
            path,
            profile,
            start,
            offset,
        })
    }

    pub fn at(&self, t: f64) -> State {
        let (ds, v, a) = self.profile.at(t - self.start);
        let raw = self.offset + ds;
        let arrived = raw >= self.path.length();
        let s = raw.min(self.path.length());
        let (p, d) = self.path.point_at(s);
        State {
            t,
            position: p,
            direction: d,
            heading: d[1].atan2(d[0]),
            distance: s,
            speed: if arrived { 0.0 } else { v },
            acceleration: if arrived { 0.0 } else { a },
            arrived,
        }
    }

    /// States at every frame from `from` to `to` (inclusive) at `fps`: frame k at from + k/fps.
    pub fn frames(&self, from: f64, to: f64, fps: f64) -> Vec<State> {
        let n = ((to - from) * fps + 1e-9).floor() as usize;
        (0..=n).map(|k| self.at(from + k as f64 / fps)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points on a circle of radius r about the origin, from angle 0 to `sweep` (rad).
    fn arc(r: f64, sweep: f64, n: usize) -> Vec<P3> {
        (0..=n)
            .map(|k| {
                let a = sweep * k as f64 / n as f64;
                [r * a.cos(), r * a.sin(), 0.0]
            })
            .collect()
    }

    /// The path's length by brute force: a polyline of a million points.
    fn brute_length(p: &Path, upto_seg: usize, u: f64) -> f64 {
        let mut s = 0.0;
        let mut last = p.at(0, 0.0);
        let steps = 20_000;
        for seg in 0..=upto_seg {
            let end = if seg == upto_seg { u } else { 1.0 };
            for k in 1..=steps {
                let q = p.at(seg, end * k as f64 / steps as f64);
                s += norm(sub(q, last));
                last = q;
            }
        }
        s
    }

    #[test]
    fn a_straight_path_at_constant_speed() {
        let p = Path::new(
            vec![[0.0, 0.0, 0.0], [30.0, 40.0, 0.0], [30.0, 40.0, 10.0]],
            Shape::Straight,
        )
        .unwrap();
        assert!((p.length() - 60.0).abs() < 1e-12);
        let t = Track::new(p, Profile::Constant { speed: 13.9 }, 2.0, 0.0).unwrap();
        // Before its start it waits at the beginning; at 2 + 50/13.9 s it is 50 m along.
        assert_eq!(t.at(0.0).position, [0.0, 0.0, 0.0]);
        let s = t.at(2.0 + 50.0 / 13.9);
        assert!(
            (s.position[2] - 0.0).abs() < 1e-9 && (s.position[0] - 30.0).abs() < 1e-9,
            "{s:?}"
        );
        let s = t.at(2.0 + 55.0 / 13.9);
        assert!((s.position[2] - 5.0).abs() < 1e-9, "{s:?}");
        assert!(t.at(100.0).arrived && t.at(100.0).speed == 0.0);
    }

    /// The acceptance test: an object at a set speed along a curved path is where it should be
    /// at every frame — its distance along the curve is v t (checked against a brute-force
    /// polyline length) and it stays on the curve.
    #[test]
    fn an_object_at_a_set_speed_is_right_at_every_frame() {
        let r = 25.0;
        let path = Path::new(arc(r, std::f64::consts::FRAC_PI_2, 8), Shape::Smooth).unwrap();
        let v = 50.0 / 3.6;
        let track = Track::new(path.clone(), Profile::Constant { speed: v }, 0.0, 0.0).unwrap();
        let frames = track.frames(0.0, path.length() / v, 30.0);
        assert!(frames.len() > 80);
        let mut worst_s = 0.0f64;
        let mut worst_r = 0.0f64;
        for f in &frames {
            // Where it should be: v t along the curve.
            worst_s = worst_s.max((f.distance - (v * f.t).min(path.length())).abs());
            worst_r = worst_r.max((norm(f.position) - r).abs());
        }
        assert!(worst_s < 1e-9, "{worst_s}");
        // The spline through 9 points on the circle stays within 3 mm of it.
        assert!(worst_r < 0.003, "{worst_r}");
        // And the arc-length parametrisation agrees with brute force at a frame mid-segment.
        let f = &frames[frames.len() / 3];
        let (seg, u) = locate(&path, f.distance);
        let brute = brute_length(&path, seg, u);
        assert!(
            (brute - f.distance).abs() < 1e-4,
            "{brute} vs {}",
            f.distance
        );
        // The whole arc: r π/2 to within the spline's approximation.
        assert!(
            (path.length() - r * std::f64::consts::FRAC_PI_2).abs() < 0.01,
            "{}",
            path.length()
        );
    }

    /// The segment and parameter of a distance, found by bisection on `point_at` (test only).
    fn locate(p: &Path, s: f64) -> (usize, f64) {
        let target = p.point_at(s).0;
        for seg in 0..p.points.len() - 1 {
            let (mut a, mut b) = (0.0, 1.0);
            let f = |u: f64| norm(sub(p.at(seg, u), target));
            for _ in 0..200 {
                let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
                if f(m1) < f(m2) {
                    b = m2
                } else {
                    a = m1
                }
            }
            if f(a) < 1e-9 {
                return (seg, a);
            }
        }
        panic!("not on the path");
    }

    #[test]
    fn braking_phases_stop_and_stay_stopped() {
        // 20 m/s, coast 1 s, brake at 7 m/s² for 5 s: stops after 20/7 s, 400/14 m.
        let p = Profile::Phases {
            speed: 20.0,
            phases: vec![
                Phase {
                    duration: 1.0,
                    acceleration: 0.0,
                },
                Phase {
                    duration: 5.0,
                    acceleration: -7.0,
                },
            ],
        };
        p.check().unwrap();
        let (s, v, _) = p.at(1.0);
        assert!((s - 20.0).abs() < 1e-12 && v == 20.0);
        let (s, v, a) = p.at(2.0);
        assert!((s - (20.0 + 20.0 - 3.5)).abs() < 1e-12 && (v - 13.0).abs() < 1e-12 && a == -7.0);
        let stop = 20.0 + 400.0 / 14.0;
        let (s, v, _) = p.at(6.0);
        assert!((s - stop).abs() < 1e-12 && v == 0.0, "{s} {v}");
        assert_eq!(p.at(60.0).0, p.at(6.0).0);
    }

    #[test]
    fn a_time_distance_table_is_followed_monotonically() {
        // An EDR-like record: 20, 20, 18, 16, 14 m/s at 0.5 s steps, as distances.
        let times = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        let distances = vec![0.0, 10.0, 19.5, 28.0, 35.5];
        let p = Profile::Table {
            times: times.clone(),
            distances: distances.clone(),
        };
        p.check().unwrap();
        for (t, s) in times.iter().zip(&distances) {
            assert!((p.at(*t).0 - s).abs() < 1e-12 || *t == 0.0);
        }
        let mut last = 0.0;
        for k in 0..=200 {
            let (s, v, _) = p.at(k as f64 * 0.01);
            assert!(s >= last - 1e-12 && v >= -1e-12);
            last = s;
        }
        // Held at the last row.
        assert_eq!(p.at(5.0), (35.5, 0.0, 0.0));
        assert!(Profile::Table {
            times: vec![0.0, 1.0],
            distances: vec![5.0, 4.0]
        }
        .check()
        .is_err());
    }
}
