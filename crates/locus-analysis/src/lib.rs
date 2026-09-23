//! Pure forensic math: bloodstain, trajectory, height, crash. No I/O, no UI.
//!
//! Every function returns its result with an uncertainty or residual, never a bare number.

pub mod defect;
pub mod handmeasure;
pub mod measure;
pub mod sun;
pub mod surface;
pub mod trajectory;
