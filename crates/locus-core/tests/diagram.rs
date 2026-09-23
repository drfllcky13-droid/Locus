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
    let same = p
        .save_diagram(first.document_id, "Scene plan", &v1)
        .unwrap();
    assert_eq!(same.revision_id, first.revision_id);

    let v2 =
        json!({ "entities": [{ "kind": "line" }, { "kind": "marker" }, { "kind": "marker" }] });
    let second = p
        .save_diagram(first.document_id, "Scene plan", &v2)
        .unwrap();
    assert_eq!(second.number, 2);
    let renamed = p
        .save_diagram(first.document_id, "Scene plan, final", &v2)
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
    assert_eq!(p.diagram_history(first.document_id).unwrap().len(), 3);
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

#[test]
fn scenes_are_stored_like_diagrams_but_apart_from_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(&dir.path().join("p.locus"), "Case", "A").unwrap();
    let d = p
        .create_diagram("Plan", &json!({ "entities": [] }))
        .unwrap();
    let doc = json!({ "version": 1, "objects": [{ "id": "w", "kind": "extrusion" }] });
    let s = p.create_scene("Scene", &doc).unwrap();
    assert_eq!(s.number, 1);
    assert_eq!(p.scenes().unwrap().len(), 1);
    assert_eq!(p.diagrams().unwrap().len(), 1);
    // Ids are per kind: the scene's first revision doesn't clash with the diagram's.
    assert_eq!(
        p.scene_latest(s.document_id).unwrap().unwrap().document,
        doc
    );
    assert_eq!(
        p.diagram_latest(d.document_id).unwrap().unwrap().name,
        "Plan"
    );
    let doc2 = json!({ "version": 1, "objects": [] });
    let s2 = p.save_scene(s.document_id, "Scene", &doc2).unwrap();
    assert_eq!(s2.number, 2);
    assert_eq!(p.scene_history(s.document_id).unwrap().len(), 2);
    let log = p.audit_log().unwrap();
    let last = log.last().unwrap();
    assert_eq!(last.action, "scene.revised");
    let details: serde_json::Value = serde_json::from_str(&last.details).unwrap();
    assert_eq!(details["scene"], json!(s.document_id));
    assert_eq!(details["objects"], json!({}));
}
