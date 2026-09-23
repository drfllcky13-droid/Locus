//! The bullet-trajectory report, built only from the stored run (so it can be reprinted
//! from the record at any time and says exactly what was computed). Reads `trajectory/1`
//! and `trajectory/2` records; parts a v1 record doesn't have are left out.

use crate::analysis::{details, Area, Block, Figure, Frame, Label, Meta, Report, Section};
use locus_analysis::measure::Measured;
use locus_analysis::trajectory::{Run, ShooterBand};

/// Figure colours (line, fill): the computed 95 % cone, and the examiner-defined zone.
pub const MEASUREMENT: (&str, &str) = ("#2f6fbf", "#bcd4f0");
pub const EXAMINER: (&str, &str) = ("#c9731a", "#f3d6a8");

fn deg(m: &Measured) -> String {
    format!("{:.2}° ± {:.2}°", m.value, m.sigma)
}

fn mm(m: f64) -> String {
    format!("{:.1} mm", m * 1000.0)
}

fn xyz(p: [f64; 3]) -> String {
    format!("{:.3}, {:.3}, {:.3}", p[0], p[1], p[2])
}

/// "12.3° ± 0.1° upward" / "… downward".
fn vertical(m: &Measured) -> String {
    let dir = if m.value >= 0.0 { "upward" } else { "downward" };
    format!("{:.2}° ± {:.2}° {dir}", m.value.abs(), m.sigma)
}

/// "20.0° ± 0.1° right of perpendicular".
fn horizontal(m: &Measured) -> String {
    let dir = if m.value >= 0.0 { "right" } else { "left" };
    format!(
        "{:.2}° ± {:.2}° {dir} of perpendicular",
        m.value.abs(),
        m.sigma
    )
}

fn measurement_label(r: &Run) -> String {
    format!(
        "Measurement uncertainty (95 %, computed): {:.2}° × {:.2}° half-angles",
        r.line.cone.major_deg, r.line.cone.minor_deg
    )
}

fn examiner_label(r: &Run) -> String {
    format!(
        "Examiner-defined zone (±{:.1}°, analyst judgment)",
        r.parameters.cone_deg
    )
}

/// The images a report prints: each defect photo's project file and recorded hash.
pub fn photo_files(r: &Run) -> Vec<(String, String)> {
    r.inputs
        .iter()
        .filter_map(|i| i.photo.as_ref().map(|p| (p.file.clone(), p.sha256.clone())))
        .collect()
}

pub fn report(meta: &Meta, r: &Run) -> Report {
    let l = &r.line;
    let p = &r.parameters;
    let conv = &p.conventions;
    let mut warnings = vec![];
    if let Some(w) = &meta.withdrawn {
        warnings.push(format!("This analysis was withdrawn: {w}."));
    }
    if r.cone_narrower_than_fit {
        warnings.push(format!(
            "The computed 95 % cone ({:.1}°) is wider than the examiner-defined zone (±{:.1}°).",
            l.cone.major_deg, p.cone_deg
        ));
    }
    if l.inflation > 1.0 {
        warnings.push(format!(
            "The points scatter more than their stated uncertainty (χ² = {:.1} on {} degrees of freedom): the direction's uncertainty has been widened {:.1}×. Check for a misplaced point or a deflection.",
            l.chi2, l.dof, l.inflation
        ));
    }
    for c in r.cross_checks.iter().filter(|c| !c.agrees) {
        warnings.push(format!(
            "Point {} ({} on {}): the hole's ellipse gives an impact angle of {}, the trajectory {} at that face. They differ by {:.1}°, beyond their combined uncertainty. Check for deflection, a damaged or non-elliptical hole, or the wrong hole.",
            c.input + 1,
            c.kind,
            c.surface,
            deg(&c.ellipse),
            deg(&c.trajectory),
            c.difference.abs()
        ));
    }
    let manual: Vec<String> = r
        .inputs
        .iter()
        .enumerate()
        .filter(|(_, i)| i.centre == "manual" && i.override_reason.is_some())
        .map(|(k, i)| {
            format!(
                "Point {} ({} on {}): picked point used as the centre. Reason: {}",
                k + 1,
                i.kind,
                i.surface,
                i.override_reason.as_deref().unwrap_or("")
            )
        })
        .collect();
    let surfaces: std::collections::BTreeSet<&str> =
        r.inputs.iter().map(|i| i.surface.as_str()).collect();
    if surfaces.len() == 1 && p.rod_play_deg == 0.0 {
        warnings.push(
            "All points are on one surface, so the direction rests on that surface's thickness alone."
                .into(),
        );
    }

    // Scene-relative.
    let bearing = r.scene_bearing.unwrap_or(l.bearing);
    let result = vec![
        [
            "Bearing".into(),
            format!("{} from {}", deg(&bearing), conv.reference),
        ],
        ["Elevation".into(), vertical(&l.elevation)],
        ["Measurement uncertainty".into(), measurement_label(r)],
        ["Examiner-defined zone".into(), examiner_label(r)],
        [
            "Direction of travel".into(),
            format!(
                "({:.5}, {:.5}, {:.5}) in the project frame",
                l.direction[0], l.direction[1], l.direction[2]
            ),
        ],
        ["Point on the path".into(), format!("{} m", xyz(l.point))],
        [
            "Fit".into(),
            if l.dof > 0 {
                format!(
                    "{} points; χ² = {:.2} on {} degrees of freedom",
                    r.inputs.len(),
                    l.chi2,
                    l.dof
                )
            } else {
                format!(
                    "{} points (no redundancy: the residuals can't check the stated uncertainty)",
                    r.inputs.len()
                )
            },
        ],
    ];

    // Surface-relative, in the chosen convention.
    let level =
        conv.surface == "level_perpendicular" && r.surfaces.iter().all(|s| s.level.is_some());
    let (surf_text, surf_head, surf_rows): (String, Vec<String>, Vec<Vec<String>>) = if level {
        (
            "Vertical angle up or down from level; horizontal angle left or right of perpendicular to the surface, viewed facing the surface from the side the bullet came from; impact angle between the path and the surface (90° is square on). A floor or ceiling has no horizontal angle. 1σ from the direction's uncertainty; the plane fitted around the defect and its RMS residual.".into(),
            ["Surface", "Vertical", "Horizontal", "Impact", "Plane RMS"]
                .map(String::from)
                .to_vec(),
            r.surfaces
                .iter()
                .map(|s| {
                    let lv = s.level.expect("checked above");
                    vec![
                        s.surface.clone(),
                        vertical(&lv.vertical),
                        lv.horizontal
                            .as_ref()
                            .map(horizontal)
                            .unwrap_or_else(|| "(horizontal surface)".into()),
                        deg(&s.angles.impact),
                        mm(s.plane_rms),
                    ]
                })
                .collect(),
        )
    } else {
        (
            "Impact angle between the path and the surface (90° is square on). Horizontal and vertical angles from the surface's normal, in the surface's own frame, seen from the shooter's side: horizontal + to the right, vertical + upward. 1σ from the direction's uncertainty; the plane fitted around each defect and its RMS residual.".into(),
            ["Surface", "Impact", "Horizontal", "Vertical", "Plane RMS"]
                .map(String::from)
                .to_vec(),
            r.surfaces
                .iter()
                .map(|s| {
                    vec![
                        s.surface.clone(),
                        deg(&s.angles.impact),
                        deg(&s.angles.horizontal),
                        deg(&s.angles.vertical),
                        mm(s.plane_rms),
                    ]
                })
                .collect(),
        )
    };

    let inputs = r
        .inputs
        .iter()
        .zip(&l.residuals)
        .enumerate()
        .map(|(i, (p, res))| {
            let centre = match p.centre.as_str() {
                "fitted" => "fitted from rim",
                "manual" if p.override_reason.is_some() => "picked (override)",
                "rod" => "rod",
                _ => "picked",
            };
            vec![
                (i + 1).to_string(),
                p.kind.clone(),
                p.surface.clone(),
                centre.into(),
                xyz(p.point),
                mm(p.sigma),
                mm(*res),
                p.source
                    .as_ref()
                    .map(|s| format!("scan {}, record {}, rev. {}", s.scan, s.index, s.revision))
                    .unwrap_or_else(|| "—".into()),
            ]
        })
        .collect();

    let holes: Vec<Vec<String>> = r
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(k, i)| {
            let f = i.defect.as_ref()?;
            let c = r.cross_checks.iter().find(|c| c.input == k);
            Some(vec![
                (k + 1).to_string(),
                format!("{} {}", i.surface, i.kind),
                mm(f.centre_sigma),
                format!(
                    "{:.1} × {:.1} mm",
                    2000.0 * f.semi_axes[0].value,
                    2000.0 * f.semi_axes[1].value
                ),
                format!("{} ({:.1} mm)", f.rim_points, f.spacing * 1000.0),
                deg(&f.impact),
                c.map(|c| deg(&c.trajectory)).unwrap_or_default(),
                c.map(|c| {
                    format!(
                        "{:+.1}° {}",
                        c.difference,
                        if c.agrees { "agrees" } else { "DISAGREES" }
                    )
                })
                .unwrap_or_default(),
            ])
        })
        .collect();

    let mut band_rows = vec![
        [
            "Height band".into(),
            format!(
                "{:.2}–{:.2} m above the floor at z = {:.3} m",
                p.band[0], p.band[1], p.floor_z
            ),
        ],
        ["Examiner-defined zone".into(), examiner_label(r)],
        ["Measurement uncertainty".into(), measurement_label(r)],
        [
            "Traced back up to".into(),
            format!("{:.1} m from the first point", p.max_range),
        ],
    ];
    match &r.band.centre {
        Some((t, pts)) => band_rows.push([
            "Centre line in the band".into(),
            format!(
                "{:.2} m to {:.2} m back from the first point, from ({}) to ({}) m",
                t[0],
                t[1],
                xyz(pts[0]),
                xyz(pts[1])
            ),
        ]),
        None => band_rows.push([
            "Centre line in the band".into(),
            "never, within the range traced back".into(),
        ]),
    }

    let mut sections = vec![
        Section {
            heading: "Result (scene-relative)".into(),
            blocks: vec![Block::Pairs { rows: result }],
        },
        Section {
            heading: "Angles to each surface".into(),
            blocks: vec![
                Block::Text { text: surf_text },
                Block::Table {
                    widths: ["1fr", "auto", "auto", "auto", "auto"]
                        .map(String::from)
                        .to_vec(),
                    head: surf_head,
                    rows: surf_rows,
                },
            ],
        },
        Section {
            heading: "Points used".into(),
            blocks: vec![
                Block::Text {
                    text: "In the order the projectile travelled. Coordinates in the project frame (m); how each point was found; its 1σ and its perpendicular distance from the fitted path; and the scan point the click resolved to (scan, record number in the source file, cleanup revision).".into(),
                },
                Block::Table {
                    widths: ["auto", "auto", "1fr", "auto", "auto", "auto", "auto", "auto"]
                        .map(String::from)
                        .to_vec(),
                    head: [
                        "#",
                        "Kind",
                        "Surface",
                        "Centre",
                        "x, y, z (m)",
                        "1σ",
                        "Residual",
                        "Source",
                    ]
                    .map(String::from)
                    .to_vec(),
                    rows: inputs,
                },
            ],
        },
    ];
    if !holes.is_empty() || !manual.is_empty() {
        let mut blocks = vec![];
        if !holes.is_empty() {
            blocks.push(Block::Text {
                text: "Each fitted hole: the centre's 1σ, the ellipse's axes (corrected for the point spacing), the rim points used and the point spacing, and the impact angle from the ellipse (asin of short over long axis) against the fitted path's impact angle at that face. They are independent. Agreement is tested at 95 % on the axis ratio (short over long) against the sine of the path's impact angle, where the errors are close to normal; the angle's own uncertainty is lopsided for near-round holes, so a large difference in degrees can still agree. With few rim points the ellipse angle is weak evidence: read its uncertainty.".into(),
            });
            blocks.push(Block::Table {
                widths: [
                    "auto", "1fr", "auto", "auto", "auto", "auto", "auto", "auto",
                ]
                .map(String::from)
                .to_vec(),
                head: [
                    "#",
                    "Hole",
                    "Centre 1σ",
                    "Axes",
                    "Rim (spacing)",
                    "Ellipse impact",
                    "Path impact",
                    "Difference",
                ]
                .map(String::from)
                .to_vec(),
                rows: holes,
            });
        }
        if !manual.is_empty() {
            blocks.push(Block::Text {
                text: "Manual centres (picked points used instead of a fitted hole centre):".into(),
            });
            blocks.push(Block::List { items: manual });
        }
        sections.push(Section {
            heading: "Hole centres".into(),
            blocks,
        });
    }
    sections.push(Section {
        heading: "Possible muzzle positions".into(),
        blocks: vec![
            Block::Pairs { rows: band_rows },
            Block::Figure(plan(r)),
            Block::Figure(elevation(r)),
        ],
    });
    let photos: Vec<Block> = r
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(k, i)| {
            let ph = i.photo.as_ref()?;
            Some(Block::Image {
                file: ph.file.clone(),
                width: 120.0,
                caption: format!(
                    "Point {} ({} on {}): {} (evidence {}, SHA-256 {})",
                    k + 1,
                    i.kind,
                    i.surface,
                    ph.name,
                    ph.evidence_id,
                    ph.sha256
                ),
            })
        })
        .collect();
    if !photos.is_empty() {
        sections.push(Section {
            heading: "Defect photographs".into(),
            blocks: photos,
        });
    }
    sections.push(Section {
        heading: "Method".into(),
        blocks: vec![
            Block::Text {
                text: "Hole centres, where the examiner didn't override them, come from an ellipse fitted to the scan points around each hole's rim on the plane of its face. A straight line is fitted to the points by weighted total least squares (weights 1/σ²): it passes through their weighted centroid along the principal axis of their weighted scatter, oriented in the order of travel. The direction's covariance is propagated to first order from every input coordinate, inflated by √(χ²/dof) when the residuals exceed the stated uncertainty, and any rod play is added in quadrature. The computed 95 % cone's half-angles are √5.991 times the square roots of the covariance's two principal values. Possible muzzle positions are where the path, traced back from the first point, and the edges of each cone pass through the height band. See docs/methods/trajectory.md.".into(),
            },
            Block::Pairs {
                rows: vec![
                    [
                        "Scene-relative angles".into(),
                        format!(
                            "Bearing clockwise from {} ({:.1}° clockwise from project +y); elevation up or down from level.",
                            conv.reference, conv.reference_deg
                        ),
                    ],
                    [
                        "Surface-relative angles".into(),
                        if level {
                            "Vertical from level; horizontal left or right of perpendicular, viewed facing the surface.".into()
                        } else {
                            "From the surface's normal, in its own frame.".into()
                        },
                    ],
                ],
            },
        ],
    });
    sections.push(Section {
        heading: "Assumptions".into(),
        blocks: vec![Block::List {
            items: r.assumptions.clone(),
        }],
    });
    sections.push(Section {
        heading: "Limitations".into(),
        blocks: vec![Block::List {
            items: r.limitations.clone(),
        }],
    });
    sections.push(Section {
        heading: "Sign-off".into(),
        blocks: vec![Block::SignOff {
            rows: vec!["Examiner".into(), "Technical review".into()],
        }],
    });

    Report {
        title: format!("Bullet trajectory: {}", meta.name),
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

const W: f64 = 178.0;
const H: f64 = 80.0;

fn zone(fig: &mut Figure, f: &Frame, b: &ShooterBand, fill: &str) {
    if b.footprint.len() >= 3 {
        fig.areas.push(Area {
            points: b.footprint.iter().map(|p| f.at(*p)).collect(),
            fill: fill.into(),
        });
    }
}

/// Plan view: both zones in the band, the path, and the points.
fn plan(r: &Run) -> Figure {
    let mut pts: Vec<[f64; 2]> = r.inputs.iter().map(|i| [i.point[0], i.point[1]]).collect();
    pts.extend(r.band.footprint.iter().copied());
    if let Some(b) = &r.band_measurement {
        pts.extend(b.footprint.iter().copied());
    }
    if let Some((_, ends)) = &r.band.centre {
        pts.extend(ends.iter().map(|p| [p[0], p[1]]));
    }
    let (lo, hi) = bounds(&pts);
    let f = Frame::new(lo, hi, W, H, 8.0);
    let mut fig = Figure {
        width: W,
        height: H,
        caption: format!(
            "Plan (x east, y north). Orange: {}. Blue: {}. Both within the height band. Line: the fitted path from the band to the last point; dots: the points used.",
            examiner_label(r),
            measurement_label(r)
        ),
        ..Figure::default()
    };
    // The wider zone first, so the narrower shows on top.
    match &r.band_measurement {
        Some(m) if r.line.cone.major_deg > r.parameters.cone_deg => {
            zone(&mut fig, &f, m, MEASUREMENT.1);
            zone(&mut fig, &f, &r.band, EXAMINER.1);
        }
        Some(m) => {
            zone(&mut fig, &f, &r.band, EXAMINER.1);
            zone(&mut fig, &f, m, MEASUREMENT.1);
        }
        None => zone(&mut fig, &f, &r.band, EXAMINER.1),
    }
    let first = r.inputs[0].point;
    let last = r.inputs[r.inputs.len() - 1].point;
    let back = r.band.centre.as_ref().map(|(_, e)| e[1]).unwrap_or(first);
    fig.line(
        f.at([back[0], back[1]]),
        f.at([last[0], last[1]]),
        0.35,
        false,
    );
    for (i, p) in r.inputs.iter().enumerate() {
        let q = f.at([p.point[0], p.point[1]]);
        fig.dots.push([q[0], q[1], 0.7]);
        if i == 0 || p.surface != r.inputs[i - 1].surface {
            fig.labels.push(Label {
                x: q[0] + 1.5,
                y: q[1] + 1.0,
                text: p.surface.clone(),
            });
        }
    }
    fig.scale_bar(&f);
    fig
}

/// Elevation along the path's bearing: horizontal distance from the first point against
/// height, with both cones' upper and lower edges.
fn elevation(r: &Run) -> Figure {
    let d = r.line.direction;
    let first = r.inputs[0].point;
    let horiz = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    let along = |p: [f64; 3]| ((p[0] - first[0]) * d[0] + (p[1] - first[1]) * d[1]) / horiz;
    let p = &r.parameters;
    let back = r
        .band
        .centre
        .as_ref()
        .map(|(t, _)| t[1])
        .unwrap_or(2.0)
        .max(1.0);
    let x_lo = -back * horiz - 0.5;
    let x_hi = r.inputs.iter().map(|i| along(i.point)).fold(0.0, f64::max) + 0.5;
    let zs: Vec<f64> = r
        .inputs
        .iter()
        .map(|i| i.point[2])
        .chain([p.floor_z, p.floor_z + p.band[1] + 0.3])
        .collect();
    let z_lo = zs.iter().cloned().fold(f64::INFINITY, f64::min);
    let z_hi = zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let f = Frame::new([x_lo, z_lo], [x_hi, z_hi], W, H, 8.0);
    let mut fig = Figure {
        width: W,
        height: H,
        caption: format!(
            "Elevation along the path's bearing (horizontal distance from the first point against height). Black: floor and path; dashed grey: the height band. Orange: {}. Blue: {}.",
            examiner_label(r),
            measurement_label(r)
        ),
        ..Figure::default()
    };
    fig.line(f.at([x_lo, p.floor_z]), f.at([x_hi, p.floor_z]), 0.5, false);
    for b in p.band {
        fig.coloured(
            f.at([x_lo, p.floor_z + b]),
            f.at([x_hi, p.floor_z + b]),
            0.25,
            true,
            "#888888",
        );
    }
    let slope = d[2] / horiz;
    let z_at = |x: f64, s: f64| first[2] + x * s;
    let e = d[2].asin();
    for (half, colour) in [
        (p.cone_deg, EXAMINER.0),
        (r.line.cone.major_deg, MEASUREMENT.0),
    ] {
        for s in [-1.0, 1.0] {
            let k = (e + s * half.to_radians()).tan();
            fig.coloured(
                f.at([x_lo, z_at(x_lo, k)]),
                f.at([0.0, first[2]]),
                0.3,
                true,
                colour,
            );
        }
    }
    fig.line(
        f.at([x_lo, z_at(x_lo, slope)]),
        f.at([x_hi, z_at(x_hi, slope)]),
        0.35,
        false,
    );
    for i in &r.inputs {
        let q = f.at([along(i.point), i.point[2]]);
        fig.dots.push([q[0], q[1], 0.7]);
    }
    fig.scale_bar(&f);
    fig
}

fn bounds(p: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for q in p {
        for k in 0..2 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::pdf;
    use locus_analysis::defect::DefectFit;
    use locus_analysis::trajectory::{
        direction, run, FittedPlane, InputPoint, Parameters, PhotoRef, PointSource,
    };

    fn sample() -> (Meta, Run) {
        let d = direction(62.0, -4.0);
        let o = [431_200.0, 5_390_110.0, 1.35];
        let pts = [
            (1.8, "vehicle door"),
            (1.801, "vehicle door"),
            (4.6, "interior wall"),
            (4.613, "interior wall"),
        ];
        let inputs = pts
            .iter()
            .enumerate()
            .map(|(i, (t, s))| {
                let point = [
                    o[0] + d[0] * t,
                    o[1] + d[1] * t,
                    o[2] + d[2] * t + if i == 2 { 0.001 } else { 0.0 },
                ];
                InputPoint {
                    kind: if i % 2 == 0 { "entry" } else { "exit" }.into(),
                    surface: (*s).into(),
                    point,
                    sigma: 0.002,
                    plane: Some(FittedPlane {
                        point: o,
                        normal: [0.0, -1.0, 0.0],
                        rms: 0.0012,
                        points: 60,
                    }),
                    centre: if i == 1 { "manual" } else { "fitted" }.into(),
                    override_reason: (i == 1).then(|| "rim torn; centre picked by eye".into()),
                    defect: (i != 1).then(|| DefectFit {
                        centre: point,
                        centre_sigma: 0.0006,
                        normal: [0.0, -1.0, 0.0],
                        semi_axes: [
                            Measured {
                                value: 0.0052,
                                sigma: 0.0006,
                            },
                            Measured {
                                value: 0.0045,
                                sigma: 0.0006,
                            },
                        ],
                        long_axis: [1.0, 0.0, 0.0],
                        impact: Measured {
                            value: if i == 3 { 20.0 } else { 60.0 },
                            sigma: 3.0,
                        },
                        rim_points: 18,
                        spacing: 0.002,
                        rms: 0.0004,
                    }),
                    source: Some(PointSource {
                        scan: "1:0".into(),
                        index: 1000 + i as u32,
                        revision: 0,
                    }),
                    ..Default::default()
                }
            })
            .collect();
        let r = run(
            inputs,
            Parameters {
                floor_z: 0.0,
                ..Parameters::default()
            },
        )
        .unwrap();
        let meta = Meta {
            project: "Main Street".into(),
            record_id: 3,
            name: "Shot through the door".into(),
            method: r.method.clone(),
            sha256: "ab".repeat(32),
            revises: None,
            created_at: "2026-09-23T10:00:00Z".into(),
            created_by: "Examiner".into(),
            withdrawn: None,
            printed_by: "Examiner".into(),
            printed_at: "2026-09-23T10:05:00Z".into(),
            app_version: "0.1.0".into(),
            audit_head: "cd".repeat(32),
            case_number: Some("2026-00417".into()),
        };
        (meta, r)
    }

    #[test]
    fn the_pdf_says_what_the_run_computed() {
        let (meta, r) = sample();
        let doc = report(&meta, &r);
        let out = pdf(&doc, vec![]).unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
        let text = out.text;
        assert!(
            text.contains(&deg(r.scene_bearing.as_ref().unwrap())),
            "{text}"
        );
        assert!(text.contains(&vertical(&r.line.elevation)));
        assert!(text.contains("2026-00417"));
        assert!(text.contains("vehicle door") && text.contains("interior wall"));
        assert!(text.contains("Measurement uncertainty (95 %, computed)"));
        assert!(text.contains("Examiner-defined zone (±5.0°, analyst judgment)"));
        assert!(text.contains("rim torn; centre picked by eye"));
        assert!(text.contains("scan 1:0, record 1002"));
        assert!(text.contains("of perpendicular"));
        assert!(text.contains("Sign-off") && text.contains("Technical review"));
        assert!(text.contains("Assumptions") && text.contains("Limitations"));
        assert!(text.contains(&meta.sha256));
        let tmp = std::env::temp_dir();
        std::fs::write(tmp.join("locus-trajectory.pdf"), &out.pdf).unwrap();
        let data = serde_json::to_vec(&doc).unwrap();
        for (i, png) in crate::pages_png(crate::analysis::TEMPLATE, data, vec![])
            .iter()
            .enumerate()
        {
            std::fs::write(tmp.join(format!("locus-trajectory-{}.png", i + 1)), png).unwrap();
        }
    }

    #[test]
    fn a_hole_that_disagrees_is_flagged_and_a_photo_is_printed() {
        let (meta, mut r) = sample();
        // Point 4's ellipse says 20° ± 3°, the path about 28°: flagged.
        let c = r.cross_checks.iter().find(|c| c.input == 3).unwrap();
        assert!(!c.agrees, "{c:?}");
        let doc = report(&meta, &r);
        assert!(doc.warnings.iter().any(|w| w.contains("Point 4")));
        r.inputs[0].photo = Some(PhotoRef {
            evidence_id: 7,
            name: "door-entry.svg".into(),
            file: "evidence/7/door-entry.svg".into(),
            sha256: "ef".repeat(32),
        });
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="40" height="30" fill="#999"/></svg>"##;
        let doc = report(&meta, &r);
        let out = pdf(
            &doc,
            vec![("evidence/7/door-entry.svg".into(), svg.to_vec())],
        )
        .unwrap();
        assert!(out.text.contains("door-entry.svg (evidence 7"));
        assert_eq!(
            photo_files(&r),
            vec![("evidence/7/door-entry.svg".into(), "ef".repeat(32))]
        );
    }

    #[test]
    fn warnings_explain_a_withdrawn_run_and_excess_scatter() {
        let (mut meta, mut r) = sample();
        meta.withdrawn = Some("2026-09-23 by Examiner: wrong defect".into());
        r.line.inflation = 3.2;
        let doc = report(&meta, &r);
        assert!(doc.warnings.iter().any(|w| w.contains("withdrawn")));
        assert!(doc.warnings.iter().any(|w| w.contains("3.2×")));
    }
}
