use crate::{Progress, ProgressFn, Stage};
use locus_core::{Bounds, ScanInfo};
use std::io::{self, BufRead, Read};

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
