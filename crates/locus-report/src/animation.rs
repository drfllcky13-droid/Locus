//! The animation's time–distance–speed report, built only from the stored run.

use crate::analysis::{details, Block, Meta, Report, Section};
use locus_analysis::animation::{RenderRecord, Source, ViewKind, HUMAN_HFOV_DEG};
use locus_analysis::tds::TdsRun;

const REFS: &[&str] = &[
    "E. Catmull and R. Rom, \"A class of local interpolating splines\", in Computer Aided Geometric Design, Academic Press, 1974; C. Yuksel, S. Schaefer and J. Keyser, \"Parameterization and applications of Catmull–Rom curves\", Computer-Aided Design 43(7), 2011: the paths (centripetal form).",
    "F. N. Fritsch and R. E. Carlson, \"Monotone piecewise cubic interpolation\", SIAM Journal on Numerical Analysis 17(2), 1980: time–distance tables without speeds.",
];

fn with_range(v: f64, r: Option<[f64; 2]>, scale: f64, d: usize) -> String {
    match r {
        Some([lo, hi]) => format!("{:.d$} ({:.d$}–{:.d$})", v * scale, lo * scale, hi * scale),
        None => format!("{:.d$}", v * scale),
    }
}

fn opt(t: Option<f64>) -> String {
    t.map_or("not within the timeline".into(), |t| format!("{t:.2} s"))
}

/// What kind of source a segment has, for a table column; assumed ones say so plainly.
fn kind(s: &Source) -> &'static str {
    match s {
        Source::Edr { .. } => "EDR",
        Source::Analysis { .. } => "Analysis",
        Source::Evidence { .. } => "Measured",
        Source::Assumption { .. } => "Assumed",
    }
}

pub fn report(meta: &Meta, r: &TdsRun, renders: &[(i64, RenderRecord)]) -> Report {
    let a = &r.animation;
    let mut sections = vec![Section {
        heading: "Result".into(),
        blocks: vec![Block::Pairs {
            rows: vec![
                [
                    "Time zero".into(),
                    format!("{}; known from {}", a.time_zero.event, a.time_zero.basis),
                ],
                [
                    "Timeline".into(),
                    format!(
                        "{:.2} to {:.2} s from time zero, tabulated every {:.2} s",
                        a.from, a.to, r.request.step
                    ),
                ],
                [
                    "Lighting".into(),
                    match a.lighting {
                        locus_analysis::animation::Lighting::Daylight => "daylight".into(),
                        locus_analysis::animation::Lighting::LowLight => {
                            "low light (see the limitations)".into()
                        }
                    },
                ],
                [
                    "Scene".into(),
                    format!("scene {} revision {}", r.scene_id, r.scene_revision),
                ],
                ["Summary".into(), r.summary.clone()],
            ],
        }],
    }];

    for (m, mover) in r.movers.iter().zip(&a.movers) {
        let rows = m
            .rows
            .iter()
            .map(|x| {
                let src = x.segment.map_or("not moving".to_string(), |k| {
                    format!("{k}: {}", kind(&mover.segments[k - 1].source))
                });
                vec![
                    format!("{:.2}", x.t),
                    with_range(x.distance, x.distance_range, 1.0, 2),
                    with_range(x.speed, x.speed_range, 3.6, 1),
                    format!("{:.2}", x.acceleration),
                    format!("{:.1}", x.heading_deg),
                    src,
                ]
            })
            .collect();
        let mut inputs = vec![
            ["Position of".into(), m.reference.clone()],
            [
                "Path".into(),
                format!("{:.2} m long, {}", m.path_length, m.path_source),
            ],
        ];
        for (i, s) in m.segments.iter().enumerate() {
            inputs.push([format!("Segment {}", i + 1), s.clone()]);
        }
        inputs.push([
            "Friction".into(),
            m.friction.clone().unwrap_or("not stated".into()),
        ]);
        sections.push(Section {
            heading: format!("{}: time, distance and speed", m.name),
            blocks: vec![
                Block::Pairs { rows: inputs },
                Block::Text { text: "Distance is along the path from its first point. Ranges, where a source gives one, are in brackets. The last column is the segment and its source; \"Assumed\" rows are the examiner's assumptions, not measured data, and are illustrative only.".into() },
                Block::Table {
                    widths: ["auto", "1fr", "1fr", "auto", "auto", "auto"].map(String::from).to_vec(),
                    head: ["t (s)", "Distance (m)", "Speed (km/h)", "Accel. (m/s²)", "Heading (°)", "Segment"]
                        .map(String::from)
                        .to_vec(),
                    rows,
                },
            ],
        });
    }

    for p in &r.pairs {
        let closing = r.request.closing;
        let mut head = vec!["t (s)".to_string(), "Distance (m)".into()];
        if closing {
            head.push("Closing speed (km/h)".into());
        }
        sections.push(Section {
            heading: format!("{} to {}", p.a, p.b),
            blocks: vec![
                Block::Text {
                    text: format!(
                        "Straight-line distance between their reference points (not the gap between their bodies). Least: {:.2} m at {:.2} s.{}",
                        p.least.0,
                        p.least.1,
                        if closing { " Closing speed is the rate the distance shrinks; negative while they separate." } else { "" }
                    ),
                },
                Block::Table {
                    widths: if closing { vec!["auto".into(), "1fr".into(), "1fr".into()] } else { vec!["auto".into(), "1fr".into()] },
                    head,
                    rows: p
                        .rows
                        .iter()
                        .map(|x| {
                            let mut row = vec![format!("{:.2}", x.t), with_range(x.distance, x.range, 1.0, 2)];
                            if let Some(c) = x.closing {
                                row.push(format!("{:.1}", c * 3.6));
                            }
                            row
                        })
                        .collect(),
                },
            ],
        });
    }

    for q in &r.points {
        let mut blocks = vec![
            Block::Text {
                text: format!(
                    "At {:.3}, {:.3}, {:.3} ({}). For each mover: where the point falls on its path, how far off the path it is, and when it gets there, with the earliest and latest from its distance range.",
                    q.position[0], q.position[1], q.position[2], q.source
                ),
            },
            Block::Table {
                widths: ["auto", "auto", "auto", "1fr", "auto"].map(String::from).to_vec(),
                head: ["Mover", "Along path (m)", "Off path (m)", "Reaches it", "Speed there (km/h)"]
                    .map(String::from)
                    .to_vec(),
                rows: q
                    .arrivals
                    .iter()
                    .map(|x| {
                        vec![
                            x.mover.clone(),
                            format!("{:.2}", x.at),
                            format!("{:.2}", x.off_path),
                            match (x.time, x.earliest, x.latest) {
                                (Some(t), e, l) if e != Some(t) || l != Some(t) => {
                                    format!("{t:.2} s (earliest {}, latest {})", opt(e), opt(l))
                                }
                                (t, ..) => opt(t),
                            },
                            x.speed.map_or(String::new(), |v| format!("{:.1}", v * 3.6)),
                        ]
                    })
                    .collect(),
            },
        ];
        let movers: Vec<&String> = q.arrivals.iter().map(|x| &x.mover).collect();
        let mut head = vec!["t (s)".to_string()];
        head.extend(movers.iter().map(|m| format!("{m}: to go (m)")));
        blocks.push(Block::Table {
            widths: std::iter::once("auto".to_string())
                .chain(movers.iter().map(|_| "1fr".to_string()))
                .collect(),
            head,
            rows: (0..q.arrivals.first().map_or(0, |x| x.to_go.len()))
                .map(|i| {
                    let mut row = vec![format!("{:.2}", q.arrivals[0].to_go[i].0)];
                    row.extend(q.arrivals.iter().map(|x| {
                        let (_, d, rg) = x.to_go[i];
                        with_range(d, rg, 1.0, 2)
                    }));
                    row
                })
                .collect(),
        });
        sections.push(Section {
            heading: format!("Time and distance to {}", q.name),
            blocks,
        });
    }

    if !a.views.is_empty() {
        sections.push(Section {
            heading: "Views".into(),
            blocks: vec![Block::Table {
                widths: ["auto", "1fr", "auto", "1fr"].map(String::from).to_vec(),
                head: ["View", "Eye", "Horizontal field of view", "Source"].map(String::from).to_vec(),
                rows: a
                    .views
                    .iter()
                    .map(|v| {
                        let mover = |id: &String| {
                            a.movers.iter().find(|m| &m.id == id).map_or(id.clone(), |m| m.name.clone())
                        };
                        let eye = match &v.kind {
                            ViewKind::Driver { mover: id, eye } => format!(
                                "driver of {}: {:.2} m forward of the rear axle, {:.2} m left, {:.2} m up",
                                mover(id), eye[0], eye[1], eye[2]
                            ),
                            ViewKind::Witness { floor, eye_height, target, target_mover } => format!(
                                "witness at {:.2}, {:.2}, {:.2}, eye {:.2} m up, looking at {}",
                                floor[0], floor[1], floor[2], eye_height,
                                target_mover.as_ref().map_or(format!("{:.2}, {:.2}, {:.2}", target[0], target[1], target[2]), |id| format!("{} (tracked)", mover(id)))
                            ),
                            ViewKind::Orbit { centre, radius, height, period } => format!(
                                "presentation orbit about {:.2}, {:.2}, {:.2}: radius {radius:.1} m, {height:.1} m up, every {period:.1} s; nobody's point of view",
                                centre[0], centre[1], centre[2]
                            ),
                            ViewKind::Follow { mover: id, offset, .. } => format!(
                                "presentation camera following {} at {:.1}, {:.1}, {:.1} m in its frame; nobody's point of view",
                                mover(id), offset[0], offset[1], offset[2]
                            ),
                        };
                        let fov = if v.kind.human() && v.hfov_deg > HUMAN_HFOV_DEG + 1e-9 {
                            format!("{:.0}° (wider than the {HUMAN_HFOV_DEG:.0}° human-like default)", v.hfov_deg)
                        } else {
                            format!("{:.0}°", v.hfov_deg)
                        };
                        vec![v.name.clone(), eye, fov, v.source.describe()]
                    })
                    .collect(),
            }],
        });
    }

    if !renders.is_empty() {
        sections.push(Section {
            heading: "Renders".into(),
            blocks: vec![Block::Table {
                widths: ["auto", "auto", "auto", "1fr"].map(String::from).to_vec(),
                head: ["Record", "View", "Frames", "File and SHA-256"]
                    .map(String::from)
                    .to_vec(),
                rows: renders
                    .iter()
                    .map(|(id, x)| {
                        vec![
                            format!("analysis {id}, scene revision {}", x.scene_revision),
                            x.view.clone(),
                            format!(
                                "{} at {} fps, {}×{}, {:.2} to {:.2} s",
                                x.frames, x.fps, x.width, x.height, x.from, x.to
                            ),
                            format!("{} {}", x.file, x.sha256),
                        ]
                    })
                    .collect(),
            }],
        });
    }

    sections.push(Section {
        heading: "Assumed".into(),
        blocks: vec![if r.assumed.is_empty() {
            Block::Text {
                text: "Nothing in this animation is assumed.".into(),
            }
        } else {
            Block::List {
                items: r
                    .assumed
                    .iter()
                    .map(|x| {
                        format!(
                            "{}{}, {:.2} to {:.2} s: {}",
                            x.mover,
                            x.segment
                                .map_or(String::new(), |k| format!(", segment {}", k + 1)),
                            x.from,
                            x.to,
                            if x.note.is_empty() {
                                "no reason stated"
                            } else {
                                &x.note
                            }
                        )
                    })
                    .collect(),
            }
        }],
    });
    sections.push(Section {
        heading: "Method".into(),
        blocks: vec![
            Block::Text { text: "Each mover follows a path through points picked in the scene (a centripetal Catmull–Rom curve or straight segments, parametrised by exact arc length), in segments of constant speed, constant acceleration, or a time–distance table (an EDR record's, with its speeds as slopes). Every value in the tables is read from that one motion, the same the timeline plays and renders show. The plausibility checks run at 100 Hz: the combined longitudinal and lateral acceleration against the stated friction (a friction circle, with μ's tolerance), speed jumps between segments, and corners in a path. See docs/methods/animation.md.".into() },
            Block::List { items: REFS.iter().map(|s| s.to_string()).collect() },
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

    let mut warnings = vec![];
    if let Some(x) = &meta.withdrawn {
        warnings.push(format!("This analysis was withdrawn: {x}."));
    }
    warnings.extend(r.flags.iter().map(|f| f.message.clone()));
    warnings.extend(r.warnings.clone());
    Report {
        title: format!("Time, distance and speed: {}", meta.name),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::pdf;
    use locus_analysis::animation::{
        Animation, Lighting, Mover, MoverKind, Segment, SegmentMotion, TimeZero, View,
    };
    use locus_analysis::motion::Shape;
    use locus_analysis::tds::{run, NamedPoint, TdsRequest};

    fn meta() -> Meta {
        Meta {
            project: "Main Street".into(),
            record_id: 9,
            name: "Crossing".into(),
            method: "animation-tds/1".into(),
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
    fn the_report_marks_sources_ranges_and_views() {
        // An EDR-driven car (distance and speed ranges) and an assumed pedestrian.
        let car = Mover {
            id: "car".into(),
            name: "Car".into(),
            object: None,
            kind: MoverKind::Vehicle { wheelbase: 2.7 },
            path: vec![[0.0, 0.0, 0.0], [40.0, 0.0, 0.0]],
            shape: Shape::Straight,
            path_source: Source::Edr {
                analysis_id: 3,
                name: "EDR".into(),
            },
            start: -2.0,
            offset: 0.0,
            segments: vec![Segment {
                duration: None,
                motion: SegmentMotion::Table {
                    times: vec![0.0, 1.0, 2.0],
                    distances: vec![0.0, 15.0, 28.0],
                    speeds: Some(vec![16.0, 14.0, 12.0]),
                    ranges: Some(vec![[-0.5, 0.5], [14.0, 16.0], [26.5, 29.5]]),
                    speed_ranges: Some(vec![[15.5, 16.5], [13.5, 14.5], [11.5, 12.5]]),
                },
                source: Source::Edr {
                    analysis_id: 3,
                    name: "EDR".into(),
                },
            }],
            friction: None,
        };
        let walker = Mover {
            id: "ped".into(),
            name: "Pedestrian".into(),
            object: None,
            kind: MoverKind::Person,
            path: vec![[25.0, -8.0, 0.0], [25.0, 8.0, 0.0]],
            shape: Shape::Straight,
            path_source: Source::Assumption {
                note: "a witness's account".into(),
            },
            start: -2.0,
            offset: 0.0,
            segments: vec![Segment {
                duration: None,
                motion: SegmentMotion::Speed {
                    speed: 1.4,
                    range: None,
                },
                source: Source::Assumption {
                    note: "typical walking speed".into(),
                },
            }],
            friction: None,
        };
        let a = Animation {
            time_zero: TimeZero {
                event: "EDR trigger".into(),
                basis: "the EDR record".into(),
            },
            from: -2.0,
            to: 0.0,
            lighting: Lighting::LowLight,
            movers: vec![car, walker],
            views: vec![View {
                id: "d".into(),
                name: "Driver".into(),
                kind: ViewKind::Driver {
                    mover: "car".into(),
                    eye: [1.2, 0.35, 1.2],
                },
                hfov_deg: 60.0,
                source: Source::Assumption {
                    note: "default eye position".into(),
                },
            }],
        };
        let req = TdsRequest {
            step: 0.5,
            pairs: vec![["car".into(), "ped".into()]],
            closing: true,
            points: vec![NamedPoint {
                name: "the crossing".into(),
                position: [25.0, 0.0, 0.0],
                source: Source::Evidence {
                    evidence_id: 1,
                    name: "scan".into(),
                    note: "scuff mark".into(),
                },
            }],
        };
        let r = run(4, 12, &a, &req).unwrap();
        let text = pdf(&report(&meta(), &r, &[]), vec![]).unwrap().text;
        for want in [
            "EDR trigger",
            "the EDR record",
            "Needs attention",
            "no friction is stated",
            "15.00 (14.00–16.00)",
            "1: EDR",
            "1: Assumed",
            "typical walking speed",
            "Car to Pedestrian",
            "Closing speed",
            "Time and distance to the crossing",
            "earliest",
            "driver of Car",
            "60°",
            "not modelled",
            "low-light",
            "scene 4 revision 12",
        ] {
            assert!(text.contains(want), "missing {want:?} in {text}");
        }
    }
}
