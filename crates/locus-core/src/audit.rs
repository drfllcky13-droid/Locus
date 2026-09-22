//! Append-only, hash-chained audit log.
//!
//! Each entry's hash covers the previous entry's hash, so editing, deleting, inserting or
//! reordering any entry breaks every hash after it. The head (last seq and hash) is mirrored
//! into `meta`, which catches entries removed from the end of the chain.
//! See `docs/methods/audit-log.md` for what this does and does not prove.

use crate::project::{meta_get, meta_set, Error, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub(crate) const SCHEMA: &str = "
CREATE TABLE audit_log (
    seq       INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL,
    actor     TEXT NOT NULL,
    action    TEXT NOT NULL,
    details   TEXT NOT NULL,
    prev_hash TEXT NOT NULL,
    hash      TEXT NOT NULL
);
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
    BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
    BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;
";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEntry {
    pub seq: i64,
    /// UTC, RFC 3339 with milliseconds.
    pub timestamp: String,
    pub actor: String,
    pub action: String,
    /// JSON object, stored exactly as hashed.
    pub details: String,
    pub prev_hash: String,
    pub hash: String,
}

/// SHA-256 over length-prefixed fields, so no two different entries can hash the same input.
pub fn entry_hash(
    prev_hash: &str,
    seq: i64,
    timestamp: &str,
    actor: &str,
    action: &str,
    details: &str,
) -> String {
    let mut h = Sha256::new();
    let seq = seq.to_le_bytes();
    let fields: [&[u8]; 6] = [
        prev_hash.as_bytes(),
        &seq,
        timestamp.as_bytes(),
        actor.as_bytes(),
        action.as_bytes(),
        details.as_bytes(),
    ];
    for f in fields {
        h.update((f.len() as u64).to_le_bytes());
        h.update(f);
    }
    hex::encode(h.finalize())
}

fn head(conn: &Connection) -> Result<(i64, String)> {
    let seq = meta_get(conn, "audit_head_seq")?.map_or(Ok(0), |s| {
        s.parse()
            .map_err(|_| tampered(0, "the recorded head of the log is not a number"))
    })?;
    let hash = meta_get(conn, "audit_head_hash")?.unwrap_or_else(|| GENESIS.into());
    Ok((seq, hash))
}

/// Append an entry. Call inside the same transaction as the change it records.
pub(crate) fn append(
    conn: &Connection,
    actor: &str,
    action: &str,
    details: &serde_json::Value,
) -> Result<AuditEntry> {
    let (last, prev_hash) = head(conn)?;
    let seq = last + 1;
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let details = details.to_string();
    let hash = entry_hash(&prev_hash, seq, &timestamp, actor, action, &details);
    conn.execute(
        "INSERT INTO audit_log (seq, timestamp, actor, action, details, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![seq, timestamp, actor, action, details, prev_hash, hash],
    )?;
    meta_set(conn, "audit_head_seq", &seq.to_string())?;
    meta_set(conn, "audit_head_hash", &hash)?;
    Ok(AuditEntry {
        seq,
        timestamp,
        actor: actor.into(),
        action: action.into(),
        details,
        prev_hash,
        hash,
    })
}

pub(crate) fn entries(conn: &Connection) -> Result<Vec<AuditEntry>> {
    let mut stmt = conn.prepare(
        "SELECT seq, timestamp, actor, action, details, prev_hash, hash FROM audit_log ORDER BY seq",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(AuditEntry {
            seq: r.get(0)?,
            timestamp: r.get(1)?,
            actor: r.get(2)?,
            action: r.get(3)?,
            details: r.get(4)?,
            prev_hash: r.get(5)?,
            hash: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn tampered(seq: i64, reason: &str) -> Error {
    Error::Tampered {
        seq,
        reason: reason.into(),
    }
}

/// Walk the whole chain. Returns the first entry that fails, if any.
pub(crate) fn verify(conn: &Connection) -> Result<()> {
    let mut prev = GENESIS.to_string();
    let mut expected = 1;
    for e in entries(conn)? {
        if e.seq != expected {
            return Err(tampered(expected, "entry is missing or out of order"));
        }
        if e.prev_hash != prev {
            return Err(tampered(e.seq, "does not link to the previous entry"));
        }
        let actual = entry_hash(
            &e.prev_hash,
            e.seq,
            &e.timestamp,
            &e.actor,
            &e.action,
            &e.details,
        );
        if actual != e.hash {
            return Err(tampered(e.seq, "contents do not match their hash"));
        }
        prev = e.hash;
        expected += 1;
    }
    let (head_seq, head_hash) = head(conn)?;
    if head_seq != expected - 1 || head_hash != prev {
        return Err(tampered(
            expected,
            &format!(
                "log ends at entry {}, but the project records {head_seq} entries",
                expected - 1
            ),
        ));
    }
    Ok(())
}
