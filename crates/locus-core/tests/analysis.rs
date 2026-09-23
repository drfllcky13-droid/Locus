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
