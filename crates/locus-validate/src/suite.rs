//! The validation suite: every tool run end to end against synthetic ground truth, many times,
//! with its errors' mean, spread, 95th percentile and worst, the bound it is held to, and how
//! often its stated 95 % interval covered the truth. `locus-validate run` prints it as a PDF
//! for each release (docs/methods/validation.md).

use locus_analysis::measure::Measured;
use std::time::Instant;

/// One tool's result over its runs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolResult {
    pub tool: String,
    /// What each error is ("origin error", "direction error").
    pub what: String,
    pub unit: String,
    /// Absolute errors, one per case, in `unit`.
    pub errors: Vec<f64>,
    /// The bound held to, and whether it was met.
    pub bound: Option<(String, bool)>,
    /// How often the stated 95 % interval covered the truth: (inside, cases).
    pub coverage: Option<(usize, usize)>,
    pub notes: Vec<String>,
    pub seconds: f64,
}

impl ToolResult {
    pub fn mean(&self) -> f64 {
        self.errors.iter().sum::<f64>() / self.errors.len().max(1) as f64
    }
    pub fn sd(&self) -> f64 {
        let m = self.mean();
        let n = self.errors.len();
        if n < 2 {
            return 0.0;
        }
        (self.errors.iter().map(|e| (e - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
    }
    pub fn max(&self) -> f64 {
        self.errors.iter().cloned().fold(0.0, f64::max)
    }
    pub fn p95(&self) -> f64 {
        let mut e = self.errors.clone();
        e.sort_by(f64::total_cmp);
        e.get(((e.len() as f64 * 0.95).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0.0)
    }
    pub fn passed(&self) -> bool {
        self.bound.as_ref().is_none_or(|b| b.1)
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    (dot(a, b) / (norm(a) * norm(b)))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// A small deterministic generator for the suite's own noise.
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

// ---------- trajectory ----------

pub fn trajectory(runs: u64) -> ToolResult {
    use locus_analysis::trajectory::{fit_line, PathPoint, CHI2_2DOF_95};
    use locus_synth::trajectory::{self as gen, Options};
    let t0 = Instant::now();
    let (mut errors, mut inside) = (vec![], 0);
    for seed in 1..=runs {
        let t = gen::truth(&Options {
            seed,
            ..Options::default()
        });
        let s = (2.0 * t.options.pick_sigma.powi(2) + t.options.scan_sigma.powi(2)).sqrt()
            / 3f64.sqrt();
        let pts: Vec<PathPoint> = t
            .panels
            .iter()
            .flat_map(|p| [p.entry_picked, p.exit_picked])
            .map(|point| PathPoint { point, sigma: s })
            .collect();
        let l = fit_line(&pts, 0.0).expect("fit");
        errors.push(deg(l.direction, t.direction));
        // Inside the 95 % cone: the error's Mahalanobis distance² in the cone's plane.
        let e = sub(t.direction, l.direction);
        let (u, d) = (l.cone.major_axis, l.direction);
        let v = [
            u[1] * d[2] - u[2] * d[1],
            u[2] * d[0] - u[0] * d[2],
            u[0] * d[1] - u[1] * d[0],
        ];
        let q = |x: [f64; 3]| -> f64 {
            (0..3)
                .map(|i| {
                    (0..3)
                        .map(|j| x[i] * l.covariance[i][j] * x[j])
                        .sum::<f64>()
                })
                .sum()
        };
        if dot(e, u).powi(2) / q(u) + dot(e, v).powi(2) / q(v) <= CHI2_2DOF_95 {
            inside += 1;
        }
    }
    let r = ToolResult {
        tool: "Bullet trajectory".into(),
        what: "direction error".into(),
        unit: "°".into(),
        bound: None,
        coverage: Some((inside, runs as usize)),
        notes: vec![
            "Synthetic rooms with several perforated panels; each defect picked with the generator's picking and scan noise; the line fitted through them as the app does.".into(),
            "Coverage: the truth inside the stated 95 % cone.".into(),
        ],
        seconds: t0.elapsed().as_secs_f64(),
        errors,
    };
    let worst = r.max();
    ToolResult {
        bound: Some(("every run under 0.5° (SPEC Phase 13)".into(), worst < 0.5)),
        ..r
    }
}

// ---------- bloodstain ----------

/// The stains as an examiner measured them by hand (the generator's noise: 0.1 mm + 1.5 % per
/// edge), with that noise as their stated uncertainty.
fn hand_stains(t: &locus_synth::bloodstain::Truth) -> Vec<locus_analysis::bloodstain::StainInput> {
    use locus_analysis::bloodstain::StainInput;
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

fn within(e: &locus_analysis::bloodstain::Ellipsoid, centre: [f64; 3], truth: [f64; 3]) -> bool {
    let d = sub(truth, centre);
    (0..3)
        .map(|k| (dot(d, e.axes[k]) / e.semi_axes[k]).powi(2))
        .sum::<f64>()
        <= 1.0
}

/// The stains measured the way the app does from each stain's photo: the photo aligned by its
/// fiducials, the edges found automatically, the ellipse fitted, the tail's tip marked.
fn photo_stains(
    o: &locus_synth::bloodstain::Options,
    t: &locus_synth::bloodstain::Truth,
) -> Option<Vec<locus_analysis::bloodstain::StainInput>> {
    use locus_analysis::bloodstain::{align_photo, stain_edges, stain_from_photo, AlignPair};
    use locus_synth::bloodstain as gen;
    let mut out = vec![];
    for (i, s) in t.stains.iter().enumerate() {
        let rgb = gen::photo(o, s);
        let [w, h] = s.photo.size_px.map(|v| v as usize);
        let luma: Vec<u8> = rgb
            .chunks(3)
            .map(|p| {
                (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64).round() as u8
            })
            .collect();
        let pairs: Vec<AlignPair> = s
            .photo
            .fiducials
            .iter()
            .map(|f| AlignPair {
                px: f.px,
                world: f.world,
            })
            .collect();
        let al = align_photo(&pairs, s.centre, s.normal, 0.0).ok()?;
        let edges = stain_edges(&luma, w, h, [w as f64 / 2.0, h as f64 / 2.0], 126).ok()?;
        // The examiner marks the tail's tip: a little beyond the stain's leading end.
        let tip = [0, 1, 2].map(|k| s.centre[k] + s.travel[k] * s.length * 0.62);
        let d = sub(tip, s.centre);
        let half = w as f64 / 2.0;
        let tail = [
            half + dot(d, s.photo.u) * s.photo.scale,
            half - dot(d, s.photo.v) * s.photo.scale,
        ];
        out.push(stain_from_photo(&format!("{i}"), &s.surface, al, edges, tail).ok()?);
    }
    Some(out)
}

/// The area of origin: from stains measured on their photos (`photos`, the SPEC's clean
/// synthetic data), or by hand. Also the conventional point on the same stains.
pub fn bloodstain(runs: u64, photos: bool) -> Vec<ToolResult> {
    use locus_analysis::bloodstain::{run, Parameters};
    use locus_synth::bloodstain::{self as gen, Options};
    let t0 = Instant::now();
    let (mut errors, mut inside, mut conv, mut conv_in, mut failed) = (vec![], 0, vec![], 0, 0);
    for seed in 1..=runs {
        let o = Options {
            seed,
            ..Options::default()
        };
        let t = gen::truth(&o);
        let inputs = if photos {
            match photo_stains(&o, &t) {
                Some(v) => v,
                None => {
                    failed += 1;
                    continue;
                }
            }
        } else {
            hand_stains(&t)
        };
        let p = Parameters {
            bootstrap: 500,
            ..Parameters::default()
        };
        let Ok(r) = run(inputs, p) else {
            failed += 1;
            continue;
        };
        errors.push(norm(sub(r.origin.point, t.options.origin)) * 100.0);
        inside += within(&r.origin.ellipsoid, r.origin.point, t.options.origin) as usize;
        if let Some(c) = &r.conventional {
            conv.push(norm(sub(c.point, t.options.origin)) * 100.0);
            conv_in += within(&c.ellipsoid, c.point, t.options.origin) as usize;
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    let label = if photos {
        "stains measured on photos"
    } else {
        "hand-measured stains"
    };
    let n = errors.len();
    let mut main = ToolResult {
        tool: format!("Bloodstain area of origin ({label})"),
        what: "origin error".into(),
        unit: "cm".into(),
        errors,
        bound: None,
        coverage: Some((inside, n)),
        notes: vec![
            format!("Synthetic rooms of stains on walls and floor from a known origin; {}.", if photos {
                "each stain measured as the app does from its rendered photo: aligned by its fiducials, edges found automatically, ellipse fitted, tail marked"
            } else {
                "each stain measured with hand measurement's noise (0.1 mm + 1.5 % per edge), so near-round stains' directions are uncertain by 15–40°: a poor case, where the check is that the stated region says so"
            }),
            "The origin is the angle-space fit, corrected for the bias the stated measurement noise gives it (method version 2, docs/methods/bloodstain.md). Coverage: the truth inside its stated 95 % region.".into(),
        ],
        seconds: secs,
    };
    if failed > 0 {
        main.notes.push(format!(
            "{failed} runs had too few usable stains and were refused (not counted)."
        ));
    }
    if photos {
        let mean = main.mean();
        main.bound = Some((
            "mean under 10 cm on clean synthetic data (SPEC Phase 13)".into(),
            mean < 10.0 && n > 0,
        ));
    }
    let c = ToolResult {
        tool: format!("Bloodstain, conventional point ({label})"),
        what: "origin error".into(),
        unit: "cm".into(),
        coverage: Some((conv_in, conv.len())),
        errors: conv,
        bound: None,
        notes: vec!["The conventional point (least squares on perpendicular distances to the stains' straight paths), shown beside the angle fit on the same stains, as the report does. Its 95 % region isn't expected to cover well: the reason the angle fit is primary (docs/methods/bloodstain.md).".into()],
        seconds: 0.0,
    };
    vec![main, c]
}

// ---------- camera and height ----------

pub fn camera(seeds: u64) -> Vec<ToolResult> {
    use locus_analysis::camera::{height, solve, CameraPair, HeightInput, LensModel};
    use locus_synth::camera::{self as gen, Options};
    let t0 = Instant::now();
    let pick = 1.0;
    let (mut all, mut good, mut inside) = (vec![], vec![], 0);
    for (seed, markers) in (1..=seeds).flat_map(|s| [(s, 30), (s, 60)]) {
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
            let Ok(s) = solve(
                &pairs,
                cam.size,
                LensModel::Auto,
                o.pick_sigma_px,
                0.0,
                300,
                seed,
            ) else {
                continue;
            };
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
                let Ok(h) = height(&s, &input, 0.0, pick, seed) else {
                    continue;
                };
                let err = (h.height.value - p.height).abs() * 100.0;
                all.push(err);
                if h.height.sigma <= 0.01 {
                    good.push(err);
                }
                inside += (h.interval95[0] <= p.height && p.height <= h.interval95[1]) as usize;
            }
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    let n = all.len();
    let g = ToolResult {
        tool: "Height by reverse projection (good camera solves)".into(),
        what: "height error".into(),
        unit: "cm".into(),
        errors: good,
        bound: None,
        coverage: None,
        notes: vec!["A good solve is one that states the height to 1 cm (1σ) or better, the spec's \"good camera solve\".".into()],
        seconds: 0.0,
    };
    let p95 = g.p95();
    let g = ToolResult {
        bound: Some((
            "95 % of errors under 2 cm (SPEC Phase 13, as adopted in DECISIONS.md)".into(),
            p95 < 2.0,
        )),
        ..g
    };
    let a = ToolResult {
        tool: "Height by reverse projection (all solves)".into(),
        what: "height error".into(),
        unit: "cm".into(),
        errors: all,
        bound: None,
        coverage: Some((inside, n)),
        notes: vec![
            "Rendered CCTV (wide lens, strong distortion) and handheld views of rooms with 30 or 60 control markers and standing people of known height; each camera solved from its picked markers with the lens model chosen by leave-one-out and a 300-draw bootstrap; feet and head picked with 1 px noise.".into(),
            "Coverage: the truth inside the stated 95 % interval, over every solve, poor ones included.".into(),
        ],
        seconds: secs,
    };
    vec![g, a]
}

// ---------- registration ----------

pub fn registration(runs: u64, points_per_scan: u64) -> ToolResult {
    use locus_register::pipeline::{register, Params, ScanInput};
    use locus_synth::{scan_points, truth, Options, StoredPose, BOARD_SIZE, SPHERE_RADIUS};
    use nalgebra::{Isometry3, Point3, Quaternion, Translation3, UnitQuaternion, Vector3};
    let iso = |p: &locus_synth::Pose| {
        let q = p.rotation;
        Isometry3::from_parts(
            Translation3::from(Vector3::from(p.translation)),
            UnitQuaternion::from_quaternion(Quaternion::new(q[0], q[1], q[2], q[3])),
        )
    };
    let t0 = Instant::now();
    let (mut errors, mut notes) = (vec![], vec![]);
    let mut angle = 0.0f64;
    for seed in 1..=runs {
        let opts = Options {
            scans: 4,
            points_per_scan,
            seed: 20 + seed,
            stored_pose: StoredPose::Perturbed {
                metres: 0.5,
                degrees: 5.0,
            },
            ..Options::default()
        };
        let t = truth(&opts);
        let scans: Vec<ScanInput> = (0..opts.scans)
            .map(|s| {
                let (mut points, mut intensity) = (vec![], vec![]);
                scan_points(&opts, &t, s, &mut |p| {
                    if let Some(p) = p {
                        points.push(p.xyz);
                        intensity.push(p.intensity as f64);
                    }
                });
                ScanInput { points, intensity }
            })
            .collect();
        let rough: Vec<Isometry3<f64>> = t
            .scans
            .iter()
            .map(|s| iso(&s.stored_pose.expect("rough pose")))
            .collect();
        let params = Params {
            sphere_radius: Some(SPHERE_RADIUS),
            board_size: Some(BOARD_SIZE),
            ..Params::default()
        };
        let reg = match register(&scans, Some(&rough), &[], &params) {
            Ok(r) => r,
            Err(e) => {
                notes.push(format!("seed {}: registration failed: {e:?}", 20 + seed));
                continue;
            }
        };
        // Worst over the scans (relative to the first) at up to 10 m from the scanner.
        let mut d = 0f64;
        for s in 1..t.scans.len() {
            let tr = iso(&t.scans[0].pose).inverse() * iso(&t.scans[s].pose);
            let est = reg.solution.poses[0].inverse() * reg.solution.poses[s];
            let e = est.inverse() * tr;
            for probe in [
                Point3::origin(),
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(0.0, 10.0, 0.0),
                Point3::new(0.0, 0.0, 3.0),
            ] {
                d = d.max((e * probe - probe).norm());
            }
            angle = angle.max(e.rotation.angle().to_degrees());
        }
        errors.push(d * 1000.0);
    }
    let r = ToolResult {
        tool: "Scan registration".into(),
        what: "worst error at up to 10 m from a scanner, per run".into(),
        unit: "mm".into(),
        bound: None,
        coverage: None,
        notes: {
            notes.insert(0, format!(
                "Synthetic four-station rooms of {points_per_scan} points per scan with spheres and checkerboards, 1 mm noise, and rough poses off by 0.5 m and 5° (as a scanner's on-site pre-registration gives); targets and cloud-to-cloud links in a pose graph, as the app does. Worst rotation error over all runs: {angle:.4}°."
            ));
            notes
        },
        seconds: t0.elapsed().as_secs_f64(),
        errors,
    };
    let worst = r.max();
    ToolResult {
        bound: Some((
            "every run under 2 mm (SPEC Phase 13)".into(),
            worst < 2.0 && !r.errors.is_empty(),
        )),
        ..r
    }
}

// ---------- crush volume ----------

pub fn crush_volume(runs: u64) -> ToolResult {
    use locus_analysis::crush_volume::{crush_volume as volume, PoseUncertainty};
    use locus_register::exemplar::align_exemplar;
    use locus_synth::crush::vehicle;
    use nalgebra::{Isometry3, Point3, Translation3, UnitQuaternion, Vector3};
    let t0 = Instant::now();
    let (mut errors, mut inside) = (vec![], 0);
    for seed in 0..runs {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (seed + 1));
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
        let reference = vehicle(None, noise, 2 * seed);
        let damaged: Vec<[f64; 3]> = vehicle(Some((r, d)), noise, 2 * seed + 1)
            .iter()
            .map(|p| (pose * Point3::from(*p)).coords.into())
            .collect();
        let feats = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.6, 0.0],
            [0.0, 1.6, 1.0],
            [0.8, 0.0, 1.0],
        ];
        let mut pick = |p: [f64; 3]| {
            Point3::from(p) + Vector3::new(rng.gauss(), rng.gauss(), rng.gauss()) * 0.003
        };
        let pairs: Vec<_> = feats
            .iter()
            .map(|f| (pick(*f).coords.into(), (pose * pick(*f)).coords.into()))
            .collect();
        let corners: Vec<Point3<f64>> = [
            [-0.25, 0.8 - r - 0.1, 0.5 - r - 0.1],
            [0.25, 0.8 + r + 0.1, 0.5 + r + 0.1],
        ]
        .iter()
        .map(|p| pose * Point3::from(*p))
        .collect();
        let lo = std::array::from_fn(|k| corners[0][k].min(corners[1][k]));
        let hi = std::array::from_fn(|k| corners[0][k].max(corners[1][k]));
        let view = (pose * Point3::new(-5.0, 0.8, 1.5)).coords.into();
        let Ok(a) = align_exemplar(&reference, &damaged, view, &pairs, lo, hi, 0.02) else {
            continue;
        };
        let moved: Vec<[f64; 3]> = reference
            .iter()
            .map(|p| (a.transform * Point3::from(*p)).coords.into())
            .collect();
        let Ok(v) = volume(
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
            seed,
        ) else {
            continue;
        };
        errors.push((v.inward.value - truth_volume).abs() / truth_volume * 100.0);
        inside += (v.inward.interval95[0] <= truth_volume && truth_volume <= v.inward.interval95[1])
            as usize;
    }
    let n = errors.len();
    ToolResult {
        tool: "Volumetric crush".into(),
        what: "volume error".into(),
        unit: "% of the dent's volume".into(),
        bound: None,
        coverage: Some((inside, n)),
        notes: vec![
            "Synthetic vehicles with a spherical-cap dent of known volume; an exemplar registered onto the damaged scan by five picked pairs (3 mm picking error) and ICP around the damage; the volume's Monte Carlo interval. The interval is deliberately conservative (docs/methods/crash-volume.md).".into(),
        ],
        seconds: t0.elapsed().as_secs_f64(),
        errors,
    }
}

// ---------- crash formulas ----------

/// Closed-form checks worked by hand: the tools' values against the formula, to rounding.
pub fn crash() -> ToolResult {
    use locus_analysis::crash::{self as c, Input, SkidSegment, G};
    let t0 = Instant::now();
    let mut errors = vec![];
    let mut notes = vec![];
    // Skid to a stop: v = √(2 μ g d).
    for (d, mu) in [(30.0, 0.7), (12.5, 0.55), (45.0, 0.8)] {
        let r = c::skid(
            vec![SkidSegment {
                label: "s".into(),
                distance: Input::exact(d),
                drag: Input::exact(mu),
                braking: Input::exact(1.0),
                grade: Input::exact(0.0),
                path: vec![],
                sources: vec![],
            }],
            Input::exact(0.0),
            1000,
            1,
        )
        .expect("skid");
        let want = (2.0 * mu * G * d).sqrt();
        errors.push((r.speed.value - want).abs() / want * 100.0);
        notes.push(format!(
            "Skid {d} m at μ {mu}: {:.3} m/s; √(2μgd) = {want:.3} m/s.",
            r.speed.value
        ));
    }
    notes.push("Yaw, momentum and CRASH3 crush are checked against hand-worked examples and the NHTSA CRASH3 manual's sample run in the unit tests (locus-analysis). Published textbook cases are awaited from the examiner (docs/ADDISON-TODO.md).".into());
    ToolResult {
        tool: "Crash reconstruction formulas".into(),
        what: "difference from the closed form".into(),
        unit: "%".into(),
        bound: Some(("under 0.01 %".into(), errors.iter().all(|e| *e < 0.01))),
        coverage: None,
        notes,
        seconds: t0.elapsed().as_secs_f64(),
        errors,
    }
}

// ---------- animation ----------

/// An object at a set speed on a curved path is where it should be at every frame.
pub fn animation() -> ToolResult {
    use locus_analysis::motion::{Path, Profile, Shape, Track};
    let t0 = Instant::now();
    let pts: Vec<[f64; 3]> = (0..=8)
        .map(|k| {
            let a = k as f64 * 0.4;
            [20.0 * a.cos(), 20.0 * a.sin(), 0.0]
        })
        .collect();
    let path = Path::new(pts, Shape::Smooth).expect("path");
    let v = 13.9;
    let track = Track {
        path,
        profile: Profile::Constant { speed: v },
        start: 0.0,
        offset: 0.0,
    };
    let errors: Vec<f64> = track
        .frames(0.0, 4.0, 30.0)
        .iter()
        .map(|s| (s.distance - v * s.t).abs() * 1000.0)
        .collect();
    let worst = errors.iter().cloned().fold(0.0, f64::max);
    ToolResult {
        tool: "Animation motion".into(),
        what: "distance along the path from v·t, per frame".into(),
        unit: "mm".into(),
        bound: Some(("under 0.001 mm at every frame (SPEC Phase 9)".into(), worst < 0.001)),
        coverage: None,
        notes: vec!["50 km/h on a smooth curve through nine points of a 20 m circle, 30 frames per second over 4 s.".into()],
        seconds: t0.elapsed().as_secs_f64(),
        errors,
    }
}

/// Photogrammetry isn't rerun here: it needs COLMAP and the ETH3D benchmark, which stay
/// outside the repository. Its recorded result, stated as such.
pub fn photogrammetry_recorded() -> ToolResult {
    ToolResult {
        tool: "Photogrammetry (recorded, not rerun here)".into(),
        what: "99th percentile of distance errors, the worst of five runs".into(),
        unit: "%".into(),
        errors: vec![0.83],
        bound: Some(("99 % of distances within 1 % in every run (SPEC Phase 8)".into(), true)),
        coverage: None,
        notes: vec![
            "Recorded on 2026-09-23 (crates/locus-photo/tests/README.md): ETH3D \"pipes\" with laser-scan ground truth, COLMAP 4.2.0 with CPU features at 4800 px, five runs of about 95,000 check distances each. Worst single distance 0.97–1.43 %. Rerun with the heavy eth3d test when COLMAP and the benchmark are available.".into(),
        ],
        seconds: 0.0,
    }
}

/// Everything, at `full` size (for a release) or quick (a few runs each, for a check).
pub fn all(full: bool, progress: &mut dyn FnMut(&str)) -> Vec<ToolResult> {
    let mut out = vec![];
    let n = |quick: u64, full_n: u64| if full { full_n } else { quick };
    progress("trajectory");
    out.push(trajectory(n(40, 400)));
    progress("bloodstain");
    out.extend(bloodstain(n(2, 8), true));
    out.extend(bloodstain(n(10, 100), false));
    progress("camera and height");
    out.extend(camera(n(4, 40)));
    progress("registration");
    out.push(registration(
        n(1, 5),
        if full { 1_000_000 } else { 300_000 },
    ));
    progress("crush volume");
    out.push(crush_volume(n(4, 40)));
    progress("crash formulas");
    out.push(crash());
    progress("animation");
    out.push(animation());
    out.push(photogrammetry_recorded());
    out
}
