use locus_core::Project;
use rusqlite::Connection;
use serde_json::json;

#[test]
fn diagrams_keep_every_revision_and_log_each_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(&dir.path().join("d.locus"), "D", "A").unwrap();
    let v1 = json!({ "entities": [{ "kind": "line" }] });
    let first = p.create_diagram("Scene plan", &v1).unwrap();
    assert_eq!(first.number, 1);

    // Unchanged: no new revision.
    let same = p.save_diagram(first.diagram_id, "Scene plan", &v1).unwrap();
    assert_eq!(same.revision_id, first.revision_id);

    let v2 =
        json!({ "entities": [{ "kind": "line" }, { "kind": "marker" }, { "kind": "marker" }] });
    let second = p.save_diagram(first.diagram_id, "Scene plan", &v2).unwrap();
    assert_eq!(second.number, 2);
    let renamed = p
        .save_diagram(first.diagram_id, "Scene plan, final", &v2)
        .unwrap();
    assert_eq!(renamed.number, 3);

    let all = p.diagrams().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "Scene plan, final");
    assert_eq!(all[0].document, v2);
    // The first revision is still there, unchanged.
    assert_eq!(
        p.diagram_revision(first.revision_id)
            .unwrap()
            .unwrap()
            .document,
        v1
    );
    assert_eq!(p.diagram_history(first.diagram_id).unwrap().len(), 3);
    assert_ne!(first.sha256, second.sha256);

    let log = p.audit_log().unwrap();
    let actions: Vec<&str> = log.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        actions,
        [
            "project.created",
            "diagram.created",
            "diagram.revised",
            "diagram.revised"
        ]
    );
    let d: serde_json::Value = serde_json::from_str(&log[2].details).unwrap();
    assert_eq!(d["entities"]["marker"], 2);
    assert_eq!(d["sha256"], second.sha256);
    assert!(p.save_diagram(99, "x", &v1).is_err());
}

#[test]
fn diagram_revisions_cannot_be_edited_or_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("d.locus");
    let mut p = Project::create(&root, "D", "A").unwrap();
    p.create_diagram("Plan", &json!({ "entities": [] }))
        .unwrap();
    drop(p);
    let c = Connection::open(root.join("project.sqlite")).unwrap();
    for sql in [
        "UPDATE diagram_revisions SET document = '{}'",
        "DELETE FROM diagram_revisions",
        "UPDATE diagrams SET created_by = 'X'",
        "DELETE FROM diagrams",
    ] {
        assert!(c.execute(sql, []).is_err(), "{sql} was allowed");
    }
}
