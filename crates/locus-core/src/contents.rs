use crate::LinearUnit;
use serde::{Deserialize, Serialize};

/// Row-major 4x4 identity.
pub const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// Axis-aligned bounds in the source file's own coordinates and units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    /// Grow `slot` to include `p`. Non-finite points are ignored.
    pub fn grow(slot: &mut Option<Bounds>, p: [f64; 3]) {
        if !p.iter().all(|v| v.is_finite()) {
            return;
        }
        match slot {
            None => *slot = Some(Bounds { min: p, max: p }),
            Some(b) => {
                b.min = std::array::from_fn(|i| b.min[i].min(p[i]));
                b.max = std::array::from_fn(|i| b.max[i].max(p[i]));
            }
        }
    }
}

/// One point cloud (scan) inside an evidence file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanInfo {
    pub name: String,
    /// Records actually read from the file, not the count the header claims.
    pub point_count: u64,
    /// Records with no valid Cartesian position (for example, no laser return).
    pub invalid_points: u64,
    /// Bounds of valid points, in scan-local source coordinates (before `pose`).
    pub bounds: Option<Bounds>,
    /// Scan-to-file transform stored in the source file, row-major.
    pub pose: [f64; 16],
    /// Per-point attributes present, e.g. "intensity", "color".
    pub attributes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshInfo {
    pub name: String,
    pub vertex_count: u64,
    pub triangle_count: u64,
    pub bounds: Option<Bounds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    /// A standalone photo file.
    Photo,
    Pinhole,
    Spherical,
    Cylindrical,
    /// A preview image with no projection model.
    VisualReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExifField {
    pub ifd: String,
    pub tag: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub name: String,
    pub kind: ImageKind,
    pub width: u32,
    pub height: u32,
    /// Index into `Contents::scans` of the scan this image was captured with.
    pub scan: Option<usize>,
    pub pose: Option<[f64; 16]>,
    pub exif: Vec<ExifField>,
}

/// What an importer found in a source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contents {
    pub format: String,
    /// Unit stated by the file itself, if the format has a way to state one.
    pub declared_unit: Option<LinearUnit>,
    /// True when the source coordinates are Y-up (glTF); the project frame is Z-up.
    #[serde(default)]
    pub y_up: bool,
    pub scans: Vec<ScanInfo>,
    pub meshes: Vec<MeshInfo>,
    pub images: Vec<ImageInfo>,
    /// Things the examiner should know about this file (count mismatches, missing references).
    pub warnings: Vec<String>,
}

impl Contents {
    pub fn new(format: &str) -> Self {
        Self {
            format: format.into(),
            declared_unit: None,
            y_up: false,
            scans: vec![],
            meshes: vec![],
            images: vec![],
            warnings: vec![],
        }
    }

    /// True when the file has geometry but doesn't say what unit it is in.
    pub fn needs_unit(&self) -> bool {
        self.declared_unit.is_none() && (!self.scans.is_empty() || !self.meshes.is_empty())
    }

    pub fn point_count(&self) -> u64 {
        self.scans.iter().map(|s| s.point_count).sum()
    }
}
