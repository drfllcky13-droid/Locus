//! Project model, IDs, units, coordinate frames, audit log.
//!
//! Every mutation of a project goes through [`Project`], which writes the change and
//! its audit entry in one SQLite transaction.

pub mod audit;
mod contents;
pub mod hash;
mod project;
mod units;

pub use audit::AuditEntry;
pub use contents::{
    Bounds, Contents, ExifField, ImageInfo, ImageKind, MeshInfo, ScanInfo, IDENTITY,
};
pub use project::{Error, EvidenceRecord, EvidenceStatus, Project, Result};
pub use units::LinearUnit;
