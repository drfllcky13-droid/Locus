//! The project's point clouds as one scene: each scan's octree with its pose and the
//! points its active cleanup operations removed.
//!
//! Coordinates: octrees hold scan-local meters (source value × unit). Project coordinates
//! are `pose × local`, computed here in f64. The GPU only ever sees node-relative f32.

use crate::{build, BuildOptions, BuildProgress, Error, Meta, Octree, Rec, Result};
use locus_core::{EvidenceRecord, Project};
use roaring::RoaringBitmap;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Row-major 4×4 rigid transform (scan-local → project).
pub type Pose = [f64; 16];

pub fn apply(m: &Pose, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| {
        m[r * 4] * p[0] + m[r * 4 + 1] * p[1] + m[r * 4 + 2] * p[2] + m[r * 4 + 3]
    })
}

/// Inverse of a rigid transform: Rᵀ and −Rᵀt.
pub fn invert_rigid(m: &Pose) -> Pose {
    let r = |i: usize, j: usize| m[j * 4 + i]; // transposed rotation
    let t = [m[3], m[7], m[11]];
    let mut out = [0.0; 16];
    for i in 0..3 {
        for j in 0..3 {
            out[i * 4 + j] = r(i, j);
        }
        out[i * 4 + 3] = -(r(i, 0) * t[0] + r(i, 1) * t[1] + r(i, 2) * t[2]);
    }
    out[15] = 1.0;
    out
}

/// Called with each point (project frame), its colour and its intensity.
pub type PointVisitor<'a> =
    dyn FnMut([f64; 3], Option<[u8; 3]>, Option<u16>) -> std::result::Result<(), String> + 'a;

/// A scan in the scene: evidence id and scan index, written "e-s" in URLs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ScanKey {
    pub evidence_id: i64,
    pub scan_idx: usize,
}

impl ScanKey {
    pub fn parse(s: &str) -> Option<ScanKey> {
        let (e, i) = s.split_once('-')?;
        Some(ScanKey {
            evidence_id: e.parse().ok()?,
            scan_idx: i.parse().ok()?,
        })
    }
}

impl std::fmt::Display for ScanKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.evidence_id, self.scan_idx)
    }
}

pub struct ScanCloud {
    pub key: ScanKey,
    pub name: String,
    pub tree: Octree,
    pub pose: Pose,
    pub removed: RoaringBitmap,
    /// Bumped whenever `removed` changes, so stale GPU nodes can be detected.
    pub revision: u64,
}

impl ScanCloud {
    /// Axis-aligned bounds of the octree cube in project coordinates.
    pub fn project_bounds(&self) -> ([f64; 3], [f64; 3]) {
        let (min, s) = (self.tree.meta.min, self.tree.meta.size);
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for c in 0..8 {
            let corner = [
                min[0] + s * (c & 1) as f64,
                min[1] + s * ((c >> 1) & 1) as f64,
                min[2] + s * ((c >> 2) & 1) as f64,
            ];
            let p = apply(&self.pose, corner);
            for d in 0..3 {
                lo[d] = lo[d].min(p[d]);
                hi[d] = hi[d].max(p[d]);
            }
        }
        (lo, hi)
    }
}

#[derive(Default)]
pub struct Scene {
    pub scans: BTreeMap<ScanKey, ScanCloud>,
}

/// A picked point, fully resolved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Resolved {
    pub scan: ScanKey,
    /// Record number in the source file.
    pub index: u32,
    /// Scan-local meters as stored (source × unit).
    pub local: [f64; 3],
    /// Project frame, meters: `pose × local`.
    pub project: [f64; 3],
}

impl Scene {
    /// Load every built octree in the project with its pose and removed points.
    pub fn load(project: &Project) -> Result<Scene> {
        let evidence: BTreeMap<i64, EvidenceRecord> = project
            .evidence()
            .map_err(core_err)?
            .into_iter()
            .map(|e| (e.id, e))
            .collect();
        let mut scene = Scene::default();
        for o in project.octrees().map_err(core_err)? {
            if o.status != "built" {
                continue;
            }
            let Some(rec) = evidence.get(&o.evidence_id) else {
                continue;
            };
            let Some(info) = rec.contents.scans.get(o.scan_idx) else {
                continue;
            };
            let key = ScanKey {
                evidence_id: o.evidence_id,
                scan_idx: o.scan_idx,
            };
            let tree = Octree::open(&project.octree_dir(key.evidence_id, key.scan_idx))?;
            scene.scans.insert(
                key,
                ScanCloud {
                    key,
                    name: info.name.clone(),
                    tree,
                    // The applied registration's pose, else the one in the file.
                    pose: project
                        .scan_pose(key.evidence_id, key.scan_idx, info.pose)
                        .map_err(core_err)?,
                    removed: RoaringBitmap::new(),
                    revision: 0,
                },
            );
        }
        scene.reload_removed(project)?;
        Ok(scene)
    }

    /// Rebuild each scan's removed set from the active cleanup operations, checking every
    /// bitmap file against the hash recorded when it was written.
    pub fn reload_removed(&mut self, project: &Project) -> Result<()> {
        let mut sets: BTreeMap<ScanKey, RoaringBitmap> = BTreeMap::new();
        for op in project.cleanups().map_err(core_err)? {
            if !op.active {
                continue;
            }
            for s in op.scans {
                let path = project.root().join(&s.file);
                let bytes = fs::read(&path)?;
                let (sha, _) = locus_core::hash::sha256_reader(&bytes[..], &mut |_| {})?;
                if sha != s.sha256 {
                    return Err(Error::Invalid(format!(
                        "cleanup file {} has been altered (SHA-256 {sha}, recorded {})",
                        s.file, s.sha256
                    )));
                }
                let bm = RoaringBitmap::deserialize_from(&bytes[..])?;
                *sets
                    .entry(ScanKey {
                        evidence_id: s.evidence_id,
                        scan_idx: s.scan_idx,
                    })
                    .or_default() |= bm;
            }
        }
        for (key, cloud) in &mut self.scans {
            let new = sets.remove(key).unwrap_or_default();
            if new != cloud.removed {
                cloud.removed = new;
                cloud.revision += 1;
            }
        }
        Ok(())
    }

    fn scan(&self, key: ScanKey) -> Result<&ScanCloud> {
        self.scans
            .get(&key)
            .ok_or_else(|| Error::Invalid(format!("no octree for scan {key}")))
    }

    /// Render origin: the centre of all scans' project-frame bounds. Kept in f64 on the CPU;
    /// everything sent to the GPU is relative to it.
    /// Every visible point (project frame) within `radius` of `c`.
    pub fn points_within(&self, c: [f64; 3], radius: f64) -> Result<Vec<[f64; 3]>> {
        let (lo, hi) = (c.map(|v| v - radius), c.map(|v| v + radius));
        let mut out = vec![];
        for cloud in self.scans.values() {
            crate::cleanup::for_points_near(cloud, lo, hi, &mut |_, _, p| {
                if (0..3).map(|k| (p[k] - c[k]).powi(2)).sum::<f64>() <= radius * radius {
                    out.push(p);
                }
            })?;
        }
        Ok(out)
    }

    /// Visit every visible point (cleanup applied) of the given scans, in the project frame,
    /// with its colour and intensity when the scan has them. Node by node, so a whole cloud
    /// never has to be in memory.
    pub fn visit_points(
        &self,
        keys: &[ScanKey],
        f: &mut PointVisitor,
    ) -> std::result::Result<u64, String> {
        let mut n = 0u64;
        for key in keys {
            let c = self
                .scans
                .get(key)
                .ok_or_else(|| format!("no scan {key} in the open project"))?;
            for i in 0..c.tree.nodes.len() {
                let pts = c.tree.read(i).map_err(|e| e.to_string())?;
                for k in 0..pts.xyz.len() {
                    if c.removed.contains(pts.index[k]) {
                        continue;
                    }
                    f(
                        crate::scene::apply(&c.pose, pts.xyz[k]),
                        pts.rgb.get(k).copied(),
                        pts.intensity.get(k).copied(),
                    )?;
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// The scans in the scene: key and name.
    pub fn scan_keys(&self) -> Vec<(ScanKey, String)> {
        self.scans
            .iter()
            .map(|(k, c)| (*k, c.name.clone()))
            .collect()
    }

    /// Every visible point of one scan (project frame) inside the box `lo`–`hi`.
    pub fn scan_points_in(
        &self,
        key: ScanKey,
        lo: [f64; 3],
        hi: [f64; 3],
    ) -> Result<Vec<[f64; 3]>> {
        let cloud = self
            .scans
            .get(&key)
            .ok_or_else(|| Error::Invalid(format!("no scan {key}")))?;
        let mut out = vec![];
        crate::cleanup::for_points_near(cloud, lo, hi, &mut |_, _, p| {
            if (0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k]) {
                out.push(p);
            }
        })?;
        Ok(out)
    }

    pub fn origin(&self) -> [f64; 3] {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for c in self.scans.values() {
            let (a, b) = c.project_bounds();
            for d in 0..3 {
                lo[d] = lo[d].min(a[d]);
                hi[d] = hi[d].max(b[d]);
            }
        }
        if self.scans.is_empty() {
            return [0.0; 3];
        }
        std::array::from_fn(|d| (lo[d] + hi[d]) / 2.0)
    }

    pub fn serve(&self, key: ScanKey, node: usize) -> Result<Vec<u8>> {
        let c = self.scan(key)?;
        c.tree.serve(node, Some(&c.removed))
    }

    /// Resolve a GPU pick (scan, node, position in the served node) to the stored point.
    /// `revision` must match the scan's current revision, or the node the GPU drew is stale.
    pub fn resolve(&self, key: ScanKey, node: usize, k: usize, revision: u64) -> Result<Resolved> {
        let c = self.scan(key)?;
        if revision != c.revision {
            return Err(Error::Invalid(
                "the view is still updating after a cleanup; pick again".into(),
            ));
        }
        let (index, local) = c.tree.resolve(node, k, Some(&c.removed))?;
        Ok(Resolved {
            scan: key,
            index,
            local,
            project: apply(&c.pose, local),
        })
    }
}

fn core_err(e: locus_core::Error) -> Error {
    Error::Invalid(e.to_string())
}

impl From<locus_io::Error> for Error {
    fn from(e: locus_io::Error) -> Self {
        Error::Invalid(e.to_string())
    }
}

/// Build (or rebuild) the octree for one scan of an evidence item into the project's
/// derived folder. Reads the evidence copy, never the original.
pub fn build_scan(
    root: &Path,
    rec: &EvidenceRecord,
    scan_idx: usize,
    progress: &mut dyn FnMut(BuildProgress),
) -> Result<Meta> {
    let info = rec
        .contents
        .scans
        .get(scan_idx)
        .ok_or_else(|| Error::Invalid(format!("evidence {} has no scan {scan_idx}", rec.id)))?;
    let unit = rec
        .unit
        .ok_or_else(|| Error::Invalid("evidence has no unit".into()))?
        .meters();
    let bounds = info
        .bounds
        .ok_or_else(|| Error::Invalid("scan has no valid points".into()))?;
    let lo = bounds.min.map(|v| v * unit);
    let extent = (0..3)
        .map(|d| (bounds.max[d] - bounds.min[d]) * unit)
        .fold(0.0, f64::max);
    // Pad so points on the far faces fall inside, and give flat or single-point scans a size.
    let size = (extent * (1.0 + 1e-9)).max(1e-3) + 1e-6;
    let path = root.join(&rec.stored_path);
    let opts = BuildOptions {
        has_color: info.attributes.iter().any(|a| a == "color"),
        has_intensity: info.attributes.iter().any(|a| a == "intensity"),
        ..BuildOptions::default()
    };
    let dir = Project::octree_dir_in(root, rec.id, scan_idx);
    let tmp = dir.with_extension("building");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    let meta = build(
        &tmp,
        lo,
        size,
        &opts,
        &mut |f| {
            let mut too_many = false;
            locus_io::for_each_point(&path, &rec.contents, scan_idx, &mut |_| {}, &mut |p| {
                too_many |= p.index > u32::MAX as u64;
                f(Rec {
                    p: p.p.map(|v| v * unit),
                    index: p.index as u32,
                    rgb: p.rgb.unwrap_or_default(),
                    intensity: p.intensity.unwrap_or(0),
                });
            })?;
            if too_many {
                return Err(Error::Invalid("scan has more than 2^32 records".into()));
            }
            Ok(())
        },
        progress,
    )?;
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::rename(&tmp, &dir)?;
    Ok(meta)
}

/// Write a removed-points bitmap under `derived/cleanup/` and return (relative path, SHA-256).
pub fn write_bitmap(root: &Path, name: &str, bm: &RoaringBitmap) -> Result<(String, String)> {
    let rel = format!("derived/cleanup/{name}.roar");
    let path = root.join(&rel);
    fs::create_dir_all(path.parent().unwrap())?;
    let mut bytes = vec![];
    bm.serialize_into(&mut bytes)?;
    fs::write(&path, &bytes)?;
    let (sha, _) = locus_core::hash::sha256_reader(&bytes[..], &mut |_| {})?;
    Ok((rel, sha))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_inverse_round_trips() {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        // 45° about z, then translate.
        let m = [
            h, -h, 0.0, 10.0, h, h, 0.0, -3.0, 0.0, 0.0, 1.0, 2.5, 0.0, 0.0, 0.0, 1.0,
        ];
        let p = [1.25, -7.5, 3.0];
        let q = apply(&invert_rigid(&m), apply(&m, p));
        assert!((0..3).all(|d| (q[d] - p[d]).abs() < 1e-12));
    }

    #[test]
    fn scan_keys_round_trip() {
        let k = ScanKey {
            evidence_id: 12,
            scan_idx: 3,
        };
        assert_eq!(ScanKey::parse(&k.to_string()), Some(k));
        assert_eq!(ScanKey::parse("x-1"), None);
    }
}
