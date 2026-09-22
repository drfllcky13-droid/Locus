use crate::{Error, Meta, Node, Result};
use roaring::RoaringBitmap;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Bytes before the point data in a served node: count u32, flags u32 (bit 0 colour,
/// bit 1 intensity), 8 bytes reserved.
pub const SERVE_HEADER: usize = 16;

pub struct Octree {
    pub dir: PathBuf,
    pub meta: Meta,
    pub nodes: Vec<Node>,
    by_name: HashMap<String, usize>,
}

/// A node's points as stored: f64 scan-local meters and source record numbers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodePoints {
    pub xyz: Vec<[f64; 3]>,
    pub index: Vec<u32>,
    pub rgb: Vec<[u8; 3]>,
    pub intensity: Vec<u16>,
}

impl Octree {
    pub fn open(dir: &Path) -> Result<Self> {
        let meta: Meta = serde_json::from_slice(&std::fs::read(dir.join("meta.json"))?)?;
        if meta.version != crate::FORMAT_VERSION {
            return Err(Error::Invalid(format!(
                "unsupported octree version {}",
                meta.version
            )));
        }
        let nodes: Vec<Node> = serde_json::from_slice(&std::fs::read(dir.join("hierarchy.json"))?)?;
        let by_name = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.name.clone(), i))
            .collect();
        Ok(Self {
            dir: dir.into(),
            meta,
            nodes,
            by_name,
        })
    }

    pub fn node(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    pub fn read(&self, i: usize) -> Result<NodePoints> {
        let node = self
            .nodes
            .get(i)
            .ok_or_else(|| Error::Invalid(format!("no node {i}")))?;
        let mut f = File::open(self.dir.join("nodes.bin"))?;
        f.seek(SeekFrom::Start(node.offset))?;
        let mut b = vec![0u8; node.bytes as usize];
        f.read_exact(&mut b)?;
        let n = node.count as usize;
        let mut at = 0;
        let mut take = |len: usize| {
            let s = &b[at..at + len];
            at += len;
            s
        };
        let pos = take(n * 24);
        let xyz = pos
            .as_chunks::<24>()
            .0
            .iter()
            .map(|c| {
                std::array::from_fn(|k| f64::from_le_bytes(c[k * 8..k * 8 + 8].try_into().unwrap()))
            })
            .collect();
        let index = take(n * 4)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        let rgb = if self.meta.has_color {
            take(n * 3).as_chunks::<3>().0.to_vec()
        } else {
            vec![]
        };
        let intensity = if self.meta.has_intensity {
            take(n * 2)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect()
        } else {
            vec![]
        };
        Ok(NodePoints {
            xyz,
            index,
            rgb,
            intensity,
        })
    }

    /// Positions of the points that survive `removed`, in stored order.
    fn kept(points: &NodePoints, removed: Option<&RoaringBitmap>) -> Vec<usize> {
        (0..points.index.len())
            .filter(|&k| removed.is_none_or(|r| !r.contains(points.index[k])))
            .collect()
    }

    /// A node for the GPU: header, then f32 xyz relative to the node's minimum corner,
    /// then u16 intensity (padded to 4 bytes), then RGB8. Removed points are left out.
    pub fn serve(&self, i: usize, removed: Option<&RoaringBitmap>) -> Result<Vec<u8>> {
        let pts = self.read(i)?;
        let min = self.nodes[i].min;
        let kept = Self::kept(&pts, removed);
        let n = kept.len();
        let mut out = Vec::with_capacity(SERVE_HEADER + n * 17 + 4);
        let flags = self.meta.has_color as u32 | (self.meta.has_intensity as u32) << 1;
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        for &k in &kept {
            for (v, m) in pts.xyz[k].iter().zip(min) {
                out.extend_from_slice(&((v - m) as f32).to_le_bytes());
            }
        }
        if self.meta.has_intensity {
            for &k in &kept {
                out.extend_from_slice(&pts.intensity[k].to_le_bytes());
            }
            out.resize(out.len().next_multiple_of(4), 0);
        }
        if self.meta.has_color {
            for &k in &kept {
                out.extend_from_slice(&pts.rgb[k]);
            }
        }
        Ok(out)
    }

    /// The stored point behind position `k` of what `serve` returned with the same
    /// `removed` set: its source record number and exact f64 scan-local position.
    pub fn resolve(
        &self,
        i: usize,
        k: usize,
        removed: Option<&RoaringBitmap>,
    ) -> Result<(u32, [f64; 3])> {
        let pts = self.read(i)?;
        let s = *Self::kept(&pts, removed)
            .get(k)
            .ok_or_else(|| Error::Invalid(format!("node {i} has no served point {k}")))?;
        Ok((pts.index[s], pts.xyz[s]))
    }

    /// Visit every stored point inside the axis-aligned box `[lo, hi]` (scan-local meters).
    pub fn query(
        &self,
        lo: [f64; 3],
        hi: [f64; 3],
        f: &mut dyn FnMut(&NodePoints, usize),
    ) -> Result<()> {
        for (i, n) in self.nodes.iter().enumerate() {
            let max = n.max();
            if (0..3).any(|d| max[d] < lo[d] || n.min[d] > hi[d]) {
                continue;
            }
            let pts = self.read(i)?;
            for (k, p) in pts.xyz.iter().enumerate() {
                if (0..3).all(|d| p[d] >= lo[d] && p[d] <= hi[d]) {
                    f(&pts, k);
                }
            }
        }
        Ok(())
    }
}
