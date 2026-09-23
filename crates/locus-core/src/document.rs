//! Stored documents: 2-D diagrams (schema 4) and 3-D scenes (schema 5). A document's content
//! is JSON (see `app/src/diagram2d/model.ts` and `app/src/scene3d/model.ts`); every saved
//! state is an immutable revision with its SHA-256, written with its audit entry in one
//! transaction. Nothing is overwritten: the current state of a document is its newest
//! revision, and every earlier one stays readable.

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

pub(crate) const SCHEMA_V5: &str = "
CREATE TABLE scenes (
    id         INTEGER PRIMARY KEY,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL
);
CREATE TRIGGER scenes_no_delete BEFORE DELETE ON scenes
    BEGIN SELECT RAISE(ABORT, 'scenes are kept'); END;
CREATE TRIGGER scenes_immutable BEFORE UPDATE ON scenes
    BEGIN SELECT RAISE(ABORT, 'a scene record cannot be edited; save a revision'); END;
CREATE TABLE scene_revisions (
    id         INTEGER PRIMARY KEY,
    scene_id   INTEGER NOT NULL REFERENCES scenes(id),
    name       TEXT NOT NULL,
    document   TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL
);
CREATE TRIGGER scene_revisions_no_delete BEFORE DELETE ON scene_revisions
    BEGIN SELECT RAISE(ABORT, 'scene revisions are kept'); END;
CREATE TRIGGER scene_revisions_immutable BEFORE UPDATE ON scene_revisions
    BEGIN SELECT RAISE(ABORT, 'a scene revision cannot be edited'); END;
";

/// Which kind of document: its tables and the names used in audit entries.
#[derive(Clone, Copy)]
struct Kind {
    table: &'static str,
    revisions: &'static str,
    key: &'static str,
    /// `diagram` → audit actions `diagram.created`, `diagram.revised`.
    name: &'static str,
    /// The document's array of items, counted by kind in the audit entry.
    items: &'static str,
}

const DIAGRAM: Kind = Kind {
    table: "diagrams",
    revisions: "diagram_revisions",
    key: "diagram_id",
    name: "diagram",
    items: "entities",
};

const SCENE: Kind = Kind {
    table: "scenes",
    revisions: "scene_revisions",
    key: "scene_id",
    name: "scene",
    items: "objects",
};

/// A document's newest revision, or any revision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Revision {
    /// The diagram's or scene's id.
    pub document_id: i64,
    pub revision_id: i64,
    /// How many revisions the document has up to and including this one.
    pub number: i64,
    pub name: String,
    pub document: Value,
    pub sha256: String,
    pub created_at: String,
    pub created_by: String,
}

pub type DiagramRevision = Revision;

/// One revision in a document's history (without its content).
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

/// Item counts by kind, for the audit log (the document itself is in the table).
fn summary(k: Kind, doc: &Value) -> Value {
    let mut counts = serde_json::Map::new();
    for e in doc[k.items].as_array().into_iter().flatten() {
        let kind = e["kind"].as_str().unwrap_or("?").to_string();
        let n = counts.get(&kind).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(kind, json!(n));
    }
    Value::Object(counts)
}

impl Project {
    fn create_document(&mut self, k: Kind, name: &str, document: &Value) -> Result<Revision> {
        let examiner = self.examiner().to_string();
        let text = document.to_string();
        let hash = sha256(&text);
        let at = now();
        let (document_id, revision_id) = self.logged(&format!("{}.created", k.name), |tx| {
            tx.execute(
                &format!(
                    "INSERT INTO {} (created_at, created_by) VALUES (?1, ?2)",
                    k.table
                ),
                params![at, examiner],
            )?;
            let d = tx.last_insert_rowid();
            tx.execute(
                &format!(
                    "INSERT INTO {} ({}, name, document, sha256, created_at, created_by)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    k.revisions, k.key
                ),
                params![d, name, text, hash, at, examiner],
            )?;
            let r = tx.last_insert_rowid();
            let mut details = json!({ "revision": r, "name": name, "sha256": hash });
            details[k.name] = json!(d);
            details[k.items] = summary(k, document);
            Ok(((d, r), details))
        })?;
        Ok(Revision {
            document_id,
            revision_id,
            number: 1,
            name: name.into(),
            document: document.clone(),
            sha256: hash,
            created_at: at,
            created_by: examiner,
        })
    }

    fn save_document(
        &mut self,
        k: Kind,
        id: i64,
        name: &str,
        document: &Value,
    ) -> Result<Revision> {
        let latest = self
            .latest(k, id)?
            .ok_or_else(|| crate::Error::NotFound(format!("{} {id}", k.name)))?;
        let text = document.to_string();
        let hash = sha256(&text);
        if hash == latest.sha256 && name == latest.name {
            return Ok(latest);
        }
        let examiner = self.examiner().to_string();
        let at = now();
        let revision_id = self.logged(&format!("{}.revised", k.name), |tx| {
            tx.execute(
                &format!(
                    "INSERT INTO {} ({}, name, document, sha256, created_at, created_by)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    k.revisions, k.key
                ),
                params![id, name, text, hash, at, examiner],
            )?;
            let r = tx.last_insert_rowid();
            let mut details = json!({
                "revision": r,
                "previous": latest.revision_id,
                "name": name,
                "sha256": hash,
            });
            details[k.name] = json!(id);
            details[k.items] = summary(k, document);
            Ok((r, details))
        })?;
        Ok(Revision {
            document_id: id,
            revision_id,
            number: latest.number + 1,
            name: name.into(),
            document: document.clone(),
            sha256: hash,
            created_at: at,
            created_by: examiner,
        })
    }

    fn revision_where(&self, k: Kind, clause: &str, param: i64) -> Result<Option<Revision>> {
        let (revs, key) = (k.revisions, k.key);
        let sql = format!(
            "SELECT r.{key}, r.id, r.name, r.document, r.sha256, r.created_at, r.created_by,
                    (SELECT COUNT(*) FROM {revs} q WHERE q.{key} = r.{key} AND q.id <= r.id)
             FROM {revs} r WHERE {clause}"
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
        let Some((
            document_id,
            revision_id,
            name,
            document,
            sha256,
            created_at,
            created_by,
            number,
        )) = row
        else {
            return Ok(None);
        };
        Ok(Some(Revision {
            document_id,
            revision_id,
            number,
            name,
            document: serde_json::from_str(&document)?,
            sha256,
            created_at,
            created_by,
        }))
    }

    fn latest(&self, k: Kind, id: i64) -> Result<Option<Revision>> {
        let clause = format!(
            "r.id = (SELECT MAX(id) FROM {} WHERE {} = ?1)",
            k.revisions, k.key
        );
        self.revision_where(k, &clause, id)
    }

    /// Every document of a kind at its newest revision, oldest first.
    fn all(&self, k: Kind) -> Result<Vec<Revision>> {
        let ids: Vec<i64> = self
            .conn
            .prepare(&format!("SELECT id FROM {} ORDER BY id", k.table))?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = vec![];
        for id in ids {
            out.extend(self.latest(k, id)?);
        }
        Ok(out)
    }

    fn history(&self, k: Kind, id: i64) -> Result<Vec<HistoryEntry>> {
        Ok(self
            .conn
            .prepare(&format!(
                "SELECT id, name, sha256, created_at, created_by FROM {}
                 WHERE {} = ?1 ORDER BY id",
                k.revisions, k.key
            ))?
            .query_map([id], |r| {
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

    // ---------- diagrams ----------

    /// Create a diagram with its first revision. Returns the revision.
    pub fn create_diagram(&mut self, name: &str, document: &Value) -> Result<Revision> {
        self.create_document(DIAGRAM, name, document)
    }

    /// Save a new revision of a diagram. If the document and name are unchanged from the
    /// newest revision, nothing is written and that revision is returned.
    pub fn save_diagram(&mut self, id: i64, name: &str, document: &Value) -> Result<Revision> {
        self.save_document(DIAGRAM, id, name, document)
    }

    pub fn diagram_latest(&self, id: i64) -> Result<Option<Revision>> {
        self.latest(DIAGRAM, id)
    }

    pub fn diagram_revision(&self, revision_id: i64) -> Result<Option<Revision>> {
        self.revision_where(DIAGRAM, "r.id = ?1", revision_id)
    }

    /// Every diagram at its newest revision, oldest diagram first.
    pub fn diagrams(&self) -> Result<Vec<Revision>> {
        self.all(DIAGRAM)
    }

    /// A diagram's revisions, oldest first.
    pub fn diagram_history(&self, id: i64) -> Result<Vec<HistoryEntry>> {
        self.history(DIAGRAM, id)
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
                "diagram": rev.document_id,
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

    /// Log an underlay being made or calibrated (`diagram.underlay_sliced`,
    /// `diagram.underlay_calibrated`) with what it was made from and how well it fits.
    pub fn record_underlay(&mut self, action: &str, details: Value) -> Result<()> {
        self.logged(action, |_| Ok(((), details)))
    }

    // ---------- 3-D scenes ----------

    /// Create a 3-D scene with its first revision.
    pub fn create_scene(&mut self, name: &str, document: &Value) -> Result<Revision> {
        self.create_document(SCENE, name, document)
    }

    /// Save a new revision of a scene (nothing is written if it is unchanged).
    pub fn save_scene(&mut self, id: i64, name: &str, document: &Value) -> Result<Revision> {
        self.save_document(SCENE, id, name, document)
    }

    pub fn scene_latest(&self, id: i64) -> Result<Option<Revision>> {
        self.latest(SCENE, id)
    }

    pub fn scene_revision(&self, revision_id: i64) -> Result<Option<Revision>> {
        self.revision_where(SCENE, "r.id = ?1", revision_id)
    }

    /// Every scene at its newest revision, oldest first.
    pub fn scenes(&self) -> Result<Vec<Revision>> {
        self.all(SCENE)
    }

    pub fn scene_history(&self, id: i64) -> Result<Vec<HistoryEntry>> {
        self.history(SCENE, id)
    }
}
