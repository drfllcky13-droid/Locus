//! Neighbourhoods: kd-tree queries, surface normals (local PCA) and voxel downsampling.

use kiddo::{ImmutableKdTree, SquaredEuclidean};
use nalgebra::{Matrix3, SymmetricEigen, Vector3};
use rayon::prelude::*;

pub type Tree = ImmutableKdTree<f64, 3>;

pub fn tree(points: &[[f64; 3]]) -> Tree {
    ImmutableKdTree::new_from_slice(points).expect("kd-tree from points")
}

/// Indices of the `k` nearest points to `p` (including `p` itself if it is in the tree).
pub fn knn(tree: &Tree, p: &[f64; 3], k: usize) -> Vec<usize> {
    let k = std::num::NonZero::new(k.max(1)).expect("k > 0");
    tree.query(p)
        .nearest_n::<SquaredEuclidean<f64>>(k)
        .execute()
        .iter()
        .map(|r| r.item as usize)
        .collect()
}

/// Index of the point nearest `p`, and its squared distance.
pub fn nearest(tree: &Tree, p: &[f64; 3]) -> (usize, f64) {
    let n = tree
        .query(p)
        .nearest_one::<SquaredEuclidean<f64>>()
        .execute();
    (n.item as usize, n.distance)
}

/// Indices of all points within `radius` of `p`.
pub fn within(tree: &Tree, p: &[f64; 3], radius: f64) -> Vec<usize> {
    tree.query(p)
        .within::<SquaredEuclidean<f64>>(radius * radius)
        .execute()
        .iter()
        .map(|r| r.item as usize)
        .collect()
}

/// Unit normal of each point from its `k` nearest neighbours (smallest principal axis),
/// oriented toward `viewpoint` (the scanner). `None` where the neighbourhood is degenerate.
pub fn estimate(
    points: &[[f64; 3]],
    tree: &Tree,
    k: usize,
    viewpoint: [f64; 3],
) -> Vec<Option<[f64; 3]>> {
    points
        .par_iter()
        .map(|p| {
            let nn = knn(tree, p, k.max(3));
            if nn.len() < 3 {
                return None;
            }
            let c = nn
                .iter()
                .fold(Vector3::zeros(), |s, &i| s + Vector3::from(points[i]))
                / nn.len() as f64;
            let cov = nn.iter().fold(Matrix3::zeros(), |s, &i| {
                let d = Vector3::from(points[i]) - c;
                s + d * d.transpose()
            });
            let e = SymmetricEigen::new(cov);
            let i = e.eigenvalues.imin();
            let mut n: Vector3<f64> = e.eigenvectors.column(i).into();
            if !n.iter().all(|v| v.is_finite()) || n.norm() < 0.5 {
                return None;
            }
            if n.dot(&(Vector3::from(viewpoint) - Vector3::from(*p))) < 0.0 {
                n = -n;
            }
            Some(n.normalize().into())
        })
        .collect()
}

/// One point per occupied voxel of edge `size`: the real point nearest the voxel's centroid,
/// so the result stays on the scanned surfaces. Deterministic.
pub fn voxel_downsample(points: &[[f64; 3]], size: f64) -> Vec<[f64; 3]> {
    let mut cells: std::collections::HashMap<[i64; 3], (Vector3<f64>, u32, Vec<usize>)> =
        std::collections::HashMap::new();
    for (i, p) in points.iter().enumerate() {
        let e = cells
            .entry(p.map(|v| (v / size).floor() as i64))
            .or_default();
        e.0 += Vector3::from(*p);
        e.1 += 1;
        e.2.push(i);
    }
    let mut keys: Vec<_> = cells.keys().copied().collect();
    keys.sort_unstable();
    keys.iter()
        .map(|k| {
            let (sum, n, ix) = &cells[k];
            let c = sum / *n as f64;
            let best = ix
                .iter()
                .min_by(|a, b| {
                    (Vector3::from(points[**a]) - c)
                        .norm_squared()
                        .total_cmp(&(Vector3::from(points[**b]) - c).norm_squared())
                })
                .expect("non-empty cell");
            points[*best]
        })
        .collect()
}
