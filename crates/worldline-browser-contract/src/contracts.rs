use serde::{Deserialize, Serialize};

use crate::{
    action::{
        ClickActionRequest, FocusActionRequest, InputActionRequest, ScrollActionRequest,
        SubmitActionRequest,
    },
    identity::{BrowserContextId, DocumentRevision, DownloadId, ElementRef, NavigationId, PageId},
    query::{QueryBounds, SemanticElement},
};

pub const BROWSER_NAMESPACE: &str = "browser";

pub const CONTRACT_CONTEXT: &str = "context";
pub const CONTRACT_PAGE: &str = "page";
pub const CONTRACT_NAVIGATE: &str = "navigate";
pub const CONTRACT_OBSERVE: &str = "observe";
pub const CONTRACT_QUERY: &str = "query";
pub const CONTRACT_ACT: &str = "act";
pub const CONTRACT_DOWNLOAD: &str = "download";
pub const CONTRACT_PERMISSION: &str = "permission";

pub const INTERFACE_MAJOR_V1: u16 = 1;
pub const INTERFACE_MINOR_V1: u16 = 0;

// --- browser.context requests / responses ---

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateContextRequest {
    pub profile_id: Option<String>,
    pub incognito: bool,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateContextResponse {
    pub context_id: BrowserContextId,
    pub profile_id: Option<String>,
    pub incognito: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloseContextRequest {
    pub context_id: BrowserContextId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloseContextResponse {
    pub context_id: BrowserContextId,
    pub closed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListContextsResponse {
    pub contexts: Vec<BrowserContextId>,
}

// --- browser.page requests / responses ---

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreatePageRequest {
    pub context_id: BrowserContextId,
    pub initial_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreatePageResponse {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub initial_revision: DocumentRevision,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClosePageRequest {
    pub page_id: PageId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClosePageResponse {
    pub page_id: PageId,
    pub closed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListPagesRequest {
    pub context_id: BrowserContextId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageSummary {
    pub page_id: PageId,
    pub url: String,
    pub title: String,
    pub document_revision: DocumentRevision,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListPagesResponse {
    pub pages: Vec<PageSummary>,
}

// --- browser.navigate requests / responses ---

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigateRequest {
    pub page_id: PageId,
    pub url: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigateResponse {
    pub page_id: PageId,
    pub navigation_id: NavigationId,
    pub committed: bool,
    pub document_revision: DocumentRevision,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReloadRequest {
    pub page_id: PageId,
    pub ignore_cache: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReloadResponse {
    pub page_id: PageId,
    pub reloaded: bool,
    pub document_revision: DocumentRevision,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StopRequest {
    pub page_id: PageId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StopResponse {
    pub page_id: PageId,
    pub stopped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoryNavRequest {
    pub page_id: PageId,
    pub delta: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoryNavResponse {
    pub page_id: PageId,
    pub success: bool,
    pub document_revision: DocumentRevision,
}

// --- browser.observe requests / responses ---

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LoadingState {
    Unloaded,
    Loading,
    Interactive,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ViewportInfo {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservePageRequest {
    pub page_id: PageId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageObservation {
    pub page_id: PageId,
    pub url: String,
    pub title: String,
    pub loading_state: LoadingState,
    pub document_revision: DocumentRevision,
    pub status_code: u16,
    pub is_secure: bool,
    pub viewport: Option<ViewportInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetTitleResponse {
    pub page_id: PageId,
    pub title: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetUrlResponse {
    pub page_id: PageId,
    pub url: String,
}

// --- browser.query requests / responses ---

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryDocumentRequest {
    pub page_id: PageId,
    pub bounds: Option<QueryBounds>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryAccessibilityRequest {
    pub page_id: PageId,
    pub bounds: Option<QueryBounds>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ElementQueryKind {
    CssSelector,
    AccessibilityRole,
    TextMatch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FindElementsRequest {
    pub page_id: PageId,
    pub query: String,
    pub kind: ElementQueryKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FindElementsResponse {
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub elements: Vec<SemanticElement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtractTextRequest {
    pub page_id: PageId,
    pub target_element: Option<ElementRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtractTextResponse {
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub text: String,
}

// --- browser.act requests / responses ---

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActRequest {
    Click(ClickActionRequest),
    Input(InputActionRequest),
    Focus(FocusActionRequest),
    Submit(SubmitActionRequest),
    Scroll(ScrollActionRequest),
}

// --- browser.download requests / responses ---

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StartDownloadRequest {
    pub page_id: PageId,
    pub url: String,
    pub destination_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DownloadAction {
    Pause,
    Resume,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlDownloadRequest {
    pub download_id: DownloadId,
    pub action: DownloadAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DownloadState {
    InProgress,
    Paused,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadStatusResponse {
    pub download_id: DownloadId,
    pub page_id: PageId,
    pub url: String,
    pub destination_path: String,
    pub total_bytes: u64,
    pub received_bytes: u64,
    pub state: DownloadState,
}

// --- browser.permission requests / responses ---

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum PermissionType {
    Geolocation,
    Notifications,
    AudioCapture,
    VideoCapture,
    ClipboardRead,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum PermissionDecision {
    Prompt,
    Granted,
    Denied,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryPermissionRequest {
    pub context_id: BrowserContextId,
    pub origin: String,
    pub permission_type: PermissionType,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetPermissionRequest {
    pub context_id: BrowserContextId,
    pub origin: String,
    pub permission_type: PermissionType,
    pub decision: PermissionDecision,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionResponse {
    pub context_id: BrowserContextId,
    pub origin: String,
    pub permission_type: PermissionType,
    pub decision: PermissionDecision,
}
