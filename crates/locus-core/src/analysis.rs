//! Analysis runs, added in schema 6: each run of an analysis tool (trajectory, bloodstain
//! area of origin, camera matching and height) is one immutable record with everything
//! needed to defend it: the inputs as resolved from stored data, the parameters, the results
//! with their uncertainties, and the method's assumptions and limitations. A record is
//! hashed (SHA-256 of its stored JSON) and audit-logged. Re-running with changes makes a new
//! record that names the one it revises. A record can be withdrawn with a reason (logged),
//! never edited or deleted.

use crate::project::{now, Project, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA_V6: &str = "
CREATE TABLE analyses (
    id              INTEGER PRIMARY KEY,
    tool            TEXT NOT NULL,
    method          TEXT NOT NULL,
    name            TEXT NOT NULL,
    record          TEXT NOT NULL,
    sha256          TEXT NOT NULL,
    revises         INTEGER REFERENCES analyses(id),
    created_at      TEXT NOT NULL,
    created_by      TEXT NOT NULL,
    withdrawn_at    TEXT,
    withdrawn_by    TEXT,
    withdrawn_why   TEXT
);
CREATE TRIGGER analyses_no_delete BEFORE DELETE ON analyses
    BEGIN SELECT RAISE(ABORT, 'analyses are withdrawn, not deleted'); END;
CREATE TRIGGER analyses_immutable
    BEFORE UPDATE OF tool, method, name, record, sha256, revises, created_at, created_by ON analyses
    BEGIN SELECT RAISE(ABORT, 'an analysis record cannot be edited; run it again'); END;
";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalysisRecord {
    pub id: i64,
    pub tool: String,
    /// Tool and method version, e.g. `trajectory/1`: which formulas produced the record.
    pub method: String,
    pub name: String,
    /// `{ inputs, parameters, result, assumptions, limitations, … }` as the tool wrote it.
    pub record: Value,
    pub sha256: String,
    pub revises: Option<i64>,
    pub created_at: String,
    pub created_by: String,
    pub withdrawn: Option<Withdrawal>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Withdrawal {
    pub at: String,
    pub by: String,
    pub reason: String,
}

const COLS: &str = "id, tool, method, name, record, sha256, revises, created_at, created_by,
                    withdrawn_at, withdrawn_by, withdrawn_why";

fn row(r: &rusqlite::Row) -> rusqlite::Result<(AnalysisRecord, String)> {
    let at: Option<String> = r.get(9)?;
    Ok((
        AnalysisRecord {
            id: r.get(0)?,
            tool: r.get(1)?,
            method: r.get(2)?,
            name: r.get(3)?,
            record: Value::Null,
            sha256: r.get(5)?,
            revises: r.get(6)?,
            created_at: r.get(7)?,
            created_by: r.get(8)?,
            withdrawn: at.map(|at| Withdrawal {
                at,
                by: r.get(10).unwrap_or_default(),
                reason: r.get(11).unwrap_or_default(),
            }),
        },
        r.get(4)?,
    ))
}

impl Project {
    /// Store a tool's run. `record` must be a JSON object; its text is hashed as stored.
    pub fn add_analysis(
        &mut self,
        tool: &str,
        method: &str,
        name: &str,
        record: &Value,
        revises: Option<i64>,
    ) -> Result<AnalysisRecord> {
        if let Some(r) = revises {
            self.analysis(r)?
                .ok_or_else(|| crate::Error::NotFound(format!("analysis {r}")))?;
        }
        let text = record.to_string();
        let sha = hex::encode(Sha256::digest(text.as_bytes()));
        let (at, by) = (now(), self.examiner().to_string());
        let id = self.logged("analysis.created", |tx| {
            tx.execute(
                "INSERT INTO analyses (tool, method, name, record, sha256, revises, created_at, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![tool, method, name, text, sha, revises, at, by],
            )?;
            let id = tx.last_insert_rowid();
            Ok((
                id,
                json!({
                    "id": id,
                    "tool": tool,
                    "method": method,
                    "name": name,
                    "sha256": sha,
                    "revises": revises,
                    "result": record.get("summary").cloned().unwrap_or(Value::Null),
                }),
            ))
        })?;
        Ok(self.analysis(id)?.expect("just stored"))
    }

    pub fn analysis(&self, id: i64) -> Result<Option<AnalysisRecord>> {
        let got = self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM analyses WHERE id = ?1"),
                [id],
                row,
            )
            .optional()?;
        Ok(match got {
            Some((mut a, text)) => {
                a.record = serde_json::from_str(&text)?;
                Some(a)
            }
            None => None,
        })
    }

    /// Every analysis run, oldest first, withdrawn ones included (marked).
    pub fn analyses(&self) -> Result<Vec<AnalysisRecord>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLS} FROM analyses ORDER BY id"))?;
        let rows = stmt.query_map([], row)?;
        let mut out = vec![];
        for r in rows {
            let (mut a, text) = r?;
            a.record = serde_json::from_str(&text)?;
            out.push(a);
        }
        Ok(out)
    }

    /// Withdraw a run (it stays readable, marked withdrawn, with the reason).
    pub fn withdraw_analysis(&mut self, id: i64, reason: &str) -> Result<()> {
        if reason.trim().is_empty() {
            return Err(crate::Error::Invalid("a withdrawal needs a reason".into()));
        }
        let (at, by) = (now(), self.examiner().to_string());
        self.logged("analysis.withdrawn", |tx| {
            let n = tx.execute(
                "UPDATE analyses SET withdrawn_at = ?1, withdrawn_by = ?2, withdrawn_why = ?3
                 WHERE id = ?4 AND withdrawn_at IS NULL",
                params![at, by, reason, id],
            )?;
            if n == 0 {
                return Err(crate::Error::NotFound(format!(
                    "analysis {id} (or already withdrawn)"
                )));
            }
            Ok(((), json!({ "id": id, "reason": reason })))
        })
    }

    /// Log that a run's report was written to `path`, with the file's hash.
    pub fn record_analysis_report(
        &mut self,
        id: i64,
        path: &str,
        sha256: &str,
        bytes: u64,
    ) -> Result<()> {
        let a = self
            .analysis(id)?
            .ok_or_else(|| crate::Error::NotFound(format!("analysis {id}")))?;
        self.logged("analysis.reported", |_| {
            Ok((
                (),
                json!({
                    "id": id,
                    "tool": a.tool,
                    "record_sha256": a.sha256,
                    "path": path,
                    "sha256": sha256,
                    "bytes": bytes,
                }),
            ))
        })
    }
}
