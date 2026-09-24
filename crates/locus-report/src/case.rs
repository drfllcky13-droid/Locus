//! The case report: the project at a glance. Its evidence with hashes, the latest integrity
//! check, and an index of every record (registrations, diagram and scene revisions, analyses,
//! measurements, renders), each with its hash and the audit entry that recorded it.

use crate::analysis::{Block, Report, Section};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceRow {
    pub id: i64,
    pub name: String,
    pub sha256: String,
    pub size: u64,
    pub format: String,
    pub unit: String,
    pub imported: String,
    pub entry: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordRow {
    /// "Registration", "Diagram", "3D scene", an analysis tool, "Measurement".
    pub kind: String,
    pub id: String,
    pub name: String,
    pub sha256: String,
    pub made: String,
    /// "applied", "withdrawn: …", or empty.
    pub status: String,
    pub entry: String,
}

#[derive(Debug, Clone)]
pub struct CaseData {
    pub project: String,
    pub case_number: Option<String>,
    pub printed_by: String,
    pub printed_at: String,
    pub app_version: String,
    /// The project state's head (prints, exports and checks excluded): its hash and number.
    /// Printing doesn't move it, so the report prints the same each time.
    pub audit_head: String,
    pub audit_entries: usize,
    /// The latest integrity check: when, and what it found.
    pub integrity: Option<(String, usize, Vec<String>)>,
    pub evidence: Vec<EvidenceRow>,
    pub records: Vec<RecordRow>,
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.2} GiB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MiB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

pub fn report(d: &CaseData) -> Report {
    let mut warnings = vec![];
    let integrity = match &d.integrity {
        Some((at, checked, failed)) if failed.is_empty() => {
            format!("{at}: all {checked} evidence files match their recorded SHA-256")
        }
        Some((at, checked, failed)) => {
            warnings.extend(failed.iter().map(|f| format!("Evidence integrity: {f}.")));
            format!(
                "{at}: {} of {checked} files did NOT match (see Needs attention)",
                failed.len()
            )
        }
        None => "no check recorded".into(),
    };
    let mut details = vec![
        [
            "Case number".into(),
            d.case_number.clone().unwrap_or_else(|| "(not set)".into()),
        ],
        ["Project".into(), d.project.clone()],
        [
            "Printed".into(),
            format!(
                "{}, {} (Locus {})",
                d.printed_by, d.printed_at, d.app_version
            ),
        ],
        [
            "Audit log".into(),
            format!(
                "project state head: entry #{}, {}",
                d.audit_entries, d.audit_head
            ),
        ],
        ["Evidence integrity".into(), integrity],
    ];
    if d.evidence.is_empty() {
        details.push(["Evidence".into(), "none imported".into()]);
    }
    let evidence = Section {
        heading: "Evidence".into(),
        blocks: vec![
            Block::Text {
                text: "Every file as imported, stored read-only and byte for byte, with the SHA-256 recorded at import. The last column is the audit entry that recorded the import.".into(),
            },
            Block::Table {
                widths: ["auto", "1fr", "auto", "auto", "auto"].map(String::from).to_vec(),
                head: ["#", "File", "Format, unit", "Size", "Imported"].map(String::from).to_vec(),
                rows: d
                    .evidence
                    .iter()
                    .map(|e| {
                        vec![
                            e.id.to_string(),
                            format!("{}\nSHA-256 {}", e.name, e.sha256),
                            format!("{}, {}", e.format, e.unit),
                            size(e.size),
                            format!("{}\n{}", e.imported, e.entry),
                        ]
                    })
                    .collect(),
            },
        ],
    };
    let mut kinds: Vec<&str> = vec![];
    for r in &d.records {
        if !kinds.contains(&r.kind.as_str()) {
            kinds.push(&r.kind);
        }
    }
    let mut sections = vec![evidence];
    for k in kinds {
        let rows: Vec<Vec<String>> = d
            .records
            .iter()
            .filter(|r| r.kind == k)
            .map(|r| {
                vec![
                    r.id.clone(),
                    if r.status.is_empty() {
                        r.name.clone()
                    } else {
                        format!("{} ({})", r.name, r.status)
                    },
                    format!("{}\nSHA-256 {}", r.made, r.sha256),
                    r.entry.clone(),
                ]
            })
            .collect();
        sections.push(Section {
            heading: k.to_string(),
            blocks: vec![Block::Table {
                widths: ["auto", "1fr", "1fr", "auto"].map(String::from).to_vec(),
                head: ["#", "Name", "Made, record hash", "Audit entry"]
                    .map(String::from)
                    .to_vec(),
                rows,
            }],
        });
    }
    sections.push(Section {
        heading: "How to check this report".into(),
        blocks: vec![Block::List {
            items: vec![
                "Each evidence file's SHA-256 can be recomputed from the file in the project's evidence folder, or from the original.".into(),
                "Each record's SHA-256 is of its stored content, and the audit entry named beside it carries that hash. Every entry is chained to the one before by its own hash, so a changed or removed entry breaks the chain; the project checks the chain each time it opens.".into(),
                "Withdrawn records are listed with their reason: nothing is deleted.".into(),
                "Every analysis has its own report with its method, inputs, results, uncertainty, assumptions and limitations.".into(),
            ],
        }],
    });
    sections.push(Section {
        heading: "Sign-off".into(),
        blocks: vec![Block::SignOff {
            rows: vec!["Examiner".into(), "Technical review".into()],
        }],
    });
    Report {
        title: format!("Case report: {}", d.project),
        header: format!(
            "{}{} · case report",
            d.case_number
                .as_ref()
                .map(|c| format!("{c} · "))
                .unwrap_or_default(),
            d.project
        ),
        details,
        warnings,
        sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::pdf;

    fn data(printed_at: &str, failed: Vec<String>) -> CaseData {
        CaseData {
            project: "Main Street".into(),
            case_number: Some("2026-00417".into()),
            printed_by: "Examiner".into(),
            printed_at: printed_at.into(),
            app_version: "0.1.0".into(),
            audit_head: "ab".repeat(32),
            audit_entries: 41,
            integrity: Some(("2026-09-24T09:00:00Z".into(), 2, failed)),
            evidence: vec![EvidenceRow {
                id: 1,
                name: "hall.e57".into(),
                sha256: "cd".repeat(32),
                size: 19_360_768,
                format: "E57".into(),
                unit: "m".into(),
                imported: "2026-09-23T10:00:00Z by Examiner".into(),
                entry: "#2, 1f2e".into(),
            }],
            records: vec![
                RecordRow {
                    kind: "Analyses".into(),
                    id: "3".into(),
                    name: "skid: Skid 1".into(),
                    sha256: "ef".repeat(32),
                    made: "2026-09-23T11:00:00Z by Examiner".into(),
                    status: "withdrawn: wrong drag factor".into(),
                    entry: "#9, 77aa".into(),
                },
                RecordRow {
                    kind: "Measurements".into(),
                    id: "1".into(),
                    name: "distance 4.217 m ± 2.8 mm".into(),
                    sha256: "—".into(),
                    made: "2026-09-23T10:30:00Z by Examiner".into(),
                    status: String::new(),
                    entry: "#5, 99bb".into(),
                },
            ],
        }
    }

    #[test]
    fn the_case_report_indexes_evidence_and_records_reproducibly() {
        let a = pdf(&report(&data("2026-09-24T10:00:00Z", vec![])), vec![]).unwrap();
        let b = pdf(&report(&data("2026-09-24T10:00:00Z", vec![])), vec![]).unwrap();
        assert!(a.pdf == b.pdf);
        let c = pdf(&report(&data("2026-09-25T10:00:00Z", vec![])), vec![]).unwrap();
        assert_eq!(
            a.text.replace("2026-09-24T10:00:00Z", "T"),
            c.text.replace("2026-09-25T10:00:00Z", "T")
        );
        for want in [
            "2026-00417",
            "hall.e57",
            &"cd".repeat(32),
            "18.5 MiB",
            "all 2 evidence files match",
            "wrong drag factor",
            "#9, 77aa",
            "Measurements",
            "Technical review",
        ] {
            assert!(a.text.contains(want), "missing {want:?}");
        }
        // A failed check is in "Needs attention".
        let f = report(&data("x", vec!["#1 hall.e57 changed".into()]));
        assert_eq!(f.warnings.len(), 1);
    }
}
