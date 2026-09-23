use locus_core::{CleanupScan, Project, DEFAULT_POINT_SIGMA_M};
use rusqlite::Connection;
use serde_json::json;

fn last_action(p: &Project) -> (String, String) {
    let e = p.audit_log().unwrap().pop().unwrap();
    (e.action, e.details)
}

#[test]
fn schema_1_projects_are_migrated_step_by_step_and_each_step_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("old.locus");
    drop(Project::create(&root, "Old", "A").unwrap());
    // Turn it back into a schema-1 project, as Phase 1 wrote them.
    let c = Connection::open(root.join("project.sqlite")).unwrap();
    c.execute_batch(
        "DROP TABLE settings; DROP TABLE octrees; DROP TABLE measurements; DROP TABLE cleanup_ops;
         DROP TABLE registration_poses; DROP TABLE registrations;
         DROP TABLE diagram_revisions; DROP TABLE diagrams;
         DROP TABLE scene_revisions; DROP TABLE scenes;
         UPDATE meta SET value = '1' WHERE key = 'schema_version';",
    )
    .unwrap();
    drop(c);

    let p = Project::open(&root, "B").unwrap();
    let actions: Vec<_> = p
        .audit_log()
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    assert_eq!(
        actions,
        [
            "project.created",
            "project.migrated",
            "project.migrated",
            "project.migrated",
            "project.migrated",
            "project.opened"
        ]
    );
    assert!(p.measurements().unwrap().is_empty());
    drop(p);
    drop(Project::open(&root, "C").unwrap()); // opens cleanly afterwards
}

#[test]
fn settings_measurements_and_cleanups_are_audit_logged() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(&dir.path().join("c.locus"), "C", "A").unwrap();

    assert_eq!(p.point_sigma().unwrap(), DEFAULT_POINT_SIGMA_M);
    p.set_setting("point_sigma_m", "0.003").unwrap();
    assert_eq!(p.point_sigma().unwrap(), 0.003);
    let (action, details) = last_action(&p);
    assert_eq!(action, "setting.changed");
    assert!(details.contains("\"old\":null") && details.contains("0.003"));

    let id = p
        .add_measurement(
            "distance",
            json!([{ "scan": "1-0", "index": 7 }]),
            json!({ "value": 1.5, "sigma": 0.003 }),
        )
        .unwrap();
    assert_eq!(last_action(&p).0, "measurement.created");
    assert_eq!(p.measurements().unwrap()[0].result["value"], 1.5);
    p.delete_measurement(id).unwrap();
    assert!(p.measurements().unwrap().is_empty());
    assert_eq!(last_action(&p).0, "measurement.deleted");
    assert!(p.delete_measurement(id).is_err(), "already deleted");

    let scan = CleanupScan {
        evidence_id: 1,
        scan_idx: 0,
        removed: 12,
        file: "derived/cleanup/1-1-0.roar".into(),
        sha256: "ab".into(),
    };
    let op = p
        .add_cleanup(
            "box_delete",
            json!({ "min": [0, 0, 0], "max": [1, 1, 1] }),
            vec![scan.clone()],
        )
        .unwrap();
    assert_eq!(last_action(&p).0, "cleanup.applied");
    p.set_cleanup_active(op, false).unwrap();
    assert_eq!(last_action(&p).0, "cleanup.undone");
    assert!(
        p.set_cleanup_active(op, false).is_err(),
        "cannot undo twice"
    );
    p.set_cleanup_active(op, true).unwrap();
    assert_eq!(last_action(&p).0, "cleanup.redone");
    let rec = &p.cleanups().unwrap()[0];
    assert!(rec.active);
    assert_eq!(rec.scans, vec![scan]);

    let src = dir.path().join("pts.xyz");
    std::fs::write(
        &src, "0 0 0
",
    )
    .unwrap();
    let sha = locus_core::hash::sha256_file(&src, &mut |_| {}).unwrap().0;
    let mut contents = locus_core::Contents::new("XYZ");
    contents.declared_unit = Some(locus_core::LinearUnit::Meter);
    let ev = p
        .import_evidence(&src, &sha, &contents, None, &mut |_| {})
        .unwrap();
    p.record_octree(ev.id, 0, "built", 1000, "").unwrap();
    assert!(
        p.record_octree(99, 0, "built", 1, "").is_err(),
        "no such evidence"
    );
    assert_eq!(last_action(&p).0, "octree.built");
    assert_eq!(p.octrees().unwrap()[0].points, 1000);

    // The whole chain still verifies.
    let root = p.root().to_path_buf();
    drop(p);
    Project::open(&root, "B").unwrap();
}

#[test]
fn measurements_and_cleanups_cannot_be_rewritten_or_removed() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(&dir.path().join("c.locus"), "C", "A").unwrap();
    p.add_measurement("distance", json!([]), json!({ "value": 1.0 }))
        .unwrap();
    p.add_cleanup("voxel", json!({}), vec![]).unwrap();
    let c = Connection::open(p.root().join("project.sqlite")).unwrap();
    assert!(c
        .execute("UPDATE measurements SET result = '{}'", [])
        .is_err());
    assert!(c.execute("DELETE FROM measurements", []).is_err());
    assert!(c
        .execute("UPDATE cleanup_ops SET params = '{}'", [])
        .is_err());
    assert!(c.execute("DELETE FROM cleanup_ops", []).is_err());
}
