//! Experimental browser.downloads v0.1 contract definitions.

use std::fmt;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::{BrowserContextId, DownloadId, PageId};

pub const CONTRACT_BROWSER_DOWNLOADS: &str = "browser.downloads";
pub const CONTRACT_BROWSER_DOWNLOADS_VERSION: &str = "0.1";

pub const OP_START_DOWNLOAD: &str = "browser.downloads.start";
pub const OP_GET_DOWNLOAD_RECORD: &str = "browser.downloads.get";
pub const OP_LIST_DOWNLOAD_RECORDS: &str = "browser.downloads.list";
pub const OP_PAUSE_DOWNLOAD: &str = "browser.downloads.pause";
pub const OP_RESUME_DOWNLOAD: &str = "browser.downloads.resume";
pub const OP_CANCEL_DOWNLOAD: &str = "browser.downloads.cancel";

pub const AUTH_DOWNLOADS_READ: &str = "browser.downloads.read";
pub const AUTH_DOWNLOADS_CONTROL: &str = "browser.downloads.control";

/// Opaque identity of a durable product-level download record.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DownloadRecordId(String);

impl DownloadRecordId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DownloadRecordId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DownloadRecordId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for DownloadRecordId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque reference to completed download content (CAS blob / artifact).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub size_bytes: u64,
    pub mime_type: Option<String>,
    pub sha256_hash: Option<String>,
}

impl ArtifactRef {
    pub fn new(
        artifact_id: impl Into<String>,
        size_bytes: u64,
        mime_type: Option<String>,
        sha256_hash: Option<String>,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            size_bytes,
            mime_type,
            sha256_hash,
        }
    }
}

/// Bounded product lifecycle status of a download.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadLifecycleStatus {
    #[default]
    Pending,
    Active,
    Paused,
    Completed,
    Cancelled,
    Failed,
    Reconciling,
}

/// Durable product-level record of a download.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub record_id: DownloadRecordId,
    pub context_id: Option<BrowserContextId>,
    pub page_id: Option<PageId>,
    pub url: String,
    pub suggested_filename: String,
    pub media_type: Option<String>,
    pub total_bytes: Option<u64>,
    pub received_bytes: u64,
    pub status: DownloadLifecycleStatus,
    pub engine_download_id: Option<DownloadId>,
    pub artifact_ref: Option<ArtifactRef>,
    pub error_message: Option<String>,
    pub started_at_unix_ms: u64,
    pub completed_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StartDownloadRequest {
    pub context_id: BrowserContextId,
    pub page_id: Option<PageId>,
    pub url: String,
    pub suggested_filename: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StartDownloadResponse {
    pub record_id: DownloadRecordId,
    pub status: DownloadLifecycleStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetDownloadRecordRequest {
    pub record_id: DownloadRecordId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetDownloadRecordResponse {
    pub record: Option<DownloadRecord>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListDownloadRecordsRequest {
    pub context_id: Option<BrowserContextId>,
    pub status: Option<DownloadLifecycleStatus>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListDownloadRecordsResponse {
    pub records: Vec<DownloadRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PauseDownloadRequest {
    pub record_id: DownloadRecordId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PauseDownloadResponse {
    pub success: bool,
    pub status: DownloadLifecycleStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResumeDownloadRequest {
    pub record_id: DownloadRecordId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResumeDownloadResponse {
    pub success: bool,
    pub status: DownloadLifecycleStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CancelDownloadRequest {
    pub record_id: DownloadRecordId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CancelDownloadResponse {
    pub success: bool,
    pub status: DownloadLifecycleStatus,
}
