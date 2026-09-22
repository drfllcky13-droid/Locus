//! The whole path: evidence file → project → octree → served node → pick → f64 point,
//! plus cleanup operations with undo.

use locus_core::hash::sha256_file;
use locus_core::{CleanupScan, LinearUnit, Project};
use locus_octree::cleanup::{box_delete, outliers, voxel_downsample};
use locus_octree::scene::{build_scan, write_bitmap, ScanKey, Scene};
use locus_octree::SERVE_HEADER;
use std::path::{Path, PathBuf};

fn import(p: &mut Project, src: &Path, unit: Option<LinearUnit>) -> i64 {
    let pv = locus_io::preview(src, &mut |_| {}).unwrap();
    let rec = locus_io::commit(p, &pv, unit, &mut |_| {}).unwrap();
    for s in 0..rec.contents.scans.len() {
        let meta = build_scan(p.root(), &rec, s, &mut |_| {}).unwrap();
        p.record_octree(rec.id, s, "built", meta.points, "")
            .unwrap();
    }
    rec.id
}

fn project(dir: &Path) -> Project {
    Project::create(&dir.join("case.locus"), "Case", "A").unwrap()
}

/// Georeferenced LAS, 0.1 mm resolution, about 500 km and 4,400 km from the origin.
fn utm_las(path: &Path) -> Vec<[f64; 3]> {
    let mut b = las::Builder::from((1, 4));
    b.transforms = las::Vector {
        x: las::Transform {
            scale: 0.0001,
            offset: 500_000.0,
        },
        y: las::Transform {
            scale: 0.0001,
            offset: 4_400_000.0,
        },
        z: las::Transform {
            scale: 0.0001,
            offset: 0.0,
        },
    };
    let mut w = las::Writer::from_path(path, b.into_header().unwrap()).unwrap();
    let mut pts = vec![];
    for i in 0..5_000 {
        let f = i as f64;
        // Exactly representable at the file's 0.1 mm resolution.
        let p = [
            500123.4567 + (f * 0.0137) % 40.0,
            4400456.789 + (f * 0.0091) % 30.0,
            312.3456 + (f * 0.0003) % 2.0,
        ];
        let p = p.map(|v| (v * 10_000.0).round() / 10_000.0);
        w.write_point(las::Point {
            x: p[0],
            y: p[1],
            z: p[2],
            ..Default::default()
        })
        .unwrap();
        pts.push(p);
    }
    w.close().unwrap();
    pts
}

/// What the GPU is given for one node: count, then f32 positions relative to the node.
fn served_count(bytes: &[u8]) -> usize {
    u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize
}

#[test]
fn picks_far_from_the_origin_resolve_to_the_source_within_a_millimetre() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("utm.las");
    let written = utm_las(&src);
    let mut p = project(dir.path());
    let e = import(&mut p, &src, Some(LinearUnit::Meter));
    let scene = Scene::load(&p).unwrap();
    let key = ScanKey {
        evidence_id: e,
        scan_idx: 0,
    };
    let cloud = &scene.scans[&key];

    let origin = scene.origin();
    assert!(
        origin[0] > 500_000.0 && origin[1] > 4_400_000.0,
        "render origin sits on the data"
    );
    let mut checked = 0;
    for node in 0..cloud.tree.nodes.len() {
        let served = scene.serve(key, node).unwrap();
        let n = served_count(&served);
        let min = cloud.tree.nodes[node].min;
        for k in (0..n).step_by(97) {
            let r = scene.resolve(key, node, k, cloud.revision).unwrap();
            let src_pt = written[r.index as usize];
            for d in 0..3 {
                // The value used for measurement: exact to far better than 1 mm.
                assert!(
                    (r.project[d] - src_pt[d]).abs() < 1e-6,
                    "axis {d}: {} vs {}",
                    r.project[d],
                    src_pt[d]
                );
                // The GPU copy is only for drawing, and is node-relative, so it stays precise too.
                let at = SERVE_HEADER + (k * 3 + d) * 4;
                let gpu =
                    f32::from_le_bytes(served[at..at + 4].try_into().unwrap()) as f64 + min[d];
                assert!((gpu - src_pt[d]).abs() < 1e-3);
            }
            checked += 1;
        }
    }
    assert!(checked > 40);
    // Without the node-relative offset, f32 would be off by centimetres at this distance.
    assert!(((500123.4567f64 as f32) as f64 - 500123.4567).abs() > 5e-3);
}

fn xyz(dir: &Path, name: &str, pts: &[[f64; 3]]) -> PathBuf {
    let p = dir.join(name);
    let text: String = pts
        .iter()
        .map(|q| format!("{} {} {}\n", q[0], q[1], q[2]))
        .collect();
    std::fs::write(&p, text).unwrap();
    p
}

fn grid(n: usize, step: f64) -> Vec<[f64; 3]> {
    (0..n * n)
        .map(|i| [(i % n) as f64 * step, (i / n) as f64 * step, 0.0])
        .collect()
}

fn visible(scene: &Scene, key: ScanKey) -> usize {
    (0..scene.scans[&key].tree.nodes.len())
        .map(|i| served_count(&scene.serve(key, i).unwrap()))
        .sum()
}

fn apply(p: &mut Project, kind: &str, removal: locus_octree::cleanup::Removal) -> i64 {
    let scans = removal
        .iter()
        .map(|(k, bm)| {
            let (file, sha256) = write_bitmap(p.root(), &format!("{kind}-{k}"), bm).unwrap();
            CleanupScan {
                evidence_id: k.evidence_id,
                scan_idx: k.scan_idx,
                removed: bm.len(),
                file,
                sha256,
            }
        })
        .collect();
    p.add_cleanup(kind, serde_json::json!({}), scans).unwrap()
}

#[test]
fn box_delete_is_undoable_and_never_touches_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = xyz(dir.path(), "floor.xyz", &grid(100, 0.1));
    let before = sha256_file(&src, &mut |_| {}).unwrap().0;
    let mut p = project(dir.path());
    let e = import(&mut p, &src, Some(LinearUnit::Meter));
    let key = ScanKey {
        evidence_id: e,
        scan_idx: 0,
    };
    let mut scene = Scene::load(&p).unwrap();
    assert_eq!(visible(&scene, key), 10_000);

    // A 1 m × 1 m box holds an 11 × 11 block of the 10 cm grid.
    let removal = box_delete(
        &scene,
        [2.0 - 1e-9, 3.0 - 1e-9, -1.0],
        [3.0 + 1e-9, 4.0 + 1e-9, 1.0],
    )
    .unwrap();
    assert_eq!(removal[0].1.len(), 121);
    let op = apply(&mut p, "box_delete", removal);
    scene.reload_removed(&p).unwrap();
    assert_eq!(visible(&scene, key), 10_000 - 121);
    assert_eq!(scene.scans[&key].revision, 1);

    p.set_cleanup_active(op, false).unwrap();
    scene.reload_removed(&p).unwrap();
    assert_eq!(visible(&scene, key), 10_000, "undo restores every point");
    p.set_cleanup_active(op, true).unwrap();
    scene.reload_removed(&p).unwrap();
    assert_eq!(visible(&scene, key), 10_000 - 121, "redo");

    let actions: Vec<String> = p
        .audit_log()
        .unwrap()
        .into_iter()
        .map(|a| a.action)
        .collect();
    assert!(actions.ends_with(&[
        "cleanup.applied".into(),
        "cleanup.undone".into(),
        "cleanup.redone".into()
    ]));
    assert_eq!(sha256_file(&src, &mut |_| {}).unwrap().0, before);
    assert!(
        p.verify_evidence(&mut |_| {}).unwrap().is_clean(),
        "stored evidence untouched too"
    );
}

#[test]
fn stale_picks_and_altered_cleanup_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let src = xyz(dir.path(), "floor.xyz", &grid(30, 0.1));
    let mut p = project(dir.path());
    let e = import(&mut p, &src, Some(LinearUnit::Meter));
    let key = ScanKey {
        evidence_id: e,
        scan_idx: 0,
    };
    let mut scene = Scene::load(&p).unwrap();
    let removal = box_delete(&scene, [-1.0; 3], [0.5, 0.5, 1.0]).unwrap();
    apply(&mut p, "box_delete", removal);
    scene.reload_removed(&p).unwrap();
    assert!(
        scene.resolve(key, 0, 0, 0).is_err(),
        "revision 0 is stale after the cleanup"
    );
    assert!(scene.resolve(key, 0, 0, 1).is_ok());

    let file = p.root().join(&p.cleanups().unwrap()[0].scans[0].file);
    let mut bytes = std::fs::read(&file).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&file, bytes).unwrap();
    let err = Scene::load(&p)
        .err()
        .expect("tampered bitmap must be refused")
        .to_string();
    assert!(err.contains("altered"), "{err}");
}

#[test]
fn outliers_are_found_and_voxels_keep_one_point_each() {
    let dir = tempfile::tempdir().unwrap();
    let mut pts = grid(60, 0.02);
    let strays = [[0.3, 0.4, 0.9], [1.0, 0.1, -0.8], [0.7, 0.7, 1.5]];
    pts.extend(strays);
    let src = xyz(dir.path(), "plane.xyz", &pts);
    let mut p = project(dir.path());
    let e = import(&mut p, &src, Some(LinearUnit::Meter));
    let key = ScanKey {
        evidence_id: e,
        scan_idx: 0,
    };
    let scene = Scene::load(&p).unwrap();

    let removal = outliers(&scene, 8, 3.0, None, &mut |_, _| {}).unwrap();
    let removed: Vec<u32> = removal[0].1.iter().collect();
    assert_eq!(
        removed,
        [3600, 3601, 3602],
        "exactly the three strays (records after the grid)"
    );
    assert_eq!(removal[0].0, key);

    // 60 × 60 points 2 cm apart, 10 cm voxels: 6 × 6... plus the partial row at 1.18 m.
    let v = voxel_downsample(&scene, 0.1, None, &mut |_, _| {}).unwrap();
    let kept = pts.len() as u64 - v[0].1.len();
    let voxels: std::collections::BTreeSet<[i64; 3]> = pts
        .iter()
        .map(|q| q.map(|c| (c / 0.1).floor() as i64))
        .collect();
    assert_eq!(kept, voxels.len() as u64);
}
