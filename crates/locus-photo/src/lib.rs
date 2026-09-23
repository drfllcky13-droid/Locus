//! Photogrammetry: COLMAP runs as a separate process (see docs/phase8-colmap-licence-review.txt);
//! this crate reads its model, scales and georeferences it, and reads the photos' GPS tags.

pub mod camera;
pub mod colmap;
pub mod exif;
pub mod geo;
pub mod model;
pub mod ply;
pub mod scale;
