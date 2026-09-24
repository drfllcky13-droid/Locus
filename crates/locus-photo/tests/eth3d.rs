//! Acceptance on a public benchmark with laser-scan ground truth: ETH3D's "pipes" (14 DSLR
//! photos; ground-truth camera poses and calibration registered to a laser scan, metric).
//!
//! COLMAP (installed by the user) reconstructs from the photos alone. Each reconstructed point
//! is also triangulated independently from its own observations with the ground-truth cameras
//! (its true position, given the matches). Points seen from well-separated views (at least 10°)
//! are kept: the scene an examiner measures, not the far background through a doorway. The model
//! is scaled by three known distances of 2 to 3 m (as tapes laid in the scene would be), and the
//! distances between other point pairs at least 0.5 m apart are compared with the truth.
//! Bound: 99 % of them within 1 %.
//!
//! Needs LOCUS_COLMAP (colmap.exe) and LOCUS_ETH3D (the folder holding `pipes/`); see
//! tests/README.md. The data stays outside the repository.

use locus_photo::{camera, colmap, model, scale};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
}

#[test]
#[ignore = "heavy: needs COLMAP and the ETH3D pipes scene (LOCUS_COLMAP, LOCUS_ETH3D)"]
fn heavy_eth3d_pipes_distances_within_one_percent() {
    let (Ok(exe), Ok(data)) = (std::env::var("LOCUS_COLMAP"), std::env::var("LOCUS_ETH3D")) else {
        panic!("set LOCUS_COLMAP and LOCUS_ETH3D");
    };
    let (exe, scene) = (PathBuf::from(exe), PathBuf::from(data).join("pipes"));
    let gt_dir = scene.join("dslr_calibration_jpg");
    let read = |f: &str| std::fs::read_to_string(gt_dir.join(f)).unwrap();
    let gt = model::read(&read("cameras.txt"), &read("images.txt"), "").unwrap();
    let build = colmap::detect(&exe).unwrap();
    println!("{}", build.banner);
    let cpu = std::env::var("LOCUS_COLMAP_GPU").is_err();
    // One camera body and lens for all 14 photos, a fisheye (the ground truth models it as
    // THIN_PRISM_FISHEYE), features from 4800 px images (Lotus's default for measurement; COLMAP's
    // own 3200 px left the 99th percentile at 1.02 %): the choices an examiner would make.
    let settings = colmap::Settings {
        cpu_only: cpu,
        dense: false,
        single_camera: true,
        camera_model: std::env::var("LOCUS_COLMAP_MODEL").unwrap_or("OPENCV_FISHEYE".into()),
        max_image_size: std::env::var("LOCUS_COLMAP_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4800),
        ..Default::default()
    };
    let work = std::env::temp_dir().join(format!("locus-eth3d-{}", std::process::id()));
    let t0 = std::time::Instant::now();
    let r = colmap::reconstruct(
        &exe,
        &build,
        &settings,
        &scene.join("images"),
        &work,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&work);
    let m = &r.model;
    println!(
        "reconstructed in {:.0} s ({}): {} of {} images, {} points, mean reprojection error {:.2} px",
        t0.elapsed().as_secs_f64(),
        if cpu { "CPU features" } else { "GPU features" },
        m.images.len(),
        gt.images.len(),
        m.points.len(),
        m.mean_error(),
    );
    // Each point's true position: its observations' rays from the ground-truth cameras.
    let gt_image = |name: &str| gt.images.values().find(|g| g.name == name);
    let mut pts = vec![];
    let mut residuals = vec![];
    for p in m.points.iter().filter(|p| p.track >= 3) {
        let mut rays = vec![];
        let mut obs = vec![];
        for &(img, k) in &p.observations {
            let im = &m.images[&img];
            let Some(g) = gt_image(&im.name) else {
                continue;
            };
            let px = im.keypoints[k as usize];
            let c = &gt.cameras[&g.camera];
            rays.push(camera::ray(c, g, px).unwrap());
            obs.push((c, g, px));
        }
        let Some((x, angle)) = camera::triangulate(&rays) else {
            continue;
        };
        if angle < 10.0 {
            continue;
        }
        // Reprojected into the ground-truth images: a check on the camera models and on the
        // match itself.
        let worst = obs
            .iter()
            .filter_map(|(c, g, px)| {
                camera::project_world(c, g, x)
                    .unwrap()
                    .map(|q| ((q[0] - px[0]).powi(2) + (q[1] - px[1]).powi(2)).sqrt())
            })
            .fold(0.0f64, f64::max);
        residuals.push(worst);
        if worst < 2.0 {
            pts.push((p.xyz, x));
        }
    }
    residuals.sort_by(f64::total_cmp);
    println!(
        "{} points triangulated with the ground-truth cameras (track >= 3, angle >= 10°); worst reprojection per point: median {:.2} px; {} kept under 2 px",
        residuals.len(),
        residuals[residuals.len() / 2],
        pts.len()
    );
    assert!(pts.len() >= 200, "only {} points", pts.len());
    // About 400 points spread over the scene, so the pairs don't run to millions.
    let sample: Vec<_> = pts.iter().step_by((pts.len() / 400).max(1)).collect();
    // Three known distances of 2 to 3 m, spread through the sample, measured to 1 mm.
    let mut tapes = vec![];
    'outer: for i in (0..sample.len()).step_by(sample.len() / 3) {
        for j in 0..sample.len() {
            let d = dist(sample[i].1, sample[j].1);
            if (2.0..=3.0).contains(&d) {
                tapes.push((i, j));
                continue 'outer;
            }
        }
    }
    let known: Vec<_> = tapes
        .iter()
        .map(|&(i, j)| scale::KnownDistance {
            a: sample[i].0,
            b: sample[j].0,
            length: dist(sample[i].1, sample[j].1),
            sigma: 0.001,
        })
        .collect();
    let s = scale::scale_from_distances(&known).unwrap();
    let mut errs = vec![];
    for i in 0..sample.len() {
        for j in i + 1..sample.len() {
            let g = dist(sample[i].1, sample[j].1);
            if !tapes.contains(&(i, j)) && g >= 0.5 {
                errs.push(((s.scale * dist(sample[i].0, sample[j].0) - g) / g * 100.0).abs());
            }
        }
    }
    // For the measurement uncertainty model: the RMS relative error of those distances, and
    // each point's error after the scaled model is placed on the truth (rotation and
    // translation fitted, scale kept), per axis.
    let rms_rel = (errs.iter().map(|e| e * e).sum::<f64>() / errs.len() as f64).sqrt();
    let (mm, ww): (Vec<[f64; 3]>, Vec<[f64; 3]>) = sample
        .iter()
        .map(|(m, w)| (m.map(|v| v * s.scale), *w))
        .unzip();
    let place = scale::fit_gcps(&mm, &ww, &[]).unwrap();
    let axis = place.rms / 3f64.sqrt();
    println!(
        "RMS relative distance error {rms_rel:.3} %; point error after placing (similarity fit scale {:.5}): RMS {:.2} mm, {:.2} mm per axis",
        place.transform.scale,
        place.rms * 1000.0,
        axis * 1000.0
    );
    errs.sort_by(f64::total_cmp);
    let q = |f: f64| errs[((errs.len() - 1) as f64 * f) as usize];
    let within = errs.iter().filter(|e| **e <= 1.0).count() as f64 / errs.len() as f64 * 100.0;
    println!(
        "{} known distances ({}; scale ±{:.2} %, the tapes agreeing to {:?} %); {} check distances >= 0.5 m: median {:.3} %, 95th percentile {:.3} %, 99th {:.3} %, worst {:.3} %; {within:.2} % within 1 %",
        known.len(),
        known.iter().map(|k| format!("{:.3} m", k.length)).collect::<Vec<_>>().join(", "),
        s.sigma / s.scale * 100.0,
        s.deviations_pct.iter().map(|d| (d * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
        errs.len(),
        q(0.5),
        q(0.95),
        q(0.99),
        q(1.0)
    );
    assert!(q(0.99) <= 1.0, "99th percentile {} %", q(0.99));
}
