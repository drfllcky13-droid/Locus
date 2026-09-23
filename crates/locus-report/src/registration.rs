//! The registration report: per-link error, overlap and consistency test, target residuals,
//! scan poses, and overall figures, for one stored registration.
//!
//! Every number is formatted here, from [`locus_register::report::RegistrationReport`], and
//! the template only lays the strings out, so what the PDF says is exactly what this module
//! computed (checked by the tests).

use crate::{render, Rendered};
use locus_register::posegraph::{LinkKind, LinkStatus};
use locus_register::report::RegistrationReport;
use serde::Serialize;

const TEMPLATE: &str = include_str!("../templates/registration.typ");

/// Facts about the registration that the report itself doesn't compute.
#[derive(Debug, Clone)]
pub struct Meta {
    pub project: String,
    pub registration_id: i64,
    pub parent: Option<i64>,
    pub applied: bool,
    pub created_at: String,
    pub created_by: String,
    /// Who produced the PDF, and when (the report has no clock of its own).
    pub printed_by: String,
    pub printed_at: String,
    pub app_version: String,
    /// Hash of the newest audit log entry when the report was made.
    pub audit_head: String,
    /// "the first scan's file pose" or "survey control".
    pub frame: String,
    /// Settings as the examiner chose them, already worded (e.g. "Sphere diameter", "145 mm").
    pub settings: Vec<(String, String)>,
    pub scan_names: Vec<String>,
    pub scan_points: Vec<usize>,
    pub iterations: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Document {
    pub title: String,
    pub header: String,
    pub details: Vec<[String; 2]>,
    pub settings: Vec<[String; 2]>,
    pub summary: Vec<[String; 2]>,
    pub warnings: Vec<String>,
    pub scans: Vec<Vec<String>>,
    pub links: Vec<Vec<String>>,
}

/// Millimetres, two decimals.
pub fn mm(m: f64) -> String {
    format!("{:.2} mm", m * 1000.0)
}

/// Metres, three decimals (millimetre resolution).
fn m3(m: f64) -> String {
    format!("{m:.3}")
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "it" or "them".
fn them(n: usize) -> &'static str {
    if n == 1 {
        "it"
    } else {
        "them"
    }
}

pub fn document(meta: &Meta, r: &RegistrationReport) -> Document {
    let name = |i: usize| {
        meta.scan_names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("scan {i}"))
    };
    let links = r
        .links
        .iter()
        .map(|l| {
            let kind = match l.kind {
                LinkKind::Target => "Target",
                LinkKind::Cloud => "Cloud",
                LinkKind::Control => "Control",
            };
            let mut status = match l.status {
                LinkStatus::Ok => "consistent".to_string(),
                LinkStatus::Flagged => "FLAGGED, set aside".to_string(),
                LinkStatus::Untested => "untested (no redundancy)".to_string(),
            };
            if l.forced {
                status.push_str(", forced");
            }
            if l.shape_only {
                status.push_str(", shape only");
            }
            vec![
                l.index.to_string(),
                kind.into(),
                format!(
                    "{} – {}",
                    name(l.a),
                    l.b.map_or("survey control".into(), name)
                ),
                status,
                l.pairs.to_string(),
                mm(l.rms),
                mm(l.max),
                format!("{:.2}", l.chi2_per_dof),
                format!("{:.2}", l.limit_per_dof),
                l.overlap
                    .map_or("—".into(), |o| format!("{:.0} %", o * 100.0)),
            ]
        })
        .collect();
    let scans = r
        .scans
        .iter()
        .map(|s| {
            vec![
                name(s.index),
                meta.scan_points
                    .get(s.index)
                    .map_or("—".into(), |p| p.to_string()),
                m3(s.position[0]),
                m3(s.position[1]),
                m3(s.position[2]),
                format!("{:.3}°", s.heading_deg),
                if s.verified { "yes" } else { "NO: check it" }.into(),
            ]
        })
        .collect();
    let mut warnings = vec![];
    if r.flagged > 0 {
        warnings.push(format!(
            "{} failed the consistency test and {} set aside; the poses do not use {}.",
            count(r.flagged, "link", "links"),
            if r.flagged == 1 { "was" } else { "were" },
            them(r.flagged)
        ));
    }
    if r.unverified > 0 {
        let t = them(r.unverified);
        warnings.push(format!(
            "{} placed by shape matching alone, with no targets or rough pose to confirm {t}. \
             Such a placement can be wrong in a symmetric scene without any test showing it; \
             check {t} against the scene before relying on {t}.",
            count(r.unverified, "scan is", "scans are"),
        ));
    }
    if r.untested > 0 {
        warnings.push(format!(
            "{} the only connection for part of the graph, so nothing can check {}.",
            count(r.untested, "link is", "links are"),
            them(r.untested)
        ));
    }
    Document {
        title: "Registration report".into(),
        header: format!("{} · registration #{}", meta.project, meta.registration_id),
        details: vec![
            ["Project".into(), meta.project.clone()],
            [
                "Registration".into(),
                match meta.parent {
                    Some(p) => format!("#{} (links edited from #{p})", meta.registration_id),
                    None => format!("#{}", meta.registration_id),
                },
            ],
            [
                "Run by".into(),
                format!("{}, {}", meta.created_by, meta.created_at),
            ],
            [
                "Applied to the scene".into(),
                if meta.applied { "yes" } else { "no" }.into(),
            ],
            ["Reference frame".into(), meta.frame.clone()],
            [
                "Report made by".into(),
                format!("{}, {}", meta.printed_by, meta.printed_at),
            ],
            ["Software".into(), format!("Locus {}", meta.app_version)],
            ["Audit log head".into(), meta.audit_head.clone()],
        ],
        settings: meta
            .settings
            .iter()
            .map(|(k, v)| [k.clone(), v.clone()])
            .collect(),
        summary: vec![
            ["Scans".into(), r.scans.len().to_string()],
            [
                "Links".into(),
                format!(
                    "{}: {} consistent, {} flagged, {} untested",
                    r.links.len(),
                    r.ok,
                    r.flagged,
                    r.untested
                ),
            ],
            ["Cloud links by shape only".into(), r.shape_only.to_string()],
            ["Scans not verified".into(), r.unverified.to_string()],
            [
                "Target residuals".into(),
                match (r.targets.mean, r.targets.max) {
                    (Some(mean), Some(max)) => format!(
                        "mean {}, largest {}, over {} target pairs",
                        mm(mean),
                        mm(max),
                        r.targets.count
                    ),
                    _ => "no target links".into(),
                },
            ],
            ["Adjustment iterations".into(), meta.iterations.to_string()],
        ],
        warnings,
        scans,
        links,
    }
}

/// Render the report to PDF.
pub fn pdf(meta: &Meta, r: &RegistrationReport) -> Result<Rendered, String> {
    let data = serde_json::to_vec(&document(meta, r)).map_err(|e| e.to_string())?;
    render(TEMPLATE, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use locus_register::pipeline::verified;
    use locus_register::posegraph::{solve, Link, Pair};
    use locus_register::report::build;
    use nalgebra::{Isometry3, Matrix3, Point3, Translation3, UnitQuaternion, Vector3};

    fn meta(n: usize) -> Meta {
        Meta {
            project: "Test scene".into(),
            registration_id: 7,
            parent: Some(3),
            applied: true,
            created_at: "2026-09-22 21:00".into(),
            created_by: "A. Examiner".into(),
            printed_by: "A. Examiner".into(),
            printed_at: "2026-09-22 21:05".into(),
            app_version: "0.1.0".into(),
            audit_head: "4ace6979a0".into(),
            frame: "the first scan's file pose".into(),
            settings: vec![("Sphere diameter".into(), "145 mm".into())],
            scan_names: (0..n).map(|i| format!("Station {}", i + 1)).collect(),
            scan_points: vec![1_000_000; n],
            iterations: 4,
        }
    }

    #[test]
    fn report_numbers_match_the_adjustment() {
        let truth = [
            Isometry3::identity(),
            Isometry3::from_parts(
                Translation3::new(8.0, 1.0, 0.0),
                UnitQuaternion::from_euler_angles(0.0, 0.0, 0.6),
            ),
            Isometry3::from_parts(
                Translation3::new(3.0, 9.0, 0.1),
                UnitQuaternion::from_euler_angles(0.0, 0.0, -1.1),
            ),
        ];
        let world = [
            [1.0, 2.0, 1.2],
            [6.0, -1.0, 1.5],
            [4.0, 6.0, 1.1],
            [9.0, 5.0, 1.8],
        ];
        let cov = Matrix3::identity() * (0.0005f64.powi(2) / 3.0);
        let mut k = 0u32;
        let mut wobble = || {
            k += 1;
            (k as f64 * 1.37).sin() * 0.0004
        };
        let mut links = vec![];
        for (a, b) in [(0, 1), (1, 2), (0, 2)] {
            let pairs = world
                .iter()
                .map(|w| {
                    let pa = (truth[a].inverse() * Point3::from(*w)).coords
                        + Vector3::new(wobble(), 0.0, 0.0);
                    let pb = (truth[b].inverse() * Point3::from(*w)).coords
                        + Vector3::new(0.0, wobble(), 0.0);
                    Pair {
                        pa: pa.into(),
                        cov_a: cov,
                        pb: pb.into(),
                        cov_b: cov,
                    }
                })
                .collect();
            links.push(Link::targets(a, b, pairs));
        }
        let wrong = Isometry3::translation(0.03, 0.0, 0.0) * truth[1].inverse() * truth[2];
        links.push(Link::cloud(
            1,
            2,
            &wrong,
            &[[0.0; 3], [5.0, 0.0, 0.0], [0.0, 5.0, 2.0]],
            0.002,
        ));
        let sol = solve(&truth, &links).unwrap();
        let v = verified(3, &links, &sol);
        let overlap = [None, None, None, Some(0.66)];
        let report = build(&links, &sol.poses, &sol.links, &overlap, &v);

        let out = pdf(&meta(3), &report).unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
        let text = out.text.replace('\u{c}', " ");
        // Every link's figures, as the adjustment computed them, are in the laid-out report.
        for (i, t) in sol.links.iter().enumerate() {
            for s in [
                mm(t.rms),
                mm(t.max),
                format!("{:.2}", t.chi2_per_dof),
                format!("{:.2}", t.limit_per_dof),
            ] {
                assert!(text.contains(&s), "link {i}: {s} not in report text");
            }
        }
        // Summary and details, scan positions, the flag and the overlap.
        let doc = document(&meta(3), &report);
        for row in doc.summary.iter().chain(&doc.details) {
            assert!(text.contains(&row[1]), "{row:?} not in report text");
        }
        for p in &sol.poses {
            assert!(text.contains(&format!("{:.3}", p.translation.vector.x)));
        }
        assert!(text.contains("FLAGGED, set aside"));
        assert!(text.contains("66 %"));
        // Copies to look at, in the temp folder.
        let tmp = std::env::temp_dir();
        std::fs::write(tmp.join("locus-registration-report.pdf"), &out.pdf).unwrap();
        let data = serde_json::to_vec(&doc).unwrap();
        for (i, png) in crate::pages_png(TEMPLATE, data, vec![]).iter().enumerate() {
            std::fs::write(
                tmp.join(format!("locus-registration-report-{}.png", i + 1)),
                png,
            )
            .unwrap();
        }
    }
}
