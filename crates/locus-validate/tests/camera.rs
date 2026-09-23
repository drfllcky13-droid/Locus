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
    for seed in 1..=40 {
        // A good solve: control points spread over the room (60 markers; each camera sees
        // about 25). With the default 30, a camera sees 9–14, and the full lens model's 14
        // unknowns are then poorly determined (the stated uncertainty says so).
        let o = Options {
            seed,
            markers: 60,
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
            // The full model: the generator's principal points are a few pixels off centre,
            // which a model holding it at the centre turns into centimetres of height.
            let model = LensModel::Full;
            let s = solve(&pairs, cam.size, model, o.pick_sigma_px, 0.0).unwrap();
            let pos_err = (0..3)
                .map(|k| (s.camera.position[k] - cam.position[k]).powi(2))
                .sum::<f64>()
                .sqrt();
            if seed == 1 {
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
            assert!(z < 4.0, "{} seed {seed}: {pos_err} m, {z}σ", cam.name);
            for (p, pv) in o.people.iter().zip(&view.people) {
                let (Some(feet), Some(head)) = (pv.feet, pv.head) else {
                    continue;
                };
                let input = HeightInput {
                    label: format!("person {}", pv.index + 1),
                    feet_px: [feet[0] + pick * rng.gauss(), feet[1] + pick * rng.gauss()],
                    head_px: [head[0] + pick * rng.gauss(), head[1] + pick * rng.gauss()],
                    matched_model: None,
                };
                let h = height(&s, &input, 0.0, pick, 1000, seed).unwrap();
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
                if !ok {
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
                if seed == 1 {
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
    eprintln!(
        "heights, {total} over 40 seeds and both cameras: mean error {:.1} mm, worst {:.1} mm; truth inside the 95 % interval {:.0} %",
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
    assert!((0.88..=1.0).contains(&cover), "coverage {cover}");
}
