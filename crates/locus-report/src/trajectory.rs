//! The bullet-trajectory report, built only from the stored run (so it can be reprinted
//! from the record at any time and says exactly what was computed).

use crate::analysis::{details, Area, Block, Figure, Frame, Label, Meta, Report, Section};
use locus_analysis::measure::Measured;
use locus_analysis::trajectory::Run;

fn deg(m: &Measured) -> String {
    format!("{:.2}° ± {:.2}°", m.value, m.sigma)
}

fn mm(m: f64) -> String {
    format!("{:.1} mm", m * 1000.0)
}

fn xyz(p: [f64; 3]) -> String {
    format!("{:.3}, {:.3}, {:.3}", p[0], p[1], p[2])
}

pub fn report(meta: &Meta, r: &Run) -> Report {
    let l = &r.line;
    let p = &r.parameters;
    let mut warnings = vec![];
    if let Some(w) = &meta.withdrawn {
        warnings.push(format!("This analysis was withdrawn: {w}."));
    }
    if r.cone_narrower_than_fit {
        warnings.push(format!(
            "The fit's own 95 % cone ({:.1}°) is wider than the {:.1}° cone used for possible muzzle positions below.",
            l.cone.major_deg, p.cone_deg
        ));
    }
    if l.inflation > 1.0 {
        warnings.push(format!(
            "The points scatter more than their stated uncertainty (χ² = {:.1} on {} degrees of freedom): the direction's uncertainty has been widened {:.1}×. Check for a misplaced pick or a deflection.",
            l.chi2, l.dof, l.inflation
        ));
    }
    let surfaces: std::collections::BTreeSet<&str> =
        r.inputs.iter().map(|i| i.surface.as_str()).collect();
    if surfaces.len() == 1 && r.parameters.rod_play_deg == 0.0 {
        warnings.push("All points are on one surface, so the direction rests on that surface's thickness alone.".into());
    }

    let summary = vec![
        [
            "Bearing".into(),
            format!("{} (clockwise from project +y)", deg(&l.bearing)),
        ],
        [
            "Elevation".into(),
            format!("{} (up from horizontal)", deg(&l.elevation)),
        ],
        [
            "95 % cone".into(),
            format!(
                "{:.2}° × {:.2}° (half-angles)",
                l.cone.major_deg, l.cone.minor_deg
            ),
        ],
        [
            "Direction of travel".into(),
            format!(
                "({:.5}, {:.5}, {:.5})",
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
    let inputs = r
        .inputs
        .iter()
        .zip(&l.residuals)
        .enumerate()
        .map(|(i, (p, res))| {
            vec![
                (i + 1).to_string(),
                p.kind.clone(),
                p.surface.clone(),
                xyz(p.point),
                mm(p.sigma),
                mm(*res),
            ]
        })
        .collect();
    let angle_rows = r
        .surfaces
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
        .collect();

    let mut band = vec![
        [
            "Height band".into(),
            format!(
                "{:.2}–{:.2} m above the floor at z = {:.3} m",
                p.band[0], p.band[1], p.floor_z
            ),
        ],
        ["Cone".into(), format!("{:.1}° half-angle", p.cone_deg)],
        [
            "Traced back up to".into(),
            format!("{:.1} m from the first point", p.max_range),
        ],
    ];
    match &r.band.centre {
        Some((t, pts)) => band.push([
            "Centre line in the band".into(),
            format!(
                "{:.2} m to {:.2} m back from the first point, from ({}) to ({}) m",
                t[0],
                t[1],
                xyz(pts[0]),
                xyz(pts[1])
            ),
        ]),
        None => band.push([
            "Centre line in the band".into(),
            "never, within the range traced back".into(),
        ]),
    }

    Report {
        title: format!("Bullet trajectory: {}", meta.name),
        header: format!("{} · analysis {} · {}", meta.project, meta.record_id, meta.method),
        details: details(meta),
        warnings,
        sections: vec![
            Section { heading: "Result".into(), blocks: vec![Block::Pairs { rows: summary }] },
            Section {
                heading: "Points used".into(),
                blocks: vec![
                    Block::Text {
                        text: "In the order the projectile travelled. Coordinates in the project frame (m); each point's stated 1σ, and its perpendicular distance from the fitted path.".into(),
                    },
                    Block::Table {
                        widths: ["auto", "auto", "1fr", "auto", "auto", "auto"].map(String::from).to_vec(),
                        head: ["#", "Kind", "Surface", "x, y, z (m)", "1σ", "Residual"].map(String::from).to_vec(),
                        rows: inputs,
                    },
                ],
            },
            Section {
                heading: "Angles to each surface".into(),
                blocks: vec![
                    Block::Text {
                        text: "Impact angle between the path and the surface (90° is square on). Horizontal and vertical angles from the surface's normal, seen from the shooter's side: horizontal + to the right, vertical + upward. 1σ from the direction's uncertainty; the plane fitted around each defect and its RMS residual.".into(),
                    },
                    Block::Table {
                        widths: ["1fr", "auto", "auto", "auto", "auto"].map(String::from).to_vec(),
                        head: ["Surface", "Impact", "Horizontal", "Vertical", "Plane RMS"].map(String::from).to_vec(),
                        rows: angle_rows,
                    },
                ],
            },
            Section {
                heading: "Possible muzzle positions".into(),
                blocks: vec![
                    Block::Pairs { rows: band },
                    Block::Figure(plan(r)),
                    Block::Figure(side(r)),
                ],
            },
            Section {
                heading: "Method".into(),
                blocks: vec![Block::Text {
                    text: "A straight line is fitted to the points by weighted total least squares (weights 1/σ²): it passes through their weighted centroid along the principal axis of their weighted scatter, oriented in the order of travel. The direction's covariance is propagated to first order from every input coordinate, inflated by √(χ²/dof) when the residuals exceed the stated uncertainty, and any rod play is added in quadrature. The 95 % cone's half-angles are √5.991 times the square roots of the covariance's two principal values. Possible muzzle positions are where the path, traced back from the first point, and the edges of the stated cone pass through the height band. See docs/methods/trajectory.md.".into(),
                }],
            },
            Section { heading: "Assumptions".into(), blocks: vec![Block::List { items: r.assumptions.clone() }] },
            Section { heading: "Limitations".into(), blocks: vec![Block::List { items: r.limitations.clone() }] },
        ],
    }
}

const W: f64 = 178.0;
const H: f64 = 80.0;

/// Plan view: the cone's footprint in the band, the path, and the points.
fn plan(r: &Run) -> Figure {
    let mut pts: Vec<[f64; 2]> = r.inputs.iter().map(|i| [i.point[0], i.point[1]]).collect();
    pts.extend(r.band.footprint.iter().copied());
    if let Some((_, ends)) = &r.band.centre {
        pts.extend(ends.iter().map(|p| [p[0], p[1]]));
    }
    let (lo, hi) = bounds(&pts);
    let f = Frame::new(lo, hi, W, H, 8.0);
    let mut fig = Figure { width: W, height: H, caption: "Plan view (x east, y north). Shaded: within the cone and the height band. Line: the fitted path from the band to the last point; dots: the points used.".into(), ..Figure::default() };
    if r.band.footprint.len() >= 3 {
        fig.areas.push(Area {
            points: r.band.footprint.iter().map(|p| f.at(*p)).collect(),
            fill: "#f2d7a6".into(),
        });
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
                y: q[1] - 4.0,
                text: p.surface.clone(),
            });
        }
    }
    fig.scale_bar(&f);
    fig
}

/// Side view along the path: distance back from the first point against height.
fn side(r: &Run) -> Figure {
    let d = r.line.direction;
    let first = r.inputs[0].point;
    let horiz = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    // Horizontal distance along the path's bearing, from the first point.
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
    let (z_lo, z_hi) = (
        zs.iter().cloned().fold(f64::INFINITY, f64::min),
        zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    );
    let f = Frame::new([x_lo, z_lo], [x_hi, z_hi], W, H, 8.0);
    let mut fig = Figure { width: W, height: H, caption: "Side view along the path's bearing (horizontal distance from the first point against height). Solid: floor; dashed: the height band; the path and the cone's upper and lower edges.".into(), ..Figure::default() };
    fig.line(f.at([x_lo, p.floor_z]), f.at([x_hi, p.floor_z]), 0.5, false);
    for b in p.band {
        fig.line(
            f.at([x_lo, p.floor_z + b]),
            f.at([x_hi, p.floor_z + b]),
            0.25,
            true,
        );
    }
    let slope = d[2] / horiz;
    let z_at = |x: f64, s: f64| first[2] + x * s;
    fig.line(
        f.at([x_lo, z_at(x_lo, slope)]),
        f.at([x_hi, z_at(x_hi, slope)]),
        0.35,
        false,
    );
    let e = d[2].asin();
    for s in [-1.0, 1.0] {
        let k = (e + s * p.cone_deg.to_radians()).tan();
        fig.line(
            f.at([x_lo, z_at(x_lo, k)]),
            f.at([0.0, first[2]]),
            0.2,
            true,
        );
    }
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
    use locus_analysis::trajectory::{direction, run, FittedPlane, InputPoint, Parameters};

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
            .map(|(i, (t, s))| InputPoint {
                kind: if i % 2 == 0 { "entry" } else { "exit" }.into(),
                surface: (*s).into(),
                point: [
                    o[0] + d[0] * t,
                    o[1] + d[1] * t,
                    o[2] + d[2] * t + if i == 2 { 0.001 } else { 0.0 },
                ],
                sigma: 0.002,
                plane: Some(FittedPlane {
                    point: o,
                    normal: [0.0, -1.0, 0.0],
                    rms: 0.0012,
                    points: 60,
                }),
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
            project: "Case 12".into(),
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
        };
        (meta, r)
    }

    #[test]
    fn the_pdf_says_what_the_run_computed() {
        let (meta, r) = sample();
        let doc = report(&meta, &r);
        let out = pdf(&doc).unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
        let text = out.text;
        assert!(
            text.contains(&format!(
                "{:.2}° ± {:.2}°",
                r.line.bearing.value, r.line.bearing.sigma
            )),
            "{text}"
        );
        assert!(text.contains(&format!(
            "{:.2}° ± {:.2}°",
            r.line.elevation.value, r.line.elevation.sigma
        )));
        assert!(text.contains("vehicle door") && text.contains("interior wall"));
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
    fn warnings_explain_a_withdrawn_run_and_excess_scatter() {
        let (mut meta, mut r) = sample();
        meta.withdrawn = Some("2026-09-23 by Examiner: wrong defect".into());
        r.line.inflation = 3.2;
        let doc = report(&meta, &r);
        assert!(doc.warnings.iter().any(|w| w.contains("withdrawn")));
        assert!(doc.warnings.iter().any(|w| w.contains("3.2×")));
    }
}
