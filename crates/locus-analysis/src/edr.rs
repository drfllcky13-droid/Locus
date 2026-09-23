//! Event data recorder (EDR) pre-crash data: imported from CSV or entered row by row, never
//! from a proprietary format. From the recorded speeds, the distance travelled from each
//! sample to the end of the record (the time the scene path ends at), with its uncertainty;
//! and, along a path picked in the scene, where the vehicle was at each sample: a track for
//! the animation timeline. See docs/methods/crash-edr.md.

use crate::crash::{spread, CrashError, Input, Spread};
use crate::measure::P3;
use crate::trajectory::PointSource;
use serde::{Deserialize, Serialize};

pub const EDR_METHOD: &str = "edr/1";

/// The recording accuracy of indicated vehicle speed that 49 CFR 563 requires, ±1 km/h (m/s):
/// the default speed tolerance, and the least the tool accepts.
pub const RECORDING_ACCURACY: f64 = 1.0 / 3.6;

fn err<T>(m: impl Into<String>) -> Result<T, CrashError> {
    Err(CrashError(m.into()))
}

/// One pre-crash sample, in SI units (time in s, negative before the trigger; speed in m/s).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub t: f64,
    pub speed: f64,
    /// Accelerator pedal (% of full travel).
    #[serde(default)]
    pub accelerator: Option<f64>,
    /// Service brake on or off.
    #[serde(default)]
    pub brake: Option<bool>,
    /// Steering wheel angle (°).
    #[serde(default)]
    pub steering: Option<f64>,
    /// Yaw rate (°/s).
    #[serde(default)]
    pub yaw_rate: Option<f64>,
    /// Longitudinal acceleration (m/s²).
    #[serde(default)]
    pub long_accel: Option<f64>,
}

/// What a CSV column was read as.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub header: String,
    /// "time", "speed", "accelerator", "brake", "steering", "yaw_rate", "long_accel", or
    /// "unused".
    pub field: String,
    /// The unit it was read in ("s", "ms", "km/h", "mph", "m/s", "%", "°", "°/s", "g", "m/s²").
    pub unit: String,
}

/// Speeds in the CSV with no unit in their header are in this unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedUnit {
    Kmh,
    Mph,
    Ms,
}

impl SpeedUnit {
    fn to_ms(self) -> f64 {
        match self {
            SpeedUnit::Kmh => 1.0 / 3.6,
            SpeedUnit::Mph => 0.44704,
            SpeedUnit::Ms => 1.0,
        }
    }
    fn name(self) -> &'static str {
        match self {
            SpeedUnit::Kmh => "km/h",
            SpeedUnit::Mph => "mph",
            SpeedUnit::Ms => "m/s",
        }
    }
}

/// The unit in a header's brackets, lower-cased ("speed (km/h)" → "km/h").
fn bracket_unit(h: &str) -> Option<String> {
    let a = h.find(['(', '['])?;
    let b = h[a..].find([')', ']'])? + a;
    Some(h[a + 1..b].trim().to_lowercase())
}

/// Read pre-crash data from CSV text: a header row, then one row per sample. Columns are
/// recognised by their headers (time, speed, accelerator / throttle, brake, steering, yaw
/// rate, longitudinal acceleration), with units from brackets in the header where given
/// ("Speed (mph)", "Time (ms)", "Long. accel (g)"); speeds without one are in `speed_unit`.
/// Commas, semicolons or tabs; lines starting with # are comments.
pub fn parse_csv(
    text: &str,
    speed_unit: SpeedUnit,
) -> Result<(Vec<Sample>, Vec<Column>), CrashError> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let Some(header) = lines.next() else {
        return err("the CSV is empty");
    };
    let delim = [',', ';', '\t']
        .into_iter()
        .max_by_key(|d| header.matches(*d).count())
        .unwrap();
    let split = |l: &str| -> Vec<String> {
        l.split(delim)
            .map(|c| c.trim().trim_matches('"').trim().to_string())
            .collect()
    };
    let mut columns = vec![];
    for h in split(header) {
        let l = h.to_lowercase();
        let unit = bracket_unit(&h);
        let (field, unit) = if l.contains("time") {
            let u = unit.unwrap_or_else(|| "s".into());
            if u != "s" && u != "sec" && u != "ms" {
                return err(format!("{h}: time must be in s or ms"));
            }
            ("time", if u == "ms" { "ms".into() } else { "s".into() })
        } else if l.contains("speed") && !l.contains("engine") && !l.contains("rpm") {
            let u = match unit.as_deref() {
                None => speed_unit.name().to_string(),
                Some("km/h" | "kph" | "kmh" | "km/hr") => "km/h".into(),
                Some("mph" | "mi/h") => "mph".into(),
                Some("m/s") => "m/s".into(),
                Some(u) => return err(format!("{h}: unknown speed unit {u}")),
            };
            ("speed", u)
        } else if l.contains("throttle") || l.contains("pedal") || l.contains("accelerator") {
            ("accelerator", "%".into())
        } else if l.contains("brake") {
            ("brake", String::new())
        } else if l.contains("steer") {
            ("steering", "°".into())
        } else if l.contains("yaw") {
            ("yaw_rate", "°/s".into())
        } else if l.contains("long") && l.contains("acc") {
            let u = match unit.as_deref() {
                Some("g") => "g".into(),
                Some("m/s²" | "m/s^2" | "m/s2") | None => "m/s²".into(),
                Some(u) => return err(format!("{h}: unknown acceleration unit {u}")),
            };
            ("long_accel", u)
        } else {
            ("unused", String::new())
        };
        if field != "unused" && columns.iter().any(|c: &Column| c.field == field) {
            return err(format!("two columns read as {field}; rename one"));
        }
        columns.push(Column {
            header: h,
            field: field.into(),
            unit,
        });
    }
    let find = |f: &str| columns.iter().position(|c| c.field == f);
    let (Some(ti), Some(si)) = (find("time"), find("speed")) else {
        return err("the CSV needs a time column and a speed column");
    };
    let mut samples = vec![];
    for (n, line) in lines.enumerate() {
        let row = split(line);
        let cell = |k: Option<usize>| k.and_then(|k| row.get(k)).map(String::as_str).unwrap_or("");
        let num = |k: Option<usize>, name: &str| -> Result<Option<f64>, CrashError> {
            let c = cell(k);
            if c.is_empty() {
                return Ok(None);
            }
            c.parse::<f64>()
                .map(Some)
                .map_err(|_| CrashError(format!("row {}: {name} \"{c}\" isn't a number", n + 1)))
        };
        let t =
            num(Some(ti), "time")?.ok_or_else(|| CrashError(format!("row {}: no time", n + 1)))?;
        let v = num(Some(si), "speed")?
            .ok_or_else(|| CrashError(format!("row {}: no speed", n + 1)))?;
        let su = match columns[si].unit.as_str() {
            "km/h" => SpeedUnit::Kmh,
            "mph" => SpeedUnit::Mph,
            _ => SpeedUnit::Ms,
        };
        let brake = match cell(find("brake")).to_lowercase().as_str() {
            "" => None,
            "on" | "yes" | "1" | "true" | "applied" => Some(true),
            "off" | "no" | "0" | "false" | "not applied" => Some(false),
            b => return err(format!("row {}: brake \"{b}\" should be on or off", n + 1)),
        };
        let g = if find("long_accel").is_some_and(|k| columns[k].unit == "g") {
            crate::crash::G
        } else {
            1.0
        };
        samples.push(Sample {
            t: if columns[ti].unit == "ms" {
                t / 1000.0
            } else {
                t
            },
            speed: v * su.to_ms(),
            accelerator: num(find("accelerator"), "accelerator")?,
            brake,
            steering: num(find("steering"), "steering")?,
            yaw_rate: num(find("yaw_rate"), "yaw rate")?,
            long_accel: num(find("long_accel"), "acceleration")?.map(|a| a * g),
        });
    }
    Ok((samples, columns))
}

/// A sample's distance to the end of the record, and where that puts the vehicle on the path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Station {
    pub t: f64,
    /// Distance travelled from this sample to the end time (m).
    pub distance: Spread,
    /// On the path (nominal distance), with the direction of travel; `None` without a path.
    pub position: Option<P3>,
    pub heading: Option<P3>,
    /// Where the distance's range method puts the vehicle: its low and high ends on the path.
    pub span: Option<[P3; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdrRun {
    pub method: String,
    pub label: String,
    /// Where the data came from (the retrieval report, the vehicle, who retrieved it).
    pub source: String,
    /// The data as imported or entered (canonical CSV), and its SHA-256.
    pub csv: String,
    #[serde(default)]
    pub csv_sha256: String,
    pub columns: Vec<Column>,
    pub samples: Vec<Sample>,
    /// The speed's accuracy: a scale (fraction, e.g. 0.01) and an offset (m/s), both as
    /// ranges about 0 (systematic: every sample off the same way).
    pub scale_tolerance: f64,
    pub offset_tolerance: f64,
    /// Why the tolerance is wider than the recording accuracy (required when it is).
    #[serde(default)]
    pub tolerance_reason: String,
    /// The time the path's end stands for (s), at or after the last sample.
    pub end_time: f64,
    /// The path picked in the scene (its end is the vehicle's position at `end_time`).
    pub path: Vec<P3>,
    #[serde(default)]
    pub path_sources: Vec<PointSource>,
    pub stations: Vec<Station>,
    pub summary: String,
    pub warnings: Vec<String>,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub const EDR_ASSUMPTIONS: &[&str] = &[
    "The recorded speeds are the vehicle's speed over the ground at each sample time. EDRs usually derive speed from wheel or transmission speed, so wheel slip (hard braking, spinning wheels), tyre size and axle ratio changes make them differ.",
    "The speed error is systematic within the stated tolerance: every sample off by the same scale and offset.",
    "Between samples the speed lies between its values at the two ends: the true distance is between the left and right Riemann sums (exact when the speed changes monotonically between samples).",
    "The path's end is where the vehicle's reference point was at the end time; the vehicle followed the path.",
];

pub const EDR_LIMITATIONS: &[&str] = &[
    "The ±1 km/h default is the recording accuracy of the indicated speed signal (49 CFR 563), not of the vehicle's true speed over the ground. Wheel slip under braking, ABS cycling (the wheels slowing below the vehicle's speed and recovering), wheelspin, and non-original tyre sizes or axle ratios can make the two differ by more; where they may apply, the tolerance should be widened (a reason is required and printed).",
    "Sample times carry their own uncertainty: a recorder may not sample exactly at the stated times, and different signals may be sampled at different moments within an interval. The distance range does not include it.",
    "EDR sample times are relative to the recorder's trigger (algorithm enable or deployment), not to impact; the offset between the two is a matter for the retrieval report.",
    "The data is as imported or entered by the examiner; this tool doesn't read or verify the recorder's own files.",
    "Positions along the path use the nominal distances; the path's own measurement uncertainty is not added.",
];

/// A point `d` back from the end of `path`, and the direction of travel there. Beyond the
/// path's start it continues straight along the first segment.
fn along(path: &[P3], d: f64) -> (P3, P3) {
    let mut left = d;
    for w in path.windows(2).rev() {
        let (a, b) = (w[0], w[1]);
        let seg = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (seg[0] * seg[0] + seg[1] * seg[1] + seg[2] * seg[2]).sqrt();
        if len == 0.0 {
            continue;
        }
        let dir = seg.map(|x| x / len);
        if left <= len || std::ptr::eq(w.as_ptr(), path.as_ptr()) {
            return (
                [
                    b[0] - dir[0] * left,
                    b[1] - dir[1] * left,
                    b[2] - dir[2] * left,
                ],
                dir,
            );
        }
        left -= len;
    }
    (path[path.len() - 1], [0.0; 3])
}

fn polyline_length(path: &[P3]) -> f64 {
    path.windows(2)
        .map(|w| {
            (0..3)
                .map(|k| (w[1][k] - w[0][k]).powi(2))
                .sum::<f64>()
                .sqrt()
        })
        .sum()
}

/// Distances from each sample to `end_time`, and positions along `path` when given (at least
/// 2 points, its end the vehicle's position at `end_time`). The speed tolerance is a scale
/// (fraction) and an offset (m/s). A gap between the last sample and `end_time` is covered at
/// the last speed, less up to 1 g of braking.
#[allow(clippy::too_many_arguments)]
pub fn edr(
    label: &str,
    source: &str,
    csv: &str,
    columns: Vec<Column>,
    samples: Vec<Sample>,
    scale_tolerance: f64,
    offset_tolerance: f64,
    tolerance_reason: &str,
    end_time: Option<f64>,
    path: Vec<P3>,
    path_sources: Vec<PointSource>,
    draws: usize,
    seed: u64,
) -> Result<EdrRun, CrashError> {
    if source.trim().is_empty() {
        return err(
            "give the data's source (the retrieval report, the vehicle and who retrieved it)",
        );
    }
    if samples.len() < 2 {
        return err("give at least 2 samples");
    }
    for (k, w) in samples.windows(2).enumerate() {
        if !w[0].t.is_finite() || !w[1].t.is_finite() || w[1].t <= w[0].t {
            return err(format!(
                "sample {}: times must increase ({} s after {} s)",
                k + 2,
                w[1].t,
                w[0].t
            ));
        }
    }
    if let Some(s) = samples
        .iter()
        .find(|s| !s.speed.is_finite() || s.speed < 0.0)
    {
        return err(format!("at {} s: the speed must be 0 or more", s.t));
    }
    if !(0.0..=0.2).contains(&scale_tolerance) || !(0.0..=5.0).contains(&offset_tolerance) {
        return err("the speed tolerance must be 0–20 % and 0–5 m/s");
    }
    if offset_tolerance < RECORDING_ACCURACY - 1e-9 {
        return err("the speed tolerance can't be tighter than the recording accuracy, ±1 km/h");
    }
    let widened = scale_tolerance > 0.0 || offset_tolerance > RECORDING_ACCURACY + 1e-9;
    if widened && tolerance_reason.trim().is_empty() {
        return err("give the reason for widening the speed tolerance beyond ±1 km/h (for example wheel slip under braking, ABS, non-original tyres)");
    }
    if path.len() == 1 {
        return err(
            "pick at least 2 points along the path, ending where the vehicle was at the end time",
        );
    }
    let last = samples[samples.len() - 1].t;
    let end = end_time.unwrap_or(last);
    if end < last {
        return err(format!(
            "the end time must be at or after the last sample ({last} s)"
        ));
    }
    let gap = end - last;
    let v_last = samples[samples.len() - 1].speed;
    let mut warnings = vec![];
    // Inputs: speed scale k, offset b (m/s), rule weight w (left 0 … right 1), braking in the
    // gap a (m/s²).
    let inputs = [
        Input::range(1.0, 1.0 - scale_tolerance, 1.0 + scale_tolerance),
        Input::range(0.0, -offset_tolerance, offset_tolerance),
        Input::range(0.5, 0.0, 1.0),
        if gap > 0.0 {
            Input::range(0.0, 0.0, crate::crash::G)
        } else {
            Input::exact(0.0)
        },
    ];
    let n = samples.len();
    let mut stations = vec![];
    let length = polyline_length(&path);
    for i in 0..n {
        // Left and right sums from sample i to the last sample.
        let (mut left, mut right) = (0.0, 0.0);
        for j in i..n - 1 {
            let dt = samples[j + 1].t - samples[j].t;
            left += samples[j].speed * dt;
            right += samples[j + 1].speed * dt;
        }
        let span = last - samples[i].t;
        let f = |x: &[f64]| -> Option<f64> {
            let (k, b, w, a) = (x[0], x[1], x[2], x[3]);
            let core = k * ((1.0 - w) * left + w * right) + b * span;
            // The gap at the last speed, braking at up to a but not past a stop.
            let v = (k * v_last + b).max(0.0);
            let g = if a > 0.0 && v / a < gap {
                v * v / (2.0 * a)
            } else {
                v * gap - 0.5 * a * gap * gap
            };
            Some((core + g).max(0.0))
        };
        let distance = spread(&inputs, &f, draws, seed.wrapping_add(i as u64))?;
        let (position, heading, span_pts) = if path.len() >= 2 {
            let (p, h) = along(&path, distance.value);
            let (lo, _) = along(&path, distance.low);
            let (hi, _) = along(&path, distance.high);
            (Some(p), Some(h), Some([lo, hi]))
        } else {
            (None, None, None)
        };
        stations.push(Station {
            t: samples[i].t,
            distance,
            position,
            heading,
            span: span_pts,
        });
    }
    let total = stations[0].distance;
    if path.len() >= 2 && total.value > length {
        warnings.push(format!(
            "The record covers {:.1} m but the path is {:.1} m long; before its start the vehicle is placed straight back along the path's first segment.",
            total.value, length
        ));
    }
    if gap > 0.0 {
        warnings.push(format!(
            "The last sample is {gap:.2} s before the end time; that gap is covered at the last recorded speed, and its range allows anything from no braking to 1 g."
        ));
    }
    let spacing = samples
        .windows(2)
        .map(|w| w[1].t - w[0].t)
        .fold(0.0, f64::max);
    if spacing > 1.0 {
        warnings.push(format!(
            "Samples are up to {spacing:.1} s apart; the distance between them depends on how the speed changed, which the left and right sums bound only if it changed steadily."
        ));
    }
    let summary = format!(
        "{label}: {:.1} m in the {:.1} s to {end:.1} s ({:.1}–{:.1} m by the range method); speed {:.1}–{:.1} km/h",
        total.value,
        samples[0].t,
        total.low,
        total.high,
        samples.iter().map(|s| s.speed).fold(f64::INFINITY, f64::min) * 3.6,
        samples.iter().map(|s| s.speed).fold(0.0, f64::max) * 3.6,
    );
    Ok(EdrRun {
        method: EDR_METHOD.into(),
        label: label.into(),
        source: source.into(),
        csv: csv.into(),
        csv_sha256: String::new(),
        columns,
        samples,
        scale_tolerance,
        offset_tolerance,
        tolerance_reason: if widened {
            tolerance_reason.trim().to_string()
        } else {
            String::new()
        },
        end_time: end,
        path,
        path_sources,
        stations,
        summary,
        warnings,
        assumptions: EDR_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: EDR_LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "# retrieved 2026-09-01\n\
        Time (sec), Speed, Accelerator Pedal (%), Service Brake, Steering Wheel Angle (deg), Engine RPM\n\
        -2.0, 72, 20, Off, 0, 1800\n\
        -1.5, 72, 0, Off, -5, 1700\n\
        -1.0, 64.8, 0, On, -5, 1500\n\
        -0.5, 57.6, 0, On, -10, 1300\n\
        0.0, 50.4, 0, On, -10, 1200\n";

    #[test]
    fn a_csv_is_read_by_its_headers() {
        let (s, cols) = parse_csv(CSV, SpeedUnit::Kmh).unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].t, -2.0);
        assert!((s[0].speed - 20.0).abs() < 1e-12);
        assert_eq!(s[2].brake, Some(true));
        assert_eq!(s[3].steering, Some(-10.0));
        assert_eq!(s[0].accelerator, Some(20.0));
        assert_eq!(cols[5].field, "unused");
        assert_eq!(cols[1].unit, "km/h");
        // Units from headers; semicolons; milliseconds; g.
        let (s, _) = parse_csv(
            "time (ms);speed (mph);long. accel (g)\n0;10;-0.5\n500;5;-0.5\n",
            SpeedUnit::Kmh,
        )
        .unwrap();
        assert_eq!(s[1].t, 0.5);
        assert!((s[0].speed - 4.4704).abs() < 1e-12);
        assert!((s[0].long_accel.unwrap() + 0.5 * crate::crash::G).abs() < 1e-12);
        assert!(parse_csv("time,rpm\n0,1\n", SpeedUnit::Kmh).is_err());
        assert!(parse_csv("time,speed\n0,fast\n", SpeedUnit::Kmh).is_err());
    }

    /// Hand-worked: 20, 20, 18, 16, 14 m/s at 0.5 s steps. Left sum 0.5 (20 + 20 + 18 + 16) =
    /// 37 m, right 0.5 (20 + 18 + 16 + 14) = 34 m, trapezoid 35.5 m.
    #[test]
    fn distances_are_bounded_by_the_left_and_right_sums() {
        let ra = RECORDING_ACCURACY;
        let run = |scale: f64, offset: f64, why: &str| {
            let (s, cols) = parse_csv(CSV, SpeedUnit::Kmh).unwrap();
            edr(
                "Car",
                "CDR report",
                CSV,
                cols,
                s,
                scale,
                offset,
                why,
                None,
                vec![],
                vec![],
                2000,
                1,
            )
        };
        // At the recording accuracy alone: 34 − 2/3.6 to 37 + 2/3.6 m.
        let r = run(0.0, ra, "").unwrap();
        let d = r.stations[0].distance;
        assert!((d.value - 35.5).abs() < 1e-9, "{}", d.value);
        assert!(
            (d.low - (34.0 - 2.0 * ra)).abs() < 1e-9 && (d.high - (37.0 + 2.0 * ra)).abs() < 1e-9,
            "{d:?}"
        );
        assert_eq!(r.stations[4].distance.value, 0.0);
        assert!(r.tolerance_reason.is_empty());
        // Widened by a 1 % scale: 34 × 0.99 − 2/3.6 to 37 × 1.01 + 2/3.6, with its reason.
        let why = "hard braking with ABS";
        let r = run(0.01, ra, why).unwrap();
        let d = r.stations[0].distance;
        assert!((d.low - (34.0 * 0.99 - 2.0 * ra)).abs() < 1e-9, "{d:?}");
        assert!((d.high - (37.0 * 1.01 + 2.0 * ra)).abs() < 1e-9, "{d:?}");
        assert_eq!(r.tolerance_reason, why);
        // Widening needs a reason; nothing tighter than ±1 km/h is taken.
        assert!(run(0.01, ra, " ").is_err());
        assert!(run(0.0, 2.0 * ra, "").is_err());
        assert!(run(0.0, 0.1, why).is_err());
    }

    #[test]
    fn the_vehicle_is_placed_back_along_the_path() {
        let (s, cols) = parse_csv(CSV, SpeedUnit::Kmh).unwrap();
        // An L-shaped path: 20 m east, then 30 m north to the end.
        let path = vec![[0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 30.0, 0.0]];
        let r = edr(
            "Car",
            "CDR",
            CSV,
            cols,
            s,
            0.0,
            RECORDING_ACCURACY,
            "",
            None,
            path,
            vec![],
            500,
            1,
        )
        .unwrap();
        // 35.5 m back from the end: 30 m down the north leg, then 5.5 m west.
        let p = r.stations[0].position.unwrap();
        assert!((p[0] - 14.5).abs() < 1e-9 && p[1].abs() < 1e-9, "{p:?}");
        assert_eq!(r.stations[0].heading.unwrap(), [1.0, 0.0, 0.0]);
        let e = r.stations[4].position.unwrap();
        assert_eq!(e, [20.0, 30.0, 0.0]);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn a_gap_to_the_end_time_allows_braking() {
        let (s, cols) = parse_csv(CSV, SpeedUnit::Kmh).unwrap();
        let ra = RECORDING_ACCURACY;
        let r = edr(
            "Car",
            "CDR",
            CSV,
            cols,
            s,
            0.0,
            ra,
            "",
            Some(0.5),
            vec![],
            vec![],
            500,
            1,
        )
        .unwrap();
        // 0.5 s after the last sample (14 m/s): 7 m at that speed; at its lowest, 1 km/h slower
        // and braking at 1 g, (14 − 1/3.6) 0.5 − ½ g 0.25.
        let d = r.stations[4].distance;
        assert!((d.value - 7.0).abs() < 1e-9, "{d:?}");
        let low = (14.0 - ra) * 0.5 - 0.5 * crate::crash::G * 0.25;
        assert!((d.low - low).abs() < 1e-9, "{d:?}");
        assert_eq!(r.warnings.len(), 1);
    }
}
