//! Writing out of the project: the case report, and exports. Each file written is hashed and
//! logged as `export.written` (a read of the project, so it doesn't move the state head).

use crate::commands::{blocking, err, CmdResult};
use locus_core::{AuditEntry, Project};
use locus_report::case::{CaseData, EvidenceRow, RecordRow};
use serde_json::Value;
use std::collections::HashMap;
use tauri::AppHandle;

/// The audit entries that recorded things, by (action, id field, id).
struct Entries(HashMap<(String, String), String>);

impl Entries {
    fn new(log: &[AuditEntry]) -> Entries {
        let mut m = HashMap::new();
        for e in log {
            let Ok(d) = serde_json::from_str::<Value>(&e.details) else {
                continue;
            };
            for key in ["id", "evidence_id", "revision"] {
                if let Some(v) = d.get(key).filter(|v| v.is_i64()) {
                    m.entry((format!("{}:{key}", e.action), v.to_string()))
                        .or_insert_with(|| format!("#{}, {}", e.seq, &e.hash[..16]));
                }
            }
        }
        Entries(m)
    }

    fn get(&self, actions: &[&str], key: &str, id: i64) -> String {
        actions
            .iter()
            .find_map(|a| self.0.get(&(format!("{a}:{key}"), id.to_string())))
            .cloned()
            .unwrap_or_else(|| "(not found)".into())
    }
}

fn case_data(p: &Project) -> CmdResult<CaseData> {
    let log = p.audit_log().map_err(err)?;
    let entries = Entries::new(&log);
    let head = p.state_head().map_err(err)?;
    let evidence_list = p.evidence().map_err(err)?;
    let name_of = |id: i64| {
        evidence_list
            .iter()
            .find(|e| e.id == id)
            .map_or(format!("#{id}"), |e| file_name(&e.original_path))
    };
    let integrity = log
        .iter()
        .rev()
        .find(|e| e.action == "evidence.verified")
        .and_then(|e| {
            let d: Value = serde_json::from_str(&e.details).ok()?;
            let failed = d["failed"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|f| {
                            let id = f["evidence_id"].as_i64().unwrap_or(0);
                            format!(
                                "#{id} {}: {}",
                                name_of(id),
                                f["result"]["status"].as_str().unwrap_or("not intact")
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some((
                e.timestamp.clone(),
                d["checked"].as_u64().unwrap_or(0) as usize,
                failed,
            ))
        });
    let evidence = evidence_list
        .iter()
        .map(|e| EvidenceRow {
            id: e.id,
            name: file_name(&e.original_path),
            sha256: e.sha256.clone(),
            size: e.size,
            format: e.contents.format.clone(),
            unit: e
                .unit
                .map_or("no unit".into(), |u| format!("{u:?}").to_lowercase()),
            imported: format!("{} by {}", e.imported_at, e.imported_by),
            entry: entries.get(&["evidence.imported"], "evidence_id", e.id),
        })
        .collect();

    let mut records = vec![];
    for r in p.registrations().map_err(err)? {
        records.push(RecordRow {
            kind: "Registrations".into(),
            id: r.id.to_string(),
            name: r
                .result
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("registration")
                .to_string(),
            sha256: "(its content is in its audit entry)".into(),
            made: format!("{} by {}", r.created_at, r.created_by),
            status: if r.applied {
                "applied".into()
            } else {
                String::new()
            },
            entry: entries.get(&["registration.run"], "id", r.id),
        });
    }
    for (kind, revs, act) in [
        ("Diagrams", p.diagrams().map_err(err)?, "diagram"),
        ("3D scenes", p.scenes().map_err(err)?, "scene"),
    ] {
        for r in revs {
            records.push(RecordRow {
                kind: kind.into(),
                id: format!("{} rev. {}", r.document_id, r.number),
                name: r.name.clone(),
                sha256: r.sha256.clone(),
                made: format!("{} by {}", r.created_at, r.created_by),
                status: String::new(),
                entry: entries.get(
                    &[&format!("{act}.revised"), &format!("{act}.created")],
                    "revision",
                    r.revision_id,
                ),
            });
        }
    }
    for a in p.analyses().map_err(err)? {
        records.push(RecordRow {
            kind: "Analyses".into(),
            id: a.id.to_string(),
            name: format!("{}: {}", a.tool, a.name),
            sha256: a.sha256.clone(),
            made: format!("{} by {}", a.created_at, a.created_by),
            status: a.withdrawn.as_ref().map_or(String::new(), |w| {
                format!("withdrawn {}: {}", w.at, w.reason)
            }),
            entry: entries.get(&["analysis.created"], "id", a.id),
        });
    }
    for m in p.measurements().map_err(err)? {
        records.push(RecordRow {
            kind: "Measurements".into(),
            id: m.id.to_string(),
            name: match measured(&m.kind, &m.result) {
                Some((v, sg, unit)) => format!("{} {v:.4} {unit} ± {sg:.4} {unit} (1σ)", m.kind),
                None => m.kind.clone(),
            },
            sha256: "(its content is in its audit entry)".into(),
            made: format!("{} by {}", m.created_at, m.created_by),
            status: String::new(),
            entry: entries.get(&["measurement.created"], "id", m.id),
        });
    }
    Ok(CaseData {
        project: p.name().map_err(err)?,
        case_number: p.setting("case_number").map_err(err)?,
        printed_by: p.examiner().to_string(),
        printed_at: locus_core::timestamp(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        audit_head: head.as_ref().map_or(String::new(), |e| e.hash.clone()),
        audit_entries: head.map_or(0, |e| e.seq as usize),
        integrity,
        evidence,
        records,
    })
}

/// A measurement's value, its 1σ and their unit, as the viewer shows them (angles in degrees).
pub(crate) fn measured(kind: &str, r: &Value) -> Option<(f64, f64, &'static str)> {
    let pair = |m: &Value| Some((m.get("value")?.as_f64()?, m.get("sigma")?.as_f64()?));
    match kind {
        "distance" => pair(r).map(|(v, s)| (v, s, "m")),
        "angle" => pair(r).map(|(v, s)| (v.to_degrees(), s.to_degrees(), "°")),
        "area" => pair(r.get("area")?).map(|(v, s)| (v, s, "m²")),
        "height" => pair(r.get("height")?).map(|(v, s)| (v, s, "m")),
        _ => None,
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Hash a written file and log it.
pub(crate) fn log_written(
    p: &mut Project,
    what: &str,
    path: &str,
    from: Value,
) -> CmdResult<String> {
    let (sha, bytes) =
        locus_core::hash::sha256_file(std::path::Path::new(path), &mut |_| {}).map_err(err)?;
    p.record_export(what, path, &sha, bytes, from)
        .map_err(err)?;
    Ok(sha)
}

/// Print the case report to `path`; returns its SHA-256.
#[tauri::command]
pub async fn case_report(app: AppHandle, path: String) -> CmdResult<String> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let data = case_data(p)?;
        let head = data.audit_head.clone();
        let out = locus_report::analysis::pdf(&locus_report::case::report(&data), vec![])?;
        std::fs::write(&path, &out.pdf).map_err(|e| format!("Could not write {path}: {e}"))?;
        log_written(
            p,
            "case report",
            &path,
            serde_json::json!({ "state_head": head }),
        )
    })
    .await
}

/// A CSV field: quoted when it holds a comma, quote or line break.
fn csv(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Every measurement as CSV (UTF-8, one row each): value and 1σ in their unit, the points as
/// stored (JSON), who made it and when, and the audit entry that recorded it.
#[tauri::command]
pub async fn measurements_csv(app: AppHandle, path: String) -> CmdResult<String> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let entries = Entries::new(&p.audit_log().map_err(err)?);
        let mut out = String::from(
            "id,kind,value,sigma_1,unit,point_sigma_m,photogrammetry_analysis,made_at,made_by,audit_entry,points\r\n",
        );
        let list = p.measurements().map_err(err)?;
        for m in &list {
            let (v, sg, unit) = measured(&m.kind, &m.result).unwrap_or((f64::NAN, f64::NAN, ""));
            let row = [
                m.id.to_string(),
                m.kind.clone(),
                format!("{v:.6}"),
                format!("{sg:.6}"),
                unit.to_string(),
                m.result
                    .get("sigma_point_m")
                    .map_or(String::new(), |x| x.to_string()),
                m.result
                    .pointer("/photogrammetry/analysis")
                    .map_or(String::new(), |x| x.to_string()),
                m.created_at.clone(),
                m.created_by.clone(),
                entries.get(&["measurement.created"], "id", m.id),
                m.points.to_string(),
            ];
            out += &row.iter().map(|f| csv(f)).collect::<Vec<_>>().join(",");
            out += "\r\n";
        }
        std::fs::write(&path, out.as_bytes()).map_err(|e| format!("Could not write {path}: {e}"))?;
        log_written(
            p,
            "measurements CSV",
            &path,
            serde_json::json!({ "measurements": list.len() }),
        )
    })
    .await
}
