//! Camera-match and witness-perspective reports, built only from the stored runs (so they can
//! be reprinted from the record at any time and say exactly what was computed).

use crate::analysis::{details, Area, Block, Figure, Frame, Label, Meta, Report, Section};
use locus_analysis::camera::{Run, WitnessRun, MIN_BLOCKING};

const RESIDUAL: &str = "#c0392b";
const SUBJECT: &str = "#1e8449";
const CLEAR: &str = "#1e8449";
const BLOCKED: &str = "#c0392b";

/// Residual vectors in the image figure are drawn this many times longer than they are.
const RESIDUAL_SCALE: f64 = 20.0;

/// A head ray passing this far from the vertical through the feet point is warned about.
const MISS_WARN: f64 = 0.05;

fn xyz(p: [f64; 3]) -> String {
    format!("{:.3}, {:.3}, {:.3}", p[0], p[1], p[2])
}

fn header(meta: &Meta) -> String {
    format!(
        "{}{} · analysis {} · {}",
        meta.case_number
            .as_ref()
            .map(|c| format!("{c} · "))
            .unwrap_or_default(),
        meta.project,
        meta.record_id,
        meta.method
    )
}

fn tail(sections: &mut Vec<Section>, assumptions: &[String], limitations: &[String]) {
    sections.push(Section {
        heading: "Assumptions".into(),
        blocks: vec![Block::List {
            items: assumptions.to_vec(),
        }],
    });
    sections.push(Section {
        heading: "Limitations".into(),
        blocks: vec![Block::List {
            items: limitations.to_vec(),
        }],
    });
    sections.push(Section {
        heading: "Sign-off".into(),
        blocks: vec![Block::SignOff {
            rows: vec!["Examiner".into(), "Technical review".into()],
        }],
    });
}

const REFERENCES: &[&str] = &[
    "R. Hartley and A. Zisserman, Multiple View Geometry in Computer Vision, 2nd ed., Cambridge University Press, 2004: the normalised DLT, its decomposition, and reprojection-error refinement.",
    "Z. Zhang, \"A flexible new technique for camera calibration\", IEEE Transactions on Pattern Analysis and Machine Intelligence, 22(11), 2000, 1330–1334.",
    "D. C. Brown, \"Decentering distortion of lenses\", Photogrammetric Engineering, 32(3), 1966, 444–462: the radial and tangential lens model.",
    "A. Criminisi, I. Reid and A. Zisserman, \"Single view metrology\", International Journal of Computer Vision, 40(2), 2000, 123–148: heights from a calibrated view.",
    "R. Drillis and R. Contini, Body Segment Parameters, Technical Report 1166.03, New York University, 1966; as reproduced in D. A. Winter, Biomechanics and Motor Control of Human Movement, 4th ed., Wiley, 2009, Fig. 4.1: the person model's proportions.",
    "R. T. Birge, \"The calculation of errors by the method of least squares\", Physical Review, 40, 1932, 207–227: inflating the uncertainty when residuals exceed their stated σ.",
];

/// The files the camera report embeds (its photo), with their recorded hashes.
pub fn photo_files(r: &Run) -> Vec<(String, String)> {
    r.photo
        .iter()
        .map(|p| (p.file.clone(), p.sha256.clone()))
        .collect()
}

pub fn report(meta: &Meta, r: &Run) -> Report {
    let s = &r.solve;
    let c = &s.camera;
    let p = &r.parameters;
    let mut warnings = vec![];
    if let Some(w) = &meta.withdrawn {
        warnings.push(format!("This analysis was withdrawn: {w}."));
    }
    warnings.extend(s.warnings.iter().cloned());
    for h in &r.heights {
        if h.miss > MISS_WARN {
            warnings.push(format!(
                "{}: the head ray passes {:.0} mm from the vertical through the feet point. The subject may not be standing upright over it, or the feet point is off.",
                h.input.label,
                h.miss * 1000.0
            ));
        }
        if h.height.sigma > 0.02 {
            warnings.push(format!(
                "{}: the height is poorly determined (1σ {:.0} mm). A better camera solve (more pairs, spread over the image and at the subject's depth) would narrow it.",
                h.input.label,
                h.height.sigma * 1000.0
            ));
        }
        if h.failed * 100 > h.draws + h.failed {
            warnings.push(format!(
                "{}: {} of {} Monte Carlo draws had no answer (the feet ray missed the floor) and were left out.",
                h.input.label,
                h.failed,
                h.draws + h.failed
            ));
        }
    }
    let [hd, pt, rl] = c.angles();
    let [fh, fv] = c.fov();
    let k = &c.distortion;
    let ks = &s.distortion_sigma;
    let result = vec![
        [
            "Camera position".into(),
            format!(
                "{} m (1σ {:.3}, {:.3}, {:.3} m)",
                xyz(c.position),
                s.position_sigma[0],
                s.position_sigma[1],
                s.position_sigma[2]
            ),
        ],
        [
            "Heading, pitch, roll".into(),
            format!(
                "{:.2}° ± {:.2}°, {:.2}° ± {:.2}°, {:.2}° ± {:.2}°",
                hd, s.angles_sigma[0], pt, s.angles_sigma[1], rl, s.angles_sigma[2]
            ),
        ],
        [
            "Focal length".into(),
            format!(
                "{:.1} ± {:.1} px (field of view {:.1}° × {:.1}° without distortion)",
                c.f, s.f_sigma, fh, fv
            ),
        ],
        [
            "Principal point".into(),
            format!(
                "{:.1}, {:.1} px{}",
                c.cx,
                c.cy,
                if s.principal_sigma == [0.0, 0.0] {
                    " (held at the image centre)".to_string()
                } else {
                    format!(
                        " (1σ {:.1}, {:.1} px)",
                        s.principal_sigma[0], s.principal_sigma[1]
                    )
                }
            ),
        ],
        [
            "Distortion k1, k2, k3; p1, p2".into(),
            format!(
                "{:.4} ± {:.4}, {:.4} ± {:.4}, {:.4} ± {:.4}; {:.5} ± {:.5}, {:.5} ± {:.5}",
                k[0], ks[0], k[1], ks[1], k[2], ks[2], k[3], ks[3], k[4], ks[4]
            ),
        ],
        ["Lens model".into(), s.model.describe().into()],
        [
            "Fit".into(),
            format!(
                "{} pairs, {:.2} px RMS; χ² = {:.1} on {} degrees of freedom{}",
                r.pairs.len(),
                s.rms_px,
                s.chi2,
                s.dof,
                if s.birge > 1.0 {
                    format!("; uncertainties inflated by {:.2}", s.birge)
                } else {
                    String::new()
                }
            ),
        ],
    ];

    let heights: Vec<Vec<String>> = r
        .heights
        .iter()
        .map(|h| {
            vec![
                h.input.label.clone(),
                format!("{:.3} ± {:.3} m", h.height.value, h.height.sigma),
                format!("{:.3} to {:.3} m", h.interval95[0], h.interval95[1]),
                format!("{:.3}, {:.3}", h.feet[0], h.feet[1]),
                match h.input.matched_model {
                    Some(m) => format!("from a matched {m:.3} m person model"),
                    None => "clicked".into(),
                },
                format!("{:.0} mm", h.miss * 1000.0),
            ]
        })
        .collect();

    let pairs: Vec<Vec<String>> = r
        .pairs
        .iter()
        .zip(&s.residuals)
        .zip(&s.residual_sigmas)
        .enumerate()
        .map(|(i, ((q, d), z))| {
            vec![
                (i + 1).to_string(),
                format!("{:.1}, {:.1}", q.px[0], q.px[1]),
                xyz(q.world),
                format!("{:+.2}, {:+.2} ({:.1}σ)", d[0], d[1], z),
                q.source
                    .as_ref()
                    .map(|s| format!("{} #{} r{}", s.scan, s.index, s.revision))
                    .unwrap_or_default(),
            ]
        })
        .collect();

    let mut sections = vec![Section {
        heading: "Result".into(),
        blocks: vec![
            Block::Text {
                text: "The camera that took the photo, solved from points picked both in the photo and on the scan: where it was, which way it pointed, and its lens, each with 1σ from the fit.".into(),
            },
            Block::Pairs { rows: result },
            Block::Figure(image_figure(r)),
        ],
    }];
    if let Some(ph) = &r.photo {
        sections[0].blocks.push(Block::Image {
            file: ph.file.clone(),
            width: 170.0,
            caption: format!(
                "The photo: {} (evidence {}, SHA-256 {})",
                ph.name, ph.evidence_id, ph.sha256
            ),
        });
    }
    if !r.heights.is_empty() {
        sections.push(Section {
            heading: "Subject heights".into(),
            blocks: vec![
                Block::Text {
                    text: "Each subject's height by reverse projection: the ray through the point between the feet meets the floor, and the height is where the ray through the top of the head passes the vertical above that point. The uncertainty is from a Monte Carlo over the camera's covariance and each image point's pick uncertainty. It is the height of the top of what was marked (hair, headwear and footwear included), for the posture in the frame, not the subject's stature.".into(),
                },
                Block::Table {
                    widths: ["1fr", "auto", "auto", "auto", "1fr", "auto"]
                        .map(String::from)
                        .to_vec(),
                    head: [
                        "Subject",
                        "Height (1σ)",
                        "95 % interval",
                        "Feet at x, y (m)",
                        "Head point",
                        "Head ray off vertical",
                    ]
                    .map(String::from)
                    .to_vec(),
                    rows: heights,
                },
            ],
        });
    }
    sections.push(Section {
        heading: "Point pairs".into(),
        blocks: vec![
            Block::Text {
                text: "Each pixel in the photo and the scan point paired with it (project frame, m), the solved camera's reprojection residual (px, and in units of its σ), and the scan point it was resolved to (scan, record, cleanup revision).".into(),
            },
            Block::Table {
                widths: ["auto", "auto", "auto", "auto", "1fr"]
                    .map(String::from)
                    .to_vec(),
                head: ["#", "Pixel", "Scan point (m)", "Residual (px)", "Scan source"]
                    .map(String::from)
                    .to_vec(),
                rows: pairs,
            },
        ],
    });
    sections.push(Section {
        heading: "Method".into(),
        blocks: vec![
            Block::Text {
                text: "The camera is started from the direct linear transform on the pairs nearest the image centre (normalised, then decomposed into the camera's intrinsics, rotation and position) and refined by Levenberg–Marquardt on the reprojection errors, each in units of its σ (the pick σ, and the scan point's σ projected at its depth), in stages: the pose and focal length, then the distortion terms the lens model has. The covariance is (JᵀJ)⁻¹ at the solution, inflated by the Birge ratio √(χ²/dof) when that exceeds 1. Heights are by reverse projection onto the floor plane, with 2,000-draw Monte Carlo uncertainty (seeded, so a run repeats exactly). See docs/methods/camera-height.md.".into(),
            },
            Block::Pairs {
                rows: vec![
                    ["Pick uncertainty".into(), format!("{:.2} px (1σ)", p.pick_sigma_px)],
                    ["Scan point uncertainty".into(), format!("{:.1} mm (1σ)", p.point_sigma * 1000.0)],
                    ["Floor".into(), format!("z = {:.3} m", p.floor_z)],
                    ["Monte Carlo".into(), format!("{} draws (seed {})", p.draws, p.seed)],
                ],
            },
            Block::List {
                items: REFERENCES.iter().map(|s| s.to_string()).collect(),
            },
        ],
    });
    tail(&mut sections, &r.assumptions, &r.limitations);
    Report {
        title: format!("Camera match: {}", meta.name),
        header: header(meta),
        details: details(meta),
        warnings,
        sections,
    }
}

/// The image frame: each pair's pixel, its residual drawn `RESIDUAL_SCALE` times longer, and
/// the subjects' feet and head points.
fn image_figure(r: &Run) -> Figure {
    let [w, h] = r.solve.camera.size.map(|v| v as f64);
    let fw = 170.0;
    let fh = (fw * h / w).min(110.0);
    let f = Frame::new([0.0, 0.0], [w, h], fw, fh, 2.0);
    // Image y runs down; the frame's y runs up.
    let at = |p: [f64; 2]| f.at([p[0], h - p[1]]);
    let mut fig = Figure {
        width: fw,
        height: fh,
        caption: format!(
            "The image frame ({} × {} px): each pair's pixel (dot, numbered), with its reprojection residual drawn {} times longer (red); subjects' feet and head points, joined (green, dashed).",
            w, h, RESIDUAL_SCALE
        ),
        ..Figure::default()
    };
    let corners = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
    for i in 0..4 {
        fig.line(at(corners[i]), at(corners[(i + 1) % 4]), 0.2, false);
    }
    for (i, (q, d)) in r.pairs.iter().zip(&r.solve.residuals).enumerate() {
        let a = at(q.px);
        fig.dots.push([a[0], a[1], 0.5]);
        let b = at([
            q.px[0] + d[0] * RESIDUAL_SCALE,
            q.px[1] + d[1] * RESIDUAL_SCALE,
        ]);
        fig.coloured(a, b, 0.3, false, RESIDUAL);
        fig.labels.push(Label {
            x: a[0] + 1.0,
            y: a[1] - 1.0,
            text: (i + 1).to_string(),
        });
    }
    for s in &r.heights {
        let (a, b) = (at(s.input.feet_px), at(s.input.head_px));
        fig.coloured(a, b, 0.3, true, SUBJECT);
        fig.dots.push([a[0], a[1], 0.6]);
        fig.dots.push([b[0], b[1], 0.6]);
        fig.labels.push(Label {
            x: b[0] + 1.0,
            y: b[1] - 1.0,
            text: s.input.label.clone(),
        });
    }
    fig
}

pub fn witness_report(meta: &Meta, r: &WitnessRun) -> Report {
    let mut warnings = vec![];
    if let Some(w) = &meta.withdrawn {
        warnings.push(format!("This analysis was withdrawn: {w}."));
    }
    let rows: Vec<Vec<String>> = r
        .sights
        .iter()
        .map(|s| {
            vec![
                s.label.clone(),
                xyz(s.to),
                format!("{:.2} m", s.length),
                if s.clear {
                    "clear".into()
                } else {
                    format!(
                        "blocked at {:.2} m from the eye ({} scan points)",
                        s.first_distance.unwrap_or(0.0),
                        s.blocking
                    )
                },
                s.target_source
                    .as_ref()
                    .map(|p| format!("{} #{} r{}", p.scan, p.index, p.revision))
                    .unwrap_or_default(),
            ]
        })
        .collect();
    let radius = r.sights.first().map_or(0.03, |s| s.radius);
    let clearance = r.sights.first().map_or(0.1, |s| s.end_clearance);
    let mut sections = vec![
        Section {
            heading: "Eye position".into(),
            blocks: vec![Block::Pairs {
                rows: vec![
                    ["Eye".into(), format!("{} m (project frame)", xyz(r.eye))],
                    [
                        "Standing on".into(),
                        format!(
                            "{} m{}",
                            xyz(r.floor_point),
                            r.floor_source
                                .as_ref()
                                .map(|p| format!(" (scan {} #{} r{})", p.scan, p.index, p.revision))
                                .unwrap_or_default()
                        ),
                    ],
                    [
                        "Eye height".into(),
                        format!("{:.2} m above that point (as stated by the examiner)", r.eye_height),
                    ],
                    ["Looking toward".into(), format!("{} m", xyz(r.look_at))],
                    ["Field of view".into(), format!("{:.0}° horizontal", r.fov_deg)],
                ],
            }],
        },
        Section {
            heading: "Lines of sight".into(),
            blocks: vec![
                Block::Text {
                    text: format!(
                        "A line of sight from the eye to each target is blocked where at least {MIN_BLOCKING} scan points lie within {:.0} mm of it, ignoring the first and last {:.0} mm (the surfaces its ends are on).",
                        radius * 1000.0,
                        clearance * 1000.0
                    ),
                },
                Block::Table {
                    widths: ["1fr", "auto", "auto", "1fr", "auto"]
                        .map(String::from)
                        .to_vec(),
                    head: ["Target", "At (m)", "Distance", "Result", "Scan source"]
                        .map(String::from)
                        .to_vec(),
                    rows,
                },
                Block::Figure(witness_plan(r)),
            ],
        },
    ];
    tail(&mut sections, &r.assumptions, &r.limitations);
    Report {
        title: format!("Witness perspective: {}", meta.name),
        header: header(meta),
        details: details(meta),
        warnings,
        sections,
    }
}

fn witness_plan(r: &WitnessRun) -> Figure {
    let mut pts = vec![[r.eye[0], r.eye[1]], [r.look_at[0], r.look_at[1]]];
    pts.extend(r.sights.iter().map(|s| [s.to[0], s.to[1]]));
    let lo = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
    let hi = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max));
    let (w, h) = (170.0, 80.0);
    let f = Frame::new(lo, hi, w, h, 8.0);
    let mut fig = Figure {
        width: w,
        height: h,
        caption: "Plan (x east, y north): the eye, its view (shaded, the field of view toward where it looks), and each line of sight, green when clear and red when blocked, with the first obstruction marked.".into(),
        ..Figure::default()
    };
    // The field of view as a wedge 1.5 × the distance to the look-at point.
    let (ex, ey) = (r.eye[0], r.eye[1]);
    let (dx, dy) = (r.look_at[0] - ex, r.look_at[1] - ey);
    let reach = dx.hypot(dy).max(1.0);
    let ang = dy.atan2(dx);
    let half = (r.fov_deg / 2.0).to_radians();
    let wedge: Vec<[f64; 2]> = std::iter::once([ex, ey])
        .chain((0..=16).map(|i| {
            let a = ang - half + 2.0 * half * i as f64 / 16.0;
            [ex + reach * a.cos(), ey + reach * a.sin()]
        }))
        .map(|p| f.at(p))
        .collect();
    fig.areas.push(Area {
        points: wedge,
        fill: "#e8eef7".into(),
    });
    let e = f.at([ex, ey]);
    for s in &r.sights {
        let t = f.at([s.to[0], s.to[1]]);
        fig.coloured(e, t, 0.3, false, if s.clear { CLEAR } else { BLOCKED });
        fig.dots.push([t[0], t[1], 0.6]);
        fig.labels.push(Label {
            x: t[0] + 1.0,
            y: t[1] - 1.0,
            text: s.label.clone(),
        });
        if let Some(b) = s.first {
            let q = f.at([b[0], b[1]]);
            fig.dots.push([q[0], q[1], 0.9]);
        }
    }
    fig.dots.push([e[0], e[1], 1.0]);
    fig.labels.push(Label {
        x: e[0] + 1.5,
        y: e[1] - 1.5,
        text: "eye".into(),
    });
    fig.scale_bar(&f);
    fig
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::pdf;
    use locus_analysis::camera::{
        line_of_sight, run, witness, Camera, HeightInput, LensModel, PairInput, Parameters,
    };
    use locus_analysis::trajectory::PointSource;

    fn meta(method: &str) -> Meta {
        Meta {
            project: "Main Street".into(),
            record_id: 7,
            name: "Hallway CCTV".into(),
            method: method.into(),
            sha256: "cd".repeat(32),
            revises: None,
            created_at: "2026-09-23T10:00:00Z".into(),
            created_by: "Examiner".into(),
            withdrawn: None,
            printed_by: "Examiner".into(),
            printed_at: "2026-09-23T10:05:00Z".into(),
            app_version: "0.1.0".into(),
            audit_head: "ef".repeat(32),
            case_number: Some("2026-00417".into()),
        }
    }

    fn sample() -> Run {
        let (h, p) = (52f64.to_radians(), (-24f64).to_radians());
        let fwd = [h.sin() * p.cos(), h.cos() * p.cos(), p.sin()];
        let right = [h.cos(), -h.sin(), 0.0];
        let down = [
            fwd[1] * right[2] - fwd[2] * right[1],
            fwd[2] * right[0] - fwd[0] * right[2],
            fwd[0] * right[1] - fwd[1] * right[0],
        ];
        let cam = Camera {
            position: [0.3, 0.3, 2.7],
            rotation: [right, down, fwd],
            size: [1280, 720],
            f: 620.0,
            cx: 640.0,
            cy: 360.0,
            distortion: [-0.2, 0.05, 0.0, 0.0, 0.0],
        };
        let mut pairs = vec![];
        for k in 0..40u32 {
            let a = (k * 37 % 97) as f64 / 97.0;
            let b = (k * 61 % 89) as f64 / 89.0;
            let w = match k % 3 {
                0 => [0.5 + 7.0 * a, 0.5 + 5.0 * b, 0.0],
                1 => [8.0, 0.5 + 5.0 * a, 0.2 + 2.6 * b],
                _ => [0.5 + 7.0 * a, 6.0, 0.2 + 2.6 * b],
            };
            if let Some(q) = cam.project(w) {
                if q[0] > 0.0 && q[1] > 0.0 && q[0] < 1280.0 && q[1] < 720.0 && pairs.len() < 16 {
                    let e = ((k * 13 % 7) as f64 - 3.0) * 0.15;
                    pairs.push(PairInput {
                        px: [q[0] + e, q[1] - e],
                        world: w,
                        source: Some(PointSource {
                            scan: "1:0".into(),
                            index: 100 + k,
                            revision: 0,
                        }),
                    });
                }
            }
        }
        let subjects = [HeightInput {
            label: "Subject A".into(),
            feet_px: cam.project([3.2, 3.4, 0.0]).unwrap(),
            head_px: cam.project([3.2, 3.4, 1.78]).unwrap(),
            matched_model: None,
        }];
        run(
            None,
            pairs,
            cam.size,
            Parameters {
                model: LensModel::Radial2,
                pick_sigma_px: 0.5,
                draws: 300,
                ..Parameters::default()
            },
            &subjects,
        )
        .unwrap()
    }

    #[test]
    fn the_camera_report_says_what_the_run_computed() {
        let r = sample();
        let doc = report(&meta("camera/1"), &r);
        let out = pdf(&doc, vec![]).unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
        let text = out.text;
        assert!(text.contains("2026-00417"));
        assert!(text.contains(&xyz(r.solve.camera.position)), "{text}");
        assert!(text.contains("Subject A"));
        assert!(text.contains(&format!("{:.3}", r.heights[0].height.value)));
        let src = r.pairs[0].source.as_ref().unwrap();
        assert!(text.contains(&format!("1:0 #{} r0", src.index)));
        assert!(text.contains("Criminisi") && text.contains("Drillis"));
        assert!(text.contains("Technical review"));
        let tmp = std::env::temp_dir();
        std::fs::write(tmp.join("locus-camera.pdf"), &out.pdf).unwrap();
    }

    #[test]
    fn the_witness_report_marks_each_line_clear_or_blocked() {
        let wall: Vec<[f64; 3]> = (0..150)
            .flat_map(|i| (0..150).map(move |j| [2.0, i as f64 * 0.02, j as f64 * 0.02]))
            .collect();
        let a = line_of_sight("Door", [0.0, 1.0, 1.6], [4.0, 1.0, 1.0], &wall, 0.03, 0.1);
        let b = line_of_sight("Table", [0.0, 1.0, 1.6], [1.5, 2.0, 0.8], &wall, 0.03, 0.1);
        let w = witness([0.0, 1.0, 0.0], 1.6, [3.0, 1.0, 1.5], 60.0, vec![a, b]);
        let doc = witness_report(&meta("witness/1"), &w);
        let text = pdf(&doc, vec![]).unwrap().text;
        assert!(
            text.contains("blocked at 2.0") && text.contains("clear"),
            "{text}"
        );
        assert!(text.contains("1.60 m above"));
    }
}
