//! Surface normals from local principal component analysis.

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
