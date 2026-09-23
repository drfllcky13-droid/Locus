use crate::audit::{self, AuditEntry};
use crate::hash::{copy_hashed, sha256_file};
use crate::{Contents, LinearUnit};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("project database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("not a Locus project (no project.sqlite): {0}")]
    NotAProject(PathBuf),
    #[error("the folder for a new project must be empty: {0}")]
    NotEmpty(PathBuf),
    #[error("project was written by a newer or unknown version of Locus (schema {0})")]
    SchemaVersion(String),
    #[error("audit log has been altered at entry {seq}: {reason}")]
    Tampered { seq: i64, reason: String },
    #[error("this file is already in the project as evidence item {existing_id}")]
    DuplicateEvidence { existing_id: i64 },
    #[error("the source file changed after preview (expected SHA-256 {expected}, found {actual})")]
    SourceChanged { expected: String, actual: String },
    #[error("the copy written to the project does not match the source hash")]
    CopyMismatch,
    #[error("this file doesn't state its unit; choose the unit its coordinates are in")]
    UnitRequired,
    #[error("the file states its unit is {declared:?}, but {chosen:?} was chosen")]
    UnitConflict {
        declared: LinearUnit,
        chosen: LinearUnit,
    },
    #[error("not a SHA-256 hex digest: {0}")]
    BadHash(String),
    #[error("not found: {0}")]
    NotFound(String),
}

const DB_FILE: &str = "project.sqlite";
const SCHEMA_VERSION: &str = "3";

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE evidence (
    id                INTEGER PRIMARY KEY,
    sha256            TEXT NOT NULL UNIQUE,
    size              INTEGER NOT NULL,
    original_path     TEXT NOT NULL,
    original_modified TEXT,
    stored_path       TEXT NOT NULL,
    format            TEXT NOT NULL,
    unit              TEXT,
    contents          TEXT NOT NULL,
    imported_at       TEXT NOT NULL,
    imported_by       TEXT NOT NULL
);
CREATE TRIGGER evidence_no_update BEFORE UPDATE ON evidence
    BEGIN SELECT RAISE(ABORT, 'evidence records are immutable'); END;
CREATE TRIGGER evidence_no_delete BEFORE DELETE ON evidence
    BEGIN SELECT RAISE(ABORT, 'evidence records are immutable'); END;
CREATE TABLE scans (
    id          INTEGER PRIMARY KEY,
    evidence_id INTEGER NOT NULL REFERENCES evidence(id),
    idx         INTEGER NOT NULL,
    name        TEXT NOT NULL,
    point_count INTEGER NOT NULL,
    source_pose TEXT NOT NULL,
    UNIQUE (evidence_id, idx)
);
";

pub(crate) fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub(crate) fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// The current time as stored in the project (UTC, RFC 3339, milliseconds).
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceRecord {
    pub id: i64,
    pub sha256: String,
    pub size: u64,
    pub original_path: String,
    pub original_modified: Option<String>,
    /// Relative to the project root.
    pub stored_path: String,
    pub unit: Option<LinearUnit>,
    pub contents: Contents,
    pub imported_at: String,
    pub imported_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EvidenceStatus {
    Intact,
    Missing,
    Changed { actual: String },
}

/// Result of re-hashing the evidence folder against the recorded hashes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IntegrityReport {
    pub results: Vec<(i64, EvidenceStatus)>,
    /// Files in `evidence/` that no evidence record accounts for (placed there by hand,
    /// or left by an interrupted import).
    pub unrecorded: Vec<String>,
}

impl IntegrityReport {
    pub fn is_clean(&self) -> bool {
        self.unrecorded.is_empty()
            && self
                .results
                .iter()
                .all(|(_, s)| *s == EvidenceStatus::Intact)
    }

    fn to_json(&self) -> serde_json::Value {
        let failed: Vec<_> = self
            .results
            .iter()
            .filter(|(_, s)| *s != EvidenceStatus::Intact)
            .map(|(id, s)| json!({ "evidence_id": id, "result": s }))
            .collect();
        json!({ "checked": self.results.len(), "failed": failed, "unrecorded": self.unrecorded })
    }
}

/// An open `.locus` project folder.
pub struct Project {
    pub(crate) root: PathBuf,
    pub(crate) conn: Connection,
    pub(crate) examiner: String,
    integrity_on_open: IntegrityReport,
}

impl Project {
    /// Create a new project in `root`, which must be missing or empty.
    pub fn create(root: &Path, name: &str, examiner: &str) -> Result<Self> {
        if root.exists() && fs::read_dir(root)?.next().is_some() {
            return Err(Error::NotEmpty(root.into()));
        }
        for dir in ["evidence", "derived", "assets"] {
            fs::create_dir_all(root.join(dir))?;
        }
        let mut conn = Connection::open(root.join(DB_FILE))?;
        let tx = conn.transaction()?;
        tx.execute_batch(SCHEMA)?;
        tx.execute_batch(audit::SCHEMA)?;
        tx.execute_batch(crate::state::SCHEMA_V2)?;
        tx.execute_batch(crate::registration::SCHEMA_V3)?;
        let created_at = now();
        for (k, v) in [
            ("schema_version", SCHEMA_VERSION),
            ("name", name),
            ("created_at", &created_at),
            ("created_by", examiner),
            ("locus_version", env!("CARGO_PKG_VERSION")),
        ] {
            meta_set(&tx, k, v)?;
        }
        audit::append(
            &tx,
            examiner,
            "project.created",
            &json!({ "name": name, "schema_version": SCHEMA_VERSION }),
        )?;
        tx.commit()?;
        Ok(Self {
            root: root.into(),
            conn,
            examiner: examiner.into(),
            integrity_on_open: IntegrityReport::default(),
        })
    }

    /// Open an existing project. See [`Project::open_with_progress`].
    pub fn open(root: &Path, examiner: &str) -> Result<Self> {
        Self::open_with_progress(root, examiner, &mut |_| {})
    }

    /// Open an existing project. Refuses to open if the audit log fails verification.
    /// Every evidence file is then re-hashed. Problems don't block opening (the examiner
    /// needs to see the case to deal with them) but are reported by
    /// [`Project::integrity_on_open`] and recorded in the `project.opened` audit entry.
    /// `progress` receives bytes hashed so far.
    pub fn open_with_progress(
        root: &Path,
        examiner: &str,
        progress: &mut dyn FnMut(u64),
    ) -> Result<Self> {
        let db = root.join(DB_FILE);
        if !db.is_file() {
            return Err(Error::NotAProject(root.into()));
        }
        let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let version = meta_get(&conn, "schema_version")?.unwrap_or_default();
        // Each later schema only adds tables: (from, to, what it adds).
        let migrations = [
            ("1", "2", crate::state::SCHEMA_V2),
            ("2", "3", crate::registration::SCHEMA_V3),
        ];
        if version != SCHEMA_VERSION && !migrations.iter().any(|m| m.0 == version) {
            return Err(Error::SchemaVersion(version));
        }
        audit::verify(&conn)?;
        if let Some((from, to, add)) = migrations.iter().find(|m| m.0 == version) {
            // Logged like any other change; repeated until the schema is current.
            let mut conn = conn;
            let tx = conn.transaction()?;
            tx.execute_batch(add)?;
            meta_set(&tx, "schema_version", to)?;
            audit::append(
                &tx,
                examiner,
                "project.migrated",
                &json!({ "from": from, "to": to }),
            )?;
            tx.commit()?;
            return Self::open_with_progress(root, examiner, progress);
        }
        let mut project = Self {
            root: root.into(),
            conn,
            examiner: examiner.into(),
            integrity_on_open: IntegrityReport::default(),
        };
        let report = project.check_evidence(progress)?;
        project.log("project.opened", json!({ "evidence": report.to_json() }))?;
        project.integrity_on_open = report;
        Ok(project)
    }

    /// What the evidence check found when this project was opened.
    pub fn integrity_on_open(&self) -> &IntegrityReport {
        &self.integrity_on_open
    }

    fn log(&mut self, action: &str, details: serde_json::Value) -> Result<AuditEntry> {
        let tx = self.conn.transaction()?;
        let entry = audit::append(&tx, &self.examiner, action, &details)?;
        tx.commit()?;
        Ok(entry)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn examiner(&self) -> &str {
        &self.examiner
    }

    pub fn name(&self) -> Result<String> {
        Ok(meta_get(&self.conn, "name")?.unwrap_or_default())
    }

    pub fn audit_log(&self) -> Result<Vec<AuditEntry>> {
        audit::entries(&self.conn)
    }

    pub fn evidence(&self) -> Result<Vec<EvidenceRecord>> {
        self.query_evidence("ORDER BY id", [])
    }

    pub fn evidence_by_hash(&self, sha256: &str) -> Result<Option<EvidenceRecord>> {
        Ok(self.query_evidence("WHERE sha256 = ?1", [sha256])?.pop())
    }

    fn query_evidence<P: rusqlite::Params>(&self, tail: &str, p: P) -> Result<Vec<EvidenceRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, sha256, size, original_path, original_modified, stored_path, unit,
                    contents, imported_at, imported_by FROM evidence {tail}"
        ))?;
        let rows = stmt.query_map(p, |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (
                id,
                sha256,
                size,
                original_path,
                original_modified,
                stored_path,
                unit,
                contents,
                at,
                by,
            ) = row?;
            out.push(EvidenceRecord {
                id,
                sha256,
                size: size as u64,
                original_path,
                original_modified,
                stored_path,
                unit: unit.map(|u| serde_json::from_value(json!(u))).transpose()?,
                contents: serde_json::from_str(&contents)?,
                imported_at: at,
                imported_by: by,
            });
        }
        Ok(out)
    }

    /// Copy `src` into the project as evidence.
    ///
    /// `expected_sha256` is the hash the examiner saw at preview; the copy is refused if the
    /// bytes differ. The source is only ever opened for reading. The stored copy is marked
    /// read-only, and the record, its scans and the audit entry are written in one transaction.
    pub fn import_evidence(
        &mut self,
        src: &Path,
        expected_sha256: &str,
        contents: &Contents,
        chosen_unit: Option<LinearUnit>,
        progress: &mut dyn FnMut(u64),
    ) -> Result<EvidenceRecord> {
        if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::BadHash(expected_sha256.into()));
        }
        let expected = expected_sha256.to_ascii_lowercase();
        let unit = match (contents.declared_unit, chosen_unit) {
            (Some(declared), Some(chosen)) if declared != chosen => {
                return Err(Error::UnitConflict { declared, chosen })
            }
            (Some(u), _) | (None, Some(u)) => Some(u),
            (None, None) if contents.needs_unit() => return Err(Error::UnitRequired),
            (None, None) => None,
        };
        if let Some(existing) = self.evidence_by_hash(&expected)? {
            return Err(Error::DuplicateEvidence {
                existing_id: existing.id,
            });
        }

        let src_meta = fs::metadata(src)?;
        let original_modified = src_meta.modified().ok().map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
        let file_name: String = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "evidence".into())
            .chars()
            .map(|c| {
                if "<>:\"/\\|?*".contains(c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let stored_path = format!("evidence/{}_{}", &expected[..12], file_name);
        let dst = self.root.join(&stored_path);
        let partial = self.root.join(format!("{stored_path}.partial"));

        let copied = copy_hashed(src, &partial, progress).and_then(|(sha, size)| {
            let (reread, _) = sha256_file(&partial, &mut |_| {})?;
            Ok((sha, reread, size))
        });
        let (sha, reread, size) = match copied {
            Ok(v) => v,
            Err(e) => {
                let _ = fs::remove_file(&partial);
                return Err(e.into());
            }
        };
        if sha != expected {
            fs::remove_file(&partial)?;
            return Err(Error::SourceChanged {
                expected,
                actual: sha,
            });
        }
        if reread != sha {
            fs::remove_file(&partial)?;
            return Err(Error::CopyMismatch);
        }
        fs::rename(&partial, &dst)?;
        let mut perms = fs::metadata(&dst)?.permissions();
        perms.set_readonly(true);
        fs::set_permissions(&dst, perms)?;

        let record = EvidenceRecord {
            id: 0,
            sha256: expected,
            size,
            original_path: src.to_string_lossy().into_owned(),
            original_modified,
            stored_path,
            unit,
            contents: contents.clone(),
            imported_at: now(),
            imported_by: self.examiner.clone(),
        };
        match self.insert_evidence(record) {
            Ok(r) => Ok(r),
            Err(e) => {
                remove_readonly(&dst);
                Err(e)
            }
        }
    }

    fn insert_evidence(&mut self, mut rec: EvidenceRecord) -> Result<EvidenceRecord> {
        let tx = self.conn.transaction()?;
        let unit = rec
            .unit
            .map(|u| json!(u).as_str().unwrap_or_default().to_string());
        tx.execute(
            "INSERT INTO evidence (sha256, size, original_path, original_modified, stored_path,
                                   format, unit, contents, imported_at, imported_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                rec.sha256,
                rec.size as i64,
                rec.original_path,
                rec.original_modified,
                rec.stored_path,
                rec.contents.format,
                unit,
                serde_json::to_string(&rec.contents)?,
                rec.imported_at,
                rec.imported_by,
            ],
        )?;
        rec.id = tx.last_insert_rowid();
        for (idx, scan) in rec.contents.scans.iter().enumerate() {
            tx.execute(
                "INSERT INTO scans (evidence_id, idx, name, point_count, source_pose)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    rec.id,
                    idx as i64,
                    scan.name,
                    scan.point_count as i64,
                    serde_json::to_string(&scan.pose)?,
                ],
            )?;
        }
        audit::append(
            &tx,
            &self.examiner,
            "evidence.imported",
            &json!({
                "evidence_id": rec.id,
                "sha256": rec.sha256,
                "size": rec.size,
                "original_path": rec.original_path,
                "original_modified": rec.original_modified,
                "stored_path": rec.stored_path,
                "format": rec.contents.format,
                "unit": rec.unit,
                "unit_declared_by_file": rec.contents.declared_unit.is_some(),
                "scans": rec.contents.scans.len(),
                "points": rec.contents.point_count(),
                "meshes": rec.contents.meshes.len(),
                "images": rec.contents.images.len(),
                "warnings": rec.contents.warnings,
            }),
        )?;
        tx.commit()?;
        Ok(rec)
    }

    /// Re-hash every stored evidence file against its recorded hash, and log the result.
    pub fn verify_evidence(&mut self, progress: &mut dyn FnMut(u64)) -> Result<IntegrityReport> {
        let report = self.check_evidence(progress)?;
        self.log("evidence.verified", report.to_json())?;
        Ok(report)
    }

    fn check_evidence(&self, progress: &mut dyn FnMut(u64)) -> Result<IntegrityReport> {
        let mut report = IntegrityReport::default();
        let mut done = 0u64;
        let records = self.evidence()?;
        for rec in &records {
            let path = self.root.join(&rec.stored_path);
            let status = if !path.is_file() {
                EvidenceStatus::Missing
            } else {
                let (actual, n) = sha256_file(&path, &mut |b| progress(done + b))?;
                done += n;
                if actual == rec.sha256 {
                    EvidenceStatus::Intact
                } else {
                    EvidenceStatus::Changed { actual }
                }
            };
            report.results.push((rec.id, status));
        }
        let recorded: std::collections::HashSet<_> = records
            .iter()
            .map(|r| self.root.join(&r.stored_path))
            .collect();
        for entry in fs::read_dir(self.root.join("evidence"))? {
            let path = entry?.path();
            if !recorded.contains(&path) {
                report.unrecorded.push(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        report.unrecorded.sort();
        Ok(report)
    }
}

fn remove_readonly(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = fs::set_permissions(path, perms);
    }
    let _ = fs::remove_file(path);
}
