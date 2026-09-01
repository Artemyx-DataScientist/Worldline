//! Engine-neutral experimental capability contracts for Worldline browser services.
//!
//! Exposes experimental v0.1 surfaces for:
//! - `browser.tabs`: tab lifecycle, ordering, grouping, pinning, and selection.
//! - `browser.history`: durable navigation history records, query, and independent clear.
//! - `browser.downloads`: product download records, lifecycle, and opaque artifact references.
//! - `browser.cookies`: cookie metadata inspection, secret value access, and mutation.
//! - `browser.site-data`: origin-scoped storage clearing.

pub mod cookies;
pub mod downloads;
pub mod history;
pub mod site_data;
pub mod tabs;

pub use cookies::{
    AUTH_COOKIES_ADMIN, AUTH_COOKIES_METADATA_READ, AUTH_COOKIES_MUTATE, AUTH_COOKIES_VALUE_READ,
    CONTRACT_BROWSER_COOKIES, CONTRACT_BROWSER_COOKIES_VERSION, CookieMetadata, CookieValue,
    DeleteCookieServiceRequest, DeleteCookieServiceResponse, GetCookieMetadataRequest,
    GetCookieMetadataResponse, GetCookieValueRequest, GetCookieValueResponse, OP_DELETE_COOKIE,
    OP_GET_COOKIE_METADATA, OP_GET_COOKIE_VALUE, OP_SET_COOKIE, SetCookieServiceRequest,
    SetCookieServiceResponse,
};
pub use downloads::{
    AUTH_DOWNLOADS_CONTROL, AUTH_DOWNLOADS_READ, ArtifactRef, CONTRACT_BROWSER_DOWNLOADS,
    CONTRACT_BROWSER_DOWNLOADS_VERSION, CancelDownloadRequest, CancelDownloadResponse,
    DownloadLifecycleStatus, DownloadRecord, DownloadRecordId, GetDownloadRecordRequest,
    GetDownloadRecordResponse, ListDownloadRecordsRequest, ListDownloadRecordsResponse,
    OP_CANCEL_DOWNLOAD, OP_GET_DOWNLOAD_RECORD, OP_LIST_DOWNLOAD_RECORDS, OP_PAUSE_DOWNLOAD,
    OP_RESUME_DOWNLOAD, OP_START_DOWNLOAD, PauseDownloadRequest, PauseDownloadResponse,
    ResumeDownloadRequest, ResumeDownloadResponse, StartDownloadRequest, StartDownloadResponse,
};
pub use history::{
    AUTH_HISTORY_DELETE, AUTH_HISTORY_READ, CONTRACT_BROWSER_HISTORY,
    CONTRACT_BROWSER_HISTORY_VERSION, ClearHistoryRequest, ClearHistoryResponse,
    DeleteHistoryEntryRequest, DeleteHistoryEntryResponse, GetHistoryEntryRequest,
    GetHistoryEntryResponse, HistoryEntry, HistoryEntryId, OP_CLEAR_HISTORY,
    OP_DELETE_HISTORY_ENTRY, OP_GET_HISTORY_ENTRY, OP_QUERY_HISTORY, QueryHistoryRequest,
    QueryHistoryResponse,
};
pub use site_data::{
    AUTH_SITE_DATA_CLEAR, CONTRACT_BROWSER_SITE_DATA, CONTRACT_BROWSER_SITE_DATA_VERSION,
    ClearSiteDataRequest, ClearSiteDataResponse, OP_CLEAR_SITE_DATA, StorageType,
};
pub use tabs::{
    AUTH_TABS_MUTATE, AUTH_TABS_READ, CONTRACT_BROWSER_TABS, CONTRACT_BROWSER_TABS_VERSION,
    CloseTabRequest, CloseTabResponse, CreateTabRequest, CreateTabResponse, GetTabRequest,
    GetTabResponse, GroupTabsRequest, GroupTabsResponse, ListTabsRequest, ListTabsResponse,
    MoveTabRequest, MoveTabResponse, OP_CLOSE_TAB, OP_CREATE_TAB, OP_GET_TAB, OP_GROUP_TABS,
    OP_LIST_TABS, OP_MOVE_TAB, OP_PIN_TAB, OP_SELECT_TAB, OP_UNGROUP_TABS, PinTabRequest,
    PinTabResponse, SelectTabRequest, SelectTabResponse, TabGroup, TabGroupId, TabId, TabState,
    UngroupTabsRequest, UngroupTabsResponse,
};
