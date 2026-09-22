//! Importers and exporters for open scan, mesh, image, and table formats.
//!
//! Every reader streams its file once, read-only, and reports what it actually found
//! (true record counts, bounds, poses) as a [`Contents`]. Nothing here writes to a source file.

mod e57;
mod image;
mod las;
mod mesh;
mod ply;
mod stats;
mod text;

use locus_core::hash::sha256_file;
use locus_core::{Contents, EvidenceRecord, LinearUnit, Project};
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub use e57::extract_images as extract_e57_images;
pub use stats::Point;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Project(#[from] locus_core::Error),
    #[error("unsupported file type: {0}")]
    Unsupported(String),
    #[error("could not read {format} file: {message}")]
    Parse {
        format: &'static str,
        message: String,
    },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn parse_err(format: &'static str, message: impl ToString) -> Error {
    Error::Parse {
        format,
        message: message.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Format {
    E57,
    Las,
    Laz,
    Ply,
    Pts,
    Xyz,
    Obj,
    Gltf,
    Glb,
    Jpeg,
    Png,
}

impl Format {
    /// Pick the reader from the extension, then check the file's leading bytes agree,
    /// so a renamed file is refused rather than misread.
    pub fn detect(path: &Path) -> Result<Format> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let format = match ext.as_str() {
            "e57" => Format::E57,
            "las" => Format::Las,
            "laz" => Format::Laz,
            "ply" => Format::Ply,
            "pts" => Format::Pts,
            "xyz" => Format::Xyz,
            "obj" => Format::Obj,
            "gltf" => Format::Gltf,
            "glb" => Format::Glb,
            "jpg" | "jpeg" => Format::Jpeg,
            "png" => Format::Png,
            _ => return Err(Error::Unsupported(path.display().to_string())),
        };
        let magic: &[u8] = match format {
            Format::E57 => b"ASTM-E57",
            Format::Las | Format::Laz => b"LASF",
            Format::Ply => b"ply",
            Format::Glb => b"glTF",
            Format::Jpeg => b"\xFF\xD8\xFF",
            Format::Png => b"\x89PNG\r\n\x1a\n",
            _ => b"",
        };
        let mut head = vec![0u8; magic.len()];
        File::open(path)?.read_exact(&mut head).ok();
        if head != magic {
            return Err(parse_err(
                format.name(),
                "the file's contents don't match its extension",
            ));
        }
        Ok(format)
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::E57 => "E57",
            Format::Las => "LAS",
            Format::Laz => "LAZ",
            Format::Ply => "PLY",
            Format::Pts => "PTS",
            Format::Xyz => "XYZ",
            Format::Obj => "OBJ",
            Format::Gltf => "glTF",
            Format::Glb => "GLB",
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
        }
    }
}

/// Import progress, reported as `done` of `total` in the stage's own unit.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub stage: Stage,
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Bytes of the source read by the format reader.
    Reading,
    /// Points read from the current scan.
    Points,
    /// Bytes hashed.
    Hashing,
    /// Bytes copied into the project.
    Copying,
}

pub type ProgressFn<'a> = &'a mut dyn FnMut(Progress);

/// Read `path` with the matching format reader.
pub fn inspect(path: &Path, progress: ProgressFn) -> Result<Contents> {
    let format = Format::detect(path)?;
    let mut contents = match format {
        Format::E57 => e57::inspect(path, progress, None)?,
        Format::Las | Format::Laz => las::inspect(path, progress, None, false)?,
        Format::Ply => ply::inspect(path, progress, None)?,
        Format::Pts => text::inspect(path, text::Kind::Pts, progress, None)?,
        Format::Xyz => text::inspect(path, text::Kind::Xyz, progress, None)?,
        Format::Obj => mesh::inspect_obj(path)?,
        Format::Gltf | Format::Glb => mesh::inspect_gltf(path)?,
        Format::Jpeg | Format::Png => image::inspect(path)?,
    };
    contents.format = format.name().into();
    Ok(contents)
}

/// Stream every point of scan `scan` (index into `Contents::scans`) of an evidence file,
/// in the file's own coordinates and unit. Records with no valid position are skipped, but
/// `Point::index` still counts them, so it matches the record number in the file.
pub fn for_each_point(
    path: &Path,
    contents: &Contents,
    scan: usize,
    progress: ProgressFn,
    f: &mut dyn FnMut(&Point),
) -> Result<()> {
    let info = contents
        .scans
        .get(scan)
        .ok_or_else(|| parse_err("scan", format!("no scan {scan} in this file")))?;
    let visit = Some((scan, f));
    match Format::detect(path)? {
        Format::E57 => e57::inspect(path, progress, visit).map(drop),
        Format::Las | Format::Laz => {
            let eight = info.attributes.iter().any(|a| a == "color_8bit");
            las::inspect(path, progress, visit, eight).map(drop)
        }
        Format::Ply => ply::inspect(path, progress, visit).map(drop),
        Format::Pts => text::inspect(path, text::Kind::Pts, progress, visit).map(drop),
        Format::Xyz => text::inspect(path, text::Kind::Xyz, progress, visit).map(drop),
        other => Err(parse_err(other.name(), "this file has no point clouds")),
    }
}

/// What the examiner sees before committing an import.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub contents: Contents,
}

/// Read and hash a source file without changing anything.
pub fn preview(path: &Path, progress: ProgressFn) -> Result<Preview> {
    let contents = inspect(path, progress)?;
    let total = std::fs::metadata(path)?.len();
    let (sha256, size) = sha256_file(path, &mut |done| {
        progress(Progress {
            stage: Stage::Hashing,
            done,
            total,
        })
    })?;
    Ok(Preview {
        path: path.into(),
        sha256,
        size,
        contents,
    })
}

/// Copy a previewed file into the project. Refused if the file changed since preview.
pub fn commit(
    project: &mut Project,
    preview: &Preview,
    unit: Option<LinearUnit>,
    progress: ProgressFn,
) -> Result<EvidenceRecord> {
    let total = preview.size;
    Ok(project.import_evidence(
        &preview.path,
        &preview.sha256,
        &preview.contents,
        unit,
        &mut |done| {
            progress(Progress {
                stage: Stage::Copying,
                done,
                total,
            })
        },
    )?)
}
