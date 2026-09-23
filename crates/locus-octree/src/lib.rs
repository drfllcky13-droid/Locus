//! Out-of-core octree builder and chunk server for point clouds.
//!
//! One octree per scan, in scan-local coordinates in meters (the scan's pose is applied by
//! the caller, so registration never needs a rebuild). Each point is stored in exactly one
//! node: an inner node keeps one point per cell of a 128³ grid over its cube, the rest go
//! to its children, and a node with few enough points keeps them all. The union of the
//! nodes along a path from the root to a leaf is therefore the full-resolution cloud there.
//!
//! On disk (`<dir>/`): `meta.json`, `hierarchy.json`, and `nodes.bin`, where each node is
//! stored column by column: f64 xyz, u32 source index, then RGB8 and u16 intensity when the
//! scan has them. Positions stay f64 so picks and measurements never depend on GPU floats.

mod build;
pub mod cleanup;
mod read;
pub mod scene;
pub mod slice;

pub use build::{build, BuildOptions, BuildProgress, BuildStage, Source};
pub use read::{NodePoints, Octree, SERVE_HEADER};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("octree: {0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One input point: scan-local position in meters and its record number in the source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rec {
    pub p: [f64; 3],
    pub index: u32,
    pub rgb: [u8; 3],
    pub intensity: u16,
}

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    /// Cube that contains every point: minimum corner and edge length, meters.
    pub min: [f64; 3],
    pub size: f64,
    pub points: u64,
    pub nodes: usize,
    pub has_color: bool,
    pub has_intensity: bool,
    pub grid: u32,
    pub leaf_max: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// "r" followed by one octant digit per level.
    pub name: String,
    pub min: [f64; 3],
    pub size: f64,
    /// Points stored in this node.
    pub count: u32,
    /// Points in this node and all its descendants.
    pub subtree: u64,
    /// Grid cell size: the typical distance between this node's points, meters.
    pub spacing: f64,
    pub offset: u64,
    pub bytes: u64,
    /// Bit `i` set when child octant `i` exists.
    pub children: u8,
}

impl Node {
    pub fn level(&self) -> usize {
        self.name.len() - 1
    }

    pub fn max(&self) -> [f64; 3] {
        self.min.map(|v| v + self.size)
    }
}

/// Octant of `p` within the cube: bit 2 = x, bit 1 = y, bit 0 = z in the upper half.
pub(crate) fn octant(p: &[f64; 3], min: &[f64; 3], size: f64) -> usize {
    let h = size / 2.0;
    (((p[0] >= min[0] + h) as usize) << 2)
        | (((p[1] >= min[1] + h) as usize) << 1)
        | ((p[2] >= min[2] + h) as usize)
}

pub(crate) fn child_min(min: &[f64; 3], size: f64, octant: usize) -> [f64; 3] {
    let h = size / 2.0;
    [
        min[0] + h * ((octant >> 2) & 1) as f64,
        min[1] + h * ((octant >> 1) & 1) as f64,
        min[2] + h * (octant & 1) as f64,
    ]
}

/// Cell of `p` in a `grid`³ lattice over the cube, clamped for points on the far faces.
pub(crate) fn cell(p: &[f64; 3], min: &[f64; 3], size: f64, grid: u32) -> u32 {
    let g = grid as f64;
    let c = |i: usize| (((p[i] - min[i]) / size * g) as i64).clamp(0, grid as i64 - 1) as u32;
    (c(0) * grid + c(1)) * grid + c(2)
}
