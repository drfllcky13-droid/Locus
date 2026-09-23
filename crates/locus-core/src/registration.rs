//! Registrations, added in schema 3. A registration run is stored whole and never changed:
//! its parameters, its result (links with their statistics, targets, overall figures) and
//! the pose it gives each scan. Editing links (delete, force) and re-solving makes a new
//! registration that names its parent. At most one registration is applied; the scene then
//! uses its poses, otherwise the poses stored in the evidence files. Every run and every
//! change of the applied registration is audit-logged in the same transaction.

use crate::project::{now, Project, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

pub(crate) const SCHEMA_V3: &str = "
CREATE TABLE registrations (
    id         INTEGER PRIMARY KEY,
    parent     INTEGER REFERENCES registrations(id),
    params     TEXT NOT NULL,
    result     TEXT NOT NULL,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL
);
CREATE TRIGGER registrations_no_delete BEFORE DELETE ON registrations
    BEGIN SELECT RAISE(ABORT, 'registrations are kept; apply another instead'); END;
CREATE TRIGGER registrations_immutable BEFORE UPDATE ON registrations
    BEGIN SELECT RAISE(ABORT, 'a registration cannot be edited; run a new one'); END;
CREATE TABLE registration_poses (
    registration_id INTEGER NOT NULL REFERENCES registrations(id),
    evidence_id     INTEGER NOT NULL,
    scan_idx        INTEGER NOT NULL,
    pose            TEXT NOT NULL,
    verified        INTEGER NOT NULL,
    PRIMARY KEY (registration_id, evidence_id, scan_idx)
);
CREATE TRIGGER registration_poses_no_delete BEFORE DELETE ON registration_poses
    BEGIN SELECT RAISE(ABORT, 'registration poses are kept'); END;
CREATE TRIGGER registration_poses_immutable BEFORE UPDATE ON registration_poses
    BEGIN SELECT RAISE(ABORT, 'registration poses cannot be edited'); END;
";

const APPLIED_KEY: &str = "applied_registration";

/// A scan's pose in one registration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScanPose {
    pub evidence_id: i64,
    pub scan_idx: usize,
    /// Row-major 4 × 4, scan-local meters → project frame.
    pub pose: [f64; 16],
    /// Tied to the reference by trusted links (see `locus_register::pipeline`).
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegistrationRecord {
    pub id: i64,
    pub parent: Option<i64>,
    pub params: Value,
    pub result: Value,
    pub poses: Vec<ScanPose>,
    pub applied: bool,
    pub created_at: String,
    pub created_by: String,
}

impl Project {
    /// Store a registration run. `parent` is the registration whose links were edited to make
    /// this one, if any.
    pub fn record_registration(
        &mut self,
        parent: Option<i64>,
        params: &Value,
        result: &Value,
        poses: &[ScanPose],
    ) -> Result<i64> {
        let examiner = self.examiner().to_string();
        self.logged("registration.run", |tx| {
            tx.execute(
                "INSERT INTO registrations (parent, params, result, created_at, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    parent,
                    params.to_string(),
                    result.to_string(),
                    now(),
                    examiner
                ],
            )?;
            let id = tx.last_insert_rowid();
            for p in poses {
                tx.execute(
                    "INSERT INTO registration_poses
                     (registration_id, evidence_id, scan_idx, pose, verified)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id,
                        p.evidence_id,
                        p.scan_idx as i64,
                        serde_json::to_string(&p.pose)?,
                        p.verified
                    ],
                )?;
            }
            let details = json!({
                "id": id,
                "parent": parent,
                "params": params,
                "summary": result.get("summary"),
                "poses": poses,
            });
            Ok((id, details))
        })
    }

    /// Make registration `id` the one whose poses the scene uses, or `None` to go back to the
    /// poses stored in the evidence files.
    pub fn apply_registration(&mut self, id: Option<i64>) -> Result<()> {
        let previous = self.applied_registration()?;
        self.logged("registration.applied", |tx| {
            match id {
                Some(id) => {
                    let exists = tx
                        .query_row(
                            "SELECT 1 FROM registrations WHERE id = ?1",
                            [id],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some();
                    if !exists {
                        return Err(crate::Error::NotFound(format!("registration {id}")));
                    }
                    tx.execute(
                        "INSERT INTO settings (key, value) VALUES (?1, ?2)
                         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                        params![APPLIED_KEY, id.to_string()],
                    )?;
                }
                None => {
                    tx.execute("DELETE FROM settings WHERE key = ?1", [APPLIED_KEY])?;
                }
            }
            Ok(((), json!({ "id": id, "previous": previous })))
        })
    }

    /// Log that a report of registration `id` was written to `path`, with the file's hash, so
    /// a printed or shared copy can be matched to the project.
    pub fn record_report_export(
        &mut self,
        id: i64,
        path: &str,
        sha256: &str,
        bytes: u64,
    ) -> Result<()> {
        self.logged("report.exported", |_| {
            let details = json!({
                "report": "registration",
                "registration": id,
                "path": path,
                "sha256": sha256,
                "bytes": bytes,
            });
            Ok(((), details))
        })
    }

    pub fn applied_registration(&self) -> Result<Option<i64>> {
        Ok(self.setting(APPLIED_KEY)?.and_then(|v| v.parse().ok()))
    }

    pub fn registrations(&self) -> Result<Vec<RegistrationRecord>> {
        let applied = self.applied_registration()?;
        let mut stmt = self.conn.prepare(
            "SELECT id, parent, params, result, created_at, created_by
             FROM registrations ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = vec![];
        for (id, parent, params, result, created_at, created_by) in rows {
            out.push(RegistrationRecord {
                id,
                parent,
                params: serde_json::from_str(&params)?,
                result: serde_json::from_str(&result)?,
                poses: self.registration_poses(id)?,
                applied: applied == Some(id),
                created_at,
                created_by,
            });
        }
        Ok(out)
    }

    fn registration_poses(&self, id: i64) -> Result<Vec<ScanPose>> {
        let mut stmt = self.conn.prepare(
            "SELECT evidence_id, scan_idx, pose, verified FROM registration_poses
             WHERE registration_id = ?1 ORDER BY evidence_id, scan_idx",
        )?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = vec![];
        for (evidence_id, scan_idx, pose, verified) in rows {
            out.push(ScanPose {
                evidence_id,
                scan_idx: scan_idx as usize,
                pose: serde_json::from_str(&pose)?,
                verified,
            });
        }
        Ok(out)
    }

    /// The pose the scene uses for a scan: from the applied registration if it covers the
    /// scan, otherwise `source` (the pose stored in the evidence file).
    pub fn scan_pose(
        &self,
        evidence_id: i64,
        scan_idx: usize,
        source: [f64; 16],
    ) -> Result<[f64; 16]> {
        let Some(id) = self.applied_registration()? else {
            return Ok(source);
        };
        let pose: Option<String> = self
            .conn
            .query_row(
                "SELECT pose FROM registration_poses
                 WHERE registration_id = ?1 AND evidence_id = ?2 AND scan_idx = ?3",
                params![id, evidence_id, scan_idx as i64],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match pose {
            Some(p) => serde_json::from_str(&p)?,
            None => source,
        })
    }
}
