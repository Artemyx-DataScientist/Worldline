//! Pluggable browser engine backend trait definition.

use std::sync::Arc;

use crate::request_policy::RequestPolicyTransport;

use worldline_browser_contract::{
    action::ActionResult,
    capture::{
        CapturePageRequest, CapturePageResponse, ReadCaptureArtifactRequest,
        ReadCaptureArtifactResponse,
    },
    contracts::{
        ActRequest, CloseContextRequest, CloseContextResponse, ClosePageRequest, ClosePageResponse,
        ControlDownloadRequest, CreateContextRequest, CreateContextResponse, CreatePageRequest,
        CreatePageResponse, DownloadStatusResponse, HistoryNavRequest, HistoryNavResponse,
        ListContextsResponse, ListPagesRequest, ListPagesResponse, NavigateRequest,
        NavigateResponse, ObservePageRequest, PageObservation, PermissionResponse,
        QueryDocumentRequest, QueryPermissionRequest, ReloadRequest, ReloadResponse,
        SetPermissionRequest, StartDownloadRequest, StopRequest, StopResponse,
    },
    error::BrowserError,
    primitives::{
        ClearStorageRequest, ClearStorageResponse, DeleteCookiesRequest, DeleteCookiesResponse,
        GetCookiesRequest, GetCookiesResponse, GetCookiesResponseV0_2, SetCookieRequest,
        SetCookieRequestV0_2, SetCookieResponse, StorageItemRequestV0_2, StorageItemResponseV0_2,
    },
    query::DocumentSnapshot,
    request_policy::RequestPolicyFailureMode,
};

/// Engine-neutral interface implemented by concrete browser providers (e.g. Reference, CEF).
pub trait BrowserBackend: Send + Sync {
    fn initialize(&mut self) -> Result<(), BrowserError>;
    fn shutdown(&mut self) -> Result<(), BrowserError>;

    /// Installs the physical request/result transport used by an engine
    /// adapter for pre-dispatch policy callbacks. The default keeps reference
    /// and other adapters transport-neutral; CEF may retain only this
    /// engine-neutral hook, never the native writer or policy implementation.
    fn set_request_policy_transport(&mut self, _transport: Arc<dyn RequestPolicyTransport>) {}

    /// Declares the registration/profile that owns engine interception and
    /// its failure semantics. A concrete engine may ignore this when it does
    /// not implement interception.
    fn set_request_policy_profile(
        &mut self,
        _registration_id: String,
        _failure_mode: RequestPolicyFailureMode,
    ) {
    }

    // Context management
    fn create_context(
        &mut self,
        req: &CreateContextRequest,
    ) -> Result<CreateContextResponse, BrowserError>;
    fn close_context(
        &mut self,
        req: &CloseContextRequest,
    ) -> Result<CloseContextResponse, BrowserError>;
    fn list_contexts(&self) -> Result<ListContextsResponse, BrowserError>;

    // Page management
    fn create_page(&mut self, req: &CreatePageRequest) -> Result<CreatePageResponse, BrowserError>;
    fn close_page(&mut self, req: &ClosePageRequest) -> Result<ClosePageResponse, BrowserError>;
    fn list_pages(&self, req: &ListPagesRequest) -> Result<ListPagesResponse, BrowserError>;

    // Navigation
    fn navigate(&mut self, req: &NavigateRequest) -> Result<NavigateResponse, BrowserError>;
    fn reload(&mut self, req: &ReloadRequest) -> Result<ReloadResponse, BrowserError>;
    fn stop(&mut self, req: &StopRequest) -> Result<StopResponse, BrowserError>;
    fn history_nav(&mut self, req: &HistoryNavRequest) -> Result<HistoryNavResponse, BrowserError>;

    // Observation and query
    fn observe(&self, req: &ObservePageRequest) -> Result<PageObservation, BrowserError>;
    fn query(&self, req: &QueryDocumentRequest) -> Result<DocumentSnapshot, BrowserError>;

    // Action
    fn act(&mut self, req: &ActRequest) -> Result<ActionResult, BrowserError>;

    // Download
    fn start_download(
        &mut self,
        req: &StartDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError>;
    fn control_download(
        &mut self,
        req: &ControlDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError>;

    // Permission
    fn query_permission(
        &self,
        req: &QueryPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError>;
    fn set_permission(
        &mut self,
        req: &SetPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError>;

    // Experimental: capture
    fn capture(&mut self, req: &CapturePageRequest) -> Result<CapturePageResponse, BrowserError>;
    fn read_capture(
        &self,
        req: &ReadCaptureArtifactRequest,
    ) -> Result<ReadCaptureArtifactResponse, BrowserError>;

    // Experimental: primitives
    fn get_cookies(&self, req: &GetCookiesRequest) -> Result<GetCookiesResponse, BrowserError>;
    fn set_cookie(&mut self, req: &SetCookieRequest) -> Result<SetCookieResponse, BrowserError>;

    /// Additive engine.cookies/0.2 path. The default adapter preserves the
    /// 0.1 behavior for backends that do not expose explicit scope semantics;
    /// native providers override it because host-only/domain-cookie scope is
    /// security-relevant and cannot be inferred from a 0.1 DTO.
    fn get_cookies_v0_2(
        &self,
        req: &GetCookiesRequest,
    ) -> Result<GetCookiesResponseV0_2, BrowserError> {
        let response = self.get_cookies(req)?;
        Ok(GetCookiesResponseV0_2 {
            cookies: response.cookies.into_iter().map(Into::into).collect(),
        })
    }

    fn set_cookie_v0_2(
        &mut self,
        req: &SetCookieRequestV0_2,
    ) -> Result<SetCookieResponse, BrowserError> {
        self.set_cookie(&SetCookieRequest {
            context_id: req.context_id.clone(),
            cookie: req.cookie.clone().into(),
        })
    }
    fn delete_cookies(
        &mut self,
        req: &DeleteCookiesRequest,
    ) -> Result<DeleteCookiesResponse, BrowserError>;
    fn clear_storage(
        &mut self,
        req: &ClearStorageRequest,
    ) -> Result<ClearStorageResponse, BrowserError>;

    fn set_storage_item(
        &mut self,
        _req: &StorageItemRequestV0_2,
    ) -> Result<StorageItemResponseV0_2, BrowserError> {
        Err(BrowserError::UnsupportedOperation(
            "storage item set is not implemented by this backend".to_string(),
        ))
    }

    fn get_storage_item(
        &self,
        _req: &StorageItemRequestV0_2,
    ) -> Result<StorageItemResponseV0_2, BrowserError> {
        Err(BrowserError::UnsupportedOperation(
            "storage item get is not implemented by this backend".to_string(),
        ))
    }
}
