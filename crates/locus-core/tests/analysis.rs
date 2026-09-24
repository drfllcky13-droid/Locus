use locus_core::Project;
use rusqlite::Connection;
use serde_json::json;

#[test]
fn analysis_runs_are_kept_hashed_logged_and_only_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("a.locus");
    let mut p = Project::create(&root, "A", "Examiner").unwrap();
    let rec = json!({ "inputs": [1, 2], "result": { "bearing": 62.0 }, "summary": "62.0°" });
    let a = p
        .add_analysis("trajectory", "trajectory/1", "Shot 1", &rec, None)
        .unwrap();
    assert_eq!(a.record, rec);
    assert_eq!(a.sha256.len(), 64);
    // A revision names its parent; a missing parent is refused.
    let b = p
        .add_analysis(
            "trajectory",
            "trajectory/1",
            "Shot 1 (rev.)",
            &rec,
            Some(a.id),
        )
        .unwrap();
    assert_eq!(b.revises, Some(a.id));
    assert!(p
        .add_analysis("trajectory", "trajectory/1", "x", &rec, Some(999))
        .is_err());
    // Withdrawal needs a reason and keeps the record.
    assert!(p.withdraw_analysis(a.id, " ").is_err());
    p.withdraw_analysis(a.id, "wrong defect picked").unwrap();
    let all = p.analyses().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        all[0].withdrawn.as_ref().unwrap().reason,
        "wrong defect picked"
    );
    assert!(p.withdraw_analysis(a.id, "again").is_err());
    p.record_analysis_report(b.id, "C:/r.pdf", &"0".repeat(64), 10)
        .unwrap();
    let actions: Vec<String> = p
        .audit_log()
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    assert!(actions.ends_with(&[
        "analysis.created".into(),
        "analysis.created".into(),
        "analysis.withdrawn".into(),
        "analysis.reported".into()
    ]));
    drop(p);
    // The database refuses edits and deletes.
    let c = Connection::open(root.join("project.sqlite")).unwrap();
    assert!(c
        .execute("UPDATE analyses SET record = '{}' WHERE id = 1", [])
        .is_err());
    assert!(c.execute("DELETE FROM analyses WHERE id = 1", []).is_err());
}

#[test]
fn printing_doesnt_move_the_state_head_and_a_record_traces_to_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(&dir.path().join("a.locus"), "A", "Examiner").unwrap();
    let rec = json!({ "summary": "x" });
    let a = p
        .add_analysis("trajectory", "trajectory/1", "Shot 1", &rec, None)
        .unwrap();
    let head = p.state_head().unwrap().unwrap();
    // The record's own entry carries its hash.
    let e = p
        .audit_entry_for("analysis.created", "id", &json!(a.id))
        .unwrap()
        .unwrap();
    assert_eq!(e.hash, head.hash);
    let d: serde_json::Value = serde_json::from_str(&e.details).unwrap();
    assert_eq!(d["sha256"], json!(a.sha256));
    // Printing is logged but doesn't change the project, so the state head stays.
    p.record_analysis_report(a.id, "r.pdf", &"ab".repeat(32), 10)
        .unwrap();
    assert_ne!(p.audit_log().unwrap().last().unwrap().hash, head.hash);
    assert_eq!(p.state_head().unwrap().unwrap().hash, head.hash);
    // A change does.
    p.add_analysis("trajectory", "trajectory/1", "Shot 2", &rec, None)
        .unwrap();
    assert_ne!(p.state_head().unwrap().unwrap().hash, head.hash);
}
