//! A photogrammetry run as stored (an audit-logged analysis record): its provenance from the
//! input files' hashes and COLMAP's version and executable hash, through every stage's command
//! line, to the scaling and the point cloud imported as evidence.

use crate::colmap::{Settings, StageRecord};
use crate::georef::ScaleRecord;
use serde::{Deserialize, Serialize};

pub const PHOTO_METHOD: &str = "photogrammetry/1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColmapInfo {
    pub path: String,
    pub banner: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceImage {
    pub evidence_id: i64,
    pub name: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Photos {
        items: Vec<SourceImage>,
    },
    Video {
        evidence_id: i64,
        name: String,
        sha256: String,
        /// Seconds between sampled frames, and each frame's name and time from the first frame.
        interval: f64,
        first_timestamp: f64,
        duration: f64,
        frames: Vec<(String, f64)>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub evidence_id: i64,
    pub sha256: String,
    pub file: String,
    pub points: usize,
    /// "dense" (COLMAP's fused point cloud) or "sparse" (the reconstruction's own points).
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhotoRun {
    pub method: String,
    pub name: String,
    pub colmap: ColmapInfo,
    pub settings: Settings,
    pub source: Source,
    pub stages: Vec<StageRecord>,
    pub images_total: usize,
    pub registered: Vec<String>,
    pub sparse_points: usize,
    pub mean_error_px: f64,
    /// Other, smaller sparse models (disconnected groups of images), by image count.
    pub other_models: Vec<usize>,
    pub dense_note: Option<String>,
    pub scale: ScaleRecord,
    pub output: Output,
    pub summary: String,
    pub warnings: Vec<String>,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub const PHOTO_ASSUMPTIONS: &[&str] = &[
    "The scene didn't change between the photos (or frames): nothing moved, and lighting changes didn't alter surfaces' appearance so much that they matched wrongly.",
    "Photos sharing a COLMAP camera share a body, lens and focal length (single camera), or each has its own when that option is off; the chosen camera model describes the lens's distortion.",
    "The scaling's inputs (known distances, control-point coordinates, or the photos' GPS positions) are what they are stated to be, with their stated uncertainty.",
];

pub const PHOTO_LIMITATIONS: &[&str] = &[
    "Accuracy depends on the photos: overlap, viewing angles, sharpness and texture. Plain, shiny, transparent or moving surfaces reconstruct poorly or not at all.",
    "A reconstruction scaled by known distances has an arbitrary position and heading in the project, and is levelled only approximately (by the photos' mean up direction); control points or registration to a scan place it.",
    "The dense point cloud's points are not individually checked: outliers near depth edges and on thin structures are common. Measure on well-covered surfaces.",
    "COLMAP is installed by the examiner; its version and executable hash are recorded, and a different version can give a slightly different result from the same photos.",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_round_trips() {
        let s = Source::Video {
            evidence_id: 3,
            name: "dashcam.mp4".into(),
            sha256: "ab".repeat(32),
            interval: 0.5,
            first_timestamp: 0.08,
            duration: 4.0,
            frames: vec![("frame_000001.png".into(), 0.0)],
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"kind\":\"video\""));
        assert_eq!(serde_json::from_str::<Source>(&j).unwrap(), s);
    }
}
