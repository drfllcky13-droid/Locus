//! Bloodstain area of origin against ground truth (Phase 6 acceptance: mean error under
//! 10 cm on clean synthetic data), the bootstrap ellipsoid checked for honesty, the
//! straight-line bias shown, and the photo pipeline (alignment, automatic edges, ellipse)
//! checked on the generator's stain photos.

use locus_analysis::bloodstain::{
    align_photo, run, stain_edges, stain_from_photo, AlignPair, Parameters, Run, StainInput,
};
use locus_analysis::measure::Measured;
use locus_synth::bloodstain::{self as gen, Flight, Options};

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// The stains as an examiner measured them (the generator's noisy ellipses), with the
/// generator's noise model as their stated uncertainty.
fn measured(t: &gen::Truth) -> Vec<StainInput> {
    let o = &t.options;
    let edge = |axis: f64| (o.edge_fixed + o.edge_rel * axis) * 2f64.sqrt();
    t.stains
        .iter()
        .enumerate()
        .map(|(i, s)| StainInput {
            label: format!("{i}"),
            surface: s.surface.clone(),
            centre: s.centre,
            normal: s.normal,
            width: Measured {
                value: s.measured_width,
                sigma: edge(s.width),
            },
            length: Measured {
                value: s.measured_length,
                sigma: edge(s.length),
            },
            travel: s.measured_axis,
            travel_sigma_deg: edge(s.length)
                .atan2((s.length - s.width).max(1e-9) / 2.0)
                .to_degrees(),
            ..Default::default()
        })
        .collect()
}

/// The stains measured cleanly: the true ellipses, with the uncertainty the photo method
/// gives (0.2 % on each axis, 0.1° on the direction).
fn clean(t: &gen::Truth) -> Vec<StainInput> {
    t.stains
        .iter()
        .enumerate()
        .map(|(i, s)| StainInput {
            label: format!("{i}"),
            surface: s.surface.clone(),
            centre: s.centre,
            normal: s.normal,
            width: Measured {
                value: s.width,
                sigma: 0.002 * s.width,
            },
            length: Measured {
                value: s.length,
                sigma: 0.002 * s.length,
            },
            travel: s.travel,
            travel_sigma_deg: 0.1,
            ..Default::default()
        })
        .collect()
}

fn inside(r: &Run, truth: [f64; 3]) -> bool {
    let e = &r.origin.ellipsoid;
    let d = sub(truth, r.origin.point);
    (0..3)
        .map(|k| (dot(d, e.axes[k]) / e.semi_axes[k]).powi(2))
        .sum::<f64>()
        <= 1.0
}

/// Hand measurement of small stains (the generator's noise model: 0.1 mm + 1.5 % per edge).
/// Most of these stains are nearly round, so their directions are uncertain by 15–40°: the
/// estimate is poor, and the check is that its ellipsoid says so.
#[test]
fn with_hand_measurement_noise_the_ellipsoid_stays_honest() {
    let (mut sum, mut worst, mut covered, mut runs, mut used) = (0.0f64, 0.0f64, 0, 0, 0);
    for seed in 1..=40 {
        let t = gen::truth(&Options {
            seed,
            ..Options::default()
        });
        let r = match run(
            measured(&t),
            Parameters {
                bootstrap: 500,
                ..Parameters::default()
            },
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("seed {seed}: {e}");
                continue;
            }
        };
        runs += 1;
        used += r.origin.stains_used;
        let err = norm(sub(r.origin.point, t.options.origin));
        sum += err;
        worst = worst.max(err);
        covered += inside(&r, t.options.origin) as usize;
        if seed == 1 {
            eprintln!(
                "seed 1: {} (error {:.1} mm; {} of {} upward)",
                r.summary,
                err * 1000.0,
                t.stains.iter().filter(|s| s.upward).count(),
                t.stains.len()
            );
        }
    }
    let mean = sum / runs as f64;
    let cover = covered as f64 / runs as f64;
    eprintln!(
        "hand-measured, {runs} runs ({:.0} stains used on average): mean error {:.0} mm, worst {:.0} mm; truth inside the 95 % ellipsoid {:.0} %",
        used as f64 / runs as f64,
        mean * 1000.0,
        worst * 1000.0,
        cover * 100.0
    );
    assert!(runs >= 30 && cover >= 0.85, "coverage {cover} over {runs}");
    assert!(mean < 0.35, "mean error {mean}");
}

#[test]
#[ignore = "heavy: 10 ballistic rooms of 600 droplets (about 30 s)"]
fn heavy_straight_lines_put_a_real_origin_too_high() {
    let mut dz = vec![];
    for seed in 1..=10 {
        let t = gen::truth(&Options {
            seed,
            droplets: 600,
            flight: Flight::Ballistic,
            ..Options::default()
        });
        // Almost no droplet is still rising when it lands (3–8 m/s, 1–2 m away), so the
        // upward-only rule would leave too few: all are included, as an examiner could with
        // a reason, to show the straight-line bias itself.
        let p = Parameters {
            include_not_upward: Some("validation: straight-line bias under gravity".into()),
            ..Parameters::default()
        };
        match run(clean(&t), p) {
            Ok(r) => dz.push(r.origin.point[2] - t.options.origin[2]),
            Err(e) => eprintln!("seed {seed}: {e}"),
        }
    }
    let mean = dz.iter().sum::<f64>() / dz.len() as f64;
    eprintln!(
        "ballistic flight, {} runs: straight-line origin {:.0} mm too high on average (range {:.0} to {:.0} mm)",
        dz.len(),
        mean * 1000.0,
        dz.iter().cloned().fold(f64::INFINITY, f64::min) * 1000.0,
        dz.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * 1000.0
    );
    assert!(dz.len() >= 8 && mean > 0.0, "{dz:?}");
}

/// Phase 6 acceptance: the origin within 10 cm on clean synthetic data, measured the way
/// the app does (photo alignment, automatic edges, ellipse fit) from the generator's photos.
#[test]
#[ignore = "heavy: about 1,700 stain photos rendered and fitted"]
fn heavy_stain_photos_give_the_ellipse_and_the_origin_within_10_cm() {
    let (mut sum, mut worst, mut covered, mut runs) = (0.0f64, 0.0f64, 0, 0);
    for seed in 1..=8 {
        let (err, inside) = from_photos(seed);
        sum += err;
        worst = worst.max(err);
        covered += inside as usize;
        runs += 1;
    }
    eprintln!(
        "from photos, {runs} runs: mean error {:.1} mm, worst {:.1} mm, truth inside the ellipsoid in {covered} of {runs}",
        sum / runs as f64 * 1000.0,
        worst * 1000.0
    );
    assert!(sum / (runs as f64) < 0.10 && worst < 0.10);
    assert!(covered >= runs - 1);
}

fn from_photos(seed: u64) -> (f64, bool) {
    let o = Options {
        seed,
        ..Options::default()
    };
    let t = gen::truth(&o);
    let mut inputs = vec![];
    let (mut worst_axis, mut worst_angle, mut worst_centre) = (0.0f64, 0.0f64, 0.0f64);
    for (i, s) in t.stains.iter().enumerate() {
        let rgb = gen::photo(&o, s);
        let [w, h] = s.photo.size_px.map(|v| v as usize);
        let luma: Vec<u8> = rgb
            .chunks(3)
            .map(|p| {
                (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64).round() as u8
            })
            .collect();
        // The fiducials: pixel and scan point (on the surface) pairs.
        let pairs: Vec<AlignPair> = s
            .photo
            .fiducials
            .iter()
            .map(|f| AlignPair {
                px: f.px,
                world: f.world,
            })
            .collect();
        let al = align_photo(&pairs, s.centre, s.normal).unwrap();
        let edges = stain_edges(&luma, w, h, [w as f64 / 2.0, h as f64 / 2.0], 126).unwrap();
        // The examiner marks the tail's tip: a little beyond the stain's leading end.
        let tip = [0, 1, 2].map(|k| s.centre[k] + s.travel[k] * s.length * 0.62);
        let r = sub(tip, s.centre);
        let half = w as f64 / 2.0;
        let tail_px = [
            half + dot(r, s.photo.u) * s.photo.scale,
            half - dot(r, s.photo.v) * s.photo.scale,
        ];
        let st = stain_from_photo(&format!("{i}"), &s.surface, al, edges, tail_px).unwrap();
        let f = st.fit.clone().unwrap();
        let rel = |m: f64, truth: f64| (m - truth).abs() / truth;
        worst_axis = worst_axis
            .max(rel(f.width.value, s.width))
            .max(rel(f.length.value, s.length));
        worst_centre = worst_centre.max(norm(sub(f.centre, s.centre)));
        if s.length / s.width > 1.1 {
            worst_angle = worst_angle.max(
                dot(st.travel, s.travel)
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees(),
            );
        }
        assert!(dot(st.normal, s.normal) > 1.0 - 1e-12);
        inputs.push(st);
    }
    eprintln!(
        "seed {seed}: {} photos: axes within {:.2} %, direction within {:.2}° (stains longer than 1.1 × wide), centre within {:.3} mm",
        t.stains.len(),
        worst_axis * 100.0,
        worst_angle,
        worst_centre * 1000.0
    );
    assert!(worst_axis < 0.02 && worst_angle < 1.0 && worst_centre < 1e-4);
    let r = run(inputs, Parameters::default()).unwrap();
    let err = norm(sub(r.origin.point, t.options.origin));
    eprintln!(
        "seed {seed}: origin from the photos {:.1} mm from truth ({:.1}, {:.1}, {:.1}), {} stains used, ellipsoid {:.1} × {:.1} × {:.1} mm, χ² {:.0} on {}",
        err * 1000.0,
        (r.origin.point[0] - t.options.origin[0]) * 1000.0,
        (r.origin.point[1] - t.options.origin[1]) * 1000.0,
        (r.origin.point[2] - t.options.origin[2]) * 1000.0,
        r.origin.stains_used,
        r.origin.ellipsoid.semi_axes[0] * 1000.0,
        r.origin.ellipsoid.semi_axes[1] * 1000.0,
        r.origin.ellipsoid.semi_axes[2] * 1000.0,
        r.origin.chi2,
        r.origin.dof
    );
    (err, inside(&r, t.options.origin))
}
