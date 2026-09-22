//! Analysis state added in schema 2: settings, octree bookkeeping, measurements and
//! cleanup operations. Every change is written with its audit entry in one transaction.

use crate::audit;
use crate::project::{now, Project, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

pub(crate) const SCHEMA_V2: &str = "
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE octrees (
    evidence_id INTEGER NOT NULL REFERENCES evidence(id),
    scan_idx    INTEGER NOT NULL,
    status      TEXT NOT NULL,
    points      INTEGER NOT NULL,
    detail      TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (evidence_id, scan_idx)
);
CREATE TABLE measurements (
    id         INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL,
    points     TEXT NOT NULL,
    result     TEXT NOT NULL,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL,
    deleted_at TEXT
);
CREATE TRIGGER measurements_no_delete BEFORE DELETE ON measurements
    BEGIN SELECT RAISE(ABORT, 'measurements are soft-deleted only'); END;
CREATE TRIGGER measurements_immutable BEFORE UPDATE OF kind, points, result, created_at, created_by ON measurements
    BEGIN SELECT RAISE(ABORT, 'a measurement cannot be edited; delete it and measure again'); END;
CREATE TABLE cleanup_ops (
    id         INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL,
    params     TEXT NOT NULL,
    scans      TEXT NOT NULL,
    active     INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL
);
CREATE TRIGGER cleanup_no_delete BEFORE DELETE ON cleanup_ops
    BEGIN SELECT RAISE(ABORT, 'cleanup operations are undone, never deleted'); END;
CREATE TRIGGER cleanup_immutable BEFORE UPDATE OF kind, params, scans, created_at, created_by ON cleanup_ops
    BEGIN SELECT RAISE(ABORT, 'a cleanup operation cannot be edited'); END;
";

/// Positional uncertainty assumed for each picked point when no setting is stored.
pub const DEFAULT_POINT_SIGMA_M: f64 = 0.002;
pub const POINT_SIGMA_KEY: &str = "point_sigma_m";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OctreeRecord {
    pub evidence_id: i64,
    pub scan_idx: usize,
    /// "building", "built" or "failed".
    pub status: String,
    pub points: u64,
    pub detail: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeasurementRecord {
    pub id: i64,
    pub kind: String,
    pub points: Value,
    pub result: Value,
    pub created_at: String,
    pub created_by: String,
}

/// What one cleanup operation removed from one scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanupScan {
    pub evidence_id: i64,
    pub scan_idx: usize,
    pub removed: u64,
    /// Bitmap of removed source record numbers, relative to the project root.
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CleanupRecord {
    pub id: i64,
    pub kind: String,
    pub params: Value,
    pub scans: Vec<CleanupScan>,
    pub active: bool,
    pub created_at: String,
    pub created_by: String,
}

impl Project {
    fn logged<T>(
        &mut self,
        action: &str,
        change: impl FnOnce(&rusqlite::Transaction) -> Result<(T, Value)>,
    ) -> Result<T> {
        let tx = self.conn.transaction()?;
        let (out, details) = change(&tx)?;
        audit::append(&tx, &self.examiner, action, &details)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        let old = self.setting(key)?;
        self.logged("setting.changed", |tx| {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [key, value],
            )?;
            Ok(((), json!({ "key": key, "old": old, "new": value })))
        })
    }

    /// Per-point positional uncertainty (1σ, meters) used for measurement uncertainty.
    pub fn point_sigma(&self) -> Result<f64> {
        Ok(self
            .setting(POINT_SIGMA_KEY)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_POINT_SIGMA_M))
    }

    pub fn octree_dir(&self, evidence_id: i64, scan_idx: usize) -> PathBuf {
        Self::octree_dir_in(&self.root, evidence_id, scan_idx)
    }

    /// Where a scan's octree lives inside a project folder.
    pub fn octree_dir_in(root: &std::path::Path, evidence_id: i64, scan_idx: usize) -> PathBuf {
        root.join("derived")
            .join("octree")
            .join(format!("{evidence_id}-{scan_idx}"))
    }

    pub fn record_octree(
        &mut self,
        evidence_id: i64,
        scan_idx: usize,
        status: &str,
        points: u64,
        detail: &str,
    ) -> Result<()> {
        let at = now();
        self.logged(&format!("octree.{status}"), |tx| {
            tx.execute(
                "INSERT INTO octrees (evidence_id, scan_idx, status, points, detail, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (evidence_id, scan_idx) DO UPDATE SET
                   status = excluded.status, points = excluded.points,
                   detail = excluded.detail, updated_at = excluded.updated_at",
                params![evidence_id, scan_idx as i64, status, points as i64, detail, at],
            )?;
            let d = json!({ "evidence_id": evidence_id, "scan": scan_idx, "points": points, "detail": detail });
            Ok(((), d))
        })
    }

    pub fn octrees(&self) -> Result<Vec<OctreeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT evidence_id, scan_idx, status, points, detail, updated_at FROM octrees
             ORDER BY evidence_id, scan_idx",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(OctreeRecord {
                evidence_id: r.get(0)?,
                scan_idx: r.get::<_, i64>(1)? as usize,
                status: r.get(2)?,
                points: r.get::<_, i64>(3)? as u64,
                detail: r.get(4)?,
                updated_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Store a measurement. `points` must identify each picked point (scan and source
    /// record number) along with its coordinates; `result` holds values and uncertainties.
    pub fn add_measurement(&mut self, kind: &str, points: Value, result: Value) -> Result<i64> {
        let (at, by) = (now(), self.examiner.clone());
        self.logged("measurement.created", |tx| {
            tx.execute(
                "INSERT INTO measurements (kind, points, result, created_at, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![kind, points.to_string(), result.to_string(), at, by],
            )?;
            let id = tx.last_insert_rowid();
            Ok((
                id,
                json!({ "id": id, "kind": kind, "points": points, "result": result }),
            ))
        })
    }

    pub fn delete_measurement(&mut self, id: i64) -> Result<()> {
        let at = now();
        self.logged("measurement.deleted", |tx| {
            let n = tx.execute(
                "UPDATE measurements SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                params![at, id],
            )?;
            if n == 0 {
                return Err(crate::Error::NotFound(format!("measurement {id}")));
            }
            Ok(((), json!({ "id": id })))
        })
    }

    pub fn measurements(&self) -> Result<Vec<MeasurementRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, points, result, created_at, created_by FROM measurements
             WHERE deleted_at IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, kind, points, result, created_at, created_by) = row?;
            out.push(MeasurementRecord {
                id,
                kind,
                points: serde_json::from_str(&points)?,
                result: serde_json::from_str(&result)?,
                created_at,
                created_by,
            });
        }
        Ok(out)
    }

    pub fn add_cleanup(
        &mut self,
        kind: &str,
        params: Value,
        scans: Vec<CleanupScan>,
    ) -> Result<i64> {
        let (at, by) = (now(), self.examiner.clone());
        let scans_json = serde_json::to_value(&scans)?;
        self.logged("cleanup.applied", |tx| {
            tx.execute(
                "INSERT INTO cleanup_ops (kind, params, scans, active, created_at, created_by)
                 VALUES (?1, ?2, ?3, 1, ?4, ?5)",
                params![kind, params.to_string(), scans_json.to_string(), at, by],
            )?;
            let id = tx.last_insert_rowid();
            Ok((
                id,
                json!({ "id": id, "kind": kind, "params": params, "scans": scans_json }),
            ))
        })
    }

    /// Undo (`active = false`) or redo a cleanup operation.
    pub fn set_cleanup_active(&mut self, id: i64, active: bool) -> Result<()> {
        let action = if active {
            "cleanup.redone"
        } else {
            "cleanup.undone"
        };
        self.logged(action, |tx| {
            let n = tx.execute(
                "UPDATE cleanup_ops SET active = ?1 WHERE id = ?2 AND active = ?3",
                params![active as i64, id, !active as i64],
            )?;
            if n == 0 {
                return Err(crate::Error::NotFound(format!(
                    "cleanup operation {id} that is currently {}",
                    if active { "undone" } else { "active" }
                )));
            }
            Ok(((), json!({ "id": id })))
        })
    }

    pub fn cleanups(&self) -> Result<Vec<CleanupRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, params, scans, active, created_at, created_by FROM cleanup_ops ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, kind, params, scans, active, created_at, created_by) = row?;
            out.push(CleanupRecord {
                id,
                kind,
                params: serde_json::from_str(&params)?,
                scans: serde_json::from_str(&scans)?,
                active: active != 0,
                created_at,
                created_by,
            });
        }
        Ok(out)
    }
}
