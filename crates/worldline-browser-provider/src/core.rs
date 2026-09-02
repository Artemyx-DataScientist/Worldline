//! BrowserProviderCore state machine, CAS generation reservation, and request dispatch.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;
use worldline_browser_contract::{
    action::{
        ClickActionRequest, FocusActionRequest, InputActionRequest, ScrollActionRequest,
        SubmitActionRequest, validate_element_reference,
    },
    authority::*,
    capture::{CapturePageRequest, ReadCaptureArtifactRequest},
    contracts::{
        ActRequest, CloseContextRequest, ClosePageRequest, ControlDownloadRequest,
        CreateContextRequest, CreatePageRequest, ElementQueryKind, ExtractTextRequest,
        ExtractTextResponse, FindElementsRequest, FindElementsResponse, HistoryNavRequest,
        ListPagesRequest, NavigateRequest, ObservePageRequest, QueryAccessibilityRequest,
        QueryDocumentRequest, QueryPermissionRequest, ReloadRequest, SetPermissionRequest,
        StartDownloadRequest, StopRequest,
    },
    error::BrowserError,
    identity::{DocumentRevision, ElementRef, PageId},
    primitives::{
        ClearStorageRequest, DeleteCookiesRequest, GetCookiesRequest, SetCookieRequest,
        SetCookieRequestV0_2, StorageItemRequestV0_2,
    },
    query::SemanticElement,
};

use crate::backend::BrowserBackend;
use crate::request_policy::RequestPolicyBroker;

/// Resource and rate budget limits for the provider.
#[derive(Clone, Debug)]
pub struct ProviderBudgetLimits {
    pub max_contexts: usize,
    pub max_pages_per_context: usize,
    pub max_action_text_len: usize,
}

impl Default for ProviderBudgetLimits {
    fn default() -> Self {
        Self {
            max_contexts: 32,
            max_pages_per_context: 64,
            max_action_text_len: 65536,
        }
    }
}

/// Core browser provider engine managing backend lifecycle, CAS generation reservations, and RPC routing.
pub struct BrowserProviderCore<B: BrowserBackend> {
    backend: Mutex<B>,
    page_generations: Mutex<BTreeMap<PageId, u64>>,
    limits: ProviderBudgetLimits,
    request_policy: RequestPolicyBroker,
}

impl<B: BrowserBackend> BrowserProviderCore<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend: Mutex::new(backend),
            page_generations: Mutex::new(BTreeMap::new()),
            limits: ProviderBudgetLimits::default(),
            request_policy: RequestPolicyBroker::new(),
        }
    }

    pub fn with_limits(backend: B, limits: ProviderBudgetLimits) -> Self {
        Self {
            backend: Mutex::new(backend),
            page_generations: Mutex::new(BTreeMap::new()),
            limits,
            request_policy: RequestPolicyBroker::new(),
        }
    }

    /// Runs a read-only inspection against the concrete backend without
    /// exposing it to callers or allowing a second ownership path. Native
    /// adapters use this to drain engine-originated events after a capability
    /// dispatch; the capability request and the event publication remain
    /// separate protocol messages.
    pub fn with_backend<R>(&self, callback: impl FnOnce(&B) -> R) -> R {
        let backend = self.backend.lock().unwrap();
        callback(&backend)
    }

    /// Returns the engine-neutral request-policy broker owned by this
    /// provider core. The broker is separate from the event transport and the
    /// concrete backend.
    pub fn request_policy(&self) -> &RequestPolicyBroker {
        &self.request_policy
    }

    /// Shuts down the concrete backend while the provider still owns it.
    ///
    /// Native provider processes use this lifecycle hook before terminating
    /// so that an engine-backed backend can tear down its UI/message-loop and
    /// subprocess tree on the owning thread.
    pub fn shutdown_backend(&self) -> Result<(), BrowserError> {
        let result = self.backend.lock().unwrap().shutdown();
        self.request_policy.invalidate_all();
        result
    }

    /// Reserves a generation for a page under CAS semantics.
    pub fn reserve_generation(
        &self,
        page_id: &PageId,
        expected: Option<u64>,
    ) -> Result<u64, BrowserError> {
        let mut gens = self.page_generations.lock().unwrap();
        let current = gens.entry(page_id.clone()).or_insert(1);
        if let Some(expected_gen) = expected
            && *current != expected_gen
        {
            return Err(BrowserError::StaleElementReference {
                expected_revision: DocumentRevision::new(expected_gen),
                actual_revision: DocumentRevision::new(*current),
            });
        }
        *current += 1;
        Ok(*current)
    }

    /// Dispatches a capability operation with explicit contract and operation strings.
    pub fn dispatch_contract(
        &self,
        contract: &str,
        operation: &str,
        payload: Value,
    ) -> Result<Value, BrowserError> {
        match (contract, operation) {
            // Context
            ("browser.context", OP_CREATE_CONTEXT) | ("browser.context", "create_context") => {
                let req: CreateContextRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("create_context payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.create_context(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.context", OP_CLOSE_CONTEXT) | ("browser.context", "close_context") => {
                let req: CloseContextRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("close_context payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.close_context(&req)?;
                drop(backend);
                self.request_policy.invalidate_context(&req.context_id);
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.context", OP_LIST_CONTEXTS) | ("browser.context", "list_contexts") => {
                let backend = self.backend.lock().unwrap();
                let res = backend.list_contexts()?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Page
            ("browser.page", OP_CREATE_PAGE) | ("browser.page", "create_page") => {
                let req: CreatePageRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("create_page payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.create_page(&req)?;
                self.page_generations
                    .lock()
                    .unwrap()
                    .insert(res.page_id.clone(), 1);
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.page", OP_CLOSE_PAGE) | ("browser.page", "close_page") => {
                let req: ClosePageRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("close_page payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.close_page(&req)?;
                drop(backend);
                self.page_generations.lock().unwrap().remove(&req.page_id);
                self.request_policy.invalidate_page(&req.page_id);
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.page", OP_LIST_PAGES) | ("browser.page", "list_pages") => {
                let req: ListPagesRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("list_pages payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.list_pages(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Navigate
            ("browser.navigate", OP_NAVIGATE) => {
                let req: NavigateRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("navigate payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.navigate(&req)?;
                self.reserve_generation(&req.page_id, None)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.navigate", OP_RELOAD) => {
                let req: ReloadRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("reload payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.reload(&req)?;
                self.reserve_generation(&req.page_id, None)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.navigate", OP_STOP) => {
                let req: StopRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("stop payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.stop(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.navigate", OP_BACK) | ("browser.navigate", OP_FORWARD) => {
                let req: HistoryNavRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("history_nav payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.history_nav(&req)?;
                self.reserve_generation(&req.page_id, None)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Observe
            ("browser.observe", OP_OBSERVE) => {
                let req: ObservePageRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("observe payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.observe(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.observe", OP_GET_TITLE) => {
                let req: ObservePageRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("get_title payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let obs = backend.observe(&req)?;
                Ok(serde_json::json!({"page_id": obs.page_id, "title": obs.title}))
            }
            ("browser.observe", OP_GET_URL) => {
                let req: ObservePageRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("get_url payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let obs = backend.observe(&req)?;
                Ok(serde_json::json!({"page_id": obs.page_id, "url": obs.url}))
            }

            // Query
            ("browser.query", OP_QUERY_DOCUMENT) => {
                let req: QueryDocumentRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("query_document payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.query(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.query", OP_QUERY_ACCESSIBILITY) => {
                let req: QueryAccessibilityRequest =
                    serde_json::from_value(payload).map_err(|e| {
                        BrowserError::InvalidRequest(format!(
                            "query_accessibility payload invalid: {e}"
                        ))
                    })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.query(&QueryDocumentRequest {
                    page_id: req.page_id,
                    bounds: req.bounds,
                })?;
                Ok(serde_json::to_value(res.accessibility_tree).unwrap())
            }
            ("browser.query", OP_FIND_ELEMENTS) => {
                let req: FindElementsRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("find_elements payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let doc = backend.query(&QueryDocumentRequest {
                    page_id: req.page_id.clone(),
                    bounds: None,
                })?;
                let mut elements = Vec::new();
                for node in doc.accessibility_tree.root.children.iter() {
                    let is_match = match req.kind {
                        ElementQueryKind::AccessibilityRole => {
                            format!("{:?}", node.role).eq_ignore_ascii_case(&req.query)
                        }
                        ElementQueryKind::TextMatch => node
                            .name
                            .as_ref()
                            .map(|n| n.contains(&req.query))
                            .unwrap_or(false),
                        ElementQueryKind::CssSelector => node.node_id.contains(&req.query),
                    };
                    if is_match {
                        elements.push(SemanticElement {
                            element_ref: ElementRef::new(
                                req.page_id.clone(),
                                doc.metadata.document_revision,
                                node.node_id.clone(),
                            ),
                            tag_name: format!("{:?}", node.role).to_lowercase(),
                            attributes: BTreeMap::new(),
                            text_content: node.name.clone().unwrap_or_default(),
                        });
                    }
                }
                let resp = FindElementsResponse {
                    page_id: req.page_id,
                    document_revision: doc.metadata.document_revision,
                    elements,
                };
                Ok(serde_json::to_value(resp).unwrap())
            }
            ("browser.query", OP_EXTRACT_TEXT) => {
                let req: ExtractTextRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("extract_text payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let doc = backend.query(&QueryDocumentRequest {
                    page_id: req.page_id.clone(),
                    bounds: None,
                })?;
                let text = doc.accessibility_tree.root.collect_text();
                let resp = ExtractTextResponse {
                    page_id: req.page_id,
                    document_revision: doc.metadata.document_revision,
                    text,
                };
                Ok(serde_json::to_value(resp).unwrap())
            }

            // Act
            ("browser.act", OP_CLICK) => {
                let req: ClickActionRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("click payload invalid: {e}"))
                })?;
                let page_id = req.element_ref.page_id().clone();
                let rev = req.element_ref.document_revision();
                self.validate_and_act(&page_id, rev, ActRequest::Click(req))
            }
            ("browser.act", OP_INPUT) => {
                let req: InputActionRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("input payload invalid: {e}"))
                })?;
                if req.text.len() > self.limits.max_action_text_len {
                    return Err(BrowserError::InvalidRequest(format!(
                        "input text length {} exceeds limit {}",
                        req.text.len(),
                        self.limits.max_action_text_len
                    )));
                }
                let page_id = req.element_ref.page_id().clone();
                let rev = req.element_ref.document_revision();
                self.validate_and_act(&page_id, rev, ActRequest::Input(req))
            }
            ("browser.act", OP_FOCUS) => {
                let req: FocusActionRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("focus payload invalid: {e}"))
                })?;
                let page_id = req.element_ref.page_id().clone();
                let rev = req.element_ref.document_revision();
                self.validate_and_act(&page_id, rev, ActRequest::Focus(req))
            }
            ("browser.act", OP_SUBMIT) => {
                let req: SubmitActionRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("submit payload invalid: {e}"))
                })?;
                let page_id = req.element_ref.page_id().clone();
                let rev = req.element_ref.document_revision();
                self.validate_and_act(&page_id, rev, ActRequest::Submit(req))
            }
            ("browser.act", OP_SCROLL) => {
                let req: ScrollActionRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("scroll payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.act(&ActRequest::Scroll(req.clone()))?;
                self.reserve_generation(&req.page_id, None)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Download
            ("browser.download", OP_DOWNLOAD_START) => {
                let req: StartDownloadRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("start_download payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.start_download(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.download", OP_DOWNLOAD_CONTROL)
            | ("browser.download", OP_DOWNLOAD_STATUS) => {
                let req: ControlDownloadRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("control_download payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.control_download(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Permission
            ("browser.permission", OP_PERMISSION_QUERY) => {
                let req: QueryPermissionRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("query_permission payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.query_permission(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.permission", OP_PERMISSION_SET) => {
                let req: SetPermissionRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("set_permission payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.set_permission(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Capture
            ("browser.capture", OP_CAPTURE) => {
                let req: CapturePageRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("capture payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.capture(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.capture", OP_READ_CAPTURE) => {
                let req: ReadCaptureArtifactRequest =
                    serde_json::from_value(payload).map_err(|e| {
                        BrowserError::InvalidRequest(format!("read_capture payload invalid: {e}"))
                    })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.read_capture(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Primitives: Cookies
            ("browser.engine.cookies", OP_COOKIE_GET) => {
                let req: GetCookiesRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("get_cookies payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.get_cookies(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.engine.cookies", OP_COOKIE_SET) => {
                let req: SetCookieRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("set_cookie payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.set_cookie(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.engine.cookies", OP_COOKIE_GET_V0_2) => {
                let req: GetCookiesRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("get_cookies v0.2 payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.get_cookies_v0_2(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.engine.cookies", OP_COOKIE_SET_V0_2) => {
                let req: SetCookieRequestV0_2 = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("set_cookie v0.2 payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.set_cookie_v0_2(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.engine.cookies", OP_COOKIE_DELETE) => {
                let req: DeleteCookiesRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("delete_cookies payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.delete_cookies(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            // Primitives: Storage
            ("browser.engine.storage", OP_STORAGE_CLEAR) => {
                let req: ClearStorageRequest = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("clear_storage payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.clear_storage(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.engine.storage", OP_STORAGE_SET_V0_2) => {
                let req: StorageItemRequestV0_2 = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("set_storage_item payload invalid: {e}"))
                })?;
                let mut backend = self.backend.lock().unwrap();
                let res = backend.set_storage_item(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }
            ("browser.engine.storage", OP_STORAGE_GET_V0_2) => {
                let req: StorageItemRequestV0_2 = serde_json::from_value(payload).map_err(|e| {
                    BrowserError::InvalidRequest(format!("get_storage_item payload invalid: {e}"))
                })?;
                let backend = self.backend.lock().unwrap();
                let res = backend.get_storage_item(&req)?;
                Ok(serde_json::to_value(res).unwrap())
            }

            (c, op) => Err(BrowserError::InvalidRequest(format!(
                "unknown operation: {c}/{op}"
            ))),
        }
    }

    /// Dispatches a single operation string or qualified operation path.
    pub fn dispatch(&self, operation: &str, payload: Value) -> Result<Value, BrowserError> {
        let (contract, op) = match operation {
            "browser.context.create" | "create_context" => ("browser.context", OP_CREATE_CONTEXT),
            "browser.context.close" | "close_context" => ("browser.context", OP_CLOSE_CONTEXT),
            "browser.context.list" | "list_contexts" => ("browser.context", OP_LIST_CONTEXTS),

            "browser.page.create" | "create_page" => ("browser.page", OP_CREATE_PAGE),
            "browser.page.close" | "close_page" => ("browser.page", OP_CLOSE_PAGE),
            "browser.page.list" | "list_pages" => ("browser.page", OP_LIST_PAGES),

            "browser.navigate.goto" | "navigate" => ("browser.navigate", OP_NAVIGATE),
            "browser.navigate.reload" | "reload" => ("browser.navigate", OP_RELOAD),
            "browser.navigate.stop" | "stop" => ("browser.navigate", OP_STOP),
            "browser.navigate.back" | "back" => ("browser.navigate", OP_BACK),
            "browser.navigate.forward" | "forward" => ("browser.navigate", OP_FORWARD),

            "browser.observe.snapshot" | "observe" => ("browser.observe", OP_OBSERVE),
            "browser.observe.title" | "get_title" => ("browser.observe", OP_GET_TITLE),
            "browser.observe.url" | "get_url" => ("browser.observe", OP_GET_URL),

            "browser.query.document" | "query_document" => ("browser.query", OP_QUERY_DOCUMENT),
            "browser.query.accessibility" | "query_accessibility" => {
                ("browser.query", OP_QUERY_ACCESSIBILITY)
            }
            "browser.query.find_elements" | "find_elements" => ("browser.query", OP_FIND_ELEMENTS),
            "browser.query.extract_text" | "extract_text" => ("browser.query", OP_EXTRACT_TEXT),

            "browser.act.click" | "click" => ("browser.act", OP_CLICK),
            "browser.act.input" | "input" => ("browser.act", OP_INPUT),
            "browser.act.focus" | "focus" => ("browser.act", OP_FOCUS),
            "browser.act.submit" | "submit" => ("browser.act", OP_SUBMIT),
            "browser.act.scroll" | "scroll" => ("browser.act", OP_SCROLL),

            "browser.download.start" | "start_download" => ("browser.download", OP_DOWNLOAD_START),
            "browser.download.control" | "control_download" => {
                ("browser.download", OP_DOWNLOAD_CONTROL)
            }
            "browser.download.status" | "download_status" => {
                ("browser.download", OP_DOWNLOAD_STATUS)
            }

            "browser.permission.query" | "query_permission" => {
                ("browser.permission", OP_PERMISSION_QUERY)
            }
            "browser.permission.set" | "set_permission" => {
                ("browser.permission", OP_PERMISSION_SET)
            }

            "browser.capture.viewport" | "capture" => ("browser.capture", OP_CAPTURE),
            "browser.capture.read_artifact" | "read_capture" => {
                ("browser.capture", OP_READ_CAPTURE)
            }

            "browser.engine.cookies.get" | "cookie_get" | "get_cookies" => {
                ("browser.engine.cookies", OP_COOKIE_GET)
            }
            "browser.engine.cookies.set" | "cookie_set" | "set_cookie" => {
                ("browser.engine.cookies", OP_COOKIE_SET)
            }
            "browser.engine.cookies.delete" | "cookie_delete" | "delete_cookies" => {
                ("browser.engine.cookies", OP_COOKIE_DELETE)
            }
            "browser.engine.cookies.v0_2.get" | "cookie_get_v0_2" => {
                ("browser.engine.cookies", OP_COOKIE_GET_V0_2)
            }
            "browser.engine.cookies.v0_2.set" | "cookie_set_v0_2" => {
                ("browser.engine.cookies", OP_COOKIE_SET_V0_2)
            }

            "browser.engine.storage.clear" | "storage_clear" | "clear_storage" => {
                ("browser.engine.storage", OP_STORAGE_CLEAR)
            }
            "browser.engine.storage.v0_2.set" | "storage_set_v0_2" => {
                ("browser.engine.storage", OP_STORAGE_SET_V0_2)
            }
            "browser.engine.storage.v0_2.get" | "storage_get_v0_2" => {
                ("browser.engine.storage", OP_STORAGE_GET_V0_2)
            }

            "create" => {
                if payload.get("context_id").is_some() {
                    ("browser.page", OP_CREATE_PAGE)
                } else {
                    ("browser.context", OP_CREATE_CONTEXT)
                }
            }
            "close" => {
                if payload.get("page_id").is_some() {
                    ("browser.page", OP_CLOSE_PAGE)
                } else {
                    ("browser.context", OP_CLOSE_CONTEXT)
                }
            }
            "list" => {
                if payload.get("context_id").is_some() {
                    ("browser.page", OP_LIST_PAGES)
                } else {
                    ("browser.context", OP_LIST_CONTEXTS)
                }
            }
            "query" => ("browser.permission", OP_PERMISSION_QUERY),
            "set" => ("browser.permission", OP_PERMISSION_SET),
            "start" => ("browser.download", OP_DOWNLOAD_START),
            "control" => ("browser.download", OP_DOWNLOAD_CONTROL),
            "status" => ("browser.download", OP_DOWNLOAD_STATUS),

            other => {
                if let Some((c, o)) = other.split_once('/') {
                    (c, o)
                } else if let Some((c, o)) = other.split_once(':') {
                    (c, o)
                } else {
                    return Err(BrowserError::InvalidRequest(format!(
                        "unrecognized operation: {other}"
                    )));
                }
            }
        };

        self.dispatch_contract(contract, op, payload)
    }

    fn validate_and_act(
        &self,
        page_id: &PageId,
        expected_revision: DocumentRevision,
        act_req: ActRequest,
    ) -> Result<Value, BrowserError> {
        let obs = {
            let backend = self.backend.lock().unwrap();
            backend.observe(&ObservePageRequest {
                page_id: page_id.clone(),
            })?
        };

        let target_elem = match &act_req {
            ActRequest::Click(c) => Some(&c.element_ref),
            ActRequest::Input(i) => Some(&i.element_ref),
            ActRequest::Focus(f) => Some(&f.element_ref),
            ActRequest::Submit(s) => Some(&s.element_ref),
            ActRequest::Scroll(_) => None,
        };

        if let Some(elem_ref) = target_elem {
            validate_element_reference(elem_ref, page_id, obs.document_revision)?;
        } else if expected_revision != obs.document_revision {
            return Err(BrowserError::StaleElementReference {
                expected_revision,
                actual_revision: obs.document_revision,
            });
        }

        let mut backend = self.backend.lock().unwrap();
        let res = backend.act(&act_req)?;
        self.reserve_generation(page_id, None)?;
        Ok(serde_json::to_value(res).unwrap())
    }
}
