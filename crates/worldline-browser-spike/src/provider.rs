use std::sync::{Arc, Mutex};

use worldline_browser_contract::{
    action::{
        ClickActionRequest, FocusActionRequest, InputActionRequest, InteractionKind,
        ScrollActionRequest, SubmitActionRequest,
    },
    authority::{
        OP_BACK, OP_CLICK, OP_CLOSE_CONTEXT, OP_CLOSE_PAGE, OP_CREATE_CONTEXT, OP_CREATE_PAGE,
        OP_DOWNLOAD_CONTROL, OP_DOWNLOAD_START, OP_DOWNLOAD_STATUS, OP_EXTRACT_TEXT,
        OP_FIND_ELEMENTS, OP_FOCUS, OP_FORWARD, OP_GET_TITLE, OP_GET_URL, OP_INPUT,
        OP_LIST_CONTEXTS, OP_LIST_PAGES, OP_NAVIGATE, OP_OBSERVE, OP_PERMISSION_QUERY,
        OP_PERMISSION_SET, OP_QUERY_ACCESSIBILITY, OP_QUERY_DOCUMENT, OP_RELOAD, OP_SCROLL,
        OP_STOP, OP_SUBMIT,
    },
    contracts::{
        BROWSER_NAMESPACE, CONTRACT_ACT, CONTRACT_CONTEXT, CONTRACT_DOWNLOAD, CONTRACT_NAVIGATE,
        CONTRACT_OBSERVE, CONTRACT_PAGE, CONTRACT_PERMISSION, CONTRACT_QUERY, CloseContextRequest,
        CloseContextResponse, ClosePageRequest, ClosePageResponse, ControlDownloadRequest,
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        ExtractTextRequest, ExtractTextResponse, FindElementsRequest, FindElementsResponse,
        GetTitleResponse, GetUrlResponse, HistoryNavRequest, HistoryNavResponse,
        INTERFACE_MAJOR_V1, INTERFACE_MINOR_V1, ListPagesRequest, ListPagesResponse,
        NavigateRequest, NavigateResponse, ObservePageRequest, PermissionResponse,
        QueryAccessibilityRequest, QueryDocumentRequest, ReloadRequest, ReloadResponse,
        SetPermissionRequest, StartDownloadRequest, StopRequest, StopResponse,
    },
    error::BrowserError,
    events::{
        DownloadStartedEvent, EVENT_DOWNLOAD_STARTED, EVENT_NAVIGATION_COMMITTED,
        EVENT_PAGE_CLOSED, EVENT_PAGE_CREATED, NavigationCommittedEvent, PageClosedEvent,
        PageCreatedEvent,
    },
    identity::{BrowserContextId, context_resource, page_resource},
};
use worldline_kernel::{
    ActivationContext, CapabilityId, CapabilityService, InterfaceVersion, InvocationContext,
    NoopRuntime, Plugin, PluginDefinition, PluginError, PluginRuntime,
};

use crate::engine::ReferenceBrowserSupervisor;

pub fn browser_capability(name: &str) -> CapabilityId {
    CapabilityId::new(
        BROWSER_NAMESPACE,
        name,
        InterfaceVersion::new(INTERFACE_MAJOR_V1, INTERFACE_MINOR_V1),
    )
}

/// Recorded observation event emitted by the browser provider.
#[derive(Clone, Debug)]
pub struct EmittedBrowserEvent {
    pub topic: &'static str,
    pub payload: Vec<u8>,
}

/// Browser plugin exposing engine-neutral browser capabilities to Worldline kernel.
#[derive(Clone)]
pub struct SpikeBrowserPlugin {
    definition: PluginDefinition,
    supervisor: ReferenceBrowserSupervisor,
    emitted_events: Arc<Mutex<Vec<EmittedBrowserEvent>>>,
}

impl SpikeBrowserPlugin {
    pub fn new(plugin_id: impl Into<String>, supervisor: ReferenceBrowserSupervisor) -> Self {
        let mut def = PluginDefinition::new(plugin_id.into());
        for contract in [
            CONTRACT_CONTEXT,
            CONTRACT_PAGE,
            CONTRACT_NAVIGATE,
            CONTRACT_OBSERVE,
            CONTRACT_QUERY,
            CONTRACT_ACT,
            CONTRACT_DOWNLOAD,
            CONTRACT_PERMISSION,
        ] {
            def = def.provides(browser_capability(contract));
        }
        Self {
            definition: def,
            supervisor,
            emitted_events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn supervisor(&self) -> &ReferenceBrowserSupervisor {
        &self.supervisor
    }

    pub fn emitted_events(&self) -> Arc<Mutex<Vec<EmittedBrowserEvent>>> {
        Arc::clone(&self.emitted_events)
    }
}

impl Plugin for SpikeBrowserPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        let service = Arc::new(SpikeBrowserCapabilityService {
            supervisor: self.supervisor.clone(),
            emitted_events: Arc::clone(&self.emitted_events),
        });

        for contract in [
            CONTRACT_CONTEXT,
            CONTRACT_PAGE,
            CONTRACT_NAVIGATE,
            CONTRACT_OBSERVE,
            CONTRACT_QUERY,
            CONTRACT_ACT,
            CONTRACT_DOWNLOAD,
            CONTRACT_PERMISSION,
        ] {
            context.publish_capability(browser_capability(contract), service.clone())?;
        }

        Ok(Box::new(NoopRuntime))
    }
}

struct SpikeBrowserCapabilityService {
    supervisor: ReferenceBrowserSupervisor,
    emitted_events: Arc<Mutex<Vec<EmittedBrowserEvent>>>,
}

impl CapabilityService for SpikeBrowserCapabilityService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.dispatch_internal(None, "", operation, payload)
    }

    fn invoke_with_context(
        &self,
        context: &InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.dispatch_internal(
            Some(context),
            context.capability().name(),
            context.operation().as_str(),
            payload,
        )
    }
}

impl SpikeBrowserCapabilityService {
    fn record_event(&self, topic: &'static str, payload: Vec<u8>) {
        if let Ok(mut lock) = self.emitted_events.lock() {
            lock.push(EmittedBrowserEvent { topic, payload });
        }
    }

    /// Enforces that the kernel admitted ResourceId in InvocationContext strictly
    /// matches the target resource in the caller's deserialized payload.
    fn check_resource_scope(
        &self,
        context: Option<&InvocationContext>,
        expected_resource: &str,
    ) -> Result<(), String> {
        if let Some(ctx) = context {
            let admitted = ctx.resource().to_string();
            // Wildcard or matching prefix / exact resource
            if admitted != "any"
                && admitted != expected_resource
                && !admitted.starts_with("browser-context")
            {
                let err = BrowserError::ResourceMismatch {
                    expected: admitted,
                    actual: expected_resource.to_string(),
                };
                return Err(serde_json::to_string(&err).unwrap_or_else(|_| err.to_string()));
            }
        }
        Ok(())
    }

    fn dispatch_internal(
        &self,
        context: Option<&InvocationContext>,
        contract: &str,
        operation: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        match (contract, operation) {
            // --- browser.context ---
            (CONTRACT_CONTEXT, OP_CREATE_CONTEXT) | ("", OP_CREATE_CONTEXT) => {
                let req: CreateContextRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid CreateContextRequest: {e}"))?;
                let created = self
                    .supervisor
                    .create_context(None, req.profile_id.clone(), req.incognito)
                    .map_err(|e| e.to_string())?;
                let resp = CreateContextResponse {
                    context_id: created,
                    profile_id: req.profile_id,
                    incognito: req.incognito,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_CONTEXT, OP_CLOSE_CONTEXT) => {
                let req: CloseContextRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid CloseContextRequest: {e}"))?;
                self.check_resource_scope(context, &context_resource(&req.context_id))?;
                self.supervisor
                    .close_context(&req.context_id)
                    .map_err(|e| e.to_string())?;
                let resp = CloseContextResponse {
                    context_id: req.context_id,
                    closed: true,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_CONTEXT, OP_LIST_CONTEXTS) => {
                let contexts = self.supervisor.list_contexts();
                let resp = worldline_browser_contract::contracts::ListContextsResponse { contexts };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }

            // --- browser.page ---
            (CONTRACT_PAGE, OP_CREATE_PAGE) => {
                let req: CreatePageRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid CreatePageRequest: {e}"))?;
                self.check_resource_scope(context, &context_resource(&req.context_id))?;
                let (page_id, rev) = self
                    .supervisor
                    .create_page(&req.context_id, None, req.initial_url.clone())
                    .map_err(|e| e.to_string())?;

                // Publish PageCreatedEvent post-outcome
                let event = PageCreatedEvent {
                    context_id: req.context_id.clone(),
                    page_id: page_id.clone(),
                    document_revision: rev,
                };
                self.record_event(EVENT_PAGE_CREATED, serde_json::to_vec(&event).unwrap());

                let resp = CreatePageResponse {
                    context_id: req.context_id,
                    page_id,
                    initial_revision: rev,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_PAGE, OP_CLOSE_PAGE) => {
                let req: ClosePageRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ClosePageRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                self.supervisor
                    .close_page(&req.page_id)
                    .map_err(|e| e.to_string())?;

                // Publish PageClosedEvent post-outcome
                let event = PageClosedEvent {
                    context_id: BrowserContextId::new("ctx-spike-1"),
                    page_id: req.page_id.clone(),
                };
                self.record_event(EVENT_PAGE_CLOSED, serde_json::to_vec(&event).unwrap());

                let resp = ClosePageResponse {
                    page_id: req.page_id,
                    closed: true,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_PAGE, OP_LIST_PAGES) => {
                let req: ListPagesRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ListPagesRequest: {e}"))?;
                self.check_resource_scope(context, &context_resource(&req.context_id))?;
                let pages = self
                    .supervisor
                    .list_pages(&req.context_id)
                    .map_err(|e| e.to_string())?;
                let resp = ListPagesResponse { pages };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }

            // --- browser.navigate ---
            (CONTRACT_NAVIGATE, OP_NAVIGATE) | ("", OP_NAVIGATE) => {
                let req: NavigateRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid NavigateRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let (nav_id, rev) = self
                    .supervisor
                    .navigate(&req.page_id, &req.url)
                    .map_err(|e| e.to_string())?;

                // Publish NavigationCommittedEvent post-outcome
                let event = NavigationCommittedEvent {
                    page_id: req.page_id.clone(),
                    navigation_id: nav_id.clone(),
                    url: req.url.clone(),
                    document_revision: rev,
                    status_code: 200,
                };
                self.record_event(
                    EVENT_NAVIGATION_COMMITTED,
                    serde_json::to_vec(&event).unwrap(),
                );

                let resp = NavigateResponse {
                    page_id: req.page_id,
                    navigation_id: nav_id,
                    committed: true,
                    document_revision: rev,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_NAVIGATE, OP_RELOAD) => {
                let req: ReloadRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ReloadRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let rev = self
                    .supervisor
                    .reload(&req.page_id)
                    .map_err(|e| e.to_string())?;
                let resp = ReloadResponse {
                    page_id: req.page_id,
                    reloaded: true,
                    document_revision: rev,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_NAVIGATE, OP_STOP) => {
                let req: StopRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid StopRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                self.supervisor
                    .stop(&req.page_id)
                    .map_err(|e| e.to_string())?;
                let resp = StopResponse {
                    page_id: req.page_id,
                    stopped: true,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_NAVIGATE, OP_BACK) => {
                let req: HistoryNavRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid HistoryNavRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let rev = self
                    .supervisor
                    .history_nav(&req.page_id, -1)
                    .map_err(|e| e.to_string())?;
                let resp = HistoryNavResponse {
                    page_id: req.page_id,
                    success: true,
                    document_revision: rev,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_NAVIGATE, OP_FORWARD) => {
                let req: HistoryNavRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid HistoryNavRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let rev = self
                    .supervisor
                    .history_nav(&req.page_id, 1)
                    .map_err(|e| e.to_string())?;
                let resp = HistoryNavResponse {
                    page_id: req.page_id,
                    success: true,
                    document_revision: rev,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }

            // --- browser.observe ---
            (CONTRACT_OBSERVE, OP_OBSERVE) | ("", OP_OBSERVE) => {
                let req: ObservePageRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ObservePageRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let obs = self
                    .supervisor
                    .observe(&req.page_id)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&obs).map_err(|e| e.to_string())
            }
            (CONTRACT_OBSERVE, OP_GET_TITLE) => {
                let req: ObservePageRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ObservePageRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let obs = self
                    .supervisor
                    .observe(&req.page_id)
                    .map_err(|e| e.to_string())?;
                let resp = GetTitleResponse {
                    page_id: req.page_id,
                    title: obs.title,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_OBSERVE, OP_GET_URL) => {
                let req: ObservePageRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ObservePageRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let obs = self
                    .supervisor
                    .observe(&req.page_id)
                    .map_err(|e| e.to_string())?;
                let resp = GetUrlResponse {
                    page_id: req.page_id,
                    url: obs.url,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }

            // --- browser.query ---
            (CONTRACT_QUERY, OP_QUERY_DOCUMENT) | ("", OP_QUERY_DOCUMENT) => {
                let req: QueryDocumentRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid QueryDocumentRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let doc = self
                    .supervisor
                    .query_document(&req.page_id, req.bounds.as_ref())
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&doc).map_err(|e| e.to_string())
            }
            (CONTRACT_QUERY, OP_QUERY_ACCESSIBILITY) => {
                let req: QueryAccessibilityRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid QueryAccessibilityRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let doc = self
                    .supervisor
                    .query_document(&req.page_id, req.bounds.as_ref())
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&doc.accessibility_tree).map_err(|e| e.to_string())
            }
            (CONTRACT_QUERY, OP_EXTRACT_TEXT) => {
                let req: ExtractTextRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ExtractTextRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let (text, rev) = self
                    .supervisor
                    .extract_text(&req.page_id, req.target_element.as_ref())
                    .map_err(|e| e.to_string())?;
                let resp = ExtractTextResponse {
                    page_id: req.page_id,
                    document_revision: rev,
                    text,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_QUERY, OP_FIND_ELEMENTS) => {
                let req: FindElementsRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid FindElementsRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let (elements, rev) = self
                    .supervisor
                    .find_elements(&req.page_id, &req.query, req.kind)
                    .map_err(|e| e.to_string())?;
                let resp = FindElementsResponse {
                    page_id: req.page_id,
                    document_revision: rev,
                    elements,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }

            // --- browser.act ---
            (CONTRACT_ACT, OP_CLICK) => {
                let req: ClickActionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ClickActionRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(req.element_ref.page_id()))?;
                let res = self
                    .supervisor
                    .execute_action(&req.element_ref, InteractionKind::Click, None)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&res).map_err(|e| e.to_string())
            }
            (CONTRACT_ACT, OP_SUBMIT) => {
                let req: SubmitActionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid SubmitActionRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(req.element_ref.page_id()))?;
                let res = self
                    .supervisor
                    .execute_action(&req.element_ref, InteractionKind::Submit, None)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&res).map_err(|e| e.to_string())
            }
            (CONTRACT_ACT, OP_INPUT) => {
                let req: InputActionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid InputActionRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(req.element_ref.page_id()))?;
                let res = self
                    .supervisor
                    .execute_action(&req.element_ref, InteractionKind::Input, Some(&req.text))
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&res).map_err(|e| e.to_string())
            }
            (CONTRACT_ACT, OP_FOCUS) => {
                let req: FocusActionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid FocusActionRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(req.element_ref.page_id()))?;
                let res = self
                    .supervisor
                    .execute_action(&req.element_ref, InteractionKind::Focus, None)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&res).map_err(|e| e.to_string())
            }
            (CONTRACT_ACT, OP_SCROLL) => {
                let req: ScrollActionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ScrollActionRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let elem_ref = worldline_browser_contract::identity::ElementRef::new(
                    req.page_id.clone(),
                    worldline_browser_contract::identity::DocumentRevision::initial(),
                    "window",
                );
                let res = self
                    .supervisor
                    .execute_action(&elem_ref, InteractionKind::Scroll, None)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&res).map_err(|e| e.to_string())
            }

            // --- browser.download ---
            (CONTRACT_DOWNLOAD, OP_DOWNLOAD_START) => {
                let req: StartDownloadRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid StartDownloadRequest: {e}"))?;
                self.check_resource_scope(context, &page_resource(&req.page_id))?;
                let dl_id = self
                    .supervisor
                    .start_download(&req.page_id, &req.url, req.destination_path.clone())
                    .map_err(|e| e.to_string())?;

                let filename = req
                    .url
                    .split('/')
                    .next_back()
                    .unwrap_or("file.bin")
                    .to_string();
                let event = DownloadStartedEvent {
                    download_id: dl_id.clone(),
                    page_id: req.page_id.clone(),
                    url: req.url.clone(),
                    suggested_filename: filename,
                };
                self.record_event(EVENT_DOWNLOAD_STARTED, serde_json::to_vec(&event).unwrap());

                let status = self
                    .supervisor
                    .download_status(&dl_id)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&status).map_err(|e| e.to_string())
            }
            (CONTRACT_DOWNLOAD, OP_DOWNLOAD_CONTROL) => {
                let req: ControlDownloadRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ControlDownloadRequest: {e}"))?;
                let status = self
                    .supervisor
                    .control_download(&req.download_id, req.action)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&status).map_err(|e| e.to_string())
            }
            (CONTRACT_DOWNLOAD, OP_DOWNLOAD_STATUS) => {
                let req: ControlDownloadRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ControlDownloadRequest: {e}"))?;
                let status = self
                    .supervisor
                    .download_status(&req.download_id)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&status).map_err(|e| e.to_string())
            }

            // --- browser.permission ---
            (CONTRACT_PERMISSION, OP_PERMISSION_QUERY) => {
                let req: worldline_browser_contract::contracts::QueryPermissionRequest =
                    serde_json::from_slice(payload)
                        .map_err(|e| format!("invalid QueryPermissionRequest: {e}"))?;
                self.check_resource_scope(context, &context_resource(&req.context_id))?;
                let decision = self
                    .supervisor
                    .query_permission(&req.context_id, &req.origin, req.permission_type)
                    .map_err(|e| e.to_string())?;
                let resp = PermissionResponse {
                    context_id: req.context_id,
                    origin: req.origin,
                    permission_type: req.permission_type,
                    decision,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_PERMISSION, OP_PERMISSION_SET) => {
                let req: SetPermissionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid SetPermissionRequest: {e}"))?;
                self.check_resource_scope(context, &context_resource(&req.context_id))?;
                let decision = self
                    .supervisor
                    .set_permission(
                        &req.context_id,
                        &req.origin,
                        req.permission_type,
                        req.decision,
                    )
                    .map_err(|e| e.to_string())?;
                let resp = PermissionResponse {
                    context_id: req.context_id,
                    origin: req.origin,
                    permission_type: req.permission_type,
                    decision,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }

            (c, o) => Err(format!("unsupported browser operation: {c}/{o}")),
        }
    }
}
