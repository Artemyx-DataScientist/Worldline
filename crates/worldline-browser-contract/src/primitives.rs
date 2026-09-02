//! Experimental 0.1 browser engine primitive contracts.
//!
//! Exposes low-level cookies, storage, and download hooks for future M1.3
//! service plugins while keeping product policy outside the engine adapter.

use serde::{Deserialize, Serialize};

use crate::identity::{BrowserContextId, DownloadId, PageId};

pub const CONTRACT_ENGINE_COOKIES: &str = "engine.cookies";
pub const CONTRACT_ENGINE_COOKIES_V0_2: &str = "engine.cookies/0.2";
pub const CONTRACT_ENGINE_STORAGE: &str = "engine.storage";
pub const CONTRACT_ENGINE_STORAGE_V0_2: &str = "engine.storage/0.2";
pub const CONTRACT_ENGINE_DOWNLOAD_HOOK: &str = "engine.download_hook";

pub const PRIMITIVES_MAJOR_V0_1: u16 = 0;
pub const PRIMITIVES_MINOR_V0_1: u16 = 1;
pub const PRIMITIVES_MAJOR_V0_2: u16 = 0;
pub const PRIMITIVES_MINOR_V0_2: u16 = 2;

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

/// Additive engine.cookies/0.2 cookie DTO. The 0.1 `Cookie` wire shape is
/// intentionally unchanged; this version is required because host-only and
/// domain-cookie semantics cannot be represented by 0.1.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CookieV0_2 {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires_epoch_sec: Option<u64>,
    pub host_only: bool,
}

impl From<Cookie> for CookieV0_2 {
    fn from(cookie: Cookie) -> Self {
        Self {
            name: cookie.name,
            value: cookie.value,
            domain: cookie.domain,
            path: cookie.path,
            secure: cookie.secure,
            http_only: cookie.http_only,
            same_site: cookie.same_site,
            expires_epoch_sec: cookie.expires_epoch_sec,
            host_only: true,
        }
    }
}

impl From<CookieV0_2> for Cookie {
    fn from(cookie: CookieV0_2) -> Self {
        Self {
            name: cookie.name,
            value: cookie.value,
            domain: cookie.domain,
            path: cookie.path,
            secure: cookie.secure,
            http_only: cookie.http_only,
            same_site: cookie.same_site,
            expires_epoch_sec: cookie.expires_epoch_sec,
        }
    }
}

pub type GetCookiesRequestV0_2 = GetCookiesRequest;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCookiesResponseV0_2 {
    pub cookies: Vec<CookieV0_2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetCookieRequestV0_2 {
    pub context_id: BrowserContextId,
    pub cookie: CookieV0_2,
}

pub type SetCookieResponseV0_2 = SetCookieResponse;

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

/// Additive engine.storage/0.2 item operation. The 0.1 clear-only fixture
/// remains compatible; `value: Some` is a set and `value: None` is a get.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageItemRequestV0_2 {
    pub context_id: BrowserContextId,
    pub origin: String,
    pub storage_type: StorageType,
    pub key: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageItemResponseV0_2 {
    pub value: Option<String>,
    pub changed: bool,
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
