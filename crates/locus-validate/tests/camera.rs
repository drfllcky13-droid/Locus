//! Camera matching and subject height against ground truth (Phase 6; the spec's bound:
//! height under 2 cm with a good camera solve): the generator's CCTV (wide lens, strong
//! distortion) and handheld photo, solved from its picked control markers, and each
//! standing person's height by reverse projection, with the stated uncertainty's coverage.

use locus_analysis::camera::{height, solve, CameraPair, HeightInput, LensModel};
use locus_synth::camera::{self as gen, Options};

struct Rng(u64);
impl Rng {
    fn gauss(&mut self) -> f64 {
        let mut u = || {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        };
        let (a, b) = (u().max(1e-300), u());
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }
}

#[test]
fn cameras_and_heights_from_the_generator_are_within_bounds() {
    // A clicked feet or head point: 1 px (1σ) from where it should be.
    let pick = 1.0;
    let (mut worst, mut inside, mut total) = (0.0f64, 0, 0);
    // "A good camera solve" (the spec's condition): one that states the height to 1 cm (1σ)
    // or better. The worst error among those.
    let (mut good, mut worst_good, mut good_errs) = (0, 0.0f64, vec![]);
    let mut sum = 0.0;
    // Coverage of the stated 95 % interval, for the rooms with few control points (poor
    // solves) and with many.
    let mut cover_by = [(0, 0); 2];
    let mut models = vec![];
    let mut pose_outliers = 0;
    for (seed, markers) in (1..=40).flat_map(|s| [(s, 30), (s, 60)]) {
        // 60 markers: each camera sees about 25, a good solve. The default 30: a camera sees
        // 9–14, and a lens model's unknowns are poorly determined (the stated uncertainty
        // should say so).
        let o = Options {
            seed,
            markers,
            ..Options::default()
        };
        let t = gen::truth(&o);
        let mut rng = Rng(seed * 7919 + 1);
        for (cam, view) in o.cameras.iter().zip(&t.views) {
            let pairs: Vec<CameraPair> = view
                .markers
                .iter()
                .filter_map(|m| {
                    m.picked.map(|px| CameraPair {
                        px,
                        world: t.markers[m.index].centre,
                    })
                })
                .collect();
            // The lens model chosen by leave-one-out, as the app does by default; the
            // uncertainty from a 300-draw bootstrap.
            let s = solve(
                &pairs,
                cam.size,
                LensModel::Auto,
                o.pick_sigma_px,
                0.0,
                300,
                seed,
            )
            .unwrap();
            let model = s.model;
            models.push(model);
            let pos_err = (0..3)
                .map(|k| (s.camera.position[k] - cam.position[k]).powi(2))
                .sum::<f64>()
                .sqrt();
            if seed == 1 && markers == 60 {
                eprintln!(
                    "{} ({:?}, {} pairs): position {:.1} mm off (1σ {:.1}, {:.1}, {:.1} mm), focal length {:.1} px (true {:.1} ± {:.1}), {:.2} px RMS, χ² {:.1} on {}",
                    cam.name,
                    model,
                    pairs.len(),
                    pos_err * 1000.0,
                    s.position_sigma[0] * 1000.0,
                    s.position_sigma[1] * 1000.0,
                    s.position_sigma[2] * 1000.0,
                    s.camera.f,
                    cam.fx,
                    s.f_sigma,
                    s.rms_px,
                    s.chi2,
                    s.dof
                );
            }
            let z = (0..3)
                .map(|k| ((s.camera.position[k] - cam.position[k]) / s.position_sigma[k]).abs())
                .fold(0.0, f64::max);
            // The camera's own position may carry a model bias (a simpler lens holding the
            // principal point at the centre, when leave-one-out prefers it); counted, not
            // asserted: the heights' coverage below is the test.
            if z > 4.0 {
                pose_outliers += 1;
                if std::env::var("CAMERA_DEBUG").is_ok() {
                    eprintln!(
                        "  pose {} seed {seed}: {:.0} mm, {z:.1}σ ({model:?})",
                        cam.name,
                        pos_err * 1000.0
                    );
                }
            }
            for (p, pv) in o.people.iter().zip(&view.people) {
                let (Some(feet), Some(head)) = (pv.feet, pv.head) else {
                    continue;
                };
                let input = HeightInput {
                    label: format!("person {}", pv.index + 1),
                    feet_px: [feet[0] + pick * rng.gauss(), feet[1] + pick * rng.gauss()],
                    head_px: [head[0] + pick * rng.gauss(), head[1] + pick * rng.gauss()],
                    matched_model: None,
                    frame: None,
                };
                let h = height(&s, &input, 0.0, pick, seed).unwrap();
                let err = h.height.value - p.height;
                worst = worst.max(err.abs());
                if h.height.sigma <= 0.01 {
                    good += 1;
                    worst_good = worst_good.max(err.abs());
                    good_errs.push(err.abs());
                }
                sum += err.abs();
                total += 1;
                let ok = h.interval95[0] <= p.height && p.height <= h.interval95[1];
                inside += ok as usize;
                let g = (markers == 60) as usize;
                cover_by[g].0 += ok as usize;
                cover_by[g].1 += 1;
                if !ok && std::env::var("CAMERA_DEBUG").is_ok() {
                    eprintln!(
                        "  outside: seed {seed} {} {}: error {:+.1} mm, σ {:.1} mm, miss {:.1} mm, rms {:.2}",
                        cam.name,
                        input.label,
                        err * 1000.0,
                        h.height.sigma * 1000.0,
                        h.miss * 1000.0,
                        s.rms_px
                    );
                }
                if seed == 1 && markers == 60 {
                    eprintln!(
                        "  {}: {:.3} ± {:.3} m (true {:.3}; error {:+.1} mm, miss {:.1} mm)",
                        input.label,
                        h.height.value,
                        h.height.sigma,
                        p.height,
                        err * 1000.0,
                        h.miss * 1000.0
                    );
                }
            }
        }
    }
    let cover = inside as f64 / total as f64;
    for (g, name) in [
        (0, "few control points (30 markers)"),
        (1, "many (60 markers)"),
    ] {
        eprintln!(
            "  95 % interval coverage, {name}: {:.1} % of {}",
            cover_by[g].0 as f64 / cover_by[g].1 as f64 * 100.0,
            cover_by[g].1
        );
    }
    eprintln!(
        "heights, {total} over 40 seeds × 2 marker counts and both cameras: mean error {:.1} mm, worst {:.1} mm; truth inside the 95 % interval {:.0} %",
        sum / total as f64 * 1000.0,
        worst * 1000.0,
        cover * 100.0
    );
    good_errs.sort_by(f64::total_cmp);
    let p95 = good_errs[(good_errs.len() * 95) / 100];
    eprintln!(
        "  with a good solve (height 1σ ≤ 10 mm): {good} of {total}; 95 % of errors under {:.1} mm, worst {:.1} mm",
        p95 * 1000.0,
        worst_good * 1000.0
    );
    assert!(
        good * 2 > total && p95 < 0.02,
        "95th percentile {p95} of {good}"
    );
    assert!(
        sum / (total as f64) < 0.01,
        "mean error {}",
        sum / total as f64
    );
    let count = |m: LensModel| models.iter().filter(|x| **x == m).count();
    eprintln!(
        "  lens models chosen by leave-one-out over {} solves: focal length only {}, k1 {}, k1 k2 {}, full {}",
        models.len(),
        count(LensModel::Pinhole),
        count(LensModel::Radial1),
        count(LensModel::Radial2),
        count(LensModel::Full)
    );
    eprintln!(
        "  camera positions more than 4σ off: {pose_outliers} of {}",
        models.len()
    );
    // The stated 95 % interval covers the truth about 95 % of the time over every case,
    // poor solves included (240 correlated cases: ±3 %).
    assert!((0.92..=0.98).contains(&cover), "coverage {cover}");
}
