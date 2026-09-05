//! Engine-neutral experimental capability contracts for Worldline browser services.
//!
//! Exposes experimental v0.1 surfaces for:
//! - `browser.tabs`: tab lifecycle, ordering, grouping, pinning, and selection.
//! - `browser.history`: durable navigation history records, query, and independent clear.
//! - `browser.downloads`: product download records, lifecycle, and opaque artifact references.
//! - `browser.cookies`: cookie metadata inspection, secret value access, and mutation.
//! - `browser.site-data`: origin-scoped storage clearing.

pub mod cookies;
pub mod devtools;
pub mod downloads;
pub mod history;
pub mod search;
pub mod site_data;
pub mod tabs;

pub use cookies::{
    AUTH_COOKIES_ADMIN, AUTH_COOKIES_METADATA_READ, AUTH_COOKIES_MUTATE, AUTH_COOKIES_VALUE_READ,
    CONTRACT_BROWSER_COOKIES, CONTRACT_BROWSER_COOKIES_V0_2, CONTRACT_BROWSER_COOKIES_V0_2_VERSION,
    CONTRACT_BROWSER_COOKIES_VERSION, CookieMetadata, CookieMetadataV0_2, CookieValue,
    DeleteCookieServiceRequest, DeleteCookieServiceResponse, GetCookieMetadataRequest,
    GetCookieMetadataResponse, GetCookieMetadataResponseV0_2, GetCookieValueRequest,
    GetCookieValueResponse, OP_DELETE_COOKIE, OP_GET_COOKIE_METADATA, OP_GET_COOKIE_VALUE,
    OP_SET_COOKIE, SetCookieServiceRequest, SetCookieServiceRequestV0_2, SetCookieServiceResponse,
};
pub use devtools::{
    AUTH_DEVTOOLS_CONTROL, AUTH_DEVTOOLS_NATIVE, AUTH_DEVTOOLS_OBSERVE, CONTRACT_BROWSER_DEVTOOLS,
    CONTRACT_BROWSER_DEVTOOLS_VERSION, ClearDiagnosticsRequest, ClearDiagnosticsResponse,
    ConsoleDiagnosticRecord, ConsoleLogLevel, DEFAULT_BUFFER_CAPACITY, DiagnosticBufferStats,
    GetRuntimeSnapshotRequest, GetRuntimeSnapshotResponse, MAX_CONSOLE_MESSAGE_LENGTH,
    MAX_DIAGNOSTIC_URL_LENGTH, MAX_SOURCE_LENGTH, NetworkDiagnosticRecord, NetworkRequestStatus,
    OP_CLEAR_DIAGNOSTICS, OP_GET_RUNTIME_SNAPSHOT, OP_QUERY_CONSOLE_RECORDS,
    OP_QUERY_NETWORK_RECORDS, OP_SHOW_NATIVE_DEVTOOLS, PageRuntimeDiagnosticSnapshot,
    QueryConsoleRecordsRequest, QueryConsoleRecordsResponse, QueryNetworkRecordsRequest,
    QueryNetworkRecordsResponse, ShowNativeDevToolsRequest, ShowNativeDevToolsResponse,
    truncate_string,
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
pub use search::{
    AUTH_SEARCH_RESOLVE, CONTRACT_BROWSER_SEARCH, CONTRACT_BROWSER_SEARCH_VERSION,
    MAX_SEARCH_QUERY_LENGTH, MAX_SEARCH_TARGET_URL_LENGTH, OP_RESOLVE_SEARCH, SearchContractError,
    SearchNavigationTarget, SearchResolveRequest,
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
