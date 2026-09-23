//! Bullet trajectory against ground truth (Phase 6 acceptance: a synthetic multi-surface
//! trajectory recovered within 0.5°), and the stated uncertainty checked for honesty.

use locus_analysis::trajectory::{fit_line, surface_angles, PathPoint, Surface};
use locus_synth::trajectory::{self as gen, Options};

fn deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let n = (a.iter().map(|x| x * x).sum::<f64>() * b.iter().map(|x| x * x).sum::<f64>()).sqrt();
    (d / n).clamp(-1.0, 1.0).acos().to_degrees()
}

/// The defects as an examiner picks them, entry then exit, in travel order.
fn picks(t: &gen::Truth) -> Vec<PathPoint> {
    let s =
        (2.0 * t.options.pick_sigma.powi(2) + t.options.scan_sigma.powi(2)).sqrt() / 3f64.sqrt();
    t.panels
        .iter()
        .flat_map(|p| [p.entry_picked, p.exit_picked])
        .map(|point| PathPoint { point, sigma: s })
        .collect()
}

#[test]
fn a_multi_surface_trajectory_is_recovered_within_half_a_degree() {
    let mut worst: f64 = 0.0;
    for seed in 1..=50 {
        let t = gen::truth(&Options {
            seed,
            ..Options::default()
        });
        let l = fit_line(&picks(&t), 0.0).unwrap();
        let err = deg(l.direction, t.direction);
        worst = worst.max(err);
        // Angles to each surface agree with the truth within their stated uncertainty ×3.
        for p in &t.panels {
            let a = surface_angles(
                &l,
                &Surface {
                    point: p.entry,
                    normal: p.normal,
                },
            );
            assert!(
                (a.impact.value - p.impact_deg).abs() < 3.0 * a.impact.sigma + 0.05,
                "seed {seed} {}: impact {} ± {} vs {}",
                p.name,
                a.impact.value,
                a.impact.sigma,
                p.impact_deg
            );
        }
    }
    eprintln!("worst direction error over 50 runs: {worst:.4}°");
    assert!(worst < 0.5, "{worst}°");
}

#[test]
fn the_95_percent_cone_covers_the_truth_about_95_percent_of_the_time() {
    let runs = 400;
    let mut inside = 0;
    for seed in 1..=runs {
        let t = gen::truth(&Options {
            seed,
            ..Options::default()
        });
        let l = fit_line(&picks(&t), 0.0).unwrap();
        // Inside the 95 % ellipse: Mahalanobis distance² of the error ≤ χ²₂(95 %).
        let e = [0, 1, 2].map(|k| t.direction[k] - l.direction[k]);
        // Covariance is rank 2 (square to d); invert on its range via its eigenvectors.
        let c = l.covariance;
        let (u, v) = {
            let a = l.cone.major_axis;
            let d = l.direction;
            let w = [
                a[1] * d[2] - a[2] * d[1],
                a[2] * d[0] - a[0] * d[2],
                a[0] * d[1] - a[1] * d[0],
            ];
            (a, w)
        };
        let q = |x: [f64; 3], y: [f64; 3]| -> f64 {
            (0..3)
                .map(|i| (0..3).map(|j| x[i] * c[i][j] * y[j]).sum::<f64>())
                .sum()
        };
        let (su, sv) = (q(u, u), q(v, v));
        let (eu, ev) = (
            [0, 1, 2].map(|k| e[k] * u[k]).iter().sum::<f64>(),
            [0, 1, 2].map(|k| e[k] * v[k]).iter().sum::<f64>(),
        );
        if eu * eu / su + ev * ev / sv <= locus_analysis::trajectory::CHI2_2DOF_95 {
            inside += 1;
        }
    }
    let share = inside as f64 / runs as f64;
    eprintln!(
        "truth inside the 95 % cone in {:.1} % of runs",
        share * 100.0
    );
    assert!((0.92..=0.98).contains(&share), "{share}");
}

#[test]
fn a_probe_rod_is_within_its_play() {
    for seed in 1..=20 {
        let t = gen::truth(&Options {
            seed,
            ..Options::default()
        });
        let pts = [t.rod.a, t.rod.b].map(|point| PathPoint {
            point,
            sigma: t.options.pick_sigma,
        });
        let l = fit_line(&pts, t.rod.max_play_deg).unwrap();
        let err = deg(l.direction, t.direction);
        // Play plus the two ends' picking noise over 0.5 m.
        assert!(
            err <= t.rod.max_play_deg + 1.0,
            "seed {seed}: {err}° vs play ≤ {}",
            t.rod.max_play_deg
        );
        assert!(l.cone.major_deg >= t.rod.max_play_deg);
    }
}
