//! 2-D diagrams, added in schema 4. A diagram's content is a JSON document (see
//! `app/src/diagram2d/model.ts`); every saved state is an immutable revision with its SHA-256,
//! written with its audit entry in one transaction. Nothing is overwritten: the current
//! state of a diagram is its newest revision, and every earlier one stays readable.

use crate::project::{now, Project, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA_V4: &str = "
CREATE TABLE diagrams (
    id         INTEGER PRIMARY KEY,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL
);
CREATE TRIGGER diagrams_no_delete BEFORE DELETE ON diagrams
    BEGIN SELECT RAISE(ABORT, 'diagrams are kept'); END;
CREATE TRIGGER diagrams_immutable BEFORE UPDATE ON diagrams
    BEGIN SELECT RAISE(ABORT, 'a diagram record cannot be edited; save a revision'); END;
CREATE TABLE diagram_revisions (
    id         INTEGER PRIMARY KEY,
    diagram_id INTEGER NOT NULL REFERENCES diagrams(id),
    name       TEXT NOT NULL,
    document   TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL
);
CREATE TRIGGER diagram_revisions_no_delete BEFORE DELETE ON diagram_revisions
    BEGIN SELECT RAISE(ABORT, 'diagram revisions are kept'); END;
CREATE TRIGGER diagram_revisions_immutable BEFORE UPDATE ON diagram_revisions
    BEGIN SELECT RAISE(ABORT, 'a diagram revision cannot be edited'); END;
";

/// A diagram's newest revision, or any revision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiagramRevision {
    pub diagram_id: i64,
    pub revision_id: i64,
    /// How many revisions the diagram has up to and including this one.
    pub number: i64,
    pub name: String,
    pub document: Value,
    pub sha256: String,
    pub created_at: String,
    pub created_by: String,
}

/// One revision in a diagram's history (without its document).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryEntry {
    pub revision_id: i64,
    pub name: String,
    pub sha256: String,
    pub created_at: String,
    pub created_by: String,
}

fn sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Entity counts by kind, for the audit log (the document itself is in the table).
fn summary(doc: &Value) -> Value {
    let mut counts = serde_json::Map::new();
    for e in doc["entities"].as_array().into_iter().flatten() {
        let k = e["kind"].as_str().unwrap_or("?").to_string();
        let n = counts.get(&k).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(k, json!(n));
    }
    Value::Object(counts)
}

impl Project {
    /// Create a diagram with its first revision. Returns the revision.
    pub fn create_diagram(&mut self, name: &str, document: &Value) -> Result<DiagramRevision> {
        let examiner = self.examiner().to_string();
        let text = document.to_string();
        let hash = sha256(&text);
        let at = now();
        let (diagram_id, revision_id) = self.logged("diagram.created", |tx| {
            tx.execute(
                "INSERT INTO diagrams (created_at, created_by) VALUES (?1, ?2)",
                params![at, examiner],
            )?;
            let d = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO diagram_revisions (diagram_id, name, document, sha256, created_at, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![d, name, text, hash, at, examiner],
            )?;
            let r = tx.last_insert_rowid();
            let details = json!({ "diagram": d, "revision": r, "name": name, "sha256": hash, "entities": summary(document) });
            Ok(((d, r), details))
        })?;
        Ok(DiagramRevision {
            diagram_id,
            revision_id,
            number: 1,
            name: name.into(),
            document: document.clone(),
            sha256: hash,
            created_at: at,
            created_by: examiner,
        })
    }

    /// Save a new revision of a diagram. If the document and name are unchanged from the
    /// newest revision, nothing is written and that revision is returned.
    pub fn save_diagram(
        &mut self,
        diagram_id: i64,
        name: &str,
        document: &Value,
    ) -> Result<DiagramRevision> {
        let latest = self
            .diagram_latest(diagram_id)?
            .ok_or_else(|| crate::Error::NotFound(format!("diagram {diagram_id}")))?;
        let text = document.to_string();
        let hash = sha256(&text);
        if hash == latest.sha256 && name == latest.name {
            return Ok(latest);
        }
        let examiner = self.examiner().to_string();
        let at = now();
        let revision_id = self.logged("diagram.revised", |tx| {
            tx.execute(
                "INSERT INTO diagram_revisions (diagram_id, name, document, sha256, created_at, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![diagram_id, name, text, hash, at, examiner],
            )?;
            let r = tx.last_insert_rowid();
            let details = json!({
                "diagram": diagram_id,
                "revision": r,
                "previous": latest.revision_id,
                "name": name,
                "sha256": hash,
                "entities": summary(document),
            });
            Ok((r, details))
        })?;
        Ok(DiagramRevision {
            diagram_id,
            revision_id,
            number: latest.number + 1,
            name: name.into(),
            document: document.clone(),
            sha256: hash,
            created_at: at,
            created_by: examiner,
        })
    }

    fn revision_where(&self, clause: &str, param: i64) -> Result<Option<DiagramRevision>> {
        let sql = format!(
            "SELECT r.diagram_id, r.id, r.name, r.document, r.sha256, r.created_at, r.created_by,
                    (SELECT COUNT(*) FROM diagram_revisions q WHERE q.diagram_id = r.diagram_id AND q.id <= r.id)
             FROM diagram_revisions r WHERE {clause}"
        );
        let row = self
            .conn
            .query_row(&sql, [param], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })
            .optional()?;
        let Some((diagram_id, revision_id, name, document, sha256, created_at, created_by, number)) =
            row
        else {
            return Ok(None);
        };
        Ok(Some(DiagramRevision {
            diagram_id,
            revision_id,
            number,
            name,
            document: serde_json::from_str(&document)?,
            sha256,
            created_at,
            created_by,
        }))
    }

    pub fn diagram_latest(&self, diagram_id: i64) -> Result<Option<DiagramRevision>> {
        self.revision_where(
            "r.id = (SELECT MAX(id) FROM diagram_revisions WHERE diagram_id = ?1)",
            diagram_id,
        )
    }

    pub fn diagram_revision(&self, revision_id: i64) -> Result<Option<DiagramRevision>> {
        self.revision_where("r.id = ?1", revision_id)
    }

    /// Every diagram at its newest revision, oldest diagram first.
    pub fn diagrams(&self) -> Result<Vec<DiagramRevision>> {
        let ids: Vec<i64> = self
            .conn
            .prepare("SELECT id FROM diagrams ORDER BY id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = vec![];
        for id in ids {
            out.extend(self.diagram_latest(id)?);
        }
        Ok(out)
    }

    /// Log that revision `revision_id` of a diagram was printed to `path` at 1:`scale`, with
    /// the file's hash.
    pub fn record_diagram_export(
        &mut self,
        revision_id: i64,
        scale: f64,
        path: &str,
        sha256: &str,
        bytes: u64,
    ) -> Result<()> {
        let rev = self
            .diagram_revision(revision_id)?
            .ok_or_else(|| crate::Error::NotFound(format!("diagram revision {revision_id}")))?;
        self.logged("diagram.exported", |_| {
            let details = json!({
                "diagram": rev.diagram_id,
                "revision": revision_id,
                "revision_sha256": rev.sha256,
                "scale": format!("1:{scale}"),
                "path": path,
                "sha256": sha256,
                "bytes": bytes,
            });
            Ok(((), details))
        })
    }

    /// A diagram's revisions, oldest first.
    pub fn diagram_history(&self, diagram_id: i64) -> Result<Vec<HistoryEntry>> {
        Ok(self
            .conn
            .prepare(
                "SELECT id, name, sha256, created_at, created_by FROM diagram_revisions
                 WHERE diagram_id = ?1 ORDER BY id",
            )?
            .query_map([diagram_id], |r| {
                Ok(HistoryEntry {
                    revision_id: r.get(0)?,
                    name: r.get(1)?,
                    sha256: r.get(2)?,
                    created_at: r.get(3)?,
                    created_by: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}
