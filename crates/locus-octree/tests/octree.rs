use locus_octree::{build, BuildOptions, Octree, Rec, SERVE_HEADER};
use proptest::prelude::*;
use roaring::RoaringBitmap;
use std::path::Path;

fn cloud(n: u32, seed: u64) -> Vec<Rec> {
    let mut s = seed | 1;
    let mut u = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    // A floor and a wall with a dense cluster: uneven density, like a scan.
    (0..n)
        .map(|i| {
            let p = match i % 4 {
                0 => [u() * 20.0, 0.0, u() * 3.0],
                1 => [5.0 + u() * 0.3, 5.0 + u() * 0.3, u() * 0.3],
                _ => [u() * 20.0, u() * 20.0, 0.0],
            };
            Rec {
                p: [p[0] + 1000.0, p[1] - 500.0, p[2] + 10.0],
                index: i,
                rgb: [i as u8, 7, 9],
                intensity: i as u16,
            }
        })
        .collect()
}

fn small_opts() -> BuildOptions {
    BuildOptions {
        leaf_max: 2_000,
        grid: 32,
        chunk_max: 20_000,
        has_color: true,
        has_intensity: true,
    }
}

fn build_from(dir: &Path, pts: &[Rec], opts: &BuildOptions) -> Octree {
    build(
        dir,
        [999.0, -501.0, 9.0],
        32.0,
        opts,
        &mut |f| {
            pts.iter().for_each(|p| f(*p));
            Ok(())
        },
        &mut |_| {},
    )
    .unwrap();
    Octree::open(dir).unwrap()
}

#[test]
fn every_point_is_stored_exactly_once_and_inside_its_node() {
    let dir = tempfile::tempdir().unwrap();
    let pts = cloud(150_000, 42);
    let tree = build_from(dir.path(), &pts, &small_opts());
    assert_eq!(tree.meta.points, 150_000);
    assert!(
        tree.nodes.len() > 20,
        "chunking and splitting happened: {} nodes",
        tree.nodes.len()
    );

    let mut seen = vec![false; pts.len()];
    for (i, n) in tree.nodes.iter().enumerate() {
        let np = tree.read(i).unwrap();
        assert_eq!(np.xyz.len(), n.count as usize);
        for (k, &idx) in np.index.iter().enumerate() {
            assert!(!seen[idx as usize], "point {idx} stored twice");
            seen[idx as usize] = true;
            let src = pts[idx as usize];
            assert_eq!(np.xyz[k], src.p, "stored f64 is the input, bit for bit");
            assert_eq!(np.rgb[k], src.rgb);
            assert_eq!(np.intensity[k], src.intensity);
            let (lo, hi) = (n.min, n.max());
            assert!(
                (0..3).all(|d| np.xyz[k][d] >= lo[d] - 1e-9 && np.xyz[k][d] <= hi[d] + 1e-9),
                "{} outside node {}",
                idx,
                n.name
            );
        }
        if n.children != 0 {
            assert!(n.count as usize <= 32 * 32 * 32);
        } else {
            assert!(
                n.count as usize <= 2_000 || n.level() > 20,
                "leaf {} holds {}",
                n.name,
                n.count
            );
        }
    }
    assert!(seen.iter().all(|&s| s), "every point stored");
    assert_eq!(tree.nodes[tree.node("r").unwrap()].subtree, 150_000);
}

#[test]
fn build_is_deterministic() {
    let pts = cloud(40_000, 7);
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    build_from(a.path(), &pts, &small_opts());
    build_from(b.path(), &pts, &small_opts());
    for f in ["nodes.bin", "hierarchy.json", "meta.json"] {
        assert_eq!(
            std::fs::read(a.path().join(f)).unwrap(),
            std::fs::read(b.path().join(f)).unwrap(),
            "{f}"
        );
    }
}

#[test]
fn coincident_points_terminate() {
    let dir = tempfile::tempdir().unwrap();
    let pts: Vec<Rec> = (0..30_000)
        .map(|i| Rec {
            p: [1000.5, -500.5, 10.5],
            index: i,
            rgb: [0; 3],
            intensity: 0,
        })
        .collect();
    let tree = build_from(dir.path(), &pts, &small_opts());
    let total: u64 = tree.nodes.iter().map(|n| n.count as u64).sum();
    assert_eq!(total, 30_000);
}

#[test]
fn query_returns_exactly_the_points_in_the_box() {
    let dir = tempfile::tempdir().unwrap();
    let pts = cloud(60_000, 3);
    let tree = build_from(dir.path(), &pts, &small_opts());
    let (lo, hi) = ([1004.0, -496.0, 9.9], [1006.0, -494.0, 10.2]);
    let mut got = vec![];
    tree.query(lo, hi, &mut |np, k| got.push(np.index[k]))
        .unwrap();
    got.sort();
    let want: Vec<u32> = pts
        .iter()
        .filter(|r| (0..3).all(|d| r.p[d] >= lo[d] && r.p[d] <= hi[d]))
        .map(|r| r.index)
        .collect();
    assert!(!want.is_empty());
    assert_eq!(got, want);
}

#[test]
fn served_nodes_resolve_back_to_exact_points() {
    let dir = tempfile::tempdir().unwrap();
    let pts = cloud(30_000, 11);
    let tree = build_from(dir.path(), &pts, &small_opts());
    let i = tree.node("r").unwrap();
    let np = tree.read(i).unwrap();
    let mut removed = RoaringBitmap::new();
    removed.insert(np.index[0]);
    removed.insert(np.index[5]);

    let served = tree.serve(i, Some(&removed)).unwrap();
    let n = u32::from_le_bytes(served[0..4].try_into().unwrap()) as usize;
    assert_eq!(n, np.xyz.len() - 2);
    assert_eq!(u32::from_le_bytes(served[4..8].try_into().unwrap()), 0b11);
    let f = |k: usize, d: usize| {
        let at = SERVE_HEADER + (k * 3 + d) * 4;
        f32::from_le_bytes(served[at..at + 4].try_into().unwrap()) as f64
    };
    let min = tree.nodes[i].min;
    for k in [0, 3, n - 1] {
        let (idx, exact) = tree.resolve(i, k, Some(&removed)).unwrap();
        assert!(!removed.contains(idx));
        assert_eq!(exact, pts[idx as usize].p);
        for d in 0..3 {
            // The GPU copy is close (it only draws); the resolved copy is exact.
            assert!((f(k, d) + min[d] - exact[d]).abs() < 1e-5);
        }
    }
    // Colours follow the (padded) intensity block.
    let colors_at = (SERVE_HEADER + n * 12 + n * 2).next_multiple_of(4);
    let (idx, _) = tree.resolve(i, 1, Some(&removed)).unwrap();
    assert_eq!(served[colors_at + 3..colors_at + 6], pts[idx as usize].rgb);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    #[test]
    fn any_cloud_is_partitioned(n in 1u32..8_000, seed in any::<u64>(), leaf in 50usize..500) {
        let dir = tempfile::tempdir().unwrap();
        let pts = cloud(n, seed);
        let opts = BuildOptions { leaf_max: leaf, grid: 16, chunk_max: 1_500, has_color: false, has_intensity: false };
        let tree = build_from(dir.path(), &pts, &opts);
        let mut idx: Vec<u32> = (0..tree.nodes.len()).flat_map(|i| tree.read(i).unwrap().index).collect();
        idx.sort();
        prop_assert_eq!(idx, (0..n).collect::<Vec<_>>());
    }
}
