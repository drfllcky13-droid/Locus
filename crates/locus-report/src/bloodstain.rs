//! The bloodstain area-of-origin report, built only from the stored run (so it can be
//! reprinted from the record at any time and says exactly what was computed).

use crate::analysis::{details, Area, Block, Figure, Frame, Label, Meta, Report, Section};
use locus_analysis::bloodstain::Run;
use locus_analysis::measure::Measured;

/// Figure colours: stains used and their paths, those left out, the origin's 95 % region.
const USED: &str = "#8e1b1f";
const UNUSED: &str = "#9a9a9a";
const REGION: &str = "#f0c9c4";

fn deg(m: &Measured) -> String {
    format!("{:.1}° ± {:.1}°", m.value, m.sigma)
}

fn mm(m: f64) -> String {
    format!("{:.1} mm", m * 1000.0)
}

fn xyz(p: [f64; 3]) -> String {
    format!("{:.3}, {:.3}, {:.3}", p[0], p[1], p[2])
}

const STRAIGHT_LINE: &str = "Straight-line paths ignore gravity and air drag. Real droplets fall along curved paths, so a straight-line origin tends to be too high: read its height as an upper bound. Only stains clearly moving upward at impact are used unless the examiner states otherwise (below).";

pub fn report(meta: &Meta, r: &Run) -> Report {
    let o = &r.origin;
    let p = &r.parameters;
    let mut warnings = vec![];
    if let Some(w) = &meta.withdrawn {
        warnings.push(format!("This analysis was withdrawn: {w}."));
    }
    if let Some(why) = &p.include_not_upward {
        warnings.push(format!(
            "Stains not clearly moving upward (floor stains, downward or uncertain directions) were included. The examiner's reason: {why}. Droplets that had started to fall make the straight-line origin much too high."
        ));
    }
    if o.dof > 0 && o.chi2 / o.dof as f64 > 2.0 {
        warnings.push(format!(
            "The stains scatter more than their stated uncertainties allow (χ² = {:.0} on {} degrees of freedom). Some stains may not come from this origin, or their measurements are worse than stated. The 95 % region is from resampling the stains and does not depend on those uncertainties.",
            o.chi2, o.dof
        ));
    }
    let behind: Vec<String> = r
        .stains
        .iter()
        .zip(&r.inputs)
        .filter(|(s, _)| s.used && s.behind)
        .map(|(_, i)| i.label.clone())
        .collect();
    if !behind.is_empty() {
        warnings.push(format!(
            "The origin is behind stain(s) {} along their paths: those stains point away from it. Check their tails.",
            behind.join(", ")
        ));
    }
    if o.stains_used < 10 {
        warnings.push(format!(
            "Only {} stains were used; the 95 % region is correspondingly wide and less certain.",
            o.stains_used
        ));
    }
    if o.bootstrap_failed > 0 {
        warnings.push(format!(
            "{} of {} resamples had nearly parallel paths and were left out of the 95 % region.",
            o.bootstrap_failed,
            o.bootstrap + o.bootstrap_failed
        ));
    }

    let e = &o.ellipsoid;
    let axis = |k: usize| {
        let a = e.axes[k];
        format!(
            "{:.3} m along ({:.2}, {:.2}, {:.2})",
            e.semi_axes[k], a[0], a[1], a[2]
        )
    };
    let result = vec![
        ["Origin".into(), format!("{} m (project frame)", xyz(o.point))],
        [
            "Height above the floor".into(),
            format!(
                "{:.3} m ± {:.3} m (floor at z = {:.3} m)",
                o.height.value, o.height.sigma, p.floor_z
            ),
        ],
        [
            "95 % region (half-axes)".into(),
            format!("{}; {}; {}", axis(0), axis(1), axis(2)),
        ],
        [
            "1σ in x, y, z".into(),
            format!(
                "{:.3}, {:.3}, {:.3} m",
                o.sigma[0], o.sigma[1], o.sigma[2]
            ),
        ],
        [
            "Stains used".into(),
            format!("{} of {}", o.stains_used, r.stains.len()),
        ],
        [
            "Fit".into(),
            format!(
                "χ² = {:.1} on {} degrees of freedom; RMS distance of the origin from the paths used {}",
                o.chi2,
                o.dof,
                mm(o.rms_residual)
            ),
        ],
    ];

    let stains = r
        .stains
        .iter()
        .zip(&r.inputs)
        .map(|(s, i)| {
            let floor = i.normal[2].abs() > 0.9;
            vec![
                i.label.clone(),
                i.surface.clone(),
                format!(
                    "{:.2} × {:.2}",
                    i.width.value * 1000.0,
                    i.length.value * 1000.0
                ),
                deg(&s.impact),
                format!(
                    "{} {}",
                    deg(&s.directionality),
                    if floor {
                        format!("from {}", p.reference)
                    } else {
                        "from up".into()
                    }
                ),
                match &s.not_used {
                    None => "used".into(),
                    Some(why) => format!("no: {why}"),
                },
                mm(s.residual),
                format!("{:.1}", s.residual_sigmas),
            ]
        })
        .collect();

    let photos: Vec<Vec<String>> = r
        .inputs
        .iter()
        .map(|i| {
            let a = i.alignment.as_ref();
            let f = i.fit.as_ref();
            vec![
                i.label.clone(),
                i.photo
                    .as_ref()
                    .map(|p| format!("{} (evidence {})", p.name, p.evidence_id))
                    .unwrap_or_else(|| "—".into()),
                a.map(|a| {
                    format!(
                        "{} pairs, {}",
                        a.pairs.len(),
                        a.rms.map(mm).unwrap_or_else(|| "exact (2 pairs)".into())
                    )
                })
                .unwrap_or_default(),
                a.map(|a| format!("{:.1}", a.pixels_per_metre / 1000.0))
                    .unwrap_or_default(),
                match (&i.auto_edge, f) {
                    (Some(e), Some(f)) => format!(
                        "automatic (threshold {}): {} points, {} left out",
                        e.threshold, f.edge_points, f.trimmed
                    ),
                    (None, Some(f)) => {
                        format!("clicked: {} points, {} left out", f.edge_points, f.trimmed)
                    }
                    _ => "—".into(),
                },
                f.map(|f| format!("{:.3} mm", f.rms * 1000.0))
                    .unwrap_or_default(),
                i.sources
                    .iter()
                    .map(|s| format!("{} #{} r{}", s.scan, s.index, s.revision))
                    .collect::<Vec<_>>()
                    .join("; "),
            ]
        })
        .collect();
    let hashes: Vec<String> = r
        .inputs
        .iter()
        .filter_map(|i| {
            i.photo
                .as_ref()
                .map(|p| format!("{}: {} SHA-256 {}", i.label, p.file, p.sha256))
        })
        .collect();

    let mut sections = vec![
        Section {
            heading: "Result".into(),
            blocks: vec![
                Block::Text {
                    text: STRAIGHT_LINE.into(),
                },
                Block::Pairs { rows: result },
                Block::Figure(plan(r)),
                Block::Figure(elevation(r)),
            ],
        },
        Section {
            heading: "Stains".into(),
            blocks: vec![
                Block::Text {
                    text: "Each stain's width and length (mm), impact angle asin(width / length), and direction of travel (on a wall clockwise from straight up, seen facing the wall; on a floor clockwise from the reference axis), with 1σ; whether it was used; and the origin's distance from its path, in metres and in units of the path's uncertainty there.".into(),
                },
                Block::Table {
                    widths: ["auto", "1fr", "auto", "auto", "auto", "1fr", "auto", "auto"]
                        .map(String::from)
                        .to_vec(),
                    head: [
                        "#",
                        "Surface",
                        "W × L (mm)",
                        "Impact",
                        "Direction",
                        "Used",
                        "Residual",
                        "σ",
                    ]
                    .map(String::from)
                    .to_vec(),
                    rows: stains,
                },
            ],
        },
        Section {
            heading: "Photos, alignment and edges".into(),
            blocks: vec![
                Block::Text {
                    text: "Each stain's photo, its alignment to the scan (point pairs and their RMS distance after the fit; pixels per millimetre), how its edge was found and the ellipse's RMS distance from the edge points kept, and the scan points the pairs resolved to (scan, record, cleanup revision).".into(),
                },
                Block::Table {
                    widths: ["auto", "1fr", "auto", "auto", "1fr", "auto", "1fr"]
                        .map(String::from)
                        .to_vec(),
                    head: [
                        "#",
                        "Photo",
                        "Alignment",
                        "px/mm",
                        "Edge",
                        "Fit RMS",
                        "Scan points",
                    ]
                    .map(String::from)
                    .to_vec(),
                    rows: photos,
                },
            ],
        },
    ];
    if !hashes.is_empty() {
        sections[2].blocks.push(Block::List { items: hashes });
    }
    sections.push(Section {
        heading: "Method".into(),
        blocks: vec![
            Block::Text {
                text: "Each photo is placed on its surface by a similarity (scale, rotation, shift) fitted to pixel–scan point pairs, on the plane fitted to the scan around them. An ellipse is fitted to the stain's edge points by least squares, leaving out points well off it (the tail). The impact angle is asin(width / length) and the direction of travel is along the long axis toward the marked tail. The origin is the point whose straight paths to the stains best match every stain's impact angle and direction, each in units of its 1σ (Levenberg–Marquardt, started from the point nearest all the paths). The 95 % region is an ellipsoid from bootstrap resampling of the stains used, with a radius from Hotelling's T² for the number of stains. See docs/methods/bloodstain.md.".into(),
            },
            Block::Pairs {
                rows: vec![
                    [
                        "Stains used".into(),
                        match &p.include_not_upward {
                            None => "wall stains moving upward by more than twice their direction's 1σ".into(),
                            Some(_) => "every stain not excluded by the examiner (see the warning)".into(),
                        },
                    ],
                    [
                        "Bootstrap".into(),
                        format!(
                            "{} resamples (seed {})",
                            o.bootstrap + o.bootstrap_failed,
                            p.seed
                        ),
                    ],
                    [
                        "Floor directions".into(),
                        format!(
                            "clockwise from {} ({:.1}° clockwise from project +y)",
                            p.reference, p.reference_deg
                        ),
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
        title: format!("Bloodstain area of origin: {}", meta.name),
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
const H: f64 = 90.0;

/// The outline of the 95 % ellipsoid seen along one axis (its shadow on the other two).
fn shadow(r: &Run, i: usize, j: usize) -> Vec<[f64; 2]> {
    let e = &r.origin.ellipsoid;
    let m = |a: usize, b: usize| {
        (0..3)
            .map(|k| e.semi_axes[k].powi(2) * e.axes[k][a] * e.axes[k][b])
            .sum::<f64>()
    };
    let (a, b, c) = (m(i, i), m(i, j), m(j, j));
    let tr = (a + c) / 2.0;
    let d = ((a - c).powi(2) / 4.0 + b * b).sqrt();
    let (l1, l2) = (tr + d, (tr - d).max(0.0));
    let th = 0.5 * (2.0 * b).atan2(a - c);
    let (ct, st) = (th.cos(), th.sin());
    let o = r.origin.point;
    (0..48)
        .map(|k| {
            let t = k as f64 / 48.0 * std::f64::consts::TAU;
            let (u, v) = (l1.sqrt() * t.cos(), l2.sqrt() * t.sin());
            [o[i] + u * ct - v * st, o[j] + u * st + v * ct]
        })
        .collect()
}

/// A view onto axes (i, j): stains, their paths to the point nearest the origin, the
/// origin and its 95 % region's outline.
fn view(r: &Run, i: usize, j: usize, caption: String) -> Figure {
    let o = r.origin.point;
    let ends: Vec<[f64; 3]> = r
        .stains
        .iter()
        .zip(&r.inputs)
        .map(|(s, inp)| {
            let d = [0, 1, 2].map(|k| o[k] - inp.centre[k]);
            let along = (0..3).map(|k| d[k] * s.ray[k]).sum::<f64>().max(0.0);
            [0, 1, 2].map(|k| inp.centre[k] + s.ray[k] * along)
        })
        .collect();
    let region = shadow(r, i, j);
    let mut pts: Vec<[f64; 2]> = r
        .inputs
        .iter()
        .map(|s| [s.centre[i], s.centre[j]])
        .collect();
    pts.extend(region.iter().copied());
    pts.push([o[i], o[j]]);
    let lo = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
    let hi = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max));
    let f = Frame::new(lo, hi, W, H, 8.0);
    let mut fig = Figure {
        width: W,
        height: H,
        caption,
        ..Figure::default()
    };
    fig.areas.push(Area {
        points: region.iter().map(|p| f.at(*p)).collect(),
        fill: REGION.into(),
    });
    // Unused first, so the used paths draw on top.
    for used in [false, true] {
        for ((s, inp), end) in r.stains.iter().zip(&r.inputs).zip(&ends) {
            if s.used != used {
                continue;
            }
            let a = f.at([inp.centre[i], inp.centre[j]]);
            let b = f.at([end[i], end[j]]);
            fig.coloured(a, b, 0.2, !used, if used { USED } else { UNUSED });
            fig.dots.push([a[0], a[1], 0.45]);
        }
    }
    let c = f.at([o[i], o[j]]);
    fig.dots.push([c[0], c[1], 0.9]);
    fig.labels.push(Label {
        x: c[0] + 1.5,
        y: c[1] - 1.5,
        text: "origin".into(),
    });
    fig.scale_bar(&f);
    fig
}

fn plan(r: &Run) -> Figure {
    view(
        r,
        0,
        1,
        "Plan (x east, y north). Dots: stains; solid dark red: paths of the stains used, back to the point nearest the origin; dashed grey: stains left out; shaded: the outline of the 95 % region seen from above.".into(),
    )
}

fn elevation(r: &Run) -> Figure {
    view(
        r,
        0,
        2,
        "Elevation looking north (x east against height z), as in the plan. The height is the least reliable coordinate: straight-line paths put it too high.".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::pdf;
    use locus_analysis::bloodstain::{run, Parameters, StainInput};
    use locus_analysis::trajectory::{PhotoRef, PointSource};

    fn sample() -> (Meta, Run) {
        let origin = [1.6, 1.4, 1.0];
        let mut inputs = vec![];
        for k in 0..14 {
            let f = k as f64 / 14.0;
            let (c, n) = if k % 2 == 0 {
                (
                    [0.0, 0.3 + 2.4 * f, 1.3 + 0.1 * (k % 5) as f64],
                    [1.0, 0.0, 0.0],
                )
            } else {
                (
                    [0.3 + 2.6 * f, 0.0, 1.25 + 0.12 * (k % 4) as f64],
                    [0.0, 1.0, 0.0],
                )
            };
            let d = [0, 1, 2].map(|i| c[i] - origin[i]);
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let v = d.map(|x| x / len);
            let sin_a = -(v[0] * n[0] + v[1] * n[1] + v[2] * n[2]);
            let t = [0, 1, 2].map(|i| v[i] + n[i] * sin_a);
            let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            // A little noise, so the fit has scatter.
            let w = 0.004 * (1.0 + 0.01 * ((k * 37 % 11) as f64 - 5.0) / 5.0);
            inputs.push(StainInput {
                label: format!("{}", k + 1),
                surface: if k % 2 == 0 { "wall x=0" } else { "wall y=0" }.into(),
                centre: c,
                normal: n,
                width: Measured {
                    value: w,
                    sigma: 0.0001,
                },
                length: Measured {
                    value: 0.004 / sin_a,
                    sigma: 0.0001,
                },
                travel: t.map(|x| x / tl),
                travel_sigma_deg: 2.0,
                excluded: (k == 5).then(|| "overlaps a cast-off stain".into()),
                photo: Some(PhotoRef {
                    evidence_id: 20 + k as i64,
                    name: format!("stain-{k:02}.jpg"),
                    file: format!("evidence/{}/stain-{k:02}.jpg", 20 + k),
                    sha256: "ab".repeat(32),
                }),
                sources: vec![PointSource {
                    scan: "1:0".into(),
                    index: 1000 + k as u32,
                    revision: 0,
                }],
                ..Default::default()
            });
        }
        let r = run(
            inputs,
            Parameters {
                bootstrap: 300,
                ..Parameters::default()
            },
        )
        .unwrap();
        let meta = Meta {
            project: "Main Street".into(),
            record_id: 4,
            name: "Kitchen spatter".into(),
            method: r.method.clone(),
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
        assert!(text.contains(&xyz(r.origin.point)), "{text}");
        assert!(text.contains("2026-00417"));
        assert!(text.contains("Straight-line paths ignore gravity"));
        assert!(text.contains("overlaps a cast-off stain"));
        assert!(text.contains(&format!("{} of 14", r.origin.stains_used)));
        assert!(text.contains("stain-03.jpg (evidence 23)"));
        assert!(text.contains("1:0 #1003 r0"));
        assert!(text.contains("Sign-off") && text.contains("Technical review"));
        assert!(text.contains("Assumptions") && text.contains("Limitations"));
        // Few stains: the report says so.
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
        let tmp = std::env::temp_dir();
        std::fs::write(tmp.join("locus-bloodstain.pdf"), &out.pdf).unwrap();
        let data = serde_json::to_vec(&doc).unwrap();
        for (i, png) in crate::pages_png(crate::analysis::TEMPLATE, data, vec![])
            .iter()
            .enumerate()
        {
            std::fs::write(tmp.join(format!("locus-bloodstain-{}.png", i + 1)), png).unwrap();
        }
    }

    #[test]
    fn an_override_is_stated_and_warned() {
        let (meta, mut r) = sample();
        r.parameters.include_not_upward = Some("the pattern is on one wall only".into());
        let doc = report(&meta, &r);
        assert!(doc
            .warnings
            .iter()
            .any(|w| w.contains("the pattern is on one wall only")));
        r.origin.chi2 = 10.0 * r.origin.dof as f64;
        r.origin.stains_used = 6;
        let doc = report(&meta, &r);
        assert!(doc.warnings.iter().any(|w| w.contains("scatter more")));
        assert!(doc.warnings.iter().any(|w| w.contains("Only 6 stains")));
    }
}
