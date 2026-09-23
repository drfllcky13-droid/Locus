//! Crash reconstruction reports (skid, yaw, momentum, crush), built only from the stored runs.

use crate::analysis::{details, Area, Block, Figure, Frame, Label, Meta, Report, Section};
use locus_analysis::crash::{
    CrushRun, Input, MomentumRun, SkidRun, Spread, YawRadius, YawRun, G, MIN_ARC_DEG,
};
use locus_analysis::crush_volume::VolumeRun;
use locus_analysis::edr::EdrRun;

const MARK: &str = "#2f6fbf";
const FIT: &str = "#c0392b";

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

fn speed(v: f64) -> String {
    format!("{v:.2} m/s ({:.1} km/h)", v * 3.6)
}

/// An input as entered: its value and its range, and how it is drawn.
fn input(i: &Input, unit: &str, digits: usize) -> String {
    if i.low == i.high {
        format!("{:.*} {unit}", digits, i.value)
    } else {
        format!(
            "{:.*} {unit} (range {:.*} to {:.*}; {})",
            digits,
            i.value,
            digits,
            i.low,
            digits,
            i.high,
            if i.normal {
                "normal, the range as ±2σ"
            } else {
                "uniform"
            }
        )
    }
}

/// A result: the value, the range method's extremes, and the Monte Carlo interval.
fn spread_rows(name: &str, s: &Spread, fmt: &dyn Fn(f64) -> String) -> Vec<[String; 2]> {
    vec![
        [name.into(), fmt(s.value)],
        [
            "  range method (every corner of the input ranges)".into(),
            format!("{} to {}", fmt(s.low), fmt(s.high)),
        ],
        [
            "  Monte Carlo".into(),
            format!(
                "95 % {} to {}; mean {}, 1σ {}{}",
                fmt(s.interval95[0]),
                fmt(s.interval95[1]),
                fmt(s.mean),
                fmt(s.sd),
                if s.failed > 0 {
                    format!(
                        " ({} of {} draws had no answer)",
                        s.failed,
                        s.draws + s.failed
                    )
                } else {
                    format!(" ({} draws)", s.draws)
                }
            ),
        ],
    ]
}

fn tail(
    sections: &mut Vec<Section>,
    method: &str,
    refs: &[&str],
    assumptions: &[String],
    limitations: &[String],
) {
    sections.push(Section {
        heading: "Method".into(),
        blocks: vec![
            Block::Text {
                text: format!("{method} Every input is a value with a range. The result is given from the inputs' values; by the range method, its smallest and largest over every corner of the inputs' ranges; and by a 20,000-draw Monte Carlo (each input uniform over its range, or normal with the range as ±2σ where marked; seeded, so a run repeats exactly), as a 95 % interval. g = 9.80665 m/s². See docs/methods/crash.md."),
            },
            Block::List {
                items: refs.iter().map(|s| s.to_string()).collect(),
            },
        ],
    });
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

fn report(meta: &Meta, title: &str, warnings: Vec<String>, sections: Vec<Section>) -> Report {
    let mut w = vec![];
    if let Some(x) = &meta.withdrawn {
        w.push(format!("This analysis was withdrawn: {x}."));
    }
    w.extend(warnings);
    Report {
        title: format!("{title}: {}", meta.name),
        header: header(meta),
        details: details(meta),
        warnings: w,
        sections,
    }
}

const REFS_SKID: &[&str] = &[
    "J. C. Collins, Accident Reconstruction, Charles C. Thomas, 1979: speed from skid marks, drag factor and grade.",
    "L. B. Fricke, Traffic Accident Reconstruction, Northwestern University Traffic Institute, 1990: drag factor, braking efficiency and combined speeds.",
];

pub fn skid(meta: &Meta, r: &SkidRun) -> Report {
    let rows = r
        .segments
        .iter()
        .zip(&r.results)
        .map(|(s, x)| {
            vec![
                s.label.clone(),
                input(&s.distance, "m", 2),
                input(&s.drag, "", 2),
                input(&s.braking, "", 2),
                input(&s.grade, "", 3),
                format!("{:.3}", x.effective_drag),
                speed(x.speed_alone),
                if s.path.len() >= 2 {
                    format!(
                        "picked on the cloud ({} points: {})",
                        s.path.len(),
                        s.sources
                            .iter()
                            .map(|p| format!("{} #{} r{}", p.scan, p.index, p.revision))
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                } else {
                    "entered".into()
                },
            ]
        })
        .collect();
    let mut result = spread_rows("Speed at the start of the marks", &r.speed, &|v| speed(v));
    result.insert(
        0,
        [
            "Speed at the end of the marks".into(),
            input(&r.end_speed, "m/s", 2),
        ],
    );
    let mut sections = vec![
        Section {
            heading: "Result".into(),
            blocks: vec![Block::Pairs { rows: result }],
        },
        Section {
            heading: "Marks and surfaces".into(),
            blocks: vec![
                Block::Text {
                    text: "Each stretch of mark: its length, the surface's drag factor, the braking efficiency and the grade (rise over run, positive uphill), the effective drag factor f = μ n cos θ + sin θ (θ = atan grade), and the speed that stretch alone takes off from a stop, √(2 g f d). The speed at the start of the marks is √(v_end² + Σ 2 g f d).".into(),
                },
                Block::Table {
                    widths: ["auto", "1fr", "auto", "auto", "auto", "auto", "auto", "1fr"]
                        .map(String::from)
                        .to_vec(),
                    head: ["Stretch", "Length", "μ", "n", "Grade", "f", "Alone", "Measured"]
                        .map(String::from)
                        .to_vec(),
                    rows,
                },
            ],
        },
    ];
    tail(
        &mut sections,
        "The speed at the start of the marks is found from the work done by friction over them: v² = v_end² + Σ 2 g f d, with each stretch's effective drag factor adjusted for grade and braking efficiency.",
        REFS_SKID,
        &r.assumptions,
        &r.limitations,
    );
    report(meta, "Speed from skid marks", vec![], sections)
}

const REFS_YAW: &[&str] = &[
    "L. B. Fricke, Traffic Accident Reconstruction, Northwestern University Traffic Institute, 1990: critical speed, chord and middle ordinate.",
];

pub fn yaw(meta: &Meta, r: &YawRun) -> Report {
    let mut inputs = vec![];
    match &r.radius_from {
        YawRadius::Chord { chord, ordinate } => {
            inputs.push(["Chord".into(), input(chord, "m", 2)]);
            inputs.push(["Middle ordinate".into(), input(ordinate, "m", 3)]);
        }
        YawRadius::Points { points, sources } => {
            inputs.push([
                "Mark".into(),
                format!(
                    "{} points picked on the cloud: {}",
                    points.len(),
                    sources
                        .iter()
                        .map(|p| format!("{} #{} r{}", p.scan, p.index, p.revision))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            ]);
        }
    }
    if let Some(c) = &r.circle {
        inputs.push([
            "Fitted circle".into(),
            format!(
                "radius {:.2} m ± {:.2} m (1σ), {:.1} mm RMS off the circle, over {:.0}° of arc",
                c.radius,
                c.radius_sigma,
                c.rms * 1000.0,
                c.arc_deg
            ),
        ]);
    }
    inputs.push(["Drag factor".into(), input(&r.drag, "", 2)]);
    inputs.push(["Superelevation".into(), input(&r.superelevation, "", 3)]);
    inputs.push([
        "To the centre of mass's path".into(),
        format!("{:.3} m inside the mark (half the track)", r.cg_offset),
    ]);
    let mut result = spread_rows("Radius of the centre of mass's path", &r.radius, &|v| {
        format!("{v:.2} m")
    });
    result.extend(spread_rows("Critical speed", &r.speed, &|v| speed(v)));
    let mut warnings = vec![];
    if let Some(c) = &r.circle {
        if c.arc_deg < MIN_ARC_DEG {
            warnings.push(format!(
                "The picked points span only {:.0}° of arc: the radius is poorly determined (its 1σ is {:.1} m). Pick along more of the mark's early part.",
                c.arc_deg, c.radius_sigma
            ));
        }
    }
    if let Some(c) = &r.circle {
        if c.rms > 0.05 {
            warnings.push(format!(
                "The picked points lie {:.0} mm (RMS) off the fitted circle: check that every point is on the mark (a point on a kerb, a wall or a vehicle pulls the circle).",
                c.rms * 1000.0
            ));
        }
    }
    if r.radius.high > 1.5 * r.radius.low {
        warnings.push(match &r.radius_from {
            YawRadius::Chord { .. } => "The radius's range is wide (the middle ordinate is small or uncertain for this chord). A longer chord, or points fitted along the mark, would narrow it.".into(),
            YawRadius::Points { .. } => "The radius's range is wide: the points are few, span little of the arc, or scatter off it. More points along the early part of the mark would narrow it.".into(),
        });
    }
    let mut blocks = vec![Block::Pairs { rows: result }];
    if let (YawRadius::Points { points, .. }, Some(c)) = (&r.radius_from, &r.circle) {
        blocks.push(Block::Figure(yaw_figure(points, c)));
    }
    let mut sections = vec![
        Section {
            heading: "Result".into(),
            blocks,
        },
        Section {
            heading: "Inputs".into(),
            blocks: vec![Block::Pairs { rows: inputs }],
        },
    ];
    tail(
        &mut sections,
        &format!("The radius is R = C²/(8M) + M/2 from a chord C and middle ordinate M, or a least-squares circle fitted to points picked along the mark (in the points' plane, by their distances from it), less the offset to the centre of mass's path. The critical speed is v = √(g R (μ + e)/(1 − μ e)) for drag factor μ and superelevation e, which is √(μ g R) on the level (g = {G} m/s²)."),
        REFS_YAW,
        &r.assumptions,
        &r.limitations,
    );
    report(meta, "Critical speed from yaw marks", warnings, sections)
}

fn yaw_figure(points: &[[f64; 3]], c: &locus_analysis::crash::CircleFit) -> Figure {
    let mut pts: Vec<[f64; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
    let ang: Vec<f64> = pts
        .iter()
        .map(|p| (p[1] - c.centre[1]).atan2(p[0] - c.centre[0]))
        .collect();
    let a0 = ang[0];
    let rel: Vec<f64> = ang
        .iter()
        .map(|a| {
            (a - a0 + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
        })
        .collect();
    let (lo, hi) = (
        rel.iter().cloned().fold(f64::INFINITY, f64::min),
        rel.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    );
    let arc: Vec<[f64; 2]> = (0..=40)
        .map(|k| {
            let t = a0 + lo + (hi - lo) * k as f64 / 40.0;
            [
                c.centre[0] + c.radius * t.cos(),
                c.centre[1] + c.radius * t.sin(),
            ]
        })
        .collect();
    pts.extend(arc.iter().copied());
    let l = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
    let h = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max));
    let f = Frame::new(l, h, 170.0, 70.0, 6.0);
    let mut fig = Figure {
        width: 170.0,
        height: 70.0,
        caption: "Plan (x east, y north): the points picked along the mark (dots) and the fitted circle's arc (red).".into(),
        ..Figure::default()
    };
    for w in arc.windows(2) {
        fig.coloured(f.at(w[0]), f.at(w[1]), 0.3, false, FIT);
    }
    for p in points {
        let q = f.at([p[0], p[1]]);
        fig.dots.push([q[0], q[1], 0.6]);
    }
    fig.scale_bar(&f);
    fig
}

const REFS_MOMENTUM: &[&str] = &[
    "J. C. Collins, Accident Reconstruction, Charles C. Thomas, 1979: linear momentum in two dimensions.",
    "R. M. Brach and R. M. Brach, Vehicle Accident Analysis and Reconstruction Methods, 2nd ed., SAE International, 2011: momentum, its conditioning and sensitivity.",
];

pub fn momentum(meta: &Meta, r: &MomentumRun) -> Report {
    let vehicles: Vec<Vec<String>> = r
        .vehicles
        .iter()
        .map(|v| {
            vec![
                v.label.clone(),
                input(&v.mass, "kg", 0),
                input(&v.approach_deg, "°", 1),
                input(&v.departure_deg, "°", 1),
                input(&v.departure_speed, "m/s", 2),
            ]
        })
        .collect();
    let mut result = vec![];
    for k in 0..2 {
        result.extend(spread_rows(
            &format!("{}: impact speed", r.vehicles[k].label),
            &r.speeds[k],
            &|v| speed(v),
        ));
    }
    for k in 0..2 {
        result.extend(spread_rows(
            &format!("{}: change in velocity (delta-V)", r.vehicles[k].label),
            &r.delta_v[k],
            &|v| speed(v),
        ));
    }
    result.push([
        "Angle between the approach directions".into(),
        format!("{:.1}° (best near 90°)", r.approach_angle_deg),
    ]);
    let sens: Vec<Vec<String>> = r
        .sensitivity
        .iter()
        .map(|s| {
            vec![
                s.input.clone(),
                format!("{:.2} to {:.2}", s.low_input, s.high_input),
                format!("{:.2} → {:.2}", s.at_low[0], s.at_high[0]),
                format!("{:.2} → {:.2}", s.at_low[1], s.at_high[1]),
            ]
        })
        .collect();
    let mut sections = vec![
        Section {
            heading: "Result".into(),
            blocks: vec![Block::Pairs { rows: result }, Block::Figure(momentum_figure(r))],
        },
        Section {
            heading: "Vehicles".into(),
            blocks: vec![
                Block::Text {
                    text: "Directions of travel are clockwise from project north (+y). The departure speed is the speed just after separation, from the post-impact travel.".into(),
                },
                Block::Table {
                    widths: ["1fr", "1fr", "1fr", "1fr", "1fr"].map(String::from).to_vec(),
                    head: ["Vehicle", "Mass", "Approach", "Departure", "Departure speed"]
                        .map(String::from)
                        .to_vec(),
                    rows: vehicles,
                },
            ],
        },
    ];
    if !sens.is_empty() {
        sections.push(Section {
            heading: "Sensitivity".into(),
            blocks: vec![
                Block::Text {
                    text: format!(
                        "Each input moved to the low and then the high end of its range, the others at their values: the impact speeds of {} and {} (m/s).",
                        r.vehicles[0].label, r.vehicles[1].label
                    ),
                },
                Block::Table {
                    widths: ["1fr", "auto", "auto", "auto"].map(String::from).to_vec(),
                    head: [
                        "Input".to_string(),
                        "Range".to_string(),
                        r.vehicles[0].label.clone(),
                        r.vehicles[1].label.clone(),
                    ]
                    .to_vec(),
                    rows: sens,
                },
            ],
        });
    }
    tail(
        &mut sections,
        "Conservation of linear momentum in the plane: m₁ v₁ d(θ₁) + m₂ v₂ d(θ₂) = m₁ u₁ d(φ₁) + m₂ u₂ d(φ₂), for masses m, approach directions θ, departure directions φ and departure speeds u (d(·) the unit vector of a direction), solved for the impact speeds v₁ and v₂. Each vehicle's delta-V is |v d(θ) − u d(φ)|.",
        REFS_MOMENTUM,
        &r.assumptions,
        &r.limitations,
    );
    report(meta, "Linear momentum", r.warnings.clone(), sections)
}

/// The momentum diagram: each vehicle's approach momentum and the total, which the departure
/// momenta also sum to.
fn momentum_figure(r: &MomentumRun) -> Figure {
    let d = |deg: f64| {
        let t = deg.to_radians();
        [t.sin(), t.cos()]
    };
    let vs = &r.vehicles;
    let p1 = d(vs[0].approach_deg.value).map(|x| x * vs[0].mass.value * r.speeds[0].value);
    let p2 = d(vs[1].approach_deg.value).map(|x| x * vs[1].mass.value * r.speeds[1].value);
    let tot = [p1[0] + p2[0], p1[1] + p2[1]];
    let q1 =
        d(vs[0].departure_deg.value).map(|x| x * vs[0].mass.value * vs[0].departure_speed.value);
    let pts = [[0.0, 0.0], p1, tot, q1];
    let lo = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
    let hi = [0, 1].map(|k| pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max));
    let f = Frame::new(lo, hi, 120.0, 70.0, 8.0);
    let mut fig = Figure {
        width: 120.0,
        height: 70.0,
        caption: format!(
            "Momentum (north up): {} before (blue, first leg) plus {} before (blue, second leg) = the total (red); the departure momenta (dashed) sum to the same total.",
            vs[0].label, vs[1].label
        ),
        ..Figure::default()
    };
    let o = f.at([0.0, 0.0]);
    fig.coloured(o, f.at(p1), 0.4, false, MARK);
    fig.coloured(f.at(p1), f.at(tot), 0.4, false, MARK);
    fig.coloured(o, f.at(tot), 0.4, false, FIT);
    fig.coloured(o, f.at(q1), 0.3, true, "#555555");
    fig.coloured(f.at(q1), f.at(tot), 0.3, true, "#555555");
    for (p, t) in [(p1, vs[0].label.clone()), (tot, "total".to_string())] {
        let q = f.at(p);
        fig.labels.push(Label {
            x: q[0] + 1.0,
            y: q[1] - 1.0,
            text: t,
        });
    }
    fig
}

const REFS_CRUSH: &[&str] = &[
    "K. L. Campbell, \"Energy basis for collision severity\", SAE technical paper 740565, 1974.",
    "National Highway Traffic Safety Administration, CRASH3 User's Guide and Technical Manual, US DOT (NTIS PB83-112201): the crush-energy integral, the force-direction factor, and the sample run reproduced in the tests.",
    "J. A. Neptune, \"Crush stiffness coefficients, restitution constants, and a revision of CRASH3 and SMAC\", SAE technical paper 980024, 1998.",
];

pub fn crush(meta: &Meta, r: &CrushRun) -> Report {
    let mut result = spread_rows("Crush energy", &r.energy, &|v| {
        format!("{:.1} kJ", v / 1000.0)
    });
    result.extend(spread_rows("Equivalent barrier speed", &r.ebs, &|v| {
        speed(v)
    }));
    let inputs = vec![
        ["Face".into(), r.label.clone()],
        ["A".into(), input(&r.a, "N/m", 0)],
        ["B".into(), input(&r.b, "N/m²", 0)],
        [
            "G = A²/2B".into(),
            format!("{:.0} N", r.a.value * r.a.value / (2.0 * r.b.value)),
        ],
        ["Stiffness source".into(), r.stiffness_source.clone()],
        ["Damage width".into(), input(&r.width, "m", 3)],
        [
            "Principal direction of force".into(),
            input(&r.pdof_deg, "° off the face's normal", 1),
        ],
        ["Mass".into(), input(&r.mass, "kg", 0)],
    ];
    let depths: Vec<Vec<String>> = r
        .depths
        .iter()
        .enumerate()
        .map(|(k, d)| vec![format!("C{}", k + 1), input(d, "m", 3)])
        .collect();
    let mut warnings = vec![];
    let measured = r.profile.as_ref().map(|p| {
        let rows = p
            .stations
            .iter()
            .enumerate()
            .map(|(k, s)| {
                vec![
                    format!("C{}", k + 1),
                    format!("{:.3}, {:.3}, {:.3}", s.at[0], s.at[1], s.at[2]),
                    format!("{:.3}, {:.3}, {:.3}", s.surface[0], s.surface[1], s.surface[2]),
                    format!("{:.0} ± {:.0}", s.depth * 1000.0, s.sigma * 1000.0),
                    s.points.to_string(),
                ]
            })
            .collect();
        vec![
            Block::Text {
                text: format!(
                    "The width and depths were measured on the scan. The damage's ends were picked on the undamaged face line ({:.3}, {:.3}, {:.3}) and ({:.3}, {:.3}, {:.3}), {:.3} m apart. At each equally spaced station, the scan points within ±{:.2} m of the ends' mean height ({:.2} m) in a strip across the line were taken, and the depth is where the surviving surface begins behind the line: the 5th percentile of their distances. Its 1σ combines the scan's point noise (for the surface and for the picked line) with the spread between the 5th and 15th percentiles. A depth enters the calculation as normal, ±2σ, or, where that would reach below 0, as uniform from 0 to d + 2σ.",
                    p.start[0], p.start[1], p.start[2], p.end[0], p.end[1], p.end[2], p.width, p.band, p.height
                ),
            },
            Block::Table {
                widths: ["auto", "1fr", "1fr", "auto", "auto"].map(String::from).to_vec(),
                head: ["Station", "On the face line (m)", "Surface (m)", "Crush (mm, 1σ)", "Points"]
                    .map(String::from)
                    .to_vec(),
                rows,
            },
        ]
    });
    if let Some(e) = &r.table_entry {
        if e.single_test {
            warnings.push(format!(
                "The stiffness coefficients come from a single NHTSA test ({}): one test can't show how much another of the same vehicle would differ. Their uncertainty is that test's alone (b0 and the measurements).",
                e.tests[0].test_no
            ));
        }
        if e.width_from_vehicle {
            warnings.push("For at least one source test the damage width wasn't recorded; the vehicle's overall width was used (a full-width frontal test crushes the whole front).".into());
        }
        if !r.label.to_lowercase().contains("front") {
            warnings.push(format!(
                "The table's coefficients are from frontal barrier tests; the damaged face here is \"{}\". Use them only for frontal damage.",
                r.label
            ));
        }
    }
    let mut sections = vec![
        Section {
            heading: "Result".into(),
            blocks: vec![
                Block::Pairs { rows: result },
                Block::Figure(crush_figure(r)),
            ],
        },
        Section {
            heading: "Inputs".into(),
            blocks: vec![
                Block::Pairs { rows: inputs },
                Block::Table {
                    widths: ["auto", "1fr"].map(String::from).to_vec(),
                    head: [
                        "Depth".to_string(),
                        "Residual crush (equally spaced across the width)".to_string(),
                    ]
                    .to_vec(),
                    rows: depths,
                },
            ]
            .into_iter()
            .chain(measured.into_iter().flatten())
            .collect(),
        },
    ];
    tail(
        &mut sections,
        "Under the CRASH3 model the force per unit width is linear in residual crush c (A + B c), so each strip of the damage absorbs A c + B c²/2 + G per unit width, with G = A²/(2B). The depths vary linearly between equally spaced measurements, and each span integrates exactly: Δ [A (c₁ + c₂)/2 + B (c₁² + c₁c₂ + c₂²)/6 + G]. The sum is multiplied by (1 + tan² α) for a principal direction of force α off the face's normal. The equivalent barrier speed is √(2E/m).",
        REFS_CRUSH,
        &r.assumptions,
        &r.limitations,
    );
    if let Some(e) = &r.table_entry {
        let rows = e
            .tests
            .iter()
            .map(|t| {
                vec![
                    t.test_no.to_string(),
                    format!("{:.0} kg", t.mass),
                    format!("{:.1} km/h", t.speed * 3.6),
                    format!(
                        "{:.3} m{}",
                        t.width,
                        if t.width_from_vehicle {
                            " (vehicle width)"
                        } else {
                            ""
                        }
                    ),
                    t.crush
                        .iter()
                        .map(|c| format!("{:.0}", c * 1000.0))
                        .collect::<Vec<_>>()
                        .join(", "),
                    format!("{:.0} ± {:.0}", t.a, t.a_sigma),
                    format!("{:.0} ± {:.0}", t.b, t.b_sigma),
                ]
            })
            .collect();
        sections.insert(
            2,
            Section {
                heading: "Stiffness source".into(),
                blocks: vec![
                    Block::Text {
                        text: format!(
                            "A and B for the {} {} {} from the bundled table: NHTSA full-width frontal rigid-barrier tests, by Campbell's method with the CRASH3 energy integral (b0 = 8 ± 3.2 km/h). Each test's A and B (1σ over b0 and the measurements) are listed; the vehicle's values are their mean, with 1σ combining each test's own and the spread between tests: A = {:.0} ± {:.0} N/m, B = {:.0} ± {:.0} N/m². They enter the calculation as normal inputs, ±2σ.",
                            e.model_year, e.make, e.model, e.a, e.a_sigma, e.b, e.b_sigma
                        ),
                    },
                    Block::Table {
                        widths: ["auto", "auto", "auto", "auto", "1fr", "auto", "auto"]
                            .map(String::from)
                            .to_vec(),
                        head: [
                            "NHTSA test",
                            "Mass",
                            "Speed",
                            "Width",
                            "Crush (mm)",
                            "A (N/m)",
                            "B (N/m²)",
                        ]
                        .map(String::from)
                        .to_vec(),
                        rows,
                    },
                ],
            },
        );
    }
    report(meta, "Crush energy", warnings, sections)
}

fn crush_figure(r: &CrushRun) -> Figure {
    let n = r.depths.len();
    let w = r.width.value;
    let pts: Vec<[f64; 2]> = r
        .depths
        .iter()
        .enumerate()
        .map(|(k, d)| [w * k as f64 / (n - 1) as f64, -d.value])
        .collect();
    let lo = [0.0, pts.iter().map(|p| p[1]).fold(0.0, f64::min)];
    let f = Frame::new(lo, [w, 0.0], 150.0, 50.0, 6.0);
    let mut fig = Figure {
        width: 150.0,
        height: 50.0,
        caption: "The crush profile: the undamaged face (line) and the residual crush at each measurement point, joined (red), true to scale.".into(),
        ..Figure::default()
    };
    fig.line(f.at([0.0, 0.0]), f.at([w, 0.0]), 0.3, false);
    for win in pts.windows(2) {
        fig.coloured(f.at(win[0]), f.at(win[1]), 0.4, false, FIT);
    }
    for (k, p) in pts.iter().enumerate() {
        let q = f.at(*p);
        fig.dots.push([q[0], q[1], 0.6]);
        fig.labels.push(Label {
            x: q[0] - 2.0,
            y: q[1] + 4.0,
            text: format!("C{}", k + 1),
        });
    }
    fig.scale_bar(&f);
    fig
}

const REFS_VOLUME: &[&str] = &[
    "P. J. Besl and N. D. McKay, \"A method for registration of 3-D shapes\", IEEE Transactions on Pattern Analysis and Machine Intelligence 14(2), 1992: iterative closest point registration.",
    "Y. Chen and G. Medioni, \"Object modelling by registration of multiple range images\", Image and Vision Computing 10(3), 1992: the point-to-plane distance.",
    "K. S. Arun, T. S. Huang and S. D. Blostein, \"Least-squares fitting of two 3-D point sets\", IEEE Transactions on Pattern Analysis and Machine Intelligence 9(5), 1987: the rigid fit to picked pairs.",
];

/// A depth as a colour: red inward, blue outward, white at 0, full at `scale`.
fn depth_colour(d: f64, scale: f64) -> String {
    let t = (d / scale).clamp(-1.0, 1.0);
    let (r, g, b) = if t >= 0.0 {
        (1.0, 1.0 - 0.8 * t, 1.0 - 0.85 * t)
    } else {
        (1.0 + 0.8 * t, 1.0 + 0.55 * t, 1.0)
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8
    )
}

fn volume_figure(r: &VolumeRun) -> Figure {
    let v = &r.result;
    let h = v.cell;
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for c in &v.cells {
        lo = [lo[0].min(c.i as f64 * h), lo[1].min(c.j as f64 * h)];
        hi = [
            hi[0].max((c.i + 1) as f64 * h),
            hi[1].max((c.j + 1) as f64 * h),
        ];
    }
    let f = Frame::new(lo, hi, 150.0, 90.0, 6.0);
    let scale = v.max_depth.max(0.01);
    let mut fig = Figure {
        width: 150.0,
        height: 90.0,
        caption: format!(
            "Crush depth over the face's plane: each {:.0} mm cell red where crushed inward (full red at {:.0} mm), blue where pushed outward, white where unchanged; true to scale.",
            h * 1000.0,
            scale * 1000.0
        ),
        ..Figure::default()
    };
    for c in &v.cells {
        let (x, y) = (c.i as f64 * h, c.j as f64 * h);
        fig.areas.push(Area {
            points: [[x, y], [x + h, y], [x + h, y + h], [x, y + h]]
                .iter()
                .map(|p| f.at(*p))
                .collect(),
            fill: depth_colour(c.depth, scale),
        });
    }
    fig.scale_bar(&f);
    fig
}

pub fn volume(meta: &Meta, r: &VolumeRun) -> Report {
    let v = &r.result;
    let l = |x: f64| format!("{:.2} L", x * 1000.0);
    let reg = &r.registration;
    let result = vec![
        [
            "Crush volume".into(),
            format!(
                "{}; Monte Carlo 95 % {} to {} (mean {}, 1σ {}; {} draws)",
                l(v.inward.value),
                l(v.inward.interval95[0]),
                l(v.inward.interval95[1]),
                l(v.inward.mean),
                l(v.inward.sd),
                v.inward.draws
            ),
        ],
        ["Pushed outward".into(), l(v.outward)],
        [
            "Deepest cell".into(),
            format!("{:.0} mm", v.max_depth * 1000.0),
        ],
        [
            "Area crushed (deeper than 3σ)".into(),
            format!("{:.3} m²", v.crushed_area),
        ],
        [
            "Cells".into(),
            format!(
                "{} of {:.0} mm with both surfaces; {} with no damaged-scan surface",
                v.cells.len(),
                v.cell * 1000.0,
                v.uncovered
            ),
        ],
    ];
    let inputs = vec![
        ["Face".into(), r.label.clone()],
        ["Damaged vehicle's scan".into(), r.damaged_scan.clone()],
        [
            "Reference (undamaged)".into(),
            match &r.mirror {
                Some(m) => format!(
                    "the same scan's opposite side, mirrored across the centre plane through {:.3}, {:.3}, {:.3} m with normal {:.4}, {:.4}, {:.4}, fitted to {} symmetric pairs (midpoints up to {:.1} mm off it); asymmetry on the undamaged surfaces {:.1} mm (1σ)",
                    m.plane_point[0], m.plane_point[1], m.plane_point[2],
                    m.normal[0], m.normal[1], m.normal[2],
                    m.midpoint_offsets.len(),
                    m.midpoint_offsets.iter().fold(0.0f64, |a, b| a.max(b.abs())) * 1000.0,
                    m.symmetry_sigma * 1000.0
                ),
                None => format!("scan {}", r.reference_scan),
            },
        ],
        [
            "Damage region (scene frame)".into(),
            format!(
                "{:.3}, {:.3}, {:.3} to {:.3}, {:.3}, {:.3} m",
                r.lo[0], r.lo[1], r.lo[2], r.hi[0], r.hi[1], r.hi[2]
            ),
        ],
    ];
    let src = |p: &locus_analysis::trajectory::PointSource| {
        format!("{} #{} r{}", p.scan, p.index, p.revision)
    };
    let rows = reg
        .pairs
        .iter()
        .enumerate()
        .map(|(k, p)| {
            vec![
                (k + 1).to_string(),
                format!(
                    "{:.3}, {:.3}, {:.3} ({})",
                    p.reference[0],
                    p.reference[1],
                    p.reference[2],
                    src(&p.reference_source)
                ),
                format!(
                    "{:.3}, {:.3}, {:.3} ({})",
                    p.damaged[0],
                    p.damaged[1],
                    p.damaged[2],
                    src(&p.damaged_source)
                ),
                format!("{:.1}", p.residual * 1000.0),
            ]
        })
        .collect();
    let registration =
        vec![
            [
                "Picked pairs".into(),
                format!(
                    "{}, fitted to {:.1} mm RMS",
                    reg.pairs.len(),
                    reg.pairs_rms * 1000.0
                ),
            ],
            [
                "ICP (point-to-plane, outside the damage region)".into(),
                format!(
                "{:.1} mm RMS over {} pairs; {:.0} % overlap; conditioning {:.3}; {} iterations{}",
                reg.icp_rms * 1000.0,
                reg.icp_pairs,
                reg.overlap * 100.0,
                reg.conditioning,
                reg.iterations,
                if reg.converged { "" } else { " (not converged)" }
            ),
            ],
            [
                "Registration 1σ".into(),
                format!(
                "{:.1} mm, {:.3}° (the formal covariance × {:.0}: one observation per 10 cm patch)",
                reg.sigma_translation * 1000.0,
                reg.sigma_rotation_deg,
                reg.inflation
            ),
            ],
        ];
    let mut sections = vec![
        Section {
            heading: "Result".into(),
            blocks: vec![
                Block::Pairs { rows: result },
                Block::Figure(volume_figure(r)),
            ],
        },
        Section {
            heading: "Inputs".into(),
            blocks: vec![Block::Pairs { rows: inputs }],
        },
        Section {
            heading: "Registration".into(),
            blocks: vec![
                Block::Pairs { rows: registration },
                Block::Table {
                    widths: ["auto", "1fr", "1fr", "auto"].map(String::from).to_vec(),
                    head: if r.mirror.is_some() {
                        ["Pair", "Left (m)", "Right (m)", "Residual, mirrored (mm)"]
                    } else {
                        ["Pair", "Reference (m)", "Damaged (m)", "Residual (mm)"]
                    }
                    .map(String::from)
                    .to_vec(),
                    rows,
                },
            ],
        },
        Section {
            heading: "Method".into(),
            blocks: vec![
                Block::Text {
                    text: format!("The reference (an undamaged vehicle of the same make and model) is fitted to the damaged vehicle's scan by the picked pairs, then refined by point-to-plane ICP on both scans' surfaces within 1 m of the pairs and the damage region, leaving the damage region out. Inside the region, the plane fitted to the reference's points is divided into {:.0} mm cells. In each cell both surfaces' heights above the plane are the median of their points, and the crush depth is the reference's minus the damaged's. A cell counts as crushed when its eight neighbours' mean depth is positive; the crush volume is those cells' depths × cell area, and the rest is the volume pushed outward. Classing each cell by its neighbours keeps its own noise from biasing either sum. The 95 % interval is a {}-draw Monte Carlo (seeded): each draw moves the reference by the registration's covariance, rebuilds the cells, and draws each cell's depth about its value with its 1σ (1.2533 × the points' spread / √n for each median). {}The interval is deliberately conservative: the registration's covariance is scaled as if each 10 cm patch were one independent observation, because real scans' errors are correlated; on synthetic vehicles with independent noise it covered the true volume in 40 of 40 runs, where a calibrated 95 % interval would miss about 2. See docs/methods/crash-volume.md.", v.cell * 1000.0, v.inward.draws, if r.mirror.is_some() { "With a mirrored reference, the vehicle's points outside the damage region are reflected across the centre plane fitted to the symmetric pairs (its normal the pairs' mean direction, through their midpoints), then registered as an exemplar would be; the asymmetry left on the undamaged surfaces (the mirrored surface's offset from the original along its normal, averaged per 5 cm patch, RMS over the patches less the points' noise) is drawn in each Monte Carlo draw as one offset of the whole reference surface. " } else { "" }),
                },
                Block::List {
                    items: REFS_VOLUME.iter().map(|s| s.to_string()).collect(),
                },
            ],
        },
    ];
    for (heading, items) in [
        ("Assumptions", &r.assumptions),
        ("Limitations", &r.limitations),
    ] {
        sections.push(Section {
            heading: heading.into(),
            blocks: vec![Block::List {
                items: items.to_vec(),
            }],
        });
    }
    sections.push(Section {
        heading: "Sign-off".into(),
        blocks: vec![Block::SignOff {
            rows: vec!["Examiner".into(), "Technical review".into()],
        }],
    });
    report(meta, "Crush volume", r.warnings.clone(), sections)
}

const REFS_EDR: &[&str] = &[
    "US Code of Federal Regulations, 49 CFR Part 563, Event Data Recorders: the pre-crash data elements, their sample rates and required accuracy (speed ±1 km/h).",
    "SAE J1698, Vehicle Event Data Interface: the output data definitions for event data recorders.",
];

/// Speed against time: the recorded speeds (dots, joined), axes labelled.
fn edr_speed_figure(r: &EdrRun) -> Figure {
    let (w, h, m) = (150.0, 60.0, 10.0);
    let (t0, t1) = (
        r.samples[0].t,
        r.end_time.max(r.samples[r.samples.len() - 1].t),
    );
    let vmax = r
        .samples
        .iter()
        .map(|s| s.speed)
        .fold(0.0, f64::max)
        .max(1.0)
        * 3.6;
    let top = (vmax / 10.0).ceil() * 10.0;
    let at = |t: f64, v: f64| -> [f64; 2] {
        [
            m + (t - t0) / (t1 - t0).max(1e-9) * (w - 2.0 * m),
            h - m - v / top * (h - 2.0 * m),
        ]
    };
    let mut fig = Figure {
        width: w,
        height: h,
        caption: "Recorded speed (km/h) against time (s), as imported.".into(),
        ..Figure::default()
    };
    fig.line(at(t0, 0.0), at(t1, 0.0), 0.3, false);
    fig.line(at(t0, 0.0), at(t0, top), 0.3, false);
    let pts: Vec<[f64; 2]> = r.samples.iter().map(|s| at(s.t, s.speed * 3.6)).collect();
    for win in pts.windows(2) {
        fig.coloured(win[0], win[1], 0.4, false, FIT);
    }
    for p in &pts {
        fig.dots.push([p[0], p[1], 0.6]);
    }
    for (t, v, text) in [
        (t0, 0.0, format!("{t0:.1} s")),
        (t1, 0.0, format!("{t1:.1} s")),
        (t0, top, format!("{top:.0} km/h")),
    ] {
        let p = at(t, v);
        fig.labels.push(Label {
            x: p[0] - 3.0,
            y: p[1] + 4.0,
            text,
        });
    }
    fig
}

/// The path in plan and the vehicle's position at each sample.
fn edr_path_figure(r: &EdrRun) -> Figure {
    let mut pts: Vec<[f64; 2]> = r.path.iter().map(|p| [p[0], p[1]]).collect();
    pts.extend(
        r.stations
            .iter()
            .filter_map(|s| s.position)
            .map(|p| [p[0], p[1]]),
    );
    let lo = pts
        .iter()
        .fold([f64::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
    let hi = pts.iter().fold([f64::NEG_INFINITY; 2], |a, p| {
        [a[0].max(p[0]), a[1].max(p[1])]
    });
    let f = Frame::new(lo, hi, 150.0, 90.0, 8.0);
    let mut fig = Figure {
        width: 150.0,
        height: 90.0,
        caption: "Plan view: the path picked in the scene (line) and the vehicle's reference point at each sample (dots), with the range of each position along the path (red); true to scale.".into(),
        ..Figure::default()
    };
    for w in r.path.windows(2) {
        fig.line(
            f.at([w[0][0], w[0][1]]),
            f.at([w[1][0], w[1][1]]),
            0.4,
            false,
        );
    }
    for s in &r.stations {
        if let (Some(p), Some([a, b])) = (s.position, s.span) {
            fig.coloured(f.at([a[0], a[1]]), f.at([b[0], b[1]]), 0.8, false, FIT);
            let q = f.at([p[0], p[1]]);
            fig.dots.push([q[0], q[1], 0.7]);
            fig.labels.push(Label {
                x: q[0] + 1.5,
                y: q[1] - 1.5,
                text: format!("{:.1}", s.t),
            });
        }
    }
    fig.scale_bar(&f);
    fig
}

pub fn edr(meta: &Meta, r: &EdrRun) -> Report {
    let opt = |v: Option<f64>, d: usize| v.map(|x| format!("{x:.d$}")).unwrap_or_default();
    let rows = r
        .samples
        .iter()
        .zip(&r.stations)
        .map(|(s, st)| {
            vec![
                format!("{:.2}", s.t),
                format!("{:.1}", s.speed * 3.6),
                opt(s.accelerator, 0),
                match s.brake {
                    Some(true) => "on".into(),
                    Some(false) => "off".into(),
                    None => String::new(),
                },
                opt(s.steering, 0),
                opt(s.yaw_rate, 1),
                opt(s.long_accel, 2),
                format!(
                    "{:.2} ({:.2}–{:.2})",
                    st.distance.value, st.distance.low, st.distance.high
                ),
            ]
        })
        .collect();
    let columns = r
        .columns
        .iter()
        .map(|c| {
            format!(
                "\"{}\": {}{}",
                c.header,
                if c.field == "unused" {
                    "not used"
                } else {
                    c.field.as_str()
                },
                if c.unit.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", c.unit)
                }
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let first = &r.stations[0].distance;
    let result = vec![
        [
            format!("Distance from {:.2} s to {:.2} s", r.samples[0].t, r.end_time),
            format!(
                "{:.2} m; range method {:.2} to {:.2} m; Monte Carlo 95 % {:.2} to {:.2} m ({} draws)",
                first.value, first.low, first.high, first.interval95[0], first.interval95[1], first.draws
            ),
        ],
        [
            "Speed tolerance".into(),
            format!(
                "±{:.1} % and ±{:.1} km/h, systematic; {}",
                r.scale_tolerance * 100.0,
                r.offset_tolerance * 3.6,
                if r.tolerance_reason.is_empty() {
                    "the recording accuracy of the indicated speed (49 CFR 563), not of the speed over the ground".to_string()
                } else {
                    format!("widened beyond the ±1 km/h recording accuracy because: {}", r.tolerance_reason)
                }
            ),
        ],
    ];
    let inputs = vec![
        ["Vehicle".into(), r.label.clone()],
        ["Source".into(), r.source.clone()],
        ["Data SHA-256".into(), r.csv_sha256.clone()],
        ["Columns".into(), columns],
        [
            "Path".into(),
            if r.path.len() >= 2 {
                format!(
                    "{} points picked in the scene ({}), ending at the vehicle's position at {:.2} s",
                    r.path.len(),
                    r.path_sources
                        .iter()
                        .map(|p| format!("{} #{} r{}", p.scan, p.index, p.revision))
                        .collect::<Vec<_>>()
                        .join("; "),
                    r.end_time
                )
            } else {
                "none".into()
            },
        ],
    ];
    let mut blocks = vec![
        Block::Pairs { rows: result },
        Block::Figure(edr_speed_figure(r)),
    ];
    if r.path.len() >= 2 {
        blocks.push(Block::Figure(edr_path_figure(r)));
    }
    let mut sections = vec![
        Section {
            heading: "Result".into(),
            blocks,
        },
        Section {
            heading: "Inputs".into(),
            blocks: vec![Block::Pairs { rows: inputs }],
        },
        Section {
            heading: "Pre-crash data".into(),
            blocks: vec![
                Block::Text {
                    text: "As imported, in SI units except speed (km/h). The last column is the distance travelled from each sample to the end time: its value (trapezoid rule) and range (the left and right sums with the speed tolerance at its extremes).".into(),
                },
                Block::Table {
                    widths: ["auto", "auto", "auto", "auto", "auto", "auto", "auto", "1fr"]
                        .map(String::from)
                        .to_vec(),
                    head: [
                        "t (s)",
                        "Speed",
                        "Accel. %",
                        "Brake",
                        "Steer °",
                        "Yaw °/s",
                        "Accel. m/s²",
                        "Distance to end (m)",
                    ]
                    .map(String::from)
                    .to_vec(),
                    rows,
                },
            ],
        },
    ];
    tail(
        &mut sections,
        "The distance from each sample to the end time is the integral of speed over time. Between samples the speed lies between its two recorded values, so the integral lies between the left and right Riemann sums; the value is their mean (the trapezoid rule), and the rule is an input ranging from left to right. The speed tolerance is systematic: every speed scaled by one factor and shifted by one offset within the stated ranges. A gap between the last sample and the end time is covered at the last speed, braking at anything from 0 to 1 g. With a path, each sample's position is that distance back along the path from its end.",
        REFS_EDR,
        &r.assumptions,
        &r.limitations,
    );
    report(meta, "EDR pre-crash data", r.warnings.clone(), sections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::pdf;
    use locus_analysis::crash::{self as c, Input, MomentumVehicle, SkidSegment};

    fn meta(method: &str) -> Meta {
        Meta {
            project: "Route 9".into(),
            record_id: 12,
            name: "Northbound car".into(),
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

    #[test]
    fn the_crash_reports_say_what_the_runs_computed() {
        let skid = c::skid(
            vec![SkidSegment {
                label: "asphalt".into(),
                distance: Input::exact(30.0),
                drag: Input::range(0.7, 0.6, 0.8),
                braking: Input::exact(1.0),
                grade: Input::exact(0.0),
                path: vec![],
                sources: vec![],
            }],
            Input::exact(0.0),
            2000,
            1,
        )
        .unwrap();
        let t = pdf(&super::skid(&meta("skid/1"), &skid), vec![])
            .unwrap()
            .text;
        assert!(t.contains("20.29 m/s (73.1 km/h)"), "{t}");
        assert!(t.contains("range method"));
        let yaw = c::yaw(
            c::YawRadius::Chord {
                chord: Input::exact(30.0),
                ordinate: Input::range(1.5, 1.4, 1.6),
            },
            Input::exact(0.7),
            Input::exact(0.0),
            0.0,
            0.0,
            2000,
            1,
        )
        .unwrap();
        let t = pdf(&super::yaw(&meta("yaw/1"), &yaw), vec![]).unwrap().text;
        assert!(t.contains("75.75 m") && t.contains("22.80 m/s"), "{t}");
        let v = |l: &str, m: f64, a: f64| MomentumVehicle {
            label: l.into(),
            mass: Input::range(m, m - 100.0, m + 100.0),
            approach_deg: Input::exact(a),
            departure_deg: Input::exact(30.964),
            departure_speed: Input::range(12.96, 12.0, 14.0),
        };
        let m = c::momentum([v("Car A", 1500.0, 0.0), v("Car B", 1200.0, 90.0)], 2000, 1).unwrap();
        let t = pdf(&super::momentum(&meta("momentum/1"), &m), vec![])
            .unwrap()
            .text;
        assert!(
            t.contains("Sensitivity") && t.contains("Car B: departure speed"),
            "{t}"
        );
        let cr = c::crush(
            "front",
            Input::exact(50_000.0),
            Input::exact(1_000_000.0),
            "test values",
            Input::exact(1.5),
            vec![Input::exact(0.3); 2],
            Input::exact(0.0),
            Input::exact(1500.0),
            2000,
            1,
        )
        .unwrap();
        let t = pdf(&super::crush(&meta("crush/1"), &cr), vec![])
            .unwrap()
            .text;
        assert!(t.contains("91.9 kJ") && t.contains("11.07 m/s"), "{t}");
        assert!(t.contains("Campbell") && t.contains("Technical review"));
    }
}
