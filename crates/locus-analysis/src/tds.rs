//! The time–distance–speed report of an animation: per mover, a table at a chosen step
//! (distance along its path, speed and their ranges, acceleration, heading, and the segment's
//! source); distances between chosen pairs of movers, with closing speed if asked; and the
//! time and distance for each mover to reach chosen points. Everything comes from
//! `animation` (the same motion the timeline plays and renders show). See
//! docs/methods/animation.md.

use crate::animation::{
    evaluate, AnimError, Animation, Assumed, Flag, MoverKind, Prepared, Source,
};
use crate::motion::P3;
use serde::{Deserialize, Serialize};

pub const TDS_METHOD: &str = "animation-tds/1";

/// Checks and table rows are made on this grid (s), whatever the report's step.
const FINE: f64 = 0.01;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedPoint {
    pub name: String,
    pub position: P3,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TdsRequest {
    /// Table interval (s).
    pub step: f64,
    /// Pairs of mover ids to give the distance between.
    #[serde(default)]
    pub pairs: Vec<[String; 2]>,
    /// Add closing speed to the pair tables.
    #[serde(default)]
    pub closing: bool,
    /// Points (a conflict point, a stop line) to give each mover's time and distance to.
    #[serde(default)]
    pub points: Vec<NamedPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub t: f64,
    pub distance: f64,
    /// Present only when a source gives a range and it isn't a single value.
    pub distance_range: Option<[f64; 2]>,
    pub speed: f64,
    pub speed_range: Option<[f64; 2]>,
    pub acceleration: f64,
    /// Degrees anticlockwise from the project's +x (east).
    pub heading_deg: f64,
    /// 1-based; `None` before the first segment or after the last timed one.
    pub segment: Option<usize>,
    pub assumed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoverTable {
    pub id: String,
    pub name: String,
    /// What its path and positions refer to.
    pub reference: String,
    pub path_length: f64,
    pub path_source: String,
    pub segments: Vec<String>,
    pub friction: Option<String>,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairRow {
    pub t: f64,
    pub distance: f64,
    pub range: Option<[f64; 2]>,
    /// Rate the distance shrinks (m/s; negative while they separate).
    pub closing: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairTable {
    pub a: String,
    pub b: String,
    pub rows: Vec<PairRow>,
    /// The least distance on the fine grid, and when.
    pub least: (f64, f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arrival {
    pub mover: String,
    /// Where the point falls on the mover's path (m along it), and how far off the path it is.
    pub at: f64,
    pub off_path: f64,
    /// When it reaches the point: nominal, earliest and latest (the ends of its distance
    /// range); `None` if it doesn't within the timeline.
    pub time: Option<f64>,
    pub earliest: Option<f64>,
    pub latest: Option<f64>,
    pub speed: Option<f64>,
    /// Per table row: distance still to go along the path (negative once past), and its range.
    pub to_go: Vec<(f64, f64, Option<[f64; 2]>)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointResult {
    pub name: String,
    pub position: P3,
    pub source: String,
    pub arrivals: Vec<Arrival>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TdsRun {
    pub method: String,
    pub scene_id: i64,
    pub scene_revision: i64,
    pub animation: Animation,
    pub request: TdsRequest,
    pub movers: Vec<MoverTable>,
    pub pairs: Vec<PairTable>,
    pub points: Vec<PointResult>,
    pub flags: Vec<Flag>,
    pub assumed: Vec<Assumed>,
    pub warnings: Vec<String>,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
    pub summary: String,
}

fn range(r: [f64; 2], v: f64) -> Option<[f64; 2]> {
    ((r[1] - r[0]).abs() > 1e-9 || (r[0] - v).abs() > 1e-9).then_some(r)
}

fn dist(a: P3, b: P3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The nearest point of a path to `q`: its distance along the path, and how far off it is.
fn nearest(p: &Prepared, q: P3) -> (f64, f64) {
    let path = p.path();
    let len = path.length();
    let n = ((len / 0.01).ceil() as usize).clamp(100, 100_000);
    let d = |s: f64| dist(path.point_at(s).0, q);
    let (mut best, mut bd) = (0.0, f64::INFINITY);
    for k in 0..=n {
        let s = len * k as f64 / n as f64;
        let x = d(s);
        if x < bd {
            (best, bd) = (s, x);
        }
    }
    // Golden-section refinement within a grid cell either side.
    let h = len / n as f64;
    let (mut a, mut b) = ((best - h).max(0.0), (best + h).min(len));
    let g = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..60 {
        let (c, e) = (b - g * (b - a), a + g * (b - a));
        if d(c) < d(e) {
            b = e;
        } else {
            a = c;
        }
    }
    let s = (a + b) / 2.0;
    (s, d(s))
}

/// First time on the fine grid that `reached(t)` holds, interpolated within its step.
fn first(times: &[f64], values: &[f64], target: f64) -> Option<f64> {
    if values.first().is_some_and(|v| *v >= target) {
        return Some(times[0]);
    }
    (1..values.len()).find(|&k| values[k] >= target).map(|k| {
        let (v0, v1) = (values[k - 1], values[k]);
        let f = if v1 > v0 {
            (target - v0) / (v1 - v0)
        } else {
            1.0
        };
        times[k - 1] + f * (times[k] - times[k - 1])
    })
}

pub fn run(
    scene_id: i64,
    scene_revision: i64,
    a: &Animation,
    req: &TdsRequest,
) -> Result<TdsRun, AnimError> {
    if !(req.step >= FINE && req.step.is_finite()) {
        return Err(AnimError(format!(
            "the table interval must be at least {FINE} s"
        )));
    }
    if a.movers.is_empty() {
        return Err(AnimError("the animation has no movers".into()));
    }
    let ev = evaluate(a, FINE)?;
    let prepared: Vec<Prepared> = a
        .movers
        .iter()
        .map(Prepared::new)
        .collect::<Result<_, _>>()?;
    let n = ((a.to - a.from) / FINE).round() as usize;
    let fine: Vec<f64> = (0..=n)
        .map(|k| (a.from + k as f64 * FINE).min(a.to))
        .collect();
    // Report times: every `step` from time zero's grid, inside the timeline, and its ends.
    let mut times: Vec<f64> = vec![a.from];
    let mut t = (a.from / req.step).ceil() * req.step;
    while t < a.to - 1e-9 {
        if t > a.from + 1e-9 {
            times.push(t);
        }
        t += req.step;
    }
    times.push(a.to);
    let index = |t: f64| (((t - a.from) / FINE).round() as usize).min(n);

    let mut movers = vec![];
    for (k, (m, p)) in a.movers.iter().zip(&prepared).enumerate() {
        let samples = &ev.samples[k].1;
        let rows = times
            .iter()
            .map(|&t| {
                let s = &samples[index(t)];
                let (d, v) = p.bounds(t);
                Row {
                    t,
                    distance: s.distance,
                    distance_range: range(d, s.distance),
                    speed: s.speed,
                    speed_range: range(v, s.speed),
                    acceleration: s.longitudinal,
                    heading_deg: s.heading.to_degrees(),
                    segment: s.segment.map(|k| k + 1),
                    assumed: s.assumed,
                }
            })
            .collect();
        movers.push(MoverTable {
            id: m.id.clone(),
            name: m.name.clone(),
            reference: match m.kind {
                MoverKind::Vehicle { wheelbase } => format!(
                    "the rear axle's centre (heading along the chord to the front axle, wheelbase {wheelbase:.2} m)"
                ),
                MoverKind::Person => "the person's position on the ground".into(),
                MoverKind::Other => "its reference point".into(),
            },
            path_length: p.path().length(),
            path_source: m.path_source.describe(),
            segments: m
                .segments
                .iter()
                .enumerate()
                .map(|(i, g)| format!("{}: {}; {}", i + 1, describe_motion(&g.motion, g.duration), g.source.describe()))
                .collect(),
            friction: m.friction.as_ref().map(|f| {
                format!("μ {:.2} ± {:.2}, {}", f.mu, f.tolerance, f.source.describe())
            }),
            rows,
        });
    }

    let find = |id: &String| {
        a.movers
            .iter()
            .position(|m| &m.id == id)
            .ok_or_else(|| AnimError(format!("no mover {id} in the animation")))
    };
    let mut pairs = vec![];
    for [ia, ib] in &req.pairs {
        let (ka, kb) = (find(ia)?, find(ib)?);
        if ka == kb {
            return Err(AnimError("a pair needs two different movers".into()));
        }
        let (sa, sb) = (&ev.samples[ka].1, &ev.samples[kb].1);
        let gap: Vec<f64> = (0..=n)
            .map(|i| dist(sa[i].position, sb[i].position))
            .collect();
        let least = gap
            .iter()
            .enumerate()
            .fold((f64::INFINITY, a.from), |acc, (i, &g)| {
                if g < acc.0 {
                    (g, fine[i])
                } else {
                    acc
                }
            });
        let rows = times
            .iter()
            .map(|&t| {
                let i = index(t);
                // The distance at the ends of each mover's distance range.
                let (da, _) = prepared[ka].bounds(t);
                let (db, _) = prepared[kb].bounds(t);
                let ends = |p: &Prepared, r: [f64; 2]| r.map(|s| p.path().point_at(s).0);
                let (pa, pb) = (ends(&prepared[ka], da), ends(&prepared[kb], db));
                let all = [
                    dist(pa[0], pb[0]),
                    dist(pa[0], pb[1]),
                    dist(pa[1], pb[0]),
                    dist(pa[1], pb[1]),
                ];
                let lo = all.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = all.iter().cloned().fold(0.0, f64::max);
                let (i0, i1) = (i.saturating_sub(1), (i + 1).min(n));
                PairRow {
                    t,
                    distance: gap[i],
                    range: range([lo.min(gap[i]), hi.max(gap[i])], gap[i]),
                    closing: req
                        .closing
                        .then(|| -(gap[i1] - gap[i0]) / (fine[i1] - fine[i0]).max(1e-9)),
                }
            })
            .collect();
        pairs.push(PairTable {
            a: a.movers[ka].name.clone(),
            b: a.movers[kb].name.clone(),
            rows,
            least,
        });
    }

    let mut points = vec![];
    for q in &req.points {
        let mut arrivals = vec![];
        for (k, p) in prepared.iter().enumerate() {
            let (at, off) = nearest(p, q.position);
            let samples = &ev.samples[k].1;
            let nominal: Vec<f64> = samples.iter().map(|s| s.distance).collect();
            let b: Vec<[f64; 2]> = fine.iter().map(|&t| p.bounds(t).0).collect();
            let lo: Vec<f64> = b.iter().map(|r| r[0]).collect();
            let hi: Vec<f64> = b.iter().map(|r| r[1]).collect();
            let target = at - 1e-6;
            let time = first(&fine, &nominal, target);
            arrivals.push(Arrival {
                mover: a.movers[k].name.clone(),
                at,
                off_path: off,
                time,
                earliest: first(&fine, &hi, target),
                latest: first(&fine, &lo, target),
                speed: time.map(|t| samples[index(t)].speed),
                to_go: times
                    .iter()
                    .map(|&t| {
                        let i = index(t);
                        let d = at - nominal[i];
                        (t, d, range([at - hi[i], at - lo[i]], d))
                    })
                    .collect(),
            });
        }
        points.push(PointResult {
            name: q.name.clone(),
            position: q.position,
            source: q.source.describe(),
            arrivals,
        });
    }

    let ranged = movers.iter().any(|m| {
        m.rows
            .iter()
            .any(|r| r.distance_range.is_some() || r.speed_range.is_some())
    });
    let mut assumptions = vec![
        format!("Times are seconds from time zero: {} ({}).", a.time_zero.event, a.time_zero.basis),
        "Distances along a path are measured along the drawn path from its first point; distances between movers are straight lines between their reference points (a vehicle's rear axle centre, a person's position), not the gap between their bodies.".into(),
    ];
    if ranged {
        assumptions.push("Ranges are the range method's: the extremes of each segment's stated range (an EDR record's distance range and speed tolerance, an analysis's speed range), with a distance range carried on from one segment into the next. Segments without a stated range are exact as stated.".into());
    }
    let mut limitations = ev.limitations.clone();
    limitations.push("Values are read from the motion at each tabulated time; the checks run at 100 Hz between them.".into());
    let summary = format!(
        "{} mover{}, {:.2} to {:.2} s from {}, every {:.2} s; {} flag{}, {} assumed item{}.",
        movers.len(),
        if movers.len() == 1 { "" } else { "s" },
        a.from,
        a.to,
        a.time_zero.event,
        req.step,
        ev.flags.len(),
        if ev.flags.len() == 1 { "" } else { "s" },
        ev.assumed.len(),
        if ev.assumed.len() == 1 { "" } else { "s" },
    );
    Ok(TdsRun {
        method: TDS_METHOD.into(),
        scene_id,
        scene_revision,
        animation: a.clone(),
        request: req.clone(),
        movers,
        pairs,
        points,
        flags: ev.flags,
        assumed: ev.assumed,
        warnings: ev.warnings,
        assumptions,
        limitations,
        summary,
    })
}

fn describe_motion(m: &crate::animation::SegmentMotion, duration: Option<f64>) -> String {
    use crate::animation::SegmentMotion as M;
    let dur = duration.map_or("to the end".to_string(), |d| format!("for {d:.2} s"));
    match m {
        M::Speed { speed, range } => format!(
            "{:.1} km/h{} {dur}",
            speed * 3.6,
            range.map_or(String::new(), |r| format!(
                " ({:.1}–{:.1})",
                r[0] * 3.6,
                r[1] * 3.6
            ))
        ),
        M::Accelerate {
            acceleration,
            start_speed,
            range,
        } => format!(
            "{:+.2} m/s² from {}{} {dur}",
            acceleration,
            start_speed.map_or("the previous speed".to_string(), |v| format!(
                "{:.1} km/h",
                v * 3.6
            )),
            range.map_or(String::new(), |r| format!(
                " ({:.1}–{:.1})",
                r[0] * 3.6,
                r[1] * 3.6
            ))
        ),
        M::Table {
            times, distances, ..
        } => format!(
            "{} time–distance rows over {:.2} s, {:.2} m",
            times.len(),
            times[times.len() - 1] - times[0],
            distances[distances.len() - 1] - distances[0]
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Lighting, Mover, Segment, SegmentMotion, TimeZero};
    use crate::motion::Shape;

    fn mover(id: &str, path: Vec<P3>, speed: f64, range: Option<[f64; 2]>) -> Mover {
        Mover {
            id: id.into(),
            name: id.into(),
            object: None,
            kind: MoverKind::Person,
            path,
            shape: Shape::Straight,
            path_source: Source::Assumption { note: "p".into() },
            start: -2.0,
            offset: 0.0,
            segments: vec![Segment {
                duration: None,
                motion: SegmentMotion::Speed { speed, range },
                source: Source::Analysis {
                    analysis_id: 1,
                    tool: "skid".into(),
                    name: "s".into(),
                },
            }],
            friction: None,
        }
    }

    #[test]
    fn tables_ranges_pairs_and_arrival_are_right() {
        // A along +x at 10 m/s (range 9–11) from t = -2; B along +y at 5 m/s, exact, from
        // (20, -10). A reaches the crossing (20, 0) at t = 0 (between -0.182 and 0.222 s).
        let a = Animation {
            time_zero: TimeZero {
                event: "impact".into(),
                basis: "test".into(),
            },
            from: -2.0,
            to: 1.0,
            lighting: Lighting::Daylight,
            movers: vec![
                mover(
                    "A",
                    vec![[0.0, 0.0, 0.0], [60.0, 0.0, 0.0]],
                    10.0,
                    Some([9.0, 11.0]),
                ),
                mover("B", vec![[20.0, -10.0, 0.0], [20.0, 20.0, 0.0]], 5.0, None),
            ],
            views: vec![],
        };
        let req = TdsRequest {
            step: 0.5,
            pairs: vec![["A".into(), "B".into()]],
            closing: true,
            points: vec![NamedPoint {
                name: "Conflict point".into(),
                position: [20.0, 0.3, 0.0],
                source: Source::Assumption { note: "x".into() },
            }],
        };
        let r = run(1, 2, &a, &req).unwrap();
        let close = |x: f64, y: f64, tol: f64| (x - y).abs() < tol;
        // Rows at -2, -1.5, …, 1.
        let rows = &r.movers[0].rows;
        assert_eq!(rows.len(), 7);
        let at0 = &rows[4];
        assert!(
            close(at0.t, 0.0, 1e-9) && close(at0.distance, 20.0, 1e-6),
            "{at0:?}"
        );
        let d = at0.distance_range.unwrap();
        assert!(close(d[0], 18.0, 1e-6) && close(d[1], 22.0, 1e-6), "{d:?}");
        let v = at0.speed_range.unwrap();
        assert!(close(v[0], 9.0, 1e-9) && close(v[1], 11.0, 1e-9));
        assert!(r.movers[1].rows[4].distance_range.is_none());
        // They meet at (20, 0) at t = 0. At t = -1, A is at (10, 0) and B at (20, -5).
        let p = &r.pairs[0];
        let g = &p.rows[2];
        assert!(close(g.distance, (100.0f64 + 25.0).sqrt(), 1e-6), "{g:?}");
        // d/dt of |(10 + 10τ - 20, -5 - 5τ)|: closing = (10·10 + 5·5)/√125 at τ = 0.
        assert!(
            close(g.closing.unwrap(), 125.0 / 125f64.sqrt(), 1e-3),
            "{g:?}"
        );
        assert!(
            close(p.least.0, 0.0, 0.2) && close(p.least.1, 0.0, 0.02),
            "{:?}",
            p.least
        );
        // The conflict point is 0.3 m off A's path, at 20 m along it.
        let arr = &r.points[0].arrivals[0];
        assert!(
            close(arr.at, 20.0, 1e-6) && close(arr.off_path, 0.3, 1e-6),
            "{arr:?}"
        );
        assert!(close(arr.time.unwrap(), 0.0, 1e-6));
        assert!(close(arr.earliest.unwrap(), -2.0 + 20.0 / 11.0, 1e-6));
        assert!(close(arr.latest.unwrap(), -2.0 + 20.0 / 9.0, 1e-6));
        assert!(close(arr.speed.unwrap(), 10.0, 1e-9));
        let (t, go, rg) = arr.to_go[2];
        assert!(close(t, -1.0, 1e-9) && close(go, 10.0, 1e-6));
        let rg = rg.unwrap();
        assert!(
            close(rg[0], 9.0, 1e-6) && close(rg[1], 11.0, 1e-6),
            "{rg:?}"
        );
        // B's arrival is exact.
        let b = &r.points[0].arrivals[1];
        assert!(b.earliest == b.time && b.latest == b.time);
        assert!(r.limitations.iter().any(|l| l.contains("100 Hz")));
    }
}
