//! Phase 1 acceptance: imports never modify the source, and audit-log tampering is detected on open.

use locus_core::hash::sha256_file;
use locus_core::{Contents, Error, EvidenceStatus, LinearUnit, Project, ScanInfo, IDENTITY};
use proptest::prelude::*;
use rusqlite::Connection;
use std::fs;
use std::path::Path;

fn scan_contents() -> Contents {
    let mut c = Contents::new("XYZ");
    c.scans.push(ScanInfo {
        name: "scan".into(),
        point_count: 3,
        invalid_points: 0,
        bounds: None,
        pose: IDENTITY,
        attributes: vec![],
    });
    c
}

fn sha(p: &Path) -> String {
    sha256_file(p, &mut |_| {}).unwrap().0
}

/// Project with three audit entries (created, imported, opened), closed again.
fn project_with_log(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("case.locus");
    let src = dir.join("pts.xyz");
    fs::write(&src, "0 0 0\n1 1 1\n2 2 2\n").unwrap();
    let mut p = Project::create(&root, "Case", "Examiner A").unwrap();
    p.import_evidence(
        &src,
        &sha(&src),
        &scan_contents(),
        Some(LinearUnit::Meter),
        &mut |_| {},
    )
    .unwrap();
    drop(p);
    drop(Project::open(&root, "Examiner B").unwrap());
    root
}

/// Raw access with the append-only triggers removed, as an attacker would.
fn raw(root: &Path) -> Connection {
    let c = Connection::open(root.join("project.sqlite")).unwrap();
    c.execute_batch("DROP TRIGGER audit_log_no_update; DROP TRIGGER audit_log_no_delete;")
        .unwrap();
    c
}

fn assert_tampered(root: &Path, at: i64) {
    match Project::open(root, "Examiner C") {
        Err(Error::Tampered { seq, .. }) => assert_eq!(seq, at),
        Err(e) => panic!("expected tamper detection, got error {e}"),
        Ok(_) => panic!("tampering went undetected"),
    }
}

#[test]
fn import_never_modifies_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("scan.xyz");
    fs::write(&src, "0 0 0\n1 2 3\n4 5 6\n").unwrap();
    let before_hash = sha(&src);
    let before_meta = fs::metadata(&src).unwrap();

    let mut p = Project::create(&dir.path().join("case.locus"), "Case", "A").unwrap();
    let rec = p
        .import_evidence(
            &src,
            &before_hash,
            &scan_contents(),
            Some(LinearUnit::Meter),
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(sha(&src), before_hash, "source bytes changed");
    let after_meta = fs::metadata(&src).unwrap();
    assert_eq!(
        after_meta.modified().unwrap(),
        before_meta.modified().unwrap()
    );
    assert_eq!(after_meta.len(), before_meta.len());

    let stored = p.root().join(&rec.stored_path);
    assert_eq!(fs::read(&stored).unwrap(), fs::read(&src).unwrap());
    assert!(fs::metadata(&stored).unwrap().permissions().readonly());
    assert_eq!(rec.sha256, before_hash);

    let entry = p.audit_log().unwrap().pop().unwrap();
    assert_eq!(entry.action, "evidence.imported");
    assert!(entry.details.contains(&before_hash));
}

#[test]
fn import_refuses_changed_source_duplicates_and_missing_units() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("scan.xyz");
    fs::write(&src, "0 0 0\n").unwrap();
    let h = sha(&src);
    let mut p = Project::create(&dir.path().join("case.locus"), "Case", "A").unwrap();
    let c = scan_contents();

    assert!(matches!(
        p.import_evidence(&src, &h, &c, None, &mut |_| {}),
        Err(Error::UnitRequired)
    ));
    let mut declared = c.clone();
    declared.declared_unit = Some(LinearUnit::Meter);
    assert!(matches!(
        p.import_evidence(&src, &h, &declared, Some(LinearUnit::Foot), &mut |_| {}),
        Err(Error::UnitConflict { .. })
    ));

    fs::write(&src, "9 9 9\n").unwrap(); // file changes between preview and commit
    assert!(matches!(
        p.import_evidence(&src, &h, &c, Some(LinearUnit::Meter), &mut |_| {}),
        Err(Error::SourceChanged { .. })
    ));
    assert!(p.evidence().unwrap().is_empty());
    assert_eq!(
        fs::read_dir(p.root().join("evidence")).unwrap().count(),
        0,
        "partial copy left behind"
    );

    let h2 = sha(&src);
    let rec = p
        .import_evidence(&src, &h2, &c, Some(LinearUnit::Meter), &mut |_| {})
        .unwrap();
    match p.import_evidence(&src, &h2, &c, Some(LinearUnit::Meter), &mut |_| {}) {
        Err(Error::DuplicateEvidence { existing_id }) => assert_eq!(existing_id, rec.id),
        other => panic!("expected duplicate refusal, got {other:?}"),
    }
}

#[test]
fn evidence_verification_detects_changes() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("scan.xyz");
    fs::write(&src, "0 0 0\n").unwrap();
    let mut p = Project::create(&dir.path().join("case.locus"), "Case", "A").unwrap();
    let rec = p
        .import_evidence(
            &src,
            &sha(&src),
            &scan_contents(),
            Some(LinearUnit::Meter),
            &mut |_| {},
        )
        .unwrap();
    let report = p.verify_evidence(&mut |_| {}).unwrap();
    assert!(report.is_clean());
    assert_eq!(report.results, vec![(rec.id, EvidenceStatus::Intact)]);

    let stored = p.root().join(&rec.stored_path);
    let mut perms = fs::metadata(&stored).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&stored, perms).unwrap();
    fs::write(&stored, "1 1 1\n").unwrap();
    let report = p.verify_evidence(&mut |_| {}).unwrap();
    assert!(matches!(
        report.results[0].1,
        EvidenceStatus::Changed { .. }
    ));
    assert!(p
        .audit_log()
        .unwrap()
        .last()
        .unwrap()
        .details
        .contains("changed"));
}

fn make_writable(path: &Path) {
    let mut perms = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(path, perms).unwrap();
}

/// The read-only flag only guards against accidents; the hash check on open is what
/// catches a deliberately edited evidence file.
#[test]
fn opening_rehashes_evidence_and_reports_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    let clean = Project::open(&root, "B").unwrap();
    assert!(clean.integrity_on_open().is_clean());
    let stored = clean.root().join(&clean.evidence().unwrap()[0].stored_path);
    drop(clean);

    make_writable(&stored);
    fs::write(&stored, "0 0 0\n1 1 1\n2 2 9\n").unwrap(); // one digit changed
    let p = Project::open(&root, "C").unwrap();
    let report = p.integrity_on_open();
    assert!(!report.is_clean());
    let actual = sha(&stored);
    assert_eq!(
        report.results,
        vec![(
            1,
            EvidenceStatus::Changed {
                actual: actual.clone()
            }
        )]
    );
    let entry = p.audit_log().unwrap().pop().unwrap();
    assert_eq!(entry.action, "project.opened");
    assert!(entry.details.contains(&actual), "{}", entry.details);
}

#[test]
fn opening_reports_missing_and_unrecorded_evidence_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    let stored = {
        let p = Project::open(&root, "B").unwrap();
        p.root().join(&p.evidence().unwrap()[0].stored_path)
    };
    make_writable(&stored);
    fs::remove_file(&stored).unwrap();
    fs::write(root.join("evidence").join("planted.xyz"), "5 5 5\n").unwrap();

    let p = Project::open(&root, "C").unwrap();
    let report = p.integrity_on_open();
    assert_eq!(report.results, vec![(1, EvidenceStatus::Missing)]);
    assert_eq!(report.unrecorded, ["planted.xyz"]);
}

#[test]
fn untouched_project_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    let p = Project::open(&root, "C").unwrap();
    let log = p.audit_log().unwrap();
    let actions: Vec<_> = log.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        actions,
        [
            "project.created",
            "evidence.imported",
            "project.opened",
            "project.opened"
        ]
    );
    assert_eq!(log[2].actor, "Examiner B");
}

#[test]
fn edited_entry_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    raw(&root)
        .execute(
            "UPDATE audit_log SET details = '{\"forged\":1}' WHERE seq = 2",
            [],
        )
        .unwrap();
    assert_tampered(&root, 2);
}

#[test]
fn edited_actor_or_timestamp_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    raw(&root)
        .execute(
            "UPDATE audit_log SET actor = 'Someone else' WHERE seq = 1",
            [],
        )
        .unwrap();
    assert_tampered(&root, 1);

    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    raw(&root)
        .execute(
            "UPDATE audit_log SET timestamp = '2020-01-01T00:00:00.000Z' WHERE seq = 3",
            [],
        )
        .unwrap();
    assert_tampered(&root, 3);
}

#[test]
fn deleted_middle_entry_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    raw(&root)
        .execute("DELETE FROM audit_log WHERE seq = 2", [])
        .unwrap();
    assert_tampered(&root, 2);
}

#[test]
fn reordered_entries_are_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    let c = raw(&root);
    c.execute_batch(
        "UPDATE audit_log SET seq = 100 WHERE seq = 2;
         UPDATE audit_log SET seq = 2 WHERE seq = 3;
         UPDATE audit_log SET seq = 3 WHERE seq = 100;",
    )
    .unwrap();
    assert_tampered(&root, 2);
}

#[test]
fn truncated_tail_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    raw(&root)
        .execute("DELETE FROM audit_log WHERE seq = 3", [])
        .unwrap();
    assert_tampered(&root, 3);
}

#[test]
fn forged_head_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    raw(&root)
        .execute(
            "UPDATE meta SET value = '2' WHERE key = 'audit_head_seq'",
            [],
        )
        .unwrap();
    assert_tampered(&root, 4);
}

#[test]
fn appended_entry_without_head_update_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    let c = raw(&root);
    let prev: String = c
        .query_row("SELECT hash FROM audit_log WHERE seq = 3", [], |r| r.get(0))
        .unwrap();
    let (ts, details) = ("2026-01-01T00:00:00.000Z", "{}");
    let hash = locus_core::audit::entry_hash(&prev, 4, ts, "X", "evidence.deleted", details);
    c.execute(
        "INSERT INTO audit_log VALUES (4, ?1, 'X', 'evidence.deleted', ?2, ?3, ?4)",
        rusqlite::params![ts, details, prev, hash],
    )
    .unwrap();
    assert_tampered(&root, 5);
}

#[test]
fn app_code_cannot_rewrite_log_or_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let root = project_with_log(dir.path());
    let c = Connection::open(root.join("project.sqlite")).unwrap();
    assert!(c.execute("UPDATE audit_log SET actor = 'x'", []).is_err());
    assert!(c.execute("DELETE FROM audit_log", []).is_err());
    assert!(c.execute("UPDATE evidence SET sha256 = 'x'", []).is_err());
    assert!(c.execute("DELETE FROM evidence", []).is_err());
}

#[test]
fn create_refuses_non_empty_folder_and_open_refuses_non_project() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("x"), "").unwrap();
    assert!(matches!(
        Project::create(dir.path(), "C", "A"),
        Err(Error::NotEmpty(_))
    ));
    assert!(matches!(
        Project::open(dir.path(), "A"),
        Err(Error::NotAProject(_))
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Changing any single stored field of any entry is detected.
    #[test]
    fn any_single_field_edit_is_detected(
        extra in 0usize..6,
        row_pick in any::<prop::sample::Index>(),
        field in 0usize..6,
        junk in "[a-z0-9]{1,8}",
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("case.locus");
        drop(Project::create(&root, "Case", "A").unwrap());
        for _ in 0..extra {
            drop(Project::open(&root, "A").unwrap());
        }
        let rows = 1 + extra as i64;
        let seq = 1 + row_pick.index(rows as usize) as i64;
        let col = ["timestamp", "actor", "action", "details", "prev_hash", "hash"][field];
        raw(&root)
            .execute(&format!("UPDATE audit_log SET {col} = {col} || ?1 WHERE seq = ?2"), rusqlite::params![junk, seq])
            .unwrap();
        let detected = matches!(Project::open(&root, "B"), Err(Error::Tampered { .. }));
        prop_assert!(detected, "edit of {} in entry {} went undetected", col, seq);
    }
}
