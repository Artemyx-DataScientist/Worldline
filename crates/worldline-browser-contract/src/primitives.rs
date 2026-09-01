//! Experimental 0.1 browser engine primitive contracts.
//!
//! Exposes low-level cookies, storage, and download hooks for future M1.3
//! service plugins while keeping product policy outside the engine adapter.

use serde::{Deserialize, Serialize};

use crate::identity::{BrowserContextId, DownloadId, PageId};

pub const CONTRACT_ENGINE_COOKIES: &str = "engine.cookies";
pub const CONTRACT_ENGINE_STORAGE: &str = "engine.storage";
pub const CONTRACT_ENGINE_DOWNLOAD_HOOK: &str = "engine.download_hook";

pub const PRIMITIVES_MAJOR_V0_1: u16 = 0;
pub const PRIMITIVES_MINOR_V0_1: u16 = 1;

// --- Cookies Primitives ---

/// A single HTTP / browser cookie structure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires_epoch_sec: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCookiesRequest {
    pub context_id: BrowserContextId,
    pub url: Option<String>,
    pub domain: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCookiesResponse {
    pub cookies: Vec<Cookie>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetCookieRequest {
    pub context_id: BrowserContextId,
    pub cookie: Cookie,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetCookieResponse {
    pub success: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteCookiesRequest {
    pub context_id: BrowserContextId,
    pub url: Option<String>,
    pub name: Option<String>,
    pub domain: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteCookiesResponse {
    pub deleted_count: u32,
}

// --- Storage Primitives ---

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum StorageType {
    #[default]
    LocalStorage,
    SessionStorage,
    IndexedDb,
    All,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearStorageRequest {
    pub context_id: BrowserContextId,
    pub origin: String,
    pub storage_type: StorageType,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearStorageResponse {
    pub cleared: bool,
}

// --- Download Hook Primitives ---

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DownloadHookAction {
    Accept,
    Cancel,
    Redirect { destination_path: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadHookDecision {
    pub download_id: DownloadId,
    pub action: DownloadHookAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadHookEvent {
    pub download_id: DownloadId,
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub url: String,
    pub suggested_filename: String,
    pub total_bytes: Option<u64>,
    pub mime_type: Option<String>,
}
