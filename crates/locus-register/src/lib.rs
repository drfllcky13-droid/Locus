//! Target detection, ICP, global registration, pose graph, registration reports.
//!
//! Pure math on points in memory; loading scans and storing results is the caller's job.

pub mod checker;
pub mod coarse;
pub mod icp;
pub mod normals;
pub mod posegraph;
pub mod rigid;
pub mod sphere;
pub mod targets;
