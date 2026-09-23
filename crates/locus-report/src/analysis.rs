//! The report layout every analysis tool shares: a title block with the record's identity
//! (id, method, hash, who and when), a box of warnings, then sections built from a few
//! kinds of block (text, key–value pairs, tables, lists, figures). Each tool turns its
//! stored run into a [`Report`] (see `trajectory.rs`); the template lays it out and
//! computes nothing, so every number printed is one the tool formatted.

use crate::{render_with, Rendered};
use serde::Serialize;

pub const TEMPLATE: &str = include_str!("../templates/analysis.typ");

/// Who, what and when, for the title block of any analysis report.
#[derive(Debug, Clone)]
pub struct Meta {
    pub project: String,
    pub record_id: i64,
    pub name: String,
    pub method: String,
    pub sha256: String,
    pub revises: Option<i64>,
    pub created_at: String,
    pub created_by: String,
    /// "Withdrawn on … by …: reason", if it was.
    pub withdrawn: Option<String>,
    pub printed_by: String,
    pub printed_at: String,
    pub app_version: String,
    /// Hash of the newest audit entry when the report was made.
    pub audit_head: String,
    /// The project's case number, if set.
    pub case_number: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub title: String,
    /// Running header (project and record).
    pub header: String,
    pub details: Vec<[String; 2]>,
    pub warnings: Vec<String>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub heading: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    Pairs {
        rows: Vec<[String; 2]>,
    },
    /// `widths` are Typst column widths ("auto", "1fr", "22mm").
    Table {
        widths: Vec<String>,
        head: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    List {
        items: Vec<String>,
    },
    Figure(Figure),
    /// An image passed in with the report (`file`), `width` mm wide.
    Image {
        file: String,
        width: f64,
        caption: String,
    },
    /// Lines to sign: a label and blank space for each.
    SignOff {
        rows: Vec<String>,
    },
}

/// A drawing in its own box, millimetres from its top left (y down).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Figure {
    pub width: f64,
    pub height: f64,
    pub caption: String,
    pub lines: Vec<Line>,
    /// Filled outlines: points and a fill colour ("#rrggbb").
    pub areas: Vec<Area>,
    /// [x, y, radius mm].
    pub dots: Vec<[f64; 3]>,
    pub labels: Vec<Label>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// Stroke width (mm).
    pub width: f64,
    pub dashed: bool,
    /// "#rrggbb".
    pub colour: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Area {
    pub points: Vec<[f64; 2]>,
    pub fill: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Label {
    pub x: f64,
    pub y: f64,
    pub text: String,
}

/// Maps a world rectangle onto a figure box with equal scale on both axes (a margin kept),
/// world y up. Also picks a round scale-bar length.
pub struct Frame {
    lo: [f64; 2],
    s: f64,
    ox: f64,
    oy: f64,
    height: f64,
}

impl Frame {
    pub fn new(lo: [f64; 2], hi: [f64; 2], width: f64, height: f64, margin: f64) -> Frame {
        let span = [(hi[0] - lo[0]).max(1e-6), (hi[1] - lo[1]).max(1e-6)];
        let s = ((width - 2.0 * margin) / span[0]).min((height - 2.0 * margin) / span[1]);
        Frame {
            lo,
            s,
            ox: (width - span[0] * s) / 2.0,
            oy: (height - span[1] * s) / 2.0,
            height,
        }
    }
    pub fn at(&self, p: [f64; 2]) -> [f64; 2] {
        [
            self.ox + (p[0] - self.lo[0]) * self.s,
            self.height - (self.oy + (p[1] - self.lo[1]) * self.s),
        ]
    }
    /// A 1–2–5 length (m) near a fifth of `width` mm, and its length on paper.
    pub fn scale_bar(&self, width: f64) -> (f64, f64) {
        let target = width / 5.0 / self.s;
        let p = 10f64.powf(target.log10().floor());
        let m = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|k| k * p)
            .rfind(|v| *v <= target)
            .unwrap_or(p);
        (m, m * self.s)
    }
}

impl Figure {
    pub fn line(&mut self, a: [f64; 2], b: [f64; 2], w: f64, dashed: bool) {
        self.coloured(a, b, w, dashed, "#000000");
    }
    pub fn coloured(&mut self, a: [f64; 2], b: [f64; 2], width: f64, dashed: bool, colour: &str) {
        self.lines.push(Line {
            a,
            b,
            width,
            dashed,
            colour: colour.into(),
        });
    }
    /// A scale bar in the bottom left corner.
    pub fn scale_bar(&mut self, f: &Frame) {
        let (m, mm) = f.scale_bar(self.width);
        let y = self.height - 4.0;
        self.line([4.0, y], [4.0 + mm, y], 0.5, false);
        self.line([4.0, y - 1.2], [4.0, y + 1.2], 0.3, false);
        self.line([4.0 + mm, y - 1.2], [4.0 + mm, y + 1.2], 0.3, false);
        self.labels.push(Label {
            x: 4.0 + mm + 2.0,
            y: y - 1.5,
            text: if m >= 1.0 {
                format!("{m} m")
            } else {
                format!("{} mm", m * 1000.0)
            },
        });
    }
}

/// Lay a report out as a PDF, with the image files its `Image` blocks name.
pub fn pdf(r: &Report, images: Vec<(String, Vec<u8>)>) -> Result<Rendered, String> {
    render_with(
        TEMPLATE,
        serde_json::to_vec(r).map_err(|e| e.to_string())?,
        images,
    )
}

/// Title-block rows every analysis report starts with.
pub fn details(m: &Meta) -> Vec<[String; 2]> {
    let mut d = vec![
        [
            "Case number".into(),
            m.case_number.clone().unwrap_or_else(|| "(not set)".into()),
        ],
        ["Project".into(), m.project.clone()],
        [
            "Record".into(),
            format!("{} (analysis {})", m.name, m.record_id),
        ],
        ["Method".into(), m.method.clone()],
        ["Record SHA-256".into(), m.sha256.clone()],
        [
            "Run by".into(),
            format!("{}, {}", m.created_by, m.created_at),
        ],
    ];
    if let Some(r) = m.revises {
        d.push(["Revises".into(), format!("analysis {r}")]);
    }
    d.push([
        "Printed".into(),
        format!(
            "{}, {} (Locus {})",
            m.printed_by, m.printed_at, m.app_version
        ),
    ]);
    d.push(["Audit log head".into(), m.audit_head.clone()]);
    d
}
