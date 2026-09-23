//! Cleanup operations. Each returns, per scan, the set of source record numbers it removes.
//! Nothing here writes: the caller stores the sets as derived bitmaps and records the
//! operation, so undo is a matter of switching it off. Points already removed by other
//! active operations are ignored when an operation decides what to remove.
//!
//! Outlier removal and voxel downsampling work tile by tile: the scan's cube is split into
//! a uniform grid fine enough that no tile holds more than `TILE_MAX` points, each point
//! belongs to exactly one tile, and each tile also reads a margin around it for neighbours.

use crate::scene::{apply, invert_rigid, ScanCloud, ScanKey, Scene};
use crate::{NodePoints, Result};
use kiddo::{ImmutableKdTree, SquaredEuclidean};
use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::num::NonZero;

pub type Removal = Vec<(ScanKey, RoaringBitmap)>;

const TILE_MAX: u64 = 3_000_000;

fn inside(p: &[f64; 3], lo: &[f64; 3], hi: &[f64; 3]) -> bool {
    (0..3).all(|d| p[d] >= lo[d] && p[d] <= hi[d])
}

/// Visit every visible point of a scan whose node could intersect the project-frame box.
pub(crate) fn for_points_near(
    c: &ScanCloud,
    lo: [f64; 3],
    hi: [f64; 3],
    f: &mut dyn FnMut(&NodePoints, usize, [f64; 3]),
) -> Result<()> {
    // Box corners into scan-local coordinates, then their bounding box.
    let inv = invert_rigid(&c.pose);
    let mut llo = [f64::INFINITY; 3];
    let mut lhi = [f64::NEG_INFINITY; 3];
    for k in 0..8 {
        let corner = [
            if k & 1 == 0 { lo[0] } else { hi[0] },
            if k & 2 == 0 { lo[1] } else { hi[1] },
            if k & 4 == 0 { lo[2] } else { hi[2] },
        ];
        let q = apply(&inv, corner);
        for d in 0..3 {
            llo[d] = llo[d].min(q[d]);
            lhi[d] = lhi[d].max(q[d]);
        }
    }
    for i in c.tree.nodes_in(llo, lhi) {
        let pts = c.tree.read(i)?;
        for k in 0..pts.xyz.len() {
            if !c.removed.contains(pts.index[k]) {
                f(&pts, k, apply(&c.pose, pts.xyz[k]));
            }
        }
    }
    Ok(())
}

/// Remove every point inside an axis-aligned box in the project frame.
pub fn box_delete(scene: &Scene, lo: [f64; 3], hi: [f64; 3]) -> Result<Removal> {
    let mut out = vec![];
    for c in scene.scans.values() {
        let mut bm = RoaringBitmap::new();
        for_points_near(c, lo, hi, &mut |pts, k, p| {
            if inside(&p, &lo, &hi) {
                bm.insert(pts.index[k]);
            }
        })?;
        if !bm.is_empty() {
            out.push((c.key, bm));
        }
    }
    Ok(out)
}

/// Even-odd point-in-polygon test.
fn in_polygon(x: f64, y: f64, poly: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Normalized device coordinates and view depth (clip w, meters) of a project-frame point,
/// or None behind the camera. `view_proj` is column-major (as three.js stores it) and
/// relative to `origin`.
fn ndc(view_proj: &[f64; 16], origin: &[f64; 3], p: [f64; 3]) -> Option<[f64; 3]> {
    let r = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
    let m = |row: usize| {
        view_proj[row] * r[0]
            + view_proj[4 + row] * r[1]
            + view_proj[8 + row] * r[2]
            + view_proj[12 + row]
    };
    let w = m(3);
    (w > 1e-12).then(|| [m(0) / w, m(1) / w, w])
}

/// Which points a lasso removes along each line of sight.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LassoDepth {
    /// Every point whose projection falls inside the lasso, however deep.
    AllDepths,
    /// Only the surface facing the camera. The view is split into cells of `cell_px`
    /// pixels (of a `viewport` of that size); in each cell a point is removed when its
    /// view depth is within `tolerance_m + tolerance_rel × depth` of the nearest point in
    /// that cell. Points further back are kept.
    VisibleSurface {
        viewport: [u32; 2],
        cell_px: u32,
        tolerance_m: f64,
        tolerance_rel: f64,
    },
}

/// What the view was clipped to when the lasso was drawn, in the project frame. Points
/// clipped out of view are never lassoed.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Clip {
    /// Axis-aligned box and whether the view showed its inside (true) or outside.
    pub clip_box: Option<([f64; 3], [f64; 3], bool)>,
    /// Keep the side where `p[axis] * sign >= offset * sign`.
    pub plane: Option<(usize, f64, bool)>,
}

impl Clip {
    fn shows(&self, p: &[f64; 3]) -> bool {
        if let Some((lo, hi, show_inside)) = self.clip_box {
            if inside(p, &lo, &hi) != show_inside {
                return false;
            }
        }
        if let Some((axis, offset, flip)) = self.plane {
            let sign = if flip { -1.0 } else { 1.0 };
            if (p[axis] - offset) * sign < 0.0 {
                return false;
            }
        }
        true
    }
}

/// Nearest view depth per screen cell, for the visible-surface lasso.
struct Surface {
    viewport: [u32; 2],
    cell: f64,
    tolerance_m: f64,
    tolerance_rel: f64,
    front: HashMap<(i64, i64), f64>,
}

impl Surface {
    fn cell_of(&self, q: &[f64; 3]) -> (i64, i64) {
        let x = (q[0] + 1.0) / 2.0 * self.viewport[0] as f64;
        let y = (1.0 - q[1]) / 2.0 * self.viewport[1] as f64;
        (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
        )
    }

    /// Is this point on the front surface of its cell?
    fn keeps(&self, q: &[f64; 3]) -> bool {
        let front = self.front[&self.cell_of(q)];
        q[2] <= front + self.tolerance_m + self.tolerance_rel * front
    }
}

/// Remove the points inside a screen-space polygon (normalized device coordinates):
/// every depth, or only the visible surface. See docs/methods/cleanup.md.
pub fn lasso_delete(
    scene: &Scene,
    view_proj: &[f64; 16],
    origin: &[f64; 3],
    polygon: &[[f64; 2]],
    depth: LassoDepth,
    clip: &Clip,
) -> Result<Removal> {
    if polygon.len() < 3 {
        return Ok(vec![]);
    }
    let (pl, ph) =
        polygon
            .iter()
            .fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(l, h), p| {
                (
                    [l[0].min(p[0]), l[1].min(p[1])],
                    [h[0].max(p[0]), h[1].max(p[1])],
                )
            });
    let mut surface = match depth {
        LassoDepth::AllDepths => None,
        LassoDepth::VisibleSurface {
            viewport,
            cell_px,
            tolerance_m,
            tolerance_rel,
        } => Some(Surface {
            viewport,
            cell: cell_px.max(1) as f64,
            tolerance_m,
            tolerance_rel,
            front: HashMap::new(),
        }),
    };
    // The front surface is judged from every visible point in the cells the lasso touches,
    // not only those inside it, so points just outside still hide what is behind them.
    let margin = surface.as_ref().map_or([0.0; 2], |sf| {
        [
            2.0 * sf.cell / sf.viewport[0] as f64,
            2.0 * sf.cell / sf.viewport[1] as f64,
        ]
    });
    let (pl, ph) = (
        [pl[0] - margin[0], pl[1] - margin[1]],
        [ph[0] + margin[0], ph[1] + margin[1]],
    );
    let near_lasso = |q: &[f64; 3]| (0..2).all(|d| q[d] >= pl[d] && q[d] <= ph[d]);
    // Every visible point inside the lasso: scan, record number, screen position and depth.
    let mut candidates: Vec<(ScanKey, u32, [f64; 3])> = vec![];
    for c in scene.scans.values() {
        for (i, n) in c.tree.nodes.iter().enumerate() {
            // Skip nodes whose projected corners all lie off to one side of the polygon.
            let corners: Vec<Option<[f64; 3]>> = (0..8)
                .map(|k| {
                    let s = n.size;
                    let q = [
                        n.min[0] + s * (k & 1) as f64,
                        n.min[1] + s * ((k >> 1) & 1) as f64,
                        n.min[2] + s * ((k >> 2) & 1) as f64,
                    ];
                    ndc(view_proj, origin, apply(&c.pose, q))
                })
                .collect();
            if corners.iter().all(|q| q.is_some()) {
                let qs: Vec<[f64; 3]> = corners.into_iter().flatten().collect();
                let off = (0..2)
                    .any(|d| qs.iter().all(|q| q[d] < pl[d]) || qs.iter().all(|q| q[d] > ph[d]));
                if off {
                    continue;
                }
            }
            let pts = c.tree.read(i)?;
            for k in 0..pts.xyz.len() {
                if c.removed.contains(pts.index[k]) {
                    continue;
                }
                let p = apply(&c.pose, pts.xyz[k]);
                if !clip.shows(&p) {
                    continue;
                }
                if let Some(q) = ndc(view_proj, origin, p) {
                    if let Some(sf) = surface.as_mut().filter(|_| near_lasso(&q)) {
                        let cell = sf.cell_of(&q);
                        let e = sf.front.entry(cell).or_insert(f64::INFINITY);
                        *e = e.min(q[2]);
                    }
                    if in_polygon(q[0], q[1], polygon) {
                        candidates.push((c.key, pts.index[k], q));
                    }
                }
            }
        }
    }
    let mut sets: BTreeMap<ScanKey, RoaringBitmap> = BTreeMap::new();
    for (scan, index, q) in &candidates {
        if surface.as_ref().is_none_or(|sf| sf.keeps(q)) {
            sets.entry(*scan).or_default().insert(*index);
        }
    }
    Ok(sets.into_iter().collect())
}

/// Uniform tile grid over a scan's cube: 2^level cells per edge, the coarsest level at
/// which no octree node holds a subtree larger than TILE_MAX.
struct Tiles {
    min: [f64; 3],
    size: f64,
    level: u32,
}

impl Tiles {
    fn new(c: &ScanCloud) -> Tiles {
        let level = (0..=20)
            .find(|&l| {
                c.tree
                    .nodes
                    .iter()
                    .filter(|n| n.level() == l)
                    .all(|n| n.subtree <= TILE_MAX)
            })
            .unwrap_or(20);
        Tiles {
            min: c.tree.meta.min,
            size: c.tree.meta.size / (1u64 << level) as f64,
            level: level as u32,
        }
    }

    fn of(&self, p: &[f64; 3]) -> [i64; 3] {
        let n = 1i64 << self.level;
        std::array::from_fn(|d| (((p[d] - self.min[d]) / self.size).floor() as i64).clamp(0, n - 1))
    }

    fn bounds(&self, t: [i64; 3], margin: f64) -> ([f64; 3], [f64; 3]) {
        let lo = std::array::from_fn(|d| self.min[d] + t[d] as f64 * self.size - margin);
        let hi = std::array::from_fn(|d| self.min[d] + (t[d] + 1) as f64 * self.size + margin);
        (lo, hi)
    }

    /// Every tile that holds at least one point: the tiles of all nodes at or below the
    /// tile level, plus the tiles of the points stored in the nodes above it.
    fn occupied(&self, c: &ScanCloud) -> Result<Vec<[i64; 3]>> {
        let mut set = std::collections::BTreeSet::new();
        for (i, n) in c.tree.nodes.iter().enumerate() {
            if n.level() as u32 >= self.level {
                let centre = n.min.map(|v| v + n.size / 2.0);
                set.insert(self.of(&centre));
            } else {
                for p in c.tree.read(i)?.xyz {
                    set.insert(self.of(&p));
                }
            }
        }
        Ok(set.into_iter().collect())
    }
}

/// Visible points of one tile plus its margin, in scan-local coordinates.
struct TilePoints {
    local: Vec<[f64; 3]>,
    index: Vec<u32>,
    owned: Vec<bool>,
}

fn tile_points(
    c: &ScanCloud,
    tiles: &Tiles,
    t: [i64; 3],
    margin: f64,
    region: Option<([f64; 3], [f64; 3])>,
    owner: &dyn Fn(&[f64; 3]) -> [i64; 3],
) -> Result<TilePoints> {
    let (lo, hi) = tiles.bounds(t, margin);
    let mut tp = TilePoints {
        local: vec![],
        index: vec![],
        owned: vec![],
    };
    c.tree.query(lo, hi, &mut |pts, k| {
        if c.removed.contains(pts.index[k]) {
            return;
        }
        let p = pts.xyz[k];
        let in_region = region.is_none_or(|(rl, rh)| inside(&apply(&c.pose, p), &rl, &rh));
        tp.local.push(p);
        tp.index.push(pts.index[k]);
        tp.owned.push(in_region && owner(&p) == t);
    })?;
    Ok(tp)
}

/// Statistical outlier removal: for each point, the mean distance to its `k` nearest
/// neighbours; remove points whose mean exceeds the scan-wide mean by `std_mult` standard
/// deviations. Only points inside `region` (project frame) are judged and counted.
pub fn outliers(
    scene: &Scene,
    k: usize,
    std_mult: f64,
    region: Option<([f64; 3], [f64; 3])>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Removal> {
    let mut out = vec![];
    let kk = NonZero::new(k + 1).expect("k + 1 > 0");
    for c in scene.scans.values() {
        let tiles = Tiles::new(c);
        // ponytail: margin is 5% of a tile edge; points whose k neighbours reach further
        // than that at tile borders get a slightly high mean distance. Grow the margin if
        // tests on real scans show edge effects.
        let margin = tiles.size * 0.05;
        let occupied = tiles.occupied(c)?;
        let owner = |p: &[f64; 3]| tiles.of(p);
        let mean_knn = |tp: &TilePoints, f: &mut dyn FnMut(usize, f64)| {
            if tp.local.len() <= k {
                return;
            }
            let tree = ImmutableKdTree::<f64, 3>::new_from_slice(&tp.local).expect("tile points");
            for (i, p) in tp.local.iter().enumerate() {
                if !tp.owned[i] {
                    continue;
                }
                let nn = tree
                    .query(p)
                    .nearest_n::<SquaredEuclidean<f64>>(kk)
                    .execute();
                let sum: f64 = nn.iter().skip(1).map(|r| r.distance.sqrt()).sum();
                f(i, sum / k as f64);
            }
        };
        // Pass 1: scan-wide mean and standard deviation (Welford).
        let (mut n, mut mean, mut m2) = (0u64, 0.0, 0.0);
        for (ti, &t) in occupied.iter().enumerate() {
            let tp = tile_points(c, &tiles, t, margin, region, &owner)?;
            mean_knn(&tp, &mut |_, d| {
                n += 1;
                let delta = d - mean;
                mean += delta / n as f64;
                m2 += delta * (d - mean);
            });
            progress(ti as u64 + 1, 2 * occupied.len() as u64);
        }
        if n < 2 {
            continue;
        }
        let threshold = mean + std_mult * (m2 / (n - 1) as f64).sqrt();
        // Pass 2: flag points over the threshold.
        let mut bm = RoaringBitmap::new();
        for (ti, &t) in occupied.iter().enumerate() {
            let tp = tile_points(c, &tiles, t, margin, region, &owner)?;
            mean_knn(&tp, &mut |i, d| {
                if d > threshold {
                    bm.insert(tp.index[i]);
                }
            });
            progress((occupied.len() + ti) as u64 + 1, 2 * occupied.len() as u64);
        }
        if !bm.is_empty() {
            out.push((c.key, bm));
        }
    }
    Ok(out)
}

/// Keep one point per voxel (the one nearest the voxel centre; ties go to the lower record
/// number) and remove the rest. Voxels are `size` meters in scan-local coordinates,
/// anchored at the scan's origin.
pub fn voxel_downsample(
    scene: &Scene,
    size: f64,
    region: Option<([f64; 3], [f64; 3])>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Removal> {
    let mut out = vec![];
    let voxel =
        |p: &[f64; 3]| -> [i64; 3] { std::array::from_fn(|d| (p[d] / size).floor() as i64) };
    for c in scene.scans.values() {
        let tiles = Tiles::new(c);
        // A voxel belongs to the tile holding its minimum corner, so it is decided once.
        let owner = |p: &[f64; 3]| {
            let v = voxel(p);
            tiles.of(&v.map(|i| i as f64 * size))
        };
        let occupied = tiles.occupied(c)?;
        let mut bm = RoaringBitmap::new();
        for (ti, &t) in occupied.iter().enumerate() {
            let tp = tile_points(c, &tiles, t, size, region, &owner)?;
            let mut best: HashMap<[i64; 3], (f64, u32)> = HashMap::new();
            for (i, p) in tp.local.iter().enumerate() {
                if !tp.owned[i] {
                    continue;
                }
                let v = voxel(p);
                let d: f64 = (0..3)
                    .map(|a| (p[a] - (v[a] as f64 + 0.5) * size).powi(2))
                    .sum();
                let e = best.entry(v).or_insert((f64::INFINITY, u32::MAX));
                if d < e.0 || (d == e.0 && tp.index[i] < e.1) {
                    *e = (d, tp.index[i]);
                }
            }
            for (i, p) in tp.local.iter().enumerate() {
                if tp.owned[i] && best[&voxel(p)].1 != tp.index[i] {
                    bm.insert(tp.index[i]);
                }
            }
            progress(ti as u64 + 1, occupied.len() as u64);
        }
        if !bm.is_empty() {
            out.push((c.key, bm));
        }
    }
    Ok(out)
}

/// Total points per scan, for reporting.
pub fn counts(removal: &Removal) -> BTreeMap<String, u64> {
    removal
        .iter()
        .map(|(k, bm)| (k.to_string(), bm.len()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_membership() {
        let square = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
        assert!(in_polygon(0.0, 0.0, &square));
        assert!(!in_polygon(0.6, 0.0, &square));
        let l = [
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 2.0],
            [0.0, 2.0],
        ];
        assert!(in_polygon(0.5, 1.5, &l));
        assert!(!in_polygon(1.5, 1.5, &l), "the notch of the L is outside");
    }

    #[test]
    fn projection_uses_column_major_and_origin() {
        // Identity view-projection: ndc = position relative to origin; w = 1.
        let mut vp = [0.0; 16];
        for i in 0..4 {
            vp[i * 5] = 1.0;
        }
        assert_eq!(
            ndc(&vp, &[10.0, 20.0, 0.0], [10.25, 19.5, 3.0]),
            Some([0.25, -0.5, 1.0])
        );
    }
}
