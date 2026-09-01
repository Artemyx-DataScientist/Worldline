//! CEF BrowserBackend implementation.

use std::sync::Arc;

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
        GetCookiesRequest, GetCookiesResponse, SetCookieRequest, SetCookieResponse,
    },
    query::DocumentSnapshot,
};
use worldline_browser_provider::{BrowserBackend, ReferenceBrowserBackend};

use crate::loop_runner::CefLoopRunner;

/// CEF implementation of BrowserBackend with thread-affine message loop management.
pub struct CefBrowserBackend {
    inner: ReferenceBrowserBackend,
    loop_runner: Option<Arc<CefLoopRunner>>,
}

impl Default for CefBrowserBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CefBrowserBackend {
    pub fn new() -> Self {
        let runner = CefLoopRunner::spawn().ok().map(Arc::new);
        Self {
            inner: ReferenceBrowserBackend::new(),
            loop_runner: runner,
        }
    }

    /// Access the thread-affine loop runner if active.
    pub fn loop_runner(&self) -> Option<&Arc<CefLoopRunner>> {
        self.loop_runner.as_ref()
    }
}

impl BrowserBackend for CefBrowserBackend {
    fn initialize(&mut self) -> Result<(), BrowserError> {
        self.inner.initialize()
    }

    fn shutdown(&mut self) -> Result<(), BrowserError> {
        self.inner.shutdown()
    }

    fn create_context(
        &mut self,
        req: &CreateContextRequest,
    ) -> Result<CreateContextResponse, BrowserError> {
        self.inner.create_context(req)
    }

    fn close_context(
        &mut self,
        req: &CloseContextRequest,
    ) -> Result<CloseContextResponse, BrowserError> {
        self.inner.close_context(req)
    }

    fn list_contexts(&self) -> Result<ListContextsResponse, BrowserError> {
        self.inner.list_contexts()
    }

    fn create_page(&mut self, req: &CreatePageRequest) -> Result<CreatePageResponse, BrowserError> {
        self.inner.create_page(req)
    }

    fn close_page(&mut self, req: &ClosePageRequest) -> Result<ClosePageResponse, BrowserError> {
        self.inner.close_page(req)
    }

    fn list_pages(&self, req: &ListPagesRequest) -> Result<ListPagesResponse, BrowserError> {
        self.inner.list_pages(req)
    }

    fn navigate(&mut self, req: &NavigateRequest) -> Result<NavigateResponse, BrowserError> {
        self.inner.navigate(req)
    }

    fn reload(&mut self, req: &ReloadRequest) -> Result<ReloadResponse, BrowserError> {
        self.inner.reload(req)
    }

    fn stop(&mut self, req: &StopRequest) -> Result<StopResponse, BrowserError> {
        self.inner.stop(req)
    }

    fn history_nav(&mut self, req: &HistoryNavRequest) -> Result<HistoryNavResponse, BrowserError> {
        self.inner.history_nav(req)
    }

    fn observe(&self, req: &ObservePageRequest) -> Result<PageObservation, BrowserError> {
        self.inner.observe(req)
    }

    fn query(&self, req: &QueryDocumentRequest) -> Result<DocumentSnapshot, BrowserError> {
        self.inner.query(req)
    }

    fn act(&mut self, req: &ActRequest) -> Result<ActionResult, BrowserError> {
        self.inner.act(req)
    }

    fn start_download(
        &mut self,
        req: &StartDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        self.inner.start_download(req)
    }

    fn control_download(
        &mut self,
        req: &ControlDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        self.inner.control_download(req)
    }

    fn query_permission(
        &self,
        req: &QueryPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError> {
        self.inner.query_permission(req)
    }

    fn set_permission(
        &mut self,
        req: &SetPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError> {
        self.inner.set_permission(req)
    }

    fn capture(&mut self, req: &CapturePageRequest) -> Result<CapturePageResponse, BrowserError> {
        self.inner.capture(req)
    }

    fn read_capture(
        &self,
        req: &ReadCaptureArtifactRequest,
    ) -> Result<ReadCaptureArtifactResponse, BrowserError> {
        self.inner.read_capture(req)
    }

    fn get_cookies(&self, req: &GetCookiesRequest) -> Result<GetCookiesResponse, BrowserError> {
        self.inner.get_cookies(req)
    }

    fn set_cookie(&mut self, req: &SetCookieRequest) -> Result<SetCookieResponse, BrowserError> {
        self.inner.set_cookie(req)
    }

    fn delete_cookies(
        &mut self,
        req: &DeleteCookiesRequest,
    ) -> Result<DeleteCookiesResponse, BrowserError> {
        self.inner.delete_cookies(req)
    }

    fn clear_storage(
        &mut self,
        req: &ClearStorageRequest,
    ) -> Result<ClearStorageResponse, BrowserError> {
        self.inner.clear_storage(req)
    }
}
