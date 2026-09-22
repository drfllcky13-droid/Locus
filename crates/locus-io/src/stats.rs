use crate::{Progress, ProgressFn, Stage};
use locus_core::{Bounds, ScanInfo};
use std::io::{self, BufRead, Read};

/// One point as stored in the source file, in the file's own coordinates and unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub p: [f64; 3],
    pub rgb: Option<[u8; 3]>,
    /// Scaled to the full u16 range from whatever range the format uses.
    pub intensity: Option<u16>,
    /// Record number within its scan, counting records with no valid position too.
    pub index: u64,
}

/// Scan to visit and the callback. Readers given a visitor may skip other scans.
pub(crate) type Visitor<'a> = Option<(usize, &'a mut dyn FnMut(&Point))>;

/// Map `v` from `[lo, hi]` onto `[0, max]`, clamped.
pub(crate) fn rescale(v: f64, lo: f64, hi: f64, max: f64) -> f64 {
    if hi > lo {
        ((v - lo) / (hi - lo) * max).clamp(0.0, max).round()
    } else {
        0.0
    }
}

/// Running count and bounds while streaming a scan.
#[derive(Default)]
pub(crate) struct ScanStats {
    pub count: u64,
    pub invalid: u64,
    pub bounds: Option<Bounds>,
}

impl ScanStats {
    pub fn point(&mut self, p: [f64; 3]) {
        self.count += 1;
        if p.iter().all(|v| v.is_finite()) {
            Bounds::grow(&mut self.bounds, p);
        } else {
            self.invalid += 1;
        }
    }

    pub fn invalid(&mut self) {
        self.count += 1;
        self.invalid += 1;
    }

    pub fn into_scan(self, name: String, pose: [f64; 16], attributes: Vec<String>) -> ScanInfo {
        ScanInfo {
            name,
            point_count: self.count,
            invalid_points: self.invalid,
            bounds: self.bounds,
            pose,
            attributes,
        }
    }
}

/// Reader that reports bytes consumed as `Stage::Reading` progress, about every 4 MiB.
pub(crate) struct Counting<'a, R> {
    inner: R,
    done: u64,
    last: u64,
    total: u64,
    progress: ProgressFn<'a>,
}

impl<'a, R> Counting<'a, R> {
    pub fn new(inner: R, total: u64, progress: ProgressFn<'a>) -> Self {
        Self {
            inner,
            done: 0,
            last: 0,
            total,
            progress,
        }
    }

    fn advance(&mut self, n: usize) {
        self.done += n as u64;
        if self.done - self.last >= 4 << 20 || self.done == self.total {
            self.last = self.done;
            (self.progress)(Progress {
                stage: Stage::Reading,
                done: self.done,
                total: self.total,
            });
        }
    }
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.advance(n);
        Ok(n)
    }
}

impl<R: BufRead> BufRead for Counting<'_, R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt);
        self.advance(amt);
    }
}

/// File stem, for naming single-scan files.
pub(crate) fn stem(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
