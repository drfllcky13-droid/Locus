//! Registration commands: run the registration pipeline on the open project's scans, store
//! the result, re-solve with links deleted or forced, and apply a registration to the scene.

use crate::commands::{blocking, err, CmdResult};
use locus_core::{Project, RegistrationRecord, ScanPose};
use locus_octree::scene::{ScanKey, Scene};
use locus_octree::Octree;
use locus_register::pipeline::{self, ControlPoint, Params, ScanInput};
use locus_register::posegraph::{self, Link, LinkKind, LinkReport, LinkStatus};
use locus_register::rigid::{from_row_major, to_row_major};
use locus_register::targets::Kind;
use nalgebra::{Isometry3, Matrix3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

/// What the examiner chose in the Register dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunParams {
    /// Sphere target radius (m), if spheres were used.
    pub sphere_radius: Option<f64>,
    /// Checkerboard edge (m), if boards were used.
    pub board_size: Option<f64>,
    /// Also register by cloud-to-cloud.
    pub cloud: bool,
    /// Use the poses stored in the files as rough starting poses.
    pub use_file_poses: bool,
    /// Precision of each cloud-link point (m).
    pub cloud_sigma: f64,
    /// Target distance tolerance (m).
    pub target_tolerance: f64,
    /// Most points loaded per scan; denser scans are thinned evenly.
    pub max_points: usize,
    /// Surveyed control, in the project frame.
    #[serde(default)]
    pub control: Vec<ControlInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlInput {
    pub name: String,
    /// "sphere" or "board".
    pub kind: String,
    pub position: [f64; 3],
    /// 1σ per axis (m).
    pub sigma: f64,
}

/// A registration as the UI shows it.
#[derive(Serialize)]
pub struct RegistrationView {
    #[serde(flatten)]
    record: RegistrationRecord,
}

fn view(p: &Project) -> CmdResult<Vec<RegistrationView>> {
    Ok(p.registrations()
        .map_err(err)?
        .into_iter()
        .map(|record| RegistrationView { record })
        .collect())
}

struct ScanSource {
    key: ScanKey,
    name: String,
    tree: Octree,
    removed: roaring::RoaringBitmap,
    file_pose: [f64; 16],
}

/// Every point of a scan that no cleanup removed, in scan-local meters, thinned evenly to at
/// most `max` points.
fn load(src: &ScanSource, max: usize) -> CmdResult<(ScanInput, usize)> {
    let total: u64 = src.tree.nodes.iter().map(|n| n.count as u64).sum();
    // ponytail: even thinning by record number; a voxel filter would keep far surfaces better.
    let step = (total as usize).div_ceil(max.max(1)).max(1);
    let (mut points, mut intensity) = (vec![], vec![]);
    for i in 0..src.tree.nodes.len() {
        let n = src.tree.read(i).map_err(err)?;
        for k in 0..n.xyz.len() {
            let idx = n.index[k];
            if src.removed.contains(idx) || !(idx as usize).is_multiple_of(step) {
                continue;
            }
            points.push(n.xyz[k]);
            intensity.push(n.intensity[k] as f64);
        }
    }
    let used = points.len();
    Ok((ScanInput { points, intensity }, used))
}

fn summary(links: &[Link], reports: &[LinkReport], verified: &[bool]) -> Value {
    let count = |s: LinkStatus| reports.iter().filter(|r| r.status == s).count();
    let target: Vec<&LinkReport> = links
        .iter()
        .zip(reports)
        .filter(|(l, r)| l.kind != LinkKind::Cloud && r.status != LinkStatus::Flagged)
        .map(|(_, r)| r)
        .collect();
    let mean_rms = if target.is_empty() {
        None
    } else {
        Some(target.iter().map(|r| r.rms).sum::<f64>() / target.len() as f64)
    };
    json!({
        "links": links.len(),
        "ok": count(LinkStatus::Ok),
        "flagged": count(LinkStatus::Flagged),
        "untested": count(LinkStatus::Untested),
        "shape_only": links.iter().filter(|l| l.shape_only).count(),
        "target_rms_mean_m": mean_rms,
        "target_residual_max_m": target.iter().map(|r| r.max).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))),
        "unverified_scans": verified.iter().filter(|v| !**v).count(),
    })
}

/// Solution poses → project frame. Without control, scan 0 keeps its file pose and the rest
/// follow it; with control, the solution is already in the (project) world frame.
fn project_poses(solution: &[Isometry3<f64>], anchor: Option<[f64; 16]>) -> Vec<[f64; 16]> {
    let base = match anchor {
        Some(f0) => from_row_major(&f0) * solution[0].inverse(),
        None => Isometry3::identity(),
    };
    solution.iter().map(|s| to_row_major(&(base * s))).collect()
}

/// A solved graph and what it was solved from, ready to store.
struct Outcome<'a> {
    /// Per scan: key, name, points used.
    keys: &'a [(ScanKey, String, usize)],
    links: &'a [Link],
    solution: &'a posegraph::Solution,
    verified: &'a [bool],
    /// Detected targets and ICP overlaps, for the report.
    extra: Value,
    /// Scan 0's file pose when the frame is the first scan's (no control).
    anchor: Option<[f64; 16]>,
}

fn record(p: &mut Project, parent: Option<i64>, params: Value, o: Outcome) -> CmdResult<i64> {
    let Outcome {
        keys,
        links,
        solution,
        verified,
        extra,
        anchor,
    } = o;
    let poses = project_poses(&solution.poses, anchor);
    let result = json!({
        "scans": keys.iter().map(|(k, name, used)| json!({ "evidence_id": k.evidence_id, "scan_idx": k.scan_idx, "name": name, "points_used": used })).collect::<Vec<_>>(),
        "links": links,
        "reports": solution.links,
        "iterations": solution.iterations,
        "solution_poses": solution.poses.iter().map(to_row_major).collect::<Vec<_>>(),
        "anchor": anchor,
        "verified": verified,
        "summary": summary(links, &solution.links, verified),
        "extra": extra,
    });
    let scan_poses: Vec<ScanPose> = keys
        .iter()
        .zip(&poses)
        .zip(verified)
        .map(|(((k, _, _), pose), v)| ScanPose {
            evidence_id: k.evidence_id,
            scan_idx: k.scan_idx,
            pose: *pose,
            verified: *v,
        })
        .collect();
    p.record_registration(parent, &params, &result, &scan_poses)
        .map_err(err)
}

/// Register every scan of the open project. The result is stored, not applied.
#[tauri::command]
pub async fn registration_run(
    app: AppHandle,
    params: RunParams,
) -> CmdResult<Vec<RegistrationView>> {
    let emitter = app.clone();
    blocking(app, move |s| {
        let progress = |stage: &str| {
            let _ = emitter.emit("registration-progress", stage);
        };
        // Gather what's needed, then release the locks for the long part.
        let sources: Vec<ScanSource> = {
            let guard = s.project.lock().unwrap();
            let p = guard.as_ref().ok_or("Open or create a project first.")?;
            let evidence = p.evidence().map_err(err)?;
            let scene = s.scene.read().unwrap();
            scene
                .scans
                .values()
                .map(|c| {
                    let rec = evidence.iter().find(|e| e.id == c.key.evidence_id).ok_or("scan without evidence")?;
                    Ok(ScanSource {
                        key: c.key,
                        name: c.name.clone(),
                        tree: Octree::open(&c.tree.dir).map_err(err)?,
                        removed: c.removed.clone(),
                        file_pose: rec.contents.scans[c.key.scan_idx].pose,
                    })
                })
                .collect::<CmdResult<_>>()?
        };
        if sources.len() < 2 {
            return Err("Registration needs at least two scans with built octrees.".into());
        }
        let mut scans = vec![];
        let mut keys = vec![];
        for (i, src) in sources.iter().enumerate() {
            progress(&format!("Loading scan {} of {}", i + 1, sources.len()));
            let (input, used) = load(src, params.max_points)?;
            keys.push((src.key, src.name.clone(), used));
            scans.push(input);
        }
        let rough: Option<Vec<Isometry3<f64>>> = params
            .use_file_poses
            .then(|| sources.iter().map(|s| from_row_major(&s.file_pose)).collect());
        let control: Vec<ControlPoint> = params
            .control
            .iter()
            .map(|c| ControlPoint {
                kind: if c.kind == "board" { Kind::Board } else { Kind::Sphere },
                position: c.position,
                covariance: Matrix3::identity() * c.sigma * c.sigma,
            })
            .collect();
        let p = Params {
            sphere_radius: params.sphere_radius,
            board_size: params.board_size,
            target_tolerance: params.target_tolerance,
            cloud: params.cloud,
            cloud_sigma: params.cloud_sigma,
            ..Params::default()
        };
        progress("Detecting targets and matching scans");
        let reg = pipeline::register(&scans, rough.as_deref(), &control, &p)
            .map_err(|e| format!("Registration failed: {e:?}"))?;
        progress("Saving");
        let has_control = reg.links.iter().any(|l| l.b.is_none());
        let anchor = (!has_control).then_some(sources[0].file_pose);
        let targets: Vec<Value> = reg
            .targets
            .iter()
            .map(|ts| {
                json!(ts
                    .iter()
                    .map(|(t, _)| json!({ "kind": format!("{:?}", t.kind), "position": t.position, "sigma": t.sigma }))
                    .collect::<Vec<_>>())
            })
            .collect();
        let mut guard = s.project.lock().unwrap();
        let pr = guard.as_mut().ok_or("The project was closed.")?;
        record(
            pr,
            None,
            serde_json::to_value(&params).map_err(err)?,
            Outcome {
                keys: &keys,
                links: &reg.links,
                solution: &reg.solution,
                verified: &reg.verified,
                extra: json!({ "targets": targets, "overlap": reg.overlap }),
                anchor,
            },
        )?;
        view(pr)
    })
    .await
}

/// Re-solve a stored registration with some links deleted and some forced (never flagged or
/// set aside). The result is a new registration naming the old one as its parent.
#[tauri::command]
pub async fn registration_edit(
    app: AppHandle,
    id: i64,
    delete: Vec<usize>,
    force: Vec<usize>,
) -> CmdResult<Vec<RegistrationView>> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let old = p
            .registrations()
            .map_err(err)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or(format!("No registration {id}."))?;
        let r = &old.result;
        let mut links: Vec<Link> = serde_json::from_value(r["links"].clone()).map_err(err)?;
        for &i in &force {
            links.get_mut(i).ok_or(format!("No link {i}."))?.forced = true;
        }
        let keep: Vec<Link> = links
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !delete.contains(i))
            .map(|(_, l)| l)
            .collect();
        let init: Vec<[f64; 16]> =
            serde_json::from_value(r["solution_poses"].clone()).map_err(err)?;
        let init: Vec<Isometry3<f64>> = init.iter().map(from_row_major).collect();
        let solution = posegraph::solve(&init, &keep).map_err(|e| match e {
            posegraph::GraphError::Disconnected(s) => {
                format!("Without those links, scans {s:?} are no longer connected.")
            }
            posegraph::GraphError::Singular => {
                "The remaining links don't determine the poses.".into()
            }
        })?;
        let verified = pipeline::verified(init.len(), &keep, &solution);
        let keys: Vec<(ScanKey, String, usize)> = r["scans"]
            .as_array()
            .ok_or("stored registration has no scans")?
            .iter()
            .map(|v| {
                (
                    ScanKey {
                        evidence_id: v["evidence_id"].as_i64().unwrap_or_default(),
                        scan_idx: v["scan_idx"].as_u64().unwrap_or_default() as usize,
                    },
                    v["name"].as_str().unwrap_or_default().to_string(),
                    v["points_used"].as_u64().unwrap_or_default() as usize,
                )
            })
            .collect();
        let anchor: Option<[f64; 16]> = serde_json::from_value(r["anchor"].clone()).map_err(err)?;
        record(
            p,
            Some(id),
            json!({ "edits": { "delete": delete, "force": force } }),
            Outcome {
                keys: &keys,
                links: &keep,
                solution: &solution,
                verified: &verified,
                extra: r["extra"].clone(),
                anchor,
            },
        )?;
        view(p)
    })
    .await
}

#[tauri::command]
pub async fn registrations(app: AppHandle) -> CmdResult<Vec<RegistrationView>> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        view(guard.as_ref().ok_or("Open or create a project first.")?)
    })
    .await
}

/// Use registration `id`'s poses for the scene, or `None` for the poses in the files.
#[tauri::command]
pub async fn registration_apply(
    app: AppHandle,
    id: Option<i64>,
) -> CmdResult<Vec<RegistrationView>> {
    let emitter = app.clone();
    let out = blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.apply_registration(id).map_err(err)?;
        *s.scene.write().unwrap() = Scene::load(p).map_err(err)?;
        view(p)
    })
    .await?;
    let _ = emitter.emit("scene-changed", ());
    Ok(out)
}

/// Settings of a run as the report states them.
fn settings(params: &Value) -> Vec<(String, String)> {
    let mm = |v: &Value, k: f64| v.as_f64().map(|x| format!("{:.1} mm", x * k));
    let mut out = vec![];
    if let Some(e) = params.get("edits") {
        out.push(("Links deleted".into(), e["delete"].to_string()));
        out.push(("Links forced".into(), e["force"].to_string()));
        return out;
    }
    let yes = |k: &str| {
        if params[k].as_bool() == Some(true) {
            "yes"
        } else {
            "no"
        }
        .to_string()
    };
    out.push((
        "Sphere diameter".into(),
        mm(&params["sphere_radius"], 2000.0).unwrap_or("not used".into()),
    ));
    out.push((
        "Checkerboard edge".into(),
        mm(&params["board_size"], 1000.0).unwrap_or("not used".into()),
    ));
    out.push((
        "Target distance tolerance".into(),
        mm(&params["target_tolerance"], 1000.0).unwrap_or_default(),
    ));
    out.push(("Cloud-to-cloud".into(), yes("cloud")));
    out.push((
        "Cloud link point precision".into(),
        mm(&params["cloud_sigma"], 1000.0).unwrap_or_default(),
    ));
    out.push(("Rough poses from the files".into(), yes("use_file_poses")));
    out.push((
        "Most points per scan".into(),
        params["max_points"].to_string(),
    ));
    out.push((
        "Survey control points".into(),
        params["control"]
            .as_array()
            .map_or(0, |c| c.len())
            .to_string(),
    ));
    out
}

/// Write the report of registration `id` to `path` as PDF, and log it with its hash.
#[tauri::command]
pub async fn registration_report(app: AppHandle, id: i64, path: String) -> CmdResult<String> {
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        let rec = p
            .registrations()
            .map_err(err)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or(format!("No registration {id}."))?;
        let r = &rec.result;
        let links: Vec<Link> = serde_json::from_value(r["links"].clone()).map_err(err)?;
        let tests: Vec<LinkReport> = serde_json::from_value(r["reports"].clone()).map_err(err)?;
        let verified: Vec<bool> = serde_json::from_value(r["verified"].clone()).map_err(err)?;
        let overlap: Vec<Option<f64>> =
            serde_json::from_value(r["extra"]["overlap"].clone()).unwrap_or_default();
        let scans = r["scans"]
            .as_array()
            .ok_or("stored registration has no scans")?;
        // Project-frame poses, in the registration's scan order. Residuals don't depend on the
        // frame (without control the whole solution moves rigidly with the first scan).
        let poses: Vec<Isometry3<f64>> = scans
            .iter()
            .map(|v| {
                rec.poses
                    .iter()
                    .find(|q| {
                        Some(q.evidence_id) == v["evidence_id"].as_i64()
                            && Some(q.scan_idx as u64) == v["scan_idx"].as_u64()
                    })
                    .map(|q| from_row_major(&q.pose))
                    .ok_or("a scan's pose is missing")
            })
            .collect::<Result<_, _>>()?;
        let report = locus_register::report::build(&links, &poses, &tests, &overlap, &verified);
        let has_control = links.iter().any(|l| l.b.is_none());
        let audit_head = p
            .audit_log()
            .map_err(err)?
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_default();
        // A re-solve's settings are its edits; show the original run's settings too.
        let mut settings_rows = settings(&rec.params);
        if rec.params.get("edits").is_some() {
            let mut parent = rec.parent;
            let all = p.registrations().map_err(err)?;
            while let Some(pid) = parent {
                let Some(pr) = all.iter().find(|x| x.id == pid) else {
                    break;
                };
                if pr.params.get("edits").is_none() {
                    settings_rows.extend(settings(&pr.params));
                    break;
                }
                parent = pr.parent;
            }
        }
        let meta = locus_report::registration::Meta {
            project: p.name().map_err(err)?,
            registration_id: rec.id,
            parent: rec.parent,
            applied: rec.applied,
            created_at: rec.created_at.clone(),
            created_by: rec.created_by.clone(),
            printed_by: p.examiner().to_string(),
            printed_at: locus_core::timestamp(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            audit_head,
            frame: if has_control {
                "survey control".into()
            } else {
                "the first scan's file pose".into()
            },
            settings: settings_rows,
            scan_names: scans
                .iter()
                .map(|v| v["name"].as_str().unwrap_or_default().to_string())
                .collect(),
            scan_points: scans
                .iter()
                .map(|v| v["points_used"].as_u64().unwrap_or_default() as usize)
                .collect(),
            iterations: r["iterations"].as_u64().unwrap_or_default() as usize,
        };
        let out = locus_report::registration::pdf(&meta, &report)?;
        std::fs::write(&path, &out.pdf).map_err(|e| format!("Could not write {path}: {e}"))?;
        let (sha256, bytes) =
            locus_core::hash::sha256_reader(out.pdf.as_slice(), &mut |_| {}).map_err(err)?;
        p.record_report_export(id, &path, &sha256, bytes)
            .map_err(err)?;
        Ok(sha256)
    })
    .await
}
