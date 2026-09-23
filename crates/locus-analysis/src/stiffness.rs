//! CRASH3 stiffness coefficients (A, B) from NHTSA full-width frontal rigid-barrier tests, by
//! Campbell's method. Method and data source: docs/methods/crash-stiffness.md.
//!
//! Each test vehicle's crush profile (C1–C6, equally spaced across the damage width L), test
//! mass m and impact speed V give b1 from the energy balance under the CRASH3 model: with
//! A = m b0 b1 / L and B = m b1² / L (so G = A²/2B = m b0² / 2L), the crush energy over the
//! measured profile equals the kinetic energy ½ m V². That is
//! Q b1² + 2 b0 C̄ b1 + (b0² − V²) = 0, for C̄ the profile's mean and Q the mean of its
//! square (linear between the points); for uniform crush it is Campbell's b1 = (V − b0)/C̄.
//! b0, the speed below which no permanent crush occurs, is assumed (8 ± 3.2 km/h, 5 ± 2 mph).
//! Each test's coefficients carry a Monte Carlo uncertainty over b0 and the measurements; a
//! vehicle tested more than once takes the mean and adds the spread between its tests.

use crate::crash::crush_energy;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// The bundled data: NHTSA full-width frontal rigid-barrier tests, as published.
pub const DATA: &str = include_str!("../data/nhtsa-frontal-barrier.csv");

/// The no-damage speed b0 (m/s) and its range: 8 km/h (5 mph) ± 3.2 km/h (2 mph).
pub const B0: f64 = 8.0 / 3.6;
pub const B0_RANGE: f64 = 3.2 / 3.6;
/// Measurement uncertainties (1σ) drawn in the Monte Carlo: speed 0.5 km/h, each crush depth
/// 10 mm, width 1 %.
const SPEED_SIGMA: f64 = 0.5 / 3.6;
const CRUSH_SIGMA: f64 = 0.010;
const WIDTH_REL: f64 = 0.01;
const DRAWS: usize = 400;

/// One test vehicle as published.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Test {
    pub test_no: u32,
    pub test_date: String,
    pub test_type: String,
    pub make: String,
    pub model: String,
    pub model_year: u32,
    pub body_type: String,
    pub mass: f64,
    /// Impact speed (m/s).
    pub speed: f64,
    /// Damage width (m): the recorded indentation length, or the vehicle's width when that is
    /// not recorded (a full-width frontal test crushes the whole front).
    pub width: f64,
    pub width_from_vehicle: bool,
    /// Crush depths (m), equally spaced across the width: six, or two or four when the test
    /// recorded that many (the database fills the rest with zeros).
    pub crush: Vec<f64>,
}

/// The measured points of a published C1–C6 row: older tests recorded two (C1, C2) or four
/// (C1–C4) and store zeros for the rest.
pub fn measured_points(c: &[f64; 6]) -> Vec<f64> {
    let zero = |k: usize| c[k] <= 0.0;
    if (2..6).all(zero) {
        c[..2].to_vec()
    } else if zero(4) && zero(5) {
        c[..4].to_vec()
    } else {
        c.to_vec()
    }
}

/// Parse the bundled CSV: rows with a usable mass, speed, width and six crush depths (small
/// negative depths, to 10 mm, read as 0; rows with larger ones or missing values are left out).
pub fn parse(csv: &str) -> Vec<Test> {
    let mut lines = csv.lines().filter(|l| !l.starts_with('#'));
    let Some(head) = lines.next() else {
        return vec![];
    };
    let cols: Vec<&str> = head.split(',').collect();
    let at = |name: &str| cols.iter().position(|c| *c == name);
    let idx: Vec<Option<usize>> = [
        "test_no",
        "test_date",
        "test_type",
        "make",
        "model",
        "model_year",
        "body_type",
        "mass_kg",
        "speed_kmh",
        "vehicle_width_mm",
        "damage_width_mm",
        "c1_mm",
        "c2_mm",
        "c3_mm",
        "c4_mm",
        "c5_mm",
        "c6_mm",
    ]
    .iter()
    .map(|n| at(n))
    .collect();
    let mut out = vec![];
    for line in lines {
        let f = split_csv(line);
        let get = |k: usize| {
            idx[k]
                .and_then(|i| f.get(i))
                .map(|s| s.trim())
                .unwrap_or("")
        };
        let num = |k: usize| get(k).parse::<f64>().ok();
        let (Some(mass), Some(speed)) = (num(7), num(8)) else {
            continue;
        };
        let damage = num(10).filter(|w| *w > 0.0);
        let Some(width) = damage.or(num(9).filter(|w| *w > 0.0)) else {
            continue;
        };
        let mut crush = [0.0; 6];
        let mut ok = true;
        for (k, c) in crush.iter_mut().enumerate() {
            match num(11 + k) {
                Some(v) if v >= -10.0 => *c = v.max(0.0) / 1000.0,
                _ => ok = false,
            }
        }
        if !ok || !(mass > 0.0 && speed > 10.0) || crush.iter().all(|c| *c <= 0.0) {
            continue;
        }
        out.push(Test {
            test_no: get(0).parse().unwrap_or(0),
            test_date: get(1).into(),
            test_type: get(2).into(),
            make: get(3).into(),
            model: get(4).into(),
            model_year: get(5).parse().unwrap_or(0),
            body_type: get(6).into(),
            mass,
            speed: speed / 3.6,
            width: width / 1000.0,
            width_from_vehicle: damage.is_none(),
            crush: measured_points(&crush),
        });
    }
    out
}

/// Split one CSV line (fields may be double-quoted, with "" for a quote).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// The mean crush and the mean squared crush over a profile linear between equally spaced
/// points.
fn profile_means(c: &[f64]) -> (f64, f64) {
    let n = (c.len() - 1) as f64;
    let (mut m, mut q) = (0.0, 0.0);
    for w in c.windows(2) {
        m += (w[0] + w[1]) / 2.0;
        q += (w[0] * w[0] + w[0] * w[1] + w[1] * w[1]) / 3.0;
    }
    (m / n, q / n)
}

/// b1 for one test from the energy balance per unit mass (see the module note); None
/// without a real, positive root (speed below b0, or no crush).
pub fn b1(speed: f64, crush: &[f64], b0: f64) -> Option<f64> {
    let (m, q) = profile_means(crush);
    if q <= 0.0 || speed <= b0 {
        return None;
    }
    let disc = (b0 * m).powi(2) - q * (b0 * b0 - speed * speed);
    (disc >= 0.0).then(|| (-b0 * m + disc.sqrt()) / q)
}

/// A and B (N/m, N/m²) for one test with a given b0.
pub fn coefficients(t: &Test, b0: f64) -> Option<(f64, f64)> {
    let b1 = b1(t.speed, &t.crush, b0)?;
    Some((t.mass * b0 * b1 / t.width, t.mass * b1 * b1 / t.width))
}

/// One test's coefficients with their Monte Carlo spread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestCoefficients {
    pub test_no: u32,
    pub mass: f64,
    pub speed: f64,
    pub width: f64,
    pub width_from_vehicle: bool,
    pub crush: Vec<f64>,
    pub a: f64,
    pub a_sigma: f64,
    pub b: f64,
    pub b_sigma: f64,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.next(), self.next());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

pub fn test_coefficients(t: &Test) -> Option<TestCoefficients> {
    let (a, b) = coefficients(t, B0)?;
    let mut rng = Rng(u64::from(t.test_no).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let (mut sa, mut sb) = (vec![], vec![]);
    for _ in 0..DRAWS {
        let draw = Test {
            speed: t.speed + SPEED_SIGMA * rng.gauss(),
            width: t.width * (1.0 + WIDTH_REL * rng.gauss()),
            crush: t
                .crush
                .iter()
                .map(|c| (c + CRUSH_SIGMA * rng.gauss()).max(0.0))
                .collect(),
            ..t.clone()
        };
        let b0 = B0 + B0_RANGE * (2.0 * rng.next() - 1.0);
        if let Some((x, y)) = coefficients(&draw, b0) {
            sa.push(x);
            sb.push(y);
        }
    }
    let sd = |v: &[f64]| {
        let n = v.len().max(2) as f64;
        let m = v.iter().sum::<f64>() / v.len().max(1) as f64;
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    };
    Some(TestCoefficients {
        test_no: t.test_no,
        mass: t.mass,
        speed: t.speed,
        width: t.width,
        width_from_vehicle: t.width_from_vehicle,
        crush: t.crush.clone(),
        a,
        a_sigma: sd(&sa),
        b,
        b_sigma: sd(&sb),
    })
}

/// A vehicle (make, model, model year) with its tests' coefficients combined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub make: String,
    pub model: String,
    pub model_year: u32,
    pub body_type: String,
    pub tests: Vec<TestCoefficients>,
    /// Mean A and B over the tests; 1σ combines each test's own uncertainty with the spread
    /// between tests (√(mean σ² + sample variance)).
    pub a: f64,
    pub a_sigma: f64,
    pub b: f64,
    pub b_sigma: f64,
    /// Only one test: its coefficients can't show how much another test would differ.
    pub single_test: bool,
    /// Some test's damage width is the vehicle's width, not a recorded indentation length.
    pub width_from_vehicle: bool,
}

fn combine(v: &[(f64, f64)]) -> (f64, f64) {
    let n = v.len() as f64;
    let mean = v.iter().map(|x| x.0).sum::<f64>() / n;
    let within = v.iter().map(|x| x.1 * x.1).sum::<f64>() / n;
    let between = if v.len() > 1 {
        v.iter().map(|x| (x.0 - mean).powi(2)).sum::<f64>() / (n - 1.0)
    } else {
        0.0
    };
    (mean, (within + between).sqrt())
}

/// Group tests by make, model and model year.
pub fn build(tests: &[Test]) -> Vec<Entry> {
    let mut map: std::collections::BTreeMap<
        (String, String, u32),
        (String, Vec<TestCoefficients>),
    > = Default::default();
    for t in tests {
        if let Some(c) = test_coefficients(t) {
            map.entry((t.make.clone(), t.model.clone(), t.model_year))
                .or_insert_with(|| (t.body_type.clone(), vec![]))
                .1
                .push(c);
        }
    }
    map.into_iter()
        .map(|((make, model, model_year), (body_type, tests))| {
            let (a, a_sigma) = combine(&tests.iter().map(|t| (t.a, t.a_sigma)).collect::<Vec<_>>());
            let (b, b_sigma) = combine(&tests.iter().map(|t| (t.b, t.b_sigma)).collect::<Vec<_>>());
            Entry {
                single_test: tests.len() == 1,
                width_from_vehicle: tests.iter().any(|t| t.width_from_vehicle),
                make,
                model,
                model_year,
                body_type,
                tests,
                a,
                a_sigma,
                b,
                b_sigma,
            }
        })
        .collect()
}

/// The bundled table, built once.
pub fn table() -> &'static [Entry] {
    static T: OnceLock<Vec<Entry>> = OnceLock::new();
    T.get_or_init(|| build(&parse(DATA)))
}

/// Entries matching a make, model (substring, case-insensitive) and model-year range.
pub fn lookup(make: &str, model: &str, years: Option<(u32, u32)>) -> Vec<&'static Entry> {
    let (mk, md) = (make.trim().to_uppercase(), model.trim().to_uppercase());
    table()
        .iter()
        .filter(|e| mk.is_empty() || e.make.to_uppercase() == mk)
        .filter(|e| md.is_empty() || e.model.to_uppercase().contains(&md))
        .filter(|e| years.is_none_or(|(a, b)| (a..=b).contains(&e.model_year)))
        .collect()
}

/// The crush energy a test's own coefficients give over its own profile: equal to its
/// kinetic energy by construction (a check on the derivation).
pub fn energy_check(t: &Test) -> Option<(f64, f64)> {
    let (a, b) = coefficients(t, B0)?;
    Some((
        crush_energy(a, b, t.width, &t.crush, 0.0)?,
        0.5 * t.mass * t.speed * t.speed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(c: f64) -> Test {
        Test {
            test_no: 1,
            test_date: "2020-01-01".into(),
            test_type: "NCAP".into(),
            make: "X".into(),
            model: "Y".into(),
            model_year: 2020,
            body_type: "car".into(),
            mass: 1500.0,
            speed: 56.0 / 3.6,
            width: 1.5,
            width_from_vehicle: false,
            crush: vec![c; 6],
        }
    }

    // Hand-worked (Campbell, uniform crush): 1,500 kg at 56 km/h (15.5556 m/s) into the
    // barrier, 0.5 m of crush over 1.5 m, b0 = 8 km/h (2.2222 m/s):
    // b1 = (15.5556 − 2.2222)/0.5 = 26.6667 /s; A = 1,500 · 2.2222 · 26.6667 / 1.5 =
    // 59,259.3 N/m; B = 1,500 · 26.6667² / 1.5 = 711,111 N/m².
    #[test]
    fn a_uniform_profile_gives_campbells_coefficients() {
        let (a, b) = coefficients(&uniform(0.5), B0).unwrap();
        assert!((a - 59_259.259).abs() < 0.01, "{a}");
        assert!((b - 711_111.111).abs() < 0.01, "{b}");
    }

    #[test]
    fn every_test_reproduces_its_own_kinetic_energy() {
        let mut t = uniform(0.0);
        t.crush = vec![0.30, 0.42, 0.50, 0.49, 0.41, 0.28];
        let (e, k) = energy_check(&t).unwrap();
        assert!((e - k).abs() < 1e-6 * k, "{e} vs {k}");
    }

    #[test]
    fn the_uncertainty_and_the_flags_follow_the_tests() {
        assert!(test_coefficients(&uniform(0.5)).unwrap().a_sigma > 0.0);
        let one = build(&[uniform(0.5)]);
        assert!(one[0].single_test && one[0].tests.len() == 1);
        let mut t2 = uniform(0.6);
        t2.test_no = 2;
        let two = build(&[uniform(0.5), t2]);
        assert!(!two[0].single_test && two[0].tests.len() == 2);
        // The spread between the two tests widens the combined σ.
        assert!(two[0].b_sigma > one[0].b_sigma);
    }

    #[test]
    fn the_bundled_table_is_plausible_and_every_test_balances() {
        let tests = parse(DATA);
        let t = table();
        let mut a: Vec<f64> = t.iter().map(|e| e.a).collect();
        let mut b: Vec<f64> = t.iter().map(|e| e.b).collect();
        a.sort_by(f64::total_cmp);
        b.sort_by(f64::total_cmp);
        let single = t.iter().filter(|e| e.single_test).count();
        eprintln!(
            "{} tests, {} vehicles ({} single-test); A median {:.0} N/m (5–95 % {:.0}–{:.0}); B median {:.0} N/m² ({:.0}–{:.0})",
            tests.len(),
            t.len(),
            single,
            a[a.len() / 2],
            a[a.len() / 20],
            a[a.len() * 19 / 20],
            b[b.len() / 2],
            b[b.len() / 20],
            b[b.len() * 19 / 20]
        );
        assert!(tests.len() > 500 && t.len() > 200);
        // Frontal A is tens of kN/m and B hundreds of kN/m² to a few MN/m² (CRASH3 Table 8-2,
        // category 4 front: 356 lb/in = 62 kN/m, 34 lb/in² = 527 kN/m²).
        assert!(a[a.len() / 2] > 20_000.0 && a[a.len() / 2] < 150_000.0);
        assert!(b[b.len() / 2] > 200_000.0 && b[b.len() / 2] < 5_000_000.0);
        for x in &tests {
            if let Some((e, k)) = energy_check(x) {
                assert!((e - k).abs() < 1e-6 * k, "test {}", x.test_no);
            }
        }
        assert!(t.iter().all(|e| e.a_sigma > 0.0 && e.b_sigma > 0.0));
    }

    #[test]
    fn the_csv_parses_with_quotes_and_the_width_fallback() {
        let csv = "# comment\ntest_no,test_date,test_type,test_reference,vehicle_no,make,model,model_year,body_type,mass_kg,speed_kmh,vehicle_width_mm,damage_width_mm,pdof_deg,c1_mm,c2_mm,c3_mm,c4_mm,c5_mm,c6_mm,vdi,barrier\n\
            15703,2021-01-01,\"FMVSS 208, FRONTAL\",x,1,TOYOTA,RAV4,2021,UTILITY VEHICLE,1696,55.70,1854,0,0,391,484,486,461,408,287,12FDEW3,rigid flat\n\
            15704,2021-01-01,NCAP,x,1,TOYOTA,RAV4,2021,UTILITY VEHICLE,1696,55.70,1854,1500,0,-30,484,486,461,408,287,,rigid flat\n";
        let t = parse(csv);
        assert_eq!(t.len(), 1);
        assert!(t[0].width_from_vehicle && (t[0].width - 1.854).abs() < 1e-12);
        assert_eq!(t[0].test_type, "FMVSS 208, FRONTAL");
        assert!((t[0].crush[0] - 0.391).abs() < 1e-12 && t[0].crush.len() == 6);
        // A two-point profile, stored with zeros after C2.
        assert_eq!(
            measured_points(&[0.7, 0.7, 0.0, 0.0, 0.0, 0.0]),
            vec![0.7, 0.7]
        );
        assert_eq!(measured_points(&[0.3, 0.5, 0.5, 0.3, 0.0, 0.0]).len(), 4);
    }
}
