//! A case package's manifest: every file in the package with its SHA-256 and size, and what
//! the package was made from. The package hash is the SHA-256 of `manifest.json` as written,
//! so it covers every file. The viewer re-hashes every file against it on opening.

use crate::hash::sha256_file;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub const MANIFEST: &str = "manifest.json";
pub const KIND: &str = "locus-case-package";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageFile {
    /// Relative to the package folder, with forward slashes.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub kind: String,
    pub version: u32,
    pub project: String,
    pub case_number: Option<String>,
    pub made_by: String,
    pub made_at: String,
    pub app_version: String,
    /// The source project's state head when the package was made.
    pub source_state_head: String,
    pub evidence_included: bool,
    /// Renders whose files couldn't be included (missing, or changed since they were logged).
    #[serde(default)]
    pub not_included: Vec<String>,
    pub files: Vec<PackageFile>,
}

/// What the viewer found on checking a package.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackageCheck {
    pub hash: String,
    pub manifest: Manifest,
    pub checked: usize,
    /// Files that are missing or don't match; files present but not listed.
    pub problems: Vec<String>,
}

fn rel(root: &Path, p: &Path) -> Option<String> {
    let r = p.strip_prefix(root).ok()?;
    Some(
        r.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

/// Hash every file under `root` (except the manifest and anything in `skip`), write the
/// manifest with them, and return the package hash.
pub fn write_manifest(root: &Path, mut m: Manifest, skip: &[&str]) -> crate::Result<String> {
    let mut all = vec![];
    walk(root, &mut all)?;
    all.sort();
    m.files.clear();
    for p in all {
        let Some(r) = rel(root, &p) else { continue };
        if r == MANIFEST || skip.contains(&r.as_str()) {
            continue;
        }
        let (sha256, bytes) = sha256_file(&p, &mut |_| {})?;
        m.files.push(PackageFile {
            path: r,
            sha256,
            bytes,
        });
    }
    let text = serde_json::to_vec_pretty(&m)?;
    std::fs::write(root.join(MANIFEST), &text)?;
    Ok(crate::hash::sha256_reader(text.as_slice(), &mut |_| {})?.0)
}

/// Is `root` a case package (a manifest of the right kind beside it)?
pub fn is_package(root: &Path) -> bool {
    std::fs::read(root.join(MANIFEST))
        .ok()
        .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
        .is_some_and(|m| m.kind == KIND)
}

/// Re-hash every file against the manifest. `skip` lists files expected beside the package
/// that aren't in it (the viewer's own executable when run from the package folder is).
pub fn verify(
    root: &Path,
    skip: &[&str],
    progress: &mut dyn FnMut(u64),
) -> crate::Result<PackageCheck> {
    let text = std::fs::read(root.join(MANIFEST))?;
    let hash = crate::hash::sha256_reader(text.as_slice(), &mut |_| {})?.0;
    let manifest: Manifest = serde_json::from_slice(&text)?;
    let mut problems = vec![];
    let mut done = 0u64;
    for f in &manifest.files {
        let rp = Path::new(&f.path);
        if !rp.components().all(|c| matches!(c, Component::Normal(_))) {
            problems.push(format!("{}: not a path inside the package", f.path));
            continue;
        }
        let p = root.join(rp);
        match sha256_file(&p, &mut |b| progress(done + b)) {
            Ok((sha, bytes)) if sha == f.sha256 && bytes == f.bytes => {}
            Ok((sha, _)) => problems.push(format!(
                "{}: changed (SHA-256 {sha}, the manifest says {})",
                f.path, f.sha256
            )),
            Err(_) => problems.push(format!("{}: missing", f.path)),
        }
        done += f.bytes;
    }
    let mut all = vec![];
    walk(root, &mut all)?;
    for p in all {
        let Some(r) = rel(root, &p) else { continue };
        if r != MANIFEST
            && !skip.contains(&r.as_str())
            && !manifest.files.iter().any(|f| f.path == r)
        {
            problems.push(format!("{r}: not in the manifest (added to the package)"));
        }
    }
    Ok(PackageCheck {
        hash,
        checked: manifest.files.len(),
        manifest,
        problems,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            kind: KIND.into(),
            version: 1,
            project: "A".into(),
            case_number: None,
            made_by: "Examiner".into(),
            made_at: "2026-09-24T10:00:00Z".into(),
            app_version: "0.1.0".into(),
            source_state_head: "ab".into(),
            evidence_included: false,
            not_included: vec![],
            files: vec![],
        }
    }

    #[test]
    fn a_package_verifies_and_any_change_shows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("case/derived")).unwrap();
        std::fs::write(root.join("case/project.sqlite"), b"db").unwrap();
        std::fs::write(root.join("case/derived/n.bin"), b"points").unwrap();
        std::fs::write(root.join("Locus Viewer.exe"), b"exe").unwrap();
        let hash = write_manifest(root, manifest(), &["Locus Viewer.exe"]).unwrap();
        assert!(is_package(root));
        let c = verify(root, &["Locus Viewer.exe"], &mut |_| {}).unwrap();
        assert_eq!(
            (c.hash.as_str(), c.checked, c.problems.len()),
            (hash.as_str(), 2, 0)
        );
        // A changed file, a missing one and an added one are each reported.
        std::fs::write(root.join("case/derived/n.bin"), b"pointZ").unwrap();
        std::fs::remove_file(root.join("case/project.sqlite")).unwrap();
        std::fs::write(root.join("case/extra.txt"), b"x").unwrap();
        let c = verify(root, &["Locus Viewer.exe"], &mut |_| {}).unwrap();
        assert_eq!(c.problems.len(), 3, "{:?}", c.problems);
        assert!(c
            .problems
            .iter()
            .any(|p| p.starts_with("case/derived/n.bin: changed")));
        assert!(c
            .problems
            .iter()
            .any(|p| p == "case/project.sqlite: missing"));
        assert!(c
            .problems
            .iter()
            .any(|p| p.starts_with("case/extra.txt: not in the manifest")));
        // The package hash is the manifest's, so a changed manifest changes it.
        assert_eq!(c.hash, hash);
    }
}
