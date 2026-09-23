//! Project model, IDs, units, coordinate frames, audit log.
//!
//! Every mutation of a project goes through [`Project`], which writes the change and
//! its audit entry in one SQLite transaction.

pub mod audit;
mod contents;
pub mod hash;
mod project;
mod registration;
mod state;
mod units;

pub use audit::AuditEntry;
pub use contents::{
    Bounds, Contents, ExifField, ImageInfo, ImageKind, MeshInfo, ScanInfo, IDENTITY,
};
pub use project::{
    now as timestamp, Error, EvidenceRecord, EvidenceStatus, IntegrityReport, Project, Result,
};
pub use registration::{RegistrationRecord, ScanPose};
pub use state::{
    CleanupRecord, CleanupScan, MeasurementRecord, OctreeRecord, DEFAULT_POINT_SIGMA_M,
    POINT_SIGMA_KEY,
};
pub use units::LinearUnit;
