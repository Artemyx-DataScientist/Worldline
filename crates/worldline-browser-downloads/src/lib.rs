//! Browser downloads service plugin for Worldline.
//!
//! Provides durable download record management with DownloadRecordId identity,
//! opaque completed artifact references, lifecycle state tracking, and
//! crash recovery reconciliation without leaking host filesystem paths.

pub mod artifact;
pub mod service;
pub mod state;

pub use artifact::ArtifactStore;
pub use service::DownloadsService;
pub use state::DownloadsSnapshot;
