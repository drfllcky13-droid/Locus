//! Volumetric crush end to end on synthetic vehicles: an exemplar registered onto a dented copy
//! (picked pairs with a few millimetres' error, then ICP around the damage), and the crush
//! volume with its Monte Carlo interval, against the dent's exact volume.

use locus_analysis::crush_volume::{crush_volume, PoseUncertainty};
use locus_register::exemplar::align_exemplar;
use locus_synth::crush::vehicle;
use nalgebra::{Isometry3, Point3, Translation3, UnitQuaternion, Vector3};

struct Rng(u64);
impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform().max(1e-300), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

#[test]
fn crush_volume_is_recovered_after_registration() {
    run(8);
}

#[test]
#[ignore = "heavy: 40 registrations and 80 volume Monte Carlos"]
fn crush_volume_coverage() {
    run(40);
}

fn run(runs: usize) {
    let (mut covered, mut worst, mut sum, mut formal) = (0, 0.0f64, 0.0, 0usize);
    for seed in 0..runs {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (seed as u64 + 1));
        let (r, d) = (0.15 + 0.15 * rng.uniform(), 0.03 + 0.12 * rng.uniform());
        let truth_volume = std::f64::consts::PI * r * r * d / 2.0;
        let noise = 0.001 + 0.002 * rng.uniform();
        let pose = Isometry3::from_parts(
            Translation3::new(
                10.0 * rng.uniform(),
                10.0 * rng.uniform(),
                0.2 * rng.uniform(),
            ),
            UnitQuaternion::from_euler_angles(
                0.02 * rng.gauss(),
                0.02 * rng.gauss(),
                0.1 * rng.gauss(),
            ),
        );
        let reference = vehicle(None, noise, 2 * seed as u64);
        let damaged: Vec<[f64; 3]> = vehicle(Some((r, d)), noise, 2 * seed as u64 + 1)
            .iter()
            .map(|p| (pose * Point3::from(*p)).coords.into())
            .collect();
        // Five undamaged features picked on both scans, each with 3 mm 1σ picking error.
        let feats = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.6, 0.0],
            [0.0, 1.6, 1.0],
            [0.8, 0.0, 1.0],
        ];
        let mut pick = |p: [f64; 3]| {
            let e = Vector3::new(rng.gauss(), rng.gauss(), rng.gauss()) * 0.003;
            Point3::from(p) + e
        };
        let pairs: Vec<_> = feats
            .iter()
            .map(|f| (pick(*f).coords.into(), (pose * pick(*f)).coords.into()))
            .collect();
        // The damage region: the front around the dent, as an examiner would draw it.
        let corners: Vec<Point3<f64>> = [
            [-0.25, 0.8 - r - 0.1, 0.5 - r - 0.1],
            [0.25, 0.8 + r + 0.1, 0.5 + r + 0.1],
        ]
        .iter()
        .flat_map(|a| [*a])
        .map(|p| pose * Point3::from(p))
        .collect();
        let lo = std::array::from_fn(|k| corners[0][k].min(corners[1][k]));
        let hi = std::array::from_fn(|k| corners[0][k].max(corners[1][k]));
        let view = (pose * Point3::new(-5.0, 0.8, 1.5)).coords.into();
        let a = align_exemplar(&reference, &damaged, view, &pairs, lo, hi, 0.02).unwrap();
        let moved: Vec<[f64; 3]> = reference
            .iter()
            .map(|p| (a.transform * Point3::from(*p)).coords.into())
            .collect();
        let v = crush_volume(
            &moved,
            &damaged,
            lo,
            hi,
            view,
            0.02,
            noise,
            Some(PoseUncertainty {
                covariance: a.covariance,
                about: a.about.coords.into(),
                surface: 0.0,
            }),
            200,
            seed as u64,
        )
        .unwrap();
        // The same with the formal ICP covariance (every pair independent), for comparison.
        let f = crush_volume(
            &moved,
            &damaged,
            lo,
            hi,
            view,
            0.02,
            noise,
            Some(PoseUncertainty {
                covariance: a.covariance / a.inflation,
                about: a.about.coords.into(),
                surface: 0.0,
            }),
            200,
            seed as u64,
        )
        .unwrap();
        formal += (f.inward.interval95[0] <= truth_volume && truth_volume <= f.inward.interval95[1])
            as usize;
        let rel = (v.inward.value - truth_volume) / truth_volume;
        let inside =
            v.inward.interval95[0] <= truth_volume && truth_volume <= v.inward.interval95[1];
        covered += inside as usize;
        worst = worst.max(rel.abs());
        sum += rel.abs();
        println!(
            "seed {seed}: R {r:.3} D {d:.3} noise {:.1} mm: {:.2} L vs {:.2} L ({:+.1} %), 95 % {:.2}–{:.2} L{}, inflation {:.0}",
            noise * 1000.0,
            v.inward.value * 1000.0,
            truth_volume * 1000.0,
            rel * 100.0,
            v.inward.interval95[0] * 1000.0,
            v.inward.interval95[1] * 1000.0,
            if inside { "" } else { " MISSED" },
            a.inflation
        );
    }
    println!(
        "crush volume: mean error {:.1} %, worst {:.1} %, 95 % interval covers {covered}/{runs} ({formal}/{runs} with the formal ICP covariance)",
        sum / runs as f64 * 100.0,
        worst * 100.0
    );
    assert!(worst < 0.10, "worst {worst}");
    assert!(
        covered >= runs * 17 / 20 || covered + 1 >= runs,
        "covered {covered}/{runs}"
    );
}

#[test]
fn mirrored_crush_volume_is_recovered() {
    run_mirror(8);
}

#[test]
#[ignore = "heavy: 40 mirrored registrations and volume Monte Carlos"]
fn mirrored_crush_volume_coverage() {
    run_mirror(40);
}

/// The damaged vehicle's own undamaged side as the reference: a symmetric vehicle front with
/// a dent right of centre and the left half standing up to 2 mm proud (real asymmetry),
/// three symmetric pairs picked with 3 mm error, the centre plane fitted, the left side
/// reflected and registered, and the volume against the dent's exact volume.
fn run_mirror(runs: usize) {
    use locus_register::exemplar::align_mirror;
    use locus_synth::crush::{symmetric_vehicle, SYMMETRIC_DENT, SYMMETRIC_PAIRS};
    let (mut covered, mut worst, mut sum) = (0, 0.0f64, 0.0);
    for seed in 0..runs {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d ^ (seed as u64 + 7));
        let (r, d) = (0.12 + 0.13 * rng.uniform(), 0.03 + 0.12 * rng.uniform());
        let truth = std::f64::consts::PI * r * r * d / 2.0;
        let noise = 0.001 + 0.002 * rng.uniform();
        let asym = 0.002 * rng.uniform();
        let pose = Isometry3::from_parts(
            Translation3::new(10.0 * rng.uniform(), 10.0 * rng.uniform(), 0.0),
            UnitQuaternion::from_euler_angles(0.0, 0.0, 0.3 * rng.gauss()),
        );
        let pts: Vec<[f64; 3]> = symmetric_vehicle(Some((r, d)), noise, asym, seed as u64)
            .iter()
            .map(|p| (pose * Point3::from(*p)).coords.into())
            .collect();
        let mut pick = |p: [f64; 3]| -> [f64; 3] {
            let e = Vector3::new(rng.gauss(), rng.gauss(), rng.gauss()) * 0.003;
            (pose * (Point3::from(p) + e)).coords.into()
        };
        let pairs: Vec<_> = SYMMETRIC_PAIRS
            .iter()
            .map(|(a, b)| (pick(*a), pick(*b)))
            .collect();
        let c = SYMMETRIC_DENT;
        let m = r + 0.1;
        let corners: Vec<Point3<f64>> = (0..8)
            .map(|k| {
                pose * Point3::new(
                    if k & 1 == 0 { -0.25 } else { d + 0.1 },
                    c[1] + if k & 2 == 0 { -m } else { m },
                    c[2] + if k & 4 == 0 { -m } else { m },
                )
            })
            .collect();
        let lo =
            std::array::from_fn(|k| corners.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
        let hi = std::array::from_fn(|k| {
            corners
                .iter()
                .map(|p| p[k])
                .fold(f64::NEG_INFINITY, f64::max)
        });
        let view = (pose * Point3::new(-5.0, 0.0, 1.5)).coords.into();
        let mi = align_mirror(&pts, view, &pairs, lo, hi, 0.02, noise).unwrap();
        let moved: Vec<[f64; 3]> = mi
            .reference
            .iter()
            .map(|p| (mi.aligned.transform * Point3::from(*p)).coords.into())
            .collect();
        let v = crush_volume(
            &moved,
            &pts,
            lo,
            hi,
            view,
            0.02,
            noise,
            Some(PoseUncertainty {
                covariance: mi.aligned.covariance,
                about: mi.aligned.about.coords.into(),
                surface: mi.symmetry_sigma,
            }),
            200,
            seed as u64,
        )
        .unwrap();
        let rel = (v.inward.value - truth) / truth;
        let inside = v.inward.interval95[0] <= truth && truth <= v.inward.interval95[1];
        covered += inside as usize;
        worst = worst.max(rel.abs());
        sum += rel.abs();
        println!(
            "mirror seed {seed}: R {r:.3} D {d:.3} asymmetry {:.1} mm: {:.2} L vs {:.2} L ({:+.1} %), 95 % {:.2}–{:.2} L{}, symmetry σ {:.1} mm",
            asym * 1000.0,
            v.inward.value * 1000.0,
            truth * 1000.0,
            rel * 100.0,
            v.inward.interval95[0] * 1000.0,
            v.inward.interval95[1] * 1000.0,
            if inside { "" } else { " MISSED" },
            mi.symmetry_sigma * 1000.0
        );
    }
    println!(
        "mirrored crush volume: mean error {:.1} %, worst {:.1} %, 95 % interval covers {covered}/{runs}",
        sum / runs as f64 * 100.0,
        worst * 100.0
    );
    assert!(
        covered >= runs * 17 / 20 || covered + 1 >= runs,
        "covered {covered}/{runs}"
    );
}
