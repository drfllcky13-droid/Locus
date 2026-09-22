//! Two passes over the source, then per-chunk indexing in memory.
//!
//! 1. Count points into a 128³ grid over the cube. From the count pyramid, choose *chunks*:
//!    the largest octree nodes holding at most `chunk_max` points. Nodes above them are
//!    *upper* nodes.
//! 2. Stream the points again. Each walks down the upper nodes; the first whose sampling
//!    grid cell is still empty keeps it. Points no upper node keeps go to their chunk's
//!    temp file.
//! 3. Load each chunk and index it top-down the same way, writing nodes as they are made.
//!
//! Memory is bounded by `chunk_max` and the upper nodes' samples, not by the scan size.

use crate::{cell, child_min, octant, Meta, Node, Rec, Result, FORMAT_VERSION};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// A node with at most this many points keeps them all and has no children.
    pub leaf_max: usize,
    /// Sampling grid per node edge.
    pub grid: u32,
    /// Largest subtree indexed in memory at once.
    pub chunk_max: u64,
    pub has_color: bool,
    pub has_intensity: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            leaf_max: 50_000,
            grid: 128,
            chunk_max: 5_000_000,
            has_color: false,
            has_intensity: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStage {
    Counting,
    Distributing,
    Indexing,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BuildProgress {
    pub stage: BuildStage,
    pub done: u64,
    pub total: u64,
}

/// Feeds every point to the callback it is given; called once per pass.
pub type Source<'a> = &'a mut dyn FnMut(&mut dyn FnMut(Rec)) -> Result<()>;

/// Counting grid is 2^COUNT_LEVELS per edge; chunks are never deeper than this.
const COUNT_LEVELS: u32 = 7;
/// Below this depth, or when points coincide, a node keeps everything it is given.
const MAX_LEVEL: usize = 24;
const REC_BYTES: usize = 33;

#[derive(Clone, Copy)]
enum Slot {
    Empty,
    Upper(usize),
    Chunk(usize),
}

struct Upper {
    name: String,
    min: [f64; 3],
    size: f64,
    children: [Slot; 8],
    cells: HashSet<u32>,
    pts: Vec<Rec>,
}

struct Chunk {
    name: String,
    min: [f64; 3],
    size: f64,
    path: PathBuf,
    file: Option<BufWriter<File>>,
    count: u64,
}

struct Writer<'a> {
    out: BufWriter<File>,
    offset: u64,
    nodes: Vec<Node>,
    opts: &'a BuildOptions,
}

impl Writer<'_> {
    fn emit(&mut self, name: String, min: [f64; 3], size: f64, pts: &[Rec]) -> Result<()> {
        let start = self.offset;
        let mut buf = Vec::with_capacity(pts.len() * REC_BYTES);
        for r in pts {
            for v in r.p {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        for r in pts {
            buf.extend_from_slice(&r.index.to_le_bytes());
        }
        if self.opts.has_color {
            for r in pts {
                buf.extend_from_slice(&r.rgb);
            }
        }
        if self.opts.has_intensity {
            for r in pts {
                buf.extend_from_slice(&r.intensity.to_le_bytes());
            }
        }
        self.out.write_all(&buf)?;
        self.offset += buf.len() as u64;
        self.nodes.push(Node {
            name,
            min,
            size,
            count: pts.len() as u32,
            subtree: 0,
            spacing: size / self.opts.grid as f64,
            offset: start,
            bytes: self.offset - start,
            children: 0,
        });
        Ok(())
    }

    /// Top-down indexing of an in-memory subtree.
    fn index(&mut self, name: String, min: [f64; 3], size: f64, pts: Vec<Rec>) -> Result<()> {
        if pts.len() <= self.opts.leaf_max || name.len() > MAX_LEVEL {
            return self.emit(name, min, size, &pts);
        }
        let grid = self.opts.grid;
        let mut cells = HashSet::with_capacity(pts.len().min(1 << 20));
        let mut kept = Vec::new();
        let mut kids: [Vec<Rec>; 8] = Default::default();
        for r in pts {
            if cells.insert(cell(&r.p, &min, size, grid)) {
                kept.push(r);
            } else {
                kids[octant(&r.p, &min, size)].push(r);
            }
        }
        drop(cells);
        self.emit(name.clone(), min, size, &kept)?;
        drop(kept);
        for (o, k) in kids.into_iter().enumerate() {
            if !k.is_empty() {
                self.index(
                    format!("{name}{o}"),
                    child_min(&min, size, o),
                    size / 2.0,
                    k,
                )?;
            }
        }
        Ok(())
    }
}

fn write_rec(w: &mut impl Write, r: &Rec) -> std::io::Result<()> {
    let mut b = [0u8; REC_BYTES];
    for (i, v) in r.p.iter().enumerate() {
        b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    b[24..28].copy_from_slice(&r.index.to_le_bytes());
    b[28..31].copy_from_slice(&r.rgb);
    b[31..33].copy_from_slice(&r.intensity.to_le_bytes());
    w.write_all(&b)
}

fn read_recs(path: &Path, count: u64) -> Result<Vec<Rec>> {
    let mut r = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut out = Vec::with_capacity(count as usize);
    let mut b = [0u8; REC_BYTES];
    for _ in 0..count {
        r.read_exact(&mut b)?;
        let f = |i: usize| f64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        out.push(Rec {
            p: [f(0), f(1), f(2)],
            index: u32::from_le_bytes(b[24..28].try_into().unwrap()),
            rgb: [b[28], b[29], b[30]],
            intensity: u16::from_le_bytes([b[31], b[32]]),
        });
    }
    Ok(out)
}

/// Integer cell of `p` at the counting level, used for every descent through the planned
/// tree so the counting and distribution passes always agree on where a point belongs.
fn count_cell(p: &[f64; 3], min: &[f64; 3], size: f64) -> [u32; 3] {
    let g = (1u32 << COUNT_LEVELS) as f64;
    std::array::from_fn(|i| (((p[i] - min[i]) / size * g) as i64).clamp(0, g as i64 - 1) as u32)
}

fn octant_at(c: &[u32; 3], level: u32) -> usize {
    let s = COUNT_LEVELS - level - 1;
    ((((c[0] >> s) & 1) << 2) | (((c[1] >> s) & 1) << 1) | ((c[2] >> s) & 1)) as usize
}

/// Build an octree in `dir` (created if missing). `min`/`size` is a cube containing every
/// point. `source` is called twice and must produce the same points in the same order.
pub fn build(
    dir: &Path,
    min: [f64; 3],
    size: f64,
    opts: &BuildOptions,
    source: Source,
    progress: &mut dyn FnMut(BuildProgress),
) -> Result<Meta> {
    if !(size > 0.0 && size.is_finite()) {
        return Err(crate::Error::Invalid(format!("bad cube size {size}")));
    }
    fs::create_dir_all(dir)?;
    let tmp = dir.join("tmp");
    fs::create_dir_all(&tmp)?;

    // Pass 1: count.
    let dim = 1usize << COUNT_LEVELS;
    let mut counts = vec![0u64; dim * dim * dim];
    let mut total = 0u64;
    source(&mut |r| {
        let c = count_cell(&r.p, &min, size);
        counts[(c[0] as usize * dim + c[1] as usize) * dim + c[2] as usize] += 1;
        total += 1;
        if total.is_multiple_of(1 << 20) {
            progress(BuildProgress {
                stage: BuildStage::Counting,
                done: total,
                total: 0,
            });
        }
    })?;
    if total > u32::MAX as u64 {
        return Err(crate::Error::Invalid(
            "more than 4,294,967,295 points in one scan".into(),
        ));
    }

    // Count pyramid: pyramid[l] has (2^l)³ cells.
    let mut pyramid = vec![counts];
    for l in (0..COUNT_LEVELS).rev() {
        let d = 1usize << l;
        let below = pyramid.last().unwrap();
        let mut level = vec![0u64; d * d * d];
        for x in 0..2 * d {
            for y in 0..2 * d {
                for z in 0..2 * d {
                    level[((x / 2) * d + y / 2) * d + z / 2] += below[(x * 2 * d + y) * 2 * d + z];
                }
            }
        }
        pyramid.push(level);
    }
    pyramid.reverse();

    // Plan upper nodes and chunks.
    let mut uppers: Vec<Upper> = vec![];
    let mut chunks: Vec<Chunk> = vec![];
    #[allow(clippy::too_many_arguments)]
    fn plan(
        l: u32,
        at: [usize; 3],
        name: String,
        min: [f64; 3],
        size: f64,
        pyramid: &[Vec<u64>],
        chunk_max: u64,
        tmp: &Path,
        uppers: &mut Vec<Upper>,
        chunks: &mut Vec<Chunk>,
    ) -> Result<Slot> {
        let d = 1usize << l;
        let count = pyramid[l as usize][(at[0] * d + at[1]) * d + at[2]];
        if count == 0 {
            return Ok(Slot::Empty);
        }
        if count <= chunk_max || l == COUNT_LEVELS {
            let path = tmp.join(format!("{name}.bin"));
            let file = Some(BufWriter::with_capacity(1 << 18, File::create(&path)?));
            chunks.push(Chunk {
                name,
                min,
                size,
                path,
                file,
                count: 0,
            });
            return Ok(Slot::Chunk(chunks.len() - 1));
        }
        let me = uppers.len();
        uppers.push(Upper {
            name: name.clone(),
            min,
            size,
            children: [Slot::Empty; 8],
            cells: HashSet::new(),
            pts: vec![],
        });
        for o in 0..8 {
            let child = [
                at[0] * 2 + ((o >> 2) & 1),
                at[1] * 2 + ((o >> 1) & 1),
                at[2] * 2 + (o & 1),
            ];
            let slot = plan(
                l + 1,
                child,
                format!("{name}{o}"),
                child_min(&min, size, o),
                size / 2.0,
                pyramid,
                chunk_max,
                tmp,
                uppers,
                chunks,
            )?;
            uppers[me].children[o] = slot;
        }
        Ok(Slot::Upper(me))
    }
    let root = plan(
        0,
        [0; 3],
        "r".into(),
        min,
        size,
        &pyramid,
        opts.chunk_max,
        &tmp,
        &mut uppers,
        &mut chunks,
    )?;
    drop(pyramid);

    // Pass 2: distribute.
    let mut done = 0u64;
    let mut failed: Option<std::io::Error> = None;
    source(&mut |r| {
        let c = count_cell(&r.p, &min, size);
        let mut slot = root;
        let mut level = 0;
        loop {
            match slot {
                Slot::Upper(i) => {
                    let u = &mut uppers[i];
                    if u.cells.insert(cell(&r.p, &u.min, u.size, opts.grid)) {
                        u.pts.push(r);
                        break;
                    }
                    slot = u.children[octant_at(&c, level)];
                    level += 1;
                }
                Slot::Chunk(k) => {
                    let ch = &mut chunks[k];
                    if let Err(e) = write_rec(ch.file.as_mut().unwrap(), &r) {
                        failed.get_or_insert(e);
                    }
                    ch.count += 1;
                    break;
                }
                Slot::Empty => unreachable!("the counting pass saw every point"),
            }
        }
        done += 1;
        if done.is_multiple_of(1 << 20) {
            progress(BuildProgress {
                stage: BuildStage::Distributing,
                done,
                total,
            });
        }
    })?;
    if let Some(e) = failed {
        return Err(e.into());
    }
    for ch in &mut chunks {
        ch.file.take().unwrap().flush()?;
    }

    // Pass 3: write upper nodes, then index each chunk.
    let mut w = Writer {
        out: BufWriter::with_capacity(1 << 20, File::create(dir.join("nodes.bin"))?),
        offset: 0,
        nodes: vec![],
        opts,
    };
    for u in uppers {
        w.emit(u.name, u.min, u.size, &u.pts)?;
    }
    let chunk_points: u64 = chunks.iter().map(|c| c.count).sum();
    let mut indexed = 0u64;
    for ch in chunks {
        let pts = read_recs(&ch.path, ch.count)?;
        fs::remove_file(&ch.path)?;
        indexed += ch.count;
        if !pts.is_empty() {
            w.index(ch.name, ch.min, ch.size, pts)?;
        }
        progress(BuildProgress {
            stage: BuildStage::Indexing,
            done: indexed,
            total: chunk_points,
        });
    }
    w.out.flush()?;
    fs::remove_dir(&tmp)?;

    // Hierarchy: sort, link children, sum subtrees.
    let mut nodes = w.nodes;
    nodes.sort_by(|a, b| {
        a.name
            .len()
            .cmp(&b.name.len())
            .then_with(|| a.name.cmp(&b.name))
    });
    let by_name: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name.clone(), i))
        .collect();
    for i in (0..nodes.len()).rev() {
        nodes[i].subtree += nodes[i].count as u64;
        let name = nodes[i].name.clone();
        if name.len() > 1 {
            let parent = by_name[&name[..name.len() - 1]];
            let digit = name.as_bytes()[name.len() - 1] - b'0';
            nodes[parent].children |= 1 << digit;
            nodes[parent].subtree += nodes[i].subtree;
        }
    }
    let meta = Meta {
        version: FORMAT_VERSION,
        min,
        size,
        points: total,
        nodes: nodes.len(),
        has_color: opts.has_color,
        has_intensity: opts.has_intensity,
        grid: opts.grid,
        leaf_max: opts.leaf_max,
    };
    fs::write(dir.join("hierarchy.json"), serde_json::to_vec(&nodes)?)?;
    fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)?;
    Ok(meta)
}
