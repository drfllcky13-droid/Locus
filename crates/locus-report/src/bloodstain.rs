//! The bloodstain area-of-origin report, built only from the stored run (so it can be
//! reprinted from the record at any time and says exactly what was computed).

use crate::analysis::{details, Area, Block, Figure, Frame, Label, Meta, Report, Section};
use locus_analysis::bloodstain::{Run, LARGE_PERSPECTIVE, NEAR_ROUND_DEG, NEAR_ROUND_DOMINANT};
use locus_analysis::measure::Measured;

/// Figure colours: stains used and their paths, those left out, the origin's 95 % region.
const USED: &str = "#8e1b1f";
const UNUSED: &str = "#9a9a9a";
const REGION: &str = "#f0c9c4";
/// The plan-view convergence of floor stains: its lines and 95 % ellipse.
const FLOOR: &str = "#2f6fbf";
const FLOOR_REGION: &str = "#c9dcf2";

/// The scale's stated size and the scan disagree by more than this, and by more than 2.5 × the
/// pairs' own scale uncertainty (the same, relative, as their rotation's in radians): warned
/// about. Scan points snap to within the point spacing, so on a small scale the pairs alone
/// can be several per cent off.
const SCALE_MISMATCH: f64 = 0.02;

/// Why the angle fit is primary, and what the conventional point is.
const WHY_ANGLES: &str = "The origin is fitted in the stains' measured angles: the point whose straight paths best match every stain's impact angle and direction, each in units of its own 1σ. That is the maximum-likelihood point when the angles' errors are independent and normal, and it weights each stain by how well it was measured, as Camana (2013) does for the directions in plan. The conventional point, the least-squares point nearest all the paths (perpendicular distances, every stain alike), is shown beside it for comparison. It is not used as the result because a stain's direction error does not scatter its path evenly about the true origin: a path turned by θ about the surface's normal misses the origin, at distance d, on one side, by d sin α cos α (1 − cos θ) for impact angle α. Errors of that kind pull the conventional point consistently in one direction, and resampling cannot show it. Illes and Boué (2013) describe the same weakness of plain averaging over stains and use robust estimators instead.";

const REFERENCES: &[&str] = &[
    "V. Balthazard, R. Piédelièvre, H. Desoille and L. Derobert, \"Étude des gouttes de sang projeté\", Annales de médecine légale, 19, 1939.",
    "T. Bevel and R. M. Gardner, Bloodstain Pattern Analysis with an Introduction to Crime Scene Reconstruction, 3rd ed., CRC Press, 2008.",
    "F. Camana, \"Determining the area of convergence in bloodstain pattern analysis: a probabilistic approach\", Forensic Science International, 231, 2013, 131–136.",
    "M. Illes and M. Boué, \"Robust estimation for area of origin in bloodstain pattern analysis via directional analysis\", Forensic Science International, 226, 2013, 223–229.",
    "A. L. Carter, \"The directional analysis of bloodstain patterns: theory and experimental validation\", Canadian Society of Forensic Science Journal, 34(4), 2001.",
    "R. Hartley and A. Zisserman, Multiple View Geometry in Computer Vision, 2nd ed., Cambridge University Press, 2004: the four-point homography and its normalisation.",
    "B. Efron and R. J. Tibshirani, An Introduction to the Bootstrap, Chapman & Hall, 1993.",
    "R. A. Johnson and D. W. Wichern, Applied Multivariate Statistical Analysis, 6th ed., Pearson, 2007: Hotelling's T² regions.",
];

/// The method's validation (docs/methods/bloodstain.md; crates/locus-validate/tests/
/// bloodstain.rs): both fits on the same stains.
const VALIDATION: [[&str; 5]; 2] = [
    [
        "Generator's photos: fiducial alignment, automatic edges (8 rooms)",
        "0.6 mm",
        "8 of 8",
        "4.8 mm",
        "0 of 8",
    ],
    [
        "Hand-measurement noise, mostly small near-round stains (40 rooms)",
        "233 mm",
        "95 %",
        "302 mm",
        "92 %",
    ],
];

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
    if o.near_round_share > NEAR_ROUND_DOMINANT {
        let round: Vec<String> = r
            .stains
            .iter()
            .zip(&r.inputs)
            .filter(|(s, _)| s.used && s.impact.value >= NEAR_ROUND_DEG)
            .map(|(_, i)| i.label.clone())
            .collect();
        warnings.push(format!(
            "Near-round stains (impact {NEAR_ROUND_DEG:.0}° or more: {}) carry {:.0} % of the fit's information. Their width over length, and so their impact angle and direction, are poorly determined: a small error in either axis moves them a lot. Check those stains' edges and tails, and compare the origin without them.",
            round.join(", "),
            o.near_round_share * 100.0
        ));
    }
    for i in &r.inputs {
        let Some(a) = &i.alignment else { continue };
        if let Some(rect) = &a.rectification {
            let stretch = i
                .fit
                .as_ref()
                .and_then(|f| f.perspective)
                .map_or(rect.stretch, |p| p.stretch);
            if stretch > LARGE_PERSPECTIVE {
                warnings.push(format!(
                    "Stain {}: its photo was corrected for perspective by {:.0} % (about {:.0}° off square-on). The correction rests on four corner clicks on the scale; a photo taken square on is better evidence.",
                    i.label,
                    stretch * 100.0,
                    (1.0 / (1.0 + stretch)).acos().to_degrees()
                ));
            }
        }
        if let Some(q) = a.scale_ratio {
            let expected = a.rotation_sigma_deg.to_radians();
            if (q - 1.0).abs() > SCALE_MISMATCH.max(2.5 * expected) {
                warnings.push(format!(
                    "Stain {}: the scale's stated size and the scan disagree by {:.1} %, more than the point pairs' uncertainty allows (1σ {:.1} %). Check the scale's size, its corners and the point pairs.",
                    i.label,
                    (q - 1.0) * 100.0,
                    expected * 100.0
                ));
            }
        }
    }
    if p.floor_convergence {
        if let Some(n) = &r.convergence_note {
            warnings.push(format!("No plan-view convergence of floor stains: {n}."));
        }
        if let Some(c) = &r.convergence {
            if c.dof > 0 && c.chi2 / c.dof as f64 > 2.0 {
                warnings.push(format!(
                    "The floor stains' directions scatter more than their uncertainties allow (χ² = {:.0} on {} degrees of freedom): some may not come from this source.",
                    c.chi2, c.dof
                ));
            }
            if c.behind.iter().any(|b| *b) {
                warnings.push("The plan-view convergence is behind some floor stains along their paths: check their tails.".into());
            }
        }
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

    let half = |e: &locus_analysis::bloodstain::Ellipsoid| {
        format!(
            "{:.3} × {:.3} × {:.3}",
            e.semi_axes[0], e.semi_axes[1], e.semi_axes[2]
        )
    };
    let mut compare = vec![vec![
        "Angles (primary)".to_string(),
        xyz(o.point),
        format!("{:.3} ± {:.3}", o.height.value, o.height.sigma),
        half(&o.ellipsoid),
        "—".into(),
    ]];
    if let Some(c) = &r.conventional {
        compare.push(vec![
            "Perpendicular distances (conventional, for comparison)".into(),
            xyz(c.point),
            format!("{:.3} ± {:.3}", c.height.value, c.height.sigma),
            half(&c.ellipsoid),
            mm(c.shift),
        ]);
    }

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
                    None => format!("used ({:.0} % of the fit)", s.influence * 100.0),
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
                    let mut t = format!(
                        "{} pairs, {}",
                        a.pairs.len(),
                        a.rms.map(mm).unwrap_or_else(|| "exact (2 pairs)".into())
                    );
                    if let Some(rect) = &a.rectification {
                        let p = f.and_then(|f| f.perspective);
                        t += &format!(
                            "; perspective corrected from a {:.0} × {:.0} mm scale (stretch {:.0} %, corner 1σ {:.1} px{}); scale over scan {:.3}",
                            rect.size[0] * 1000.0,
                            rect.size[1] * 1000.0,
                            p.map_or(rect.stretch, |p| p.stretch) * 100.0,
                            rect.corner_sigma_px,
                            p.map(|p| format!(", adding {:.3} mm to the width", p.width_sigma * 1000.0))
                                .unwrap_or_default(),
                            a.scale_ratio.unwrap_or(1.0)
                        );
                    }
                    t
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
                Block::Text {
                    text: "The origin fitted in the stains' angles is the result. The conventional point, nearest all the paths by perpendicular distance, is shown for comparison, with its own 95 % region from the same resamples and its distance from the result (see Method for why it is not used).".into(),
                },
                Block::Table {
                    widths: ["1fr", "auto", "auto", "auto", "auto"]
                        .map(String::from)
                        .to_vec(),
                    head: [
                        "Fit",
                        "Point (m)",
                        "Height (m)",
                        "95 % half-axes (m)",
                        "From the result",
                    ]
                    .map(String::from)
                    .to_vec(),
                    rows: compare,
                },
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
    if let Some(c) = &r.convergence {
        let labels: Vec<String> = c
            .stains
            .iter()
            .map(|&k| r.inputs[k].label.clone())
            .collect();
        let rows = c
            .stains
            .iter()
            .zip(&c.residuals)
            .zip(&c.behind)
            .map(|((&k, res), behind)| {
                vec![
                    r.inputs[k].label.clone(),
                    deg(&r.stains[k].directionality),
                    mm(*res),
                    if *behind { "yes".into() } else { "no".into() },
                ]
            })
            .collect();
        sections.insert(
            1,
            Section {
                heading: "Floor stains: convergence in plan".into(),
                blocks: vec![
                    Block::Text {
                        text: "A separate 2-D result: where the floor stains' directions of travel, traced back, converge in plan. It uses no impact angles and gives no height, and it is never mixed into the 3-D origin above. It is fitted the same way, in the directions' angles weighted by their 1σ, with the conventional point (least squares on perpendicular distances) for comparison and a 95 % ellipse from bootstrap resampling of the floor stains (Hotelling's T², 2 dimensions).".into(),
                    },
                    Block::Pairs {
                        rows: vec![
                            [
                                "Convergence (plan)".into(),
                                format!("x {:.3} m, y {:.3} m", c.point[0], c.point[1]),
                            ],
                            [
                                "1σ".into(),
                                format!("{:.3}, {:.3} m", c.sigma[0], c.sigma[1]),
                            ],
                            [
                                "95 % ellipse (half-axes)".into(),
                                format!(
                                    "{:.3} m along ({:.2}, {:.2}); {:.3} m across",
                                    c.semi_axes[0], c.axis[0], c.axis[1], c.semi_axes[1]
                                ),
                            ],
                            [
                                "Conventional point, for comparison".into(),
                                format!(
                                    "x {:.3} m, y {:.3} m ({} from the result)",
                                    c.conventional[0],
                                    c.conventional[1],
                                    mm((c.conventional[0] - c.point[0])
                                        .hypot(c.conventional[1] - c.point[1]))
                                ),
                            ],
                            [
                                "Fit".into(),
                                format!(
                                    "χ² = {:.1} on {} degrees of freedom; {} floor stains ({})",
                                    c.chi2,
                                    c.dof,
                                    c.stains.len(),
                                    labels.join(", ")
                                ),
                            ],
                        ],
                    },
                    Block::Table {
                        widths: ["auto", "1fr", "auto", "auto"].map(String::from).to_vec(),
                        head: ["#", "Direction", "Distance from its line", "Behind it"]
                            .map(String::from)
                            .to_vec(),
                        rows,
                    },
                    Block::Figure(floor_plan(r, c)),
                ],
            },
        );
    }
    sections.push(Section {
        heading: "Method".into(),
        blocks: vec![
            Block::Text {
                text: "Each photo is placed on its surface by a similarity (scale, rotation, shift) fitted to pixel–scan point pairs, on the plane fitted to the scan around them. A photo not taken square on is first corrected for perspective by the homography taking four corners of a rectangle on its scale onto that rectangle's stated size (Hartley and Zisserman 2004); the corners' click error is carried to each stain by Monte Carlo (64 redraws). An ellipse is fitted to the stain's edge points by least squares, leaving out points well off it (the tail). The impact angle is asin(width / length) and the direction of travel is along the long axis toward the marked tail. The origin is the point whose straight paths to the stains best match every stain's impact angle and direction, each in units of its 1σ (Levenberg–Marquardt, started from the point nearest all the paths). The 95 % region is an ellipsoid from bootstrap resampling of the stains used, with a radius from Hotelling's T² for the number of stains. See docs/methods/bloodstain.md.".into(),
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
                        "Near-round stains".into(),
                        format!(
                            "impact {NEAR_ROUND_DEG:.0}° or more; flagged when they carry more than {:.0} % of the fit's information (the summed squared derivatives of their misfits at the origin); here {:.0} %",
                            NEAR_ROUND_DOMINANT * 100.0,
                            o.near_round_share * 100.0
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
    let method = sections.len() - 1;
    sections[method].blocks.extend([
        Block::Text {
            text: WHY_ANGLES.into(),
        },
        Block::Text {
            text: "Validation on synthetic scenes with known origins, both fits on the same stains used:".into(),
        },
        Block::Table {
            widths: ["1fr", "auto", "auto", "auto", "auto"]
                .map(String::from)
                .to_vec(),
            head: [
                "Case",
                "Angles: mean error",
                "Region covers truth",
                "Conventional: mean error",
                "Region covers truth",
            ]
            .map(String::from)
            .to_vec(),
            rows: VALIDATION
                .iter()
                .map(|r| r.iter().map(|c| c.to_string()).collect())
                .collect(),
        },
        Block::List {
            items: REFERENCES.iter().map(|r| r.to_string()).collect(),
        },
    ]);
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

/// The floor stains in plan, their lines back to the convergence, and its 95 % ellipse.
fn floor_plan(r: &Run, c: &locus_analysis::bloodstain::Convergence) -> Figure {
    let ellipse: Vec<[f64; 2]> = (0..48)
        .map(|k| {
            let t = k as f64 / 48.0 * std::f64::consts::TAU;
            let (u, v) = (c.semi_axes[0] * t.cos(), c.semi_axes[1] * t.sin());
            [
                c.point[0] + u * c.axis[0] - v * c.axis[1],
                c.point[1] + u * c.axis[1] + v * c.axis[0],
            ]
        })
        .collect();
    let mut pts: Vec<[f64; 2]> = c
        .stains
        .iter()
        .map(|&k| [r.inputs[k].centre[0], r.inputs[k].centre[1]])
        .collect();
    pts.extend(ellipse.iter().copied());
    pts.push(c.point);
    let lo = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
    let hi = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max));
    let f = Frame::new(lo, hi, W, H, 8.0);
    let mut fig = Figure {
        width: W,
        height: H,
        caption: "Floor stains in plan (x east, y north): each stain's direction traced back toward the convergence (blue), and its 95 % ellipse. A 2-D result, separate from the 3-D origin.".into(),
        ..Figure::default()
    };
    fig.areas.push(Area {
        points: ellipse.iter().map(|p| f.at(*p)).collect(),
        fill: FLOOR_REGION.into(),
    });
    for &k in &c.stains {
        let i = &r.inputs[k];
        let (a, t) = ([i.centre[0], i.centre[1]], i.travel);
        let n = t[0].hypot(t[1]).max(1e-12);
        let d = [c.point[0] - a[0], c.point[1] - a[1]];
        let along = (-(d[0] * t[0] + d[1] * t[1]) / n).max(0.0);
        let end = [a[0] - t[0] / n * along, a[1] - t[1] / n * along];
        let (pa, pb) = (f.at(a), f.at(end));
        fig.coloured(pa, pb, 0.2, false, FLOOR);
        fig.dots.push([pa[0], pa[1], 0.45]);
        fig.labels.push(Label {
            x: pa[0] + 1.0,
            y: pa[1] + 2.5,
            text: i.label.clone(),
        });
    }
    let q = f.at(c.point);
    fig.dots.push([q[0], q[1], 0.9]);
    fig.labels.push(Label {
        x: q[0] + 1.5,
        y: q[1] - 1.5,
        text: "convergence".into(),
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
            record_entry: Some("#12, ab12".into()),
        };
        (meta, r)
    }

    #[test]
    fn the_pdf_says_what_the_run_computed() {
        let (meta, r) = sample();
        crate::analysis::assert_reproducible(&meta, |m| report(m, &r));
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
    fn the_conventional_point_floor_convergence_and_flags_are_reported() {
        let (meta, mut r) = sample();
        let c = r.conventional.clone().unwrap();
        let text = pdf(&report(&meta, &r), vec![]).unwrap().text;
        assert!(
            text.contains("Perpendicular distances (conventional"),
            "{text}"
        );
        assert!(text.contains(&xyz(c.point)));
        assert!(text.contains("Illes and M. Boué") && text.contains("Camana"));
        assert!(text.contains("302 mm"));
        // Floor convergence, as its own section, never in the origin.
        let origin = r.origin.clone();
        let mut inputs = r.inputs.clone();
        for (k, (x, y)) in [(0.8, 0.6), (2.4, 0.9), (2.2, 2.3), (0.9, 2.2)]
            .iter()
            .enumerate()
        {
            let d: [f64; 3] = [x - 1.6, y - 1.4, -1.0];
            let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let sin_a = -d[2] / n;
            let h = d[0].hypot(d[1]);
            inputs.push(StainInput {
                label: format!("F{}", k + 1),
                surface: "floor".into(),
                centre: [*x, *y, 0.0],
                normal: [0.0, 0.0, 1.0],
                width: Measured {
                    value: 0.004,
                    sigma: 0.0001,
                },
                length: Measured {
                    value: 0.004 / sin_a,
                    sigma: 0.0001,
                },
                travel: [d[0] / h, d[1] / h, 0.0],
                travel_sigma_deg: 2.0,
                ..Default::default()
            });
        }
        r = run(
            inputs,
            Parameters {
                bootstrap: 300,
                floor_convergence: true,
                ..Parameters::default()
            },
        )
        .unwrap();
        assert_eq!(r.origin.stains_used, origin.stains_used);
        let doc = report(&meta, &r);
        let text = pdf(&doc, vec![]).unwrap().text;
        assert!(text.contains("Floor stains: convergence in plan"), "{text}");
        assert!(text.contains("x 1.600 m, y 1.400 m"), "{text}");
        // Flags: near-round stains dominating, a large perspective correction.
        r.origin.near_round_share = 0.8;
        let doc = report(&meta, &r);
        assert!(doc.warnings.iter().any(|w| w.contains("Near-round stains")));
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
