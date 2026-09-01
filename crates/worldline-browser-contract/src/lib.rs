//! Engine-neutral capability contracts, authority separation, and event
//! vocabulary for Worldline browser plugins.
//!
//! Provides the foundational contracts without embedding dependencies.

#![forbid(unsafe_code)]

pub mod action;
pub mod authority;
pub mod capture;
pub mod contracts;
pub mod error;
pub mod events;
pub mod identity;
pub mod primitives;
pub mod query;

pub use action::{
    ActionResult, ClickActionRequest, FocusActionRequest, InputActionRequest, InteractionKind,
    ScrollActionRequest, SubmitActionRequest, validate_element_reference,
};
pub use authority::{
    BrowserAuthority, BrowserAuthoritySet, OP_BACK, OP_CAPTURE, OP_CLICK, OP_CLOSE_CONTEXT,
    OP_CLOSE_PAGE, OP_COOKIE_DELETE, OP_COOKIE_GET, OP_COOKIE_SET, OP_CREATE_CONTEXT,
    OP_CREATE_PAGE, OP_DOWNLOAD_CONTROL, OP_DOWNLOAD_HOOK, OP_DOWNLOAD_START, OP_DOWNLOAD_STATUS,
    OP_EXTRACT_TEXT, OP_FIND_ELEMENTS, OP_FOCUS, OP_FORWARD, OP_GET_TITLE, OP_GET_URL, OP_INPUT,
    OP_LIST_CONTEXTS, OP_LIST_PAGES, OP_NAVIGATE, OP_OBSERVE, OP_PERMISSION_QUERY,
    OP_PERMISSION_SET, OP_QUERY_ACCESSIBILITY, OP_QUERY_DOCUMENT, OP_READ_CAPTURE, OP_RELOAD,
    OP_SCROLL, OP_STOP, OP_STORAGE_CLEAR, OP_SUBMIT,
};
pub use capture::{
    CAPTURE_MAJOR_V0_1, CAPTURE_MINOR_V0_1, CONTRACT_CAPTURE, CaptureArtifactRef, CaptureFormat,
    CapturePageRequest, CapturePageResponse, CaptureTarget, ReadCaptureArtifactRequest,
    ReadCaptureArtifactResponse,
};
pub use contracts::{
    ActRequest, BROWSER_NAMESPACE, CONTRACT_ACT, CONTRACT_CONTEXT, CONTRACT_DOWNLOAD,
    CONTRACT_NAVIGATE, CONTRACT_OBSERVE, CONTRACT_PAGE, CONTRACT_PERMISSION, CONTRACT_QUERY,
    CloseContextRequest, CloseContextResponse, ClosePageRequest, ClosePageResponse,
    ControlDownloadRequest, CreateContextRequest, CreateContextResponse, CreatePageRequest,
    CreatePageResponse, DownloadAction, DownloadState, DownloadStatusResponse, ElementQueryKind,
    ExtractTextRequest, ExtractTextResponse, FindElementsRequest, FindElementsResponse,
    HistoryNavRequest, INTERFACE_MAJOR_V1, INTERFACE_MINOR_V1, ListContextsResponse,
    ListPagesRequest, ListPagesResponse, LoadingState, NavigateRequest, NavigateResponse,
    ObservePageRequest, PageObservation, PageSummary, PermissionDecision, PermissionResponse,
    PermissionType, QueryDocumentRequest, QueryPermissionRequest, ReloadRequest,
    SetPermissionRequest, StartDownloadRequest, StopRequest, ViewportInfo,
};
pub use error::BrowserError;
pub use events::{
    DownloadCompletedEvent, DownloadStartedEvent, EVENT_DOWNLOAD_COMPLETED, EVENT_DOWNLOAD_STARTED,
    EVENT_ENGINE_CRASHED, EVENT_NAVIGATION_COMMITTED, EVENT_NAVIGATION_FAILED,
    EVENT_NAVIGATION_STARTED, EVENT_PAGE_CLOSED, EVENT_PAGE_READY, EVENT_PAGE_RESTORED,
    EVENT_RENDERER_CRASHED, EngineCrashedEvent, NavigationCommittedEvent, NavigationFailedEvent,
    NavigationStartedEvent, PageClosedEvent, PageReadyEvent, PageRestoredEvent,
    RendererCrashedEvent,
};
pub use identity::{
    BrowserContextId, DocumentRevision, DownloadId, ElementRef, NavigationId, PageId,
    context_resource, download_resource, page_resource,
};
pub use primitives::{
    CONTRACT_ENGINE_COOKIES, CONTRACT_ENGINE_DOWNLOAD_HOOK, CONTRACT_ENGINE_STORAGE,
    ClearStorageRequest, ClearStorageResponse, Cookie, DeleteCookiesRequest, DeleteCookiesResponse,
    DownloadHookAction, DownloadHookDecision, DownloadHookEvent, GetCookiesRequest,
    GetCookiesResponse, PRIMITIVES_MAJOR_V0_1, PRIMITIVES_MINOR_V0_1, SetCookieRequest,
    SetCookieResponse, StorageType,
};
