//! PTS and XYZ: whitespace/comma separated text, one point per line.
//!
//! PTS files may contain several blocks, each introduced by a line holding only the
//! block's point count; each block becomes its own scan. Malformed lines are an error,
//! never skipped silently.

use crate::stats::{stem, Counting, ScanStats};
use crate::{parse_err, ProgressFn, Result};
use locus_core::{Contents, IDENTITY};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Pts,
    Xyz,
}

struct Block {
    declared: Option<u64>,
    columns: usize,
    mixed_columns: bool,
    stats: ScanStats,
}

impl Block {
    fn new(declared: Option<u64>) -> Self {
        Self {
            declared,
            columns: 0,
            mixed_columns: false,
            stats: ScanStats::default(),
        }
    }
}

fn attributes(kind: Kind, columns: usize) -> Vec<String> {
    match (kind, columns) {
        (_, 3) => vec![],
        (Kind::Pts, 4) => vec!["intensity".into()],
        (Kind::Pts, 6) => vec!["color".into()],
        (Kind::Pts, 7) => vec!["intensity".into(), "color".into()],
        (_, n) => vec![format!("{} extra columns", n - 3)],
    }
}

pub(crate) fn inspect(path: &Path, kind: Kind, progress: ProgressFn) -> Result<Contents> {
    let name = if kind == Kind::Pts { "PTS" } else { "XYZ" };
    let total = std::fs::metadata(path)?.len();
    let mut r = Counting::new(
        BufReader::with_capacity(1 << 20, File::open(path)?),
        total,
        progress,
    );
    let mut blocks: Vec<Block> = vec![];
    let mut line = String::new();
    let mut line_no = 0u64;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        line_no += 1;
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with("//") {
            continue;
        }
        let mut tokens = t
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .filter(|s| !s.is_empty());
        let first = tokens.next().unwrap_or_default();
        let second = tokens.next();
        if kind == Kind::Pts && second.is_none() {
            if let Ok(n) = first.parse::<u64>() {
                blocks.push(Block::new(Some(n)));
                continue;
            }
        }
        let third = tokens.next();
        let bad = || {
            parse_err(
                name,
                format!("line {line_no}: expected at least x y z, found {t:.60}"),
            )
        };
        let (Some(y), Some(z)) = (second, third) else {
            return Err(bad());
        };
        let p = [first, y, z].map(|s| s.parse::<f64>());
        let [Ok(x), Ok(y), Ok(z)] = p else {
            return Err(bad());
        };
        let columns = 3 + tokens.count();
        if blocks.is_empty() {
            blocks.push(Block::new(None));
        }
        let b = blocks.last_mut().unwrap();
        if b.columns == 0 {
            b.columns = columns;
        } else if b.columns != columns {
            b.mixed_columns = true;
        }
        b.stats.point([x, y, z]);
    }

    let mut c = Contents::new(name);
    let many = blocks.len() > 1;
    for (i, b) in blocks.into_iter().enumerate() {
        let scan_name = if many {
            format!("Block {}", i + 1)
        } else {
            stem(path)
        };
        if let Some(d) = b.declared.filter(|&d| d != b.stats.count) {
            c.warnings.push(format!(
                "{scan_name}: header says {d} points, file contains {}",
                b.stats.count
            ));
        }
        if b.mixed_columns {
            c.warnings
                .push(format!("{scan_name}: lines have differing column counts"));
        }
        c.scans.push(
            b.stats
                .into_scan(scan_name, IDENTITY, attributes(kind, b.columns)),
        );
    }
    if c.scans.is_empty() {
        return Err(parse_err(name, "no points found"));
    }
    Ok(c)
}
