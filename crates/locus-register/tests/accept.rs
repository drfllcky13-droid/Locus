//! Phase 3 acceptance on synthetic scans with known poses: registration error under 2 mm and
//! 0.02° relative to the first scan, and an injected bad link flagged.
//!
//! The scans come as a scanner exports them after its on-site pre-registration: each with a
//! rough pose (here off by 0.5 m and 5°). Without rough poses or shared targets, a scan
//! placed by shape alone must be marked for the examiner to confirm.

use locus_register::pipeline::{register, Params, ScanInput};
use locus_register::posegraph::{solve, LinkKind, LinkStatus};
use locus_synth::{scan_points, truth, Options, StoredPose, Truth, BOARD_SIZE, SPHERE_RADIUS};
use nalgebra::{Isometry3, Point3, Quaternion, Translation3, UnitQuaternion, Vector3};

fn iso(p: &locus_synth::Pose) -> Isometry3<f64> {
    let q = p.rotation;
    Isometry3::from_parts(
        Translation3::from(Vector3::from(p.translation)),
        UnitQuaternion::from_quaternion(Quaternion::new(q[0], q[1], q[2], q[3])),
    )
}

fn scans(opts: &Options, t: &Truth) -> Vec<ScanInput> {
    (0..opts.scans)
        .map(|s| {
            let (mut points, mut intensity) = (vec![], vec![]);
            scan_points(opts, t, s, &mut |p| {
                if let Some(p) = p {
                    points.push(p.xyz);
                    intensity.push(p.intensity as f64);
                }
            });
            ScanInput { points, intensity }
        })
        .collect()
}

/// Worst error over the scans, relative to scan 0: (m at up to 10 m from the scanner, °).
fn worst(reg: &locus_register::pipeline::Registration, t: &Truth) -> (f64, f64) {
    let (mut d, mut a) = (0f64, 0f64);
    for s in 1..t.scans.len() {
        let truth = iso(&t.scans[0].pose).inverse() * iso(&t.scans[s].pose);
        let est = reg.solution.poses[0].inverse() * reg.solution.poses[s];
        let e = est.inverse() * truth;
        for probe in [
            Point3::origin(),
            Point3::new(10.0, 0.0, 0.0),
            Point3::new(0.0, 10.0, 0.0),
            Point3::new(0.0, 0.0, 3.0),
        ] {
            d = d.max((e * probe - probe).norm());
        }
        a = a.max(e.rotation.angle().to_degrees());
    }
    (d, a)
}

fn rough(t: &Truth) -> Vec<Isometry3<f64>> {
    t.scans
        .iter()
        .map(|s| iso(&s.stored_pose.expect("rough pose")))
        .collect()
}

fn params() -> Params {
    Params {
        sphere_radius: Some(SPHERE_RADIUS),
        board_size: Some(BOARD_SIZE),
        ..Params::default()
    }
}

#[test]
#[ignore = "heavy: synthetic scans of millions of points (see .config/nextest.toml)"]
fn heavy_hybrid_registration_meets_2_mm_and_0_02_degrees() {
    let opts = Options {
        scans: 6,
        points_per_scan: 4_000_000,
        seed: 21,
        stored_pose: StoredPose::Perturbed {
            metres: 0.5,
            degrees: 5.0,
        },
        ..Options::default()
    };
    let t = truth(&opts);
    let reg = register(&scans(&opts, &t), Some(&rough(&t)), &[], &params()).expect("registration");
    let (d, a) = worst(&reg, &t);
    for (l, r) in reg.links.iter().zip(&reg.solution.links) {
        eprintln!(
            "{:?} {}-{:?}: {:?} rms {:.2} mm",
            l.kind,
            l.a,
            l.b,
            r.status,
            r.rms * 1e3
        );
    }
    eprintln!("worst error {:.3} mm, {:.4}°", d * 1e3, a);
    assert!(d < 0.002, "{:.3} mm", d * 1e3);
    assert!(a < 0.02, "{a:.4}°");
    assert!(reg.links.iter().any(|l| l.kind == LinkKind::Cloud));
    assert!(
        reg.solution
            .links
            .iter()
            .all(|l| l.status != LinkStatus::Flagged),
        "a good link was flagged"
    );
    assert!(
        reg.verified.iter().all(|&v| v),
        "with rough poses every scan is verified"
    );
}

#[test]
#[ignore = "heavy: synthetic scans of millions of points (see .config/nextest.toml)"]
fn heavy_an_injected_bad_link_is_flagged_and_the_rest_still_meets_accuracy() {
    let opts = Options {
        scans: 6,
        points_per_scan: 4_000_000,
        seed: 21,
        stored_pose: StoredPose::Perturbed {
            metres: 0.5,
            degrees: 5.0,
        },
        ..Options::default()
    };
    let t = truth(&opts);
    let mut reg =
        register(&scans(&opts, &t), Some(&rough(&t)), &[], &params()).expect("registration");
    // A copy of a real cloud link, 2 cm and 0.1° wrong: a bad ICP result.
    let good = reg
        .links
        .iter()
        .position(|l| l.kind == LinkKind::Cloud)
        .expect("a cloud link");
    let mut bad = reg.links[good].clone();
    let err = Isometry3::new(
        Vector3::new(0.02, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 0.1f64.to_radians()),
    );
    for p in &mut bad.pairs {
        p.pa = (err * Point3::from(p.pa)).coords.into();
    }
    reg.links.push(bad);
    let injected = reg.links.len() - 1;
    reg.solution = solve(&reg.solution.poses, &reg.links).expect("solve");
    for (i, (l, r)) in reg.links.iter().zip(&reg.solution.links).enumerate() {
        eprintln!(
            "{i} {:?} {}-{:?}: {:?} rms {:.2} mm, chi2/dof {:.2} (limit {:.2})",
            l.kind,
            l.a,
            l.b,
            r.status,
            r.rms * 1e3,
            r.chi2_per_dof,
            r.limit_per_dof
        );
        let expect = if i == injected {
            LinkStatus::Flagged
        } else {
            LinkStatus::Ok
        };
        assert_eq!(r.status, expect, "link {i}");
    }
    let (d, a) = worst(&reg, &t);
    assert!(d < 0.002 && a < 0.02, "{:.3} mm, {a:.4}°", d * 1e3);
}

#[test]
#[ignore = "heavy: synthetic scans of millions of points (see .config/nextest.toml)"]
fn heavy_without_rough_poses_a_scan_placed_by_shape_alone_is_marked_for_review() {
    // The synthetic room is nearly symmetric under a half turn, so shape matching alone can
    // place a scan flipped, consistently across all its links. That can't be tested for; it
    // must be marked.
    let opts = Options {
        scans: 6,
        points_per_scan: 4_000_000,
        seed: 21,
        stored_pose: StoredPose::None,
        ..Options::default()
    };
    let t = truth(&opts);
    let reg = register(&scans(&opts, &t), None, &[], &params()).expect("registration");
    let mut wrong = 0;
    for s in 1..opts.scans {
        let truth = iso(&t.scans[0].pose).inverse() * iso(&t.scans[s].pose);
        let e = (reg.solution.poses[0].inverse() * reg.solution.poses[s]).inverse() * truth;
        if e.translation.vector.norm() > 0.002 || e.rotation.angle().to_degrees() > 0.02 {
            wrong += 1;
            assert!(
                !reg.verified[s],
                "scan {s} is misplaced but reported as verified"
            );
        }
    }
    eprintln!("{wrong} scans misplaced; verified {:?}", reg.verified);
}
