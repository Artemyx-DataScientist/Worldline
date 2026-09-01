use serde::{Deserialize, Serialize};

use crate::identity::{BrowserContextId, DocumentRevision, DownloadId, NavigationId, PageId};

pub const EVENT_NAVIGATION_STARTED: &str = "browser.navigation.started";
pub const EVENT_NAVIGATION_COMMITTED: &str = "browser.navigation.committed";
pub const EVENT_NAVIGATION_FAILED: &str = "browser.navigation.failed";
pub const EVENT_PAGE_CREATED: &str = "browser.page.created";
pub const EVENT_PAGE_READY: &str = "browser.page.ready";
pub const EVENT_PAGE_CLOSED: &str = "browser.page.closed";
pub const EVENT_ENGINE_CRASHED: &str = "browser.engine.crashed";
pub const EVENT_PAGE_RESTORED: &str = "browser.page.restored";
pub const EVENT_RENDERER_CRASHED: &str = "browser.renderer.crashed";
pub const EVENT_DOWNLOAD_STARTED: &str = "browser.download.started";
pub const EVENT_DOWNLOAD_COMPLETED: &str = "browser.download.completed";

/// Published when a navigation attempt begins.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationStartedEvent {
    pub page_id: PageId,
    pub navigation_id: NavigationId,
    pub url: String,
    pub timestamp_ms: u64,
}

/// Published when a navigation has committed and document loading has begun.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationCommittedEvent {
    pub page_id: PageId,
    pub navigation_id: NavigationId,
    pub url: String,
    pub document_revision: DocumentRevision,
    pub status_code: u16,
}

/// Published when a navigation attempt fails.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationFailedEvent {
    pub page_id: PageId,
    pub navigation_id: NavigationId,
    pub url: String,
    pub error: String,
    pub document_revision: DocumentRevision,
}

/// Published when a page surface is created.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageCreatedEvent {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
}

/// Published when a page finishes loading its initial document.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageReadyEvent {
    pub page_id: PageId,
    pub url: String,
    pub title: String,
    pub document_revision: DocumentRevision,
}

/// Published when a page surface is closed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageClosedEvent {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
}

/// Published when an engine child process crashes or is killed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EngineCrashedEvent {
    pub context_id: BrowserContextId,
    pub page_id: Option<PageId>,
    pub reason: String,
    pub exit_code: Option<i32>,
    pub recoverable: bool,
}

/// Published when a download begins.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadStartedEvent {
    pub download_id: DownloadId,
    pub page_id: PageId,
    pub url: String,
    pub suggested_filename: String,
}

/// Published when a download finishes (successfully or failed).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadCompletedEvent {
    pub download_id: DownloadId,
    pub destination_path: String,
    pub total_bytes: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// Published when a page finishes restoring after an engine or host restart.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageRestoredEvent {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub url: String,
    pub document_revision: DocumentRevision,
}

/// Published when a specific page renderer process crashes or exits abnormally.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RendererCrashedEvent {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub exit_code: Option<i32>,
    pub reason: String,
}
