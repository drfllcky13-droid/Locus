//! The validation report: the suite's results as a PDF in the analysis reports' layout.

use crate::suite::ToolResult;
use locus_report::analysis::{Block, Report, Section};

fn fmt(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else if v.abs() >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.3}")
    }
}

pub struct Run {
    pub version: String,
    pub commit: String,
    pub made_at: String,
    pub machine: String,
    pub full: bool,
    pub seconds: f64,
}

/// A coverage below this, over enough cases, is flagged.
const LOW_COVERAGE: f64 = 0.90;

pub fn report(r: &Run, results: &[ToolResult]) -> Report {
    let mut warnings = vec![];
    for t in results {
        if let Some((text, false)) = &t.bound {
            warnings.push(format!("{}: the bound \"{text}\" was NOT met.", t.tool));
        }
        if let Some((i, n)) = t.coverage {
            if n >= 20 && (i as f64) < LOW_COVERAGE * n as f64 {
                warnings.push(format!(
                    "{}: its stated 95 % region covered the truth in {i} of {n} cases ({:.0} %).",
                    t.tool,
                    i as f64 / n as f64 * 100.0
                ));
            }
        }
    }
    let rows = results
        .iter()
        .map(|t| {
            vec![
                t.tool.clone(),
                t.errors.len().to_string(),
                format!("{} {}", fmt(t.mean()), t.unit),
                fmt(t.sd()),
                fmt(t.p95()),
                fmt(t.max()),
                t.coverage.map_or("—".into(), |(i, n)| {
                    format!("{i}/{n} ({:.0} %)", i as f64 / n.max(1) as f64 * 100.0)
                }),
                match &t.bound {
                    Some((_, true)) => "met".into(),
                    Some((_, false)) => "NOT MET".into(),
                    None => "—".into(),
                },
            ]
        })
        .collect();
    let mut sections = vec![Section {
        heading: "Summary".into(),
        blocks: vec![
            Block::Text {
                text: "Each tool run end to end on synthetic ground truth. Errors are absolute; the columns are the mean (with its unit), standard deviation, 95th percentile and worst, then how often the tool's stated 95 % interval or region contained the truth, and whether the tool met its bound.".into(),
            },
            Block::Table {
                widths: ["1fr", "auto", "auto", "auto", "auto", "auto", "auto", "auto"]
                    .map(String::from)
                    .to_vec(),
                head: ["Tool", "Cases", "Mean", "SD", "95th pct", "Worst", "95 % coverage", "Bound"]
                    .map(String::from)
                    .to_vec(),
                rows,
            },
        ],
    }];
    for t in results {
        let mut pairs = vec![
            ["Error".into(), format!("{} ({})", t.what, t.unit)],
            [
                "Cases".into(),
                format!("{} ({:.1} s)", t.errors.len(), t.seconds),
            ],
            [
                "Mean, SD".into(),
                format!("{} ± {} {}", fmt(t.mean()), fmt(t.sd()), t.unit),
            ],
            [
                "95th percentile, worst".into(),
                format!("{}, {} {}", fmt(t.p95()), fmt(t.max()), t.unit),
            ],
        ];
        if let Some((i, n)) = t.coverage {
            pairs.push([
                "Stated 95 % coverage".into(),
                format!("{i} of {n} ({:.1} %)", i as f64 / n.max(1) as f64 * 100.0),
            ]);
        }
        if let Some((text, ok)) = &t.bound {
            pairs.push([
                "Bound".into(),
                format!("{text}: {}", if *ok { "met" } else { "NOT MET" }),
            ]);
        }
        sections.push(Section {
            heading: t.tool.clone(),
            blocks: vec![
                Block::Pairs { rows: pairs },
                Block::List {
                    items: t.notes.clone(),
                },
            ],
        });
    }
    sections.push(Section {
        heading: "What this does and doesn't show".into(),
        blocks: vec![Block::List {
            items: vec![
                "It shows each tool recovers known truth on synthetic data, with its stated uncertainty holding, across many cases.".into(),
                "Synthetic data is not casework: real surfaces, lighting, scanners and examiners add error it cannot model. Physical validation studies (staged scenes, several blind examiners) are described in docs/methods/validation-protocol.md; their results are reported separately.".into(),
                "The generators, the seeds and this program are in the repository, so every number here can be regenerated.".into(),
            ],
        }],
    });
    sections.push(Section {
        heading: "Sign-off".into(),
        blocks: vec![Block::SignOff {
            rows: vec!["Prepared by".into(), "Technical review".into()],
        }],
    });
    Report {
        title: format!("Validation report: Lotus {}", r.version),
        header: format!("Lotus {} · validation", r.version),
        details: vec![
            [
                "Software".into(),
                format!("Lotus {} (commit {})", r.version, r.commit),
            ],
            ["Run".into(), format!("{}, {}", r.made_at, r.machine)],
            [
                "Size".into(),
                format!(
                    "{} ({:.0} s)",
                    if r.full {
                        "full, for a release"
                    } else {
                        "quick check (few cases per tool)"
                    },
                    r.seconds
                ),
            ],
            [
                "Result".into(),
                if warnings.is_empty() {
                    "every bound met".into()
                } else {
                    format!("{} item(s) need attention", warnings.len())
                },
            ],
        ],
        warnings,
        sections,
    }
}
