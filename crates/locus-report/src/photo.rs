//! The photogrammetry report, built only from the stored run.

use crate::analysis::{details, Block, Meta, Report, Section};
use locus_photo::record::{PhotoRun, Source};

const REFS: &[&str] = &[
    "J. L. Schönberger and J.-M. Frahm, \"Structure-from-Motion Revisited\", CVPR 2016: COLMAP's sparse reconstruction.",
    "J. L. Schönberger, E. Zheng, M. Pollefeys and J.-M. Frahm, \"Pixelwise View Selection for Unstructured Multi-View Stereo\", ECCV 2016: COLMAP's dense reconstruction.",
    "S. Umeyama, \"Least-squares estimation of transformation parameters between two point patterns\", IEEE Transactions on Pattern Analysis and Machine Intelligence 13(4), 1991: the similarity fit to control points.",
    "T. Schöps et al., \"A Multi-View Stereo Benchmark with High-Resolution Images and Multi-Camera Videos\", CVPR 2017 (ETH3D): the benchmark Locus's photogrammetry is validated on.",
];

pub fn report(meta: &Meta, r: &PhotoRun) -> Report {
    let sc = &r.scale;
    let u = &sc.uncertainty;
    let method = match sc.method.as_str() {
        "gps" => "the photos' GPS/RTK positions (similarity from the camera centres)",
        "distances" => "known distances clicked in the photos (scale only)",
        _ => "ground control points clicked in the photos (similarity)",
    };
    let result = vec![
        [
            "Point cloud".into(),
            format!(
                "{} points ({}), imported as evidence {} (SHA-256 {})",
                r.output.points, r.output.from, r.output.evidence_id, r.output.sha256
            ),
        ],
        [
            "Images placed".into(),
            format!("{} of {}", r.registered.len(), r.images_total),
        ],
        [
            "Sparse points".into(),
            format!(
                "{}; mean reprojection error {:.2} px",
                r.sparse_points, r.mean_error_px
            ),
        ],
        ["Scaled by".into(), method.into()],
        [
            "Measurement uncertainty".into(),
            format!(
                "every measurement on this point cloud takes 1σ = max({:.2} % of the length, {:.1} mm), from {} with the scaling's own uncertainty",
                u.percent * 100.0,
                u.floor * 1000.0,
                if u.from_checks {
                    "the case's check measurements (larger than the benchmark's)"
                } else {
                    "the ETH3D benchmark (0.35 %, 6 mm)"
                }
            ),
        ],
        [
            "Scale".into(),
            format!(
                "{:.6} (±{:.3} % 1σ); fit RMS {:.1} mm",
                sc.transform.scale,
                sc.scale_sigma_rel * 100.0,
                sc.rms * 1000.0
            ),
        ],
    ];
    let source = match &r.source {
        Source::Photos { items } => vec![Block::Table {
            widths: ["auto", "1fr", "1fr"].map(String::from).to_vec(),
            head: ["Evidence", "Image", "SHA-256"].map(String::from).to_vec(),
            rows: items
                .iter()
                .map(|i| vec![
                    i.evidence_id.to_string(),
                    format!("{}{}", i.name, if r.registered.contains(&i.name) { "" } else { " (not placed)" }),
                    i.sha256.clone(),
                ])
                .collect(),
        }],
        Source::Video { evidence_id, name, sha256, interval, first_timestamp, duration, frames } => vec![Block::Pairs {
            rows: vec![
                ["Video".into(), format!("{name}, evidence {evidence_id}, SHA-256 {sha256}")],
                ["Frames".into(), format!(
                    "{} sampled every {interval:.2} s of {duration:.1} s (times from the first frame, whose timestamp is {first_timestamp:.3} s), by Windows Media Foundation; {} placed",
                    frames.len(),
                    r.registered.len()
                )],
            ],
        }],
    };
    let rows = sc
        .rows
        .iter()
        .map(|w| {
            vec![
                format!("{}{}", w.label, if w.check { " (check)" } else { "" }),
                w.target.clone(),
                w.measured.clone(),
                if w.angle_deg > 0.0 {
                    format!("{:.1}°", w.angle_deg)
                } else {
                    String::new()
                },
                format!("{:.1}", w.residual * 1000.0),
                if w.check {
                    format!(
                        "{:.1}{}",
                        w.limit * 1000.0,
                        if w.exceeds { " EXCEEDED" } else { "" }
                    )
                } else {
                    String::new()
                },
            ]
        })
        .collect();

    let stages = r
        .stages
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                format!("{:.0} s", s.seconds),
                format!("colmap {}", s.args.join(" ")),
            ]
        })
        .collect();
    let mut scale_blocks = vec![];
    for n in &sc.notes {
        scale_blocks.push(Block::Text { text: n.clone() });
    }
    if let Some(o) = sc.enu_origin {
        scale_blocks.push(Block::Pairs {
            rows: vec![[
                "Local frame's origin".into(),
                format!("{:.8}°, {:.8}°, {:.3} m", o[0], o[1], o[2]),
            ]],
        });
    }
    scale_blocks.push(Block::Table {
        widths: ["1fr", "1fr", "1fr", "auto", "auto", "auto"]
            .map(String::from)
            .to_vec(),
        head: [
            "Target",
            "Known",
            "Measured",
            "Ray angle",
            "Residual (mm)",
            "Check's 95 % limit (mm)",
        ]
        .map(String::from)
        .to_vec(),
        rows,
    });
    scale_blocks.push(Block::Text {
        text: "Check targets (marked) are held out of the scaling. A check's 95 % limit combines its stated uncertainty with the measurement model's at that length (for a check point, in 3-D: 2.80σ per axis, from its coordinates' σ, a point's benchmark σ and the scale's at its distance from the control points' centre); a residual beyond it is flagged and warned.".into(),
    });
    let s = &r.settings;
    let sections = vec![
        Section { heading: "Result".into(), blocks: vec![Block::Pairs { rows: result }] },
        Section { heading: "Input".into(), blocks: source },
        Section { heading: "Scaling".into(), blocks: scale_blocks },
        Section {
            heading: "COLMAP".into(),
            blocks: vec![
                Block::Pairs {
                    rows: vec![
                        ["Program".into(), format!("{} (installed by the examiner at {})", r.colmap.banner, r.colmap.path)],
                        ["Executable SHA-256".into(), r.colmap.sha256.clone()],
                        ["Settings".into(), format!(
                            "camera model {}{}; features on the {}; images up to {} px for features; {} matching; dense {}",
                            s.camera_model,
                            if s.single_camera { ", one camera for all images" } else { "" },
                            if s.cpu_only { "CPU (COLMAP's SIFT)" } else { "GPU where available" },
                            if s.max_image_size > 0 { s.max_image_size.to_string() } else { "COLMAP's default".into() },
                            match s.matcher { locus_photo::colmap::Matcher::Exhaustive => "exhaustive", _ => "sequential" },
                            if r.output.from == "dense" { format!("yes, images up to {} px", s.dense_max_image_size) } else { "no".into() }
                        )],
                    ],
                },
                Block::Table {
                    widths: ["auto", "auto", "1fr"].map(String::from).to_vec(),
                    head: ["Stage", "Time", "Command"].map(String::from).to_vec(),
                    rows: stages,
                },
            ],
        },
        Section {
            heading: "Method".into(),
            blocks: vec![
                Block::Text { text: "COLMAP finds features in each image, matches them between images, and solves the cameras and sparse points together (incremental structure from motion with bundle adjustment); when it has CUDA, it then computes a depth map per image and fuses them into the dense point cloud. The largest reconstruction is kept. Its arbitrary scale and frame are then fixed: by known distances (each end clicked in two or more photos and triangulated with the reconstruction's cameras; the scale is their weighted mean, levelled by the photos' mean up direction), by control points (clicked and triangulated likewise; a similarity fitted to their coordinates, with check points held out), or by the photos' GPS positions (a similarity from the camera centres to a local east-north-up frame). The scaled points are written as E57 and imported as evidence with their own hash. See docs/methods/photogrammetry.md.".into() },
                Block::List { items: REFS.iter().map(|s| s.to_string()).collect() },
            ],
        },
        Section { heading: "Assumptions".into(), blocks: vec![Block::List { items: r.assumptions.clone() }] },
        Section { heading: "Limitations".into(), blocks: vec![Block::List { items: r.limitations.clone() }] },
        Section {
            heading: "Sign-off".into(),
            blocks: vec![Block::SignOff { rows: vec!["Examiner".into(), "Technical review".into()] }],
        },
    ];
    let mut warnings = vec![];
    if let Some(x) = &meta.withdrawn {
        warnings.push(format!("This analysis was withdrawn: {x}."));
    }
    warnings.extend(r.warnings.clone());
    Report {
        title: format!("Photogrammetry: {}", meta.name),
        header: format!(
            "{}{} · analysis {} · {}",
            meta.case_number
                .as_ref()
                .map(|c| format!("{c} · "))
                .unwrap_or_default(),
            meta.project,
            meta.record_id,
            meta.method
        ),
        details: details(meta),
        warnings,
        sections,
    }
}
