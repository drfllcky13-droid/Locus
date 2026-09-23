use locus_core::{Project, ScanPose, IDENTITY};
use rusqlite::Connection;
use serde_json::json;

fn pose(x: f64) -> [f64; 16] {
    let mut m = IDENTITY;
    m[3] = x;
    m
}

#[test]
fn a_registration_is_stored_applied_and_undone_with_every_step_logged() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(&dir.path().join("r.locus"), "R", "A").unwrap();
    let source = pose(0.0);
    assert_eq!(p.scan_pose(1, 0, source).unwrap(), source);

    let poses = [
        ScanPose {
            evidence_id: 1,
            scan_idx: 0,
            pose: pose(1.5),
            verified: true,
        },
        ScanPose {
            evidence_id: 1,
            scan_idx: 1,
            pose: pose(-2.0),
            verified: false,
        },
    ];
    let result = json!({ "summary": { "links": 3, "flagged": 1 } });
    let id = p
        .record_registration(None, &json!({ "cloud": true }), &result, &poses)
        .unwrap();
    // Recorded but not applied: the scene still uses the file poses.
    assert_eq!(p.scan_pose(1, 0, source).unwrap(), source);

    p.apply_registration(Some(id)).unwrap();
    assert_eq!(p.scan_pose(1, 0, source).unwrap(), pose(1.5));
    assert_eq!(p.scan_pose(1, 1, source).unwrap(), pose(-2.0));
    // A scan the registration doesn't cover keeps its file pose.
    assert_eq!(p.scan_pose(2, 0, source).unwrap(), source);

    // A re-solve with edited links names its parent.
    let child = p
        .record_registration(
            Some(id),
            &json!({ "edits": [{ "link": 2, "action": "delete" }] }),
            &result,
            &poses[..1],
        )
        .unwrap();
    let regs = p.registrations().unwrap();
    assert_eq!(regs.len(), 2);
    assert!(regs[0].applied && !regs[1].applied);
    assert_eq!(regs[1].parent, Some(id));
    assert_eq!(regs[0].poses, poses.to_vec());

    p.apply_registration(None).unwrap();
    assert_eq!(p.scan_pose(1, 0, source).unwrap(), source);
    assert!(p.apply_registration(Some(child + 10)).is_err());

    let actions: Vec<String> = p
        .audit_log()
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    assert_eq!(
        actions,
        [
            "project.created",
            "registration.run",
            "registration.applied",
            "registration.run",
            "registration.applied"
        ]
    );
}

#[test]
fn registrations_cannot_be_edited_or_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("r.locus");
    let mut p = Project::create(&root, "R", "A").unwrap();
    let poses = [ScanPose {
        evidence_id: 1,
        scan_idx: 0,
        pose: pose(1.0),
        verified: true,
    }];
    p.record_registration(None, &json!({}), &json!({}), &poses)
        .unwrap();
    drop(p);
    let c = Connection::open(root.join("project.sqlite")).unwrap();
    for sql in [
        "UPDATE registrations SET result = '{}'",
        "DELETE FROM registrations",
        "UPDATE registration_poses SET pose = '[]'",
        "DELETE FROM registration_poses",
    ] {
        assert!(c.execute(sql, []).is_err(), "{sql} was allowed");
    }
}
