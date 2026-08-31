use worldline_browser_contract::{
    action::{ClickActionRequest, InputActionRequest},
    authority::{
        OP_CLICK, OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_DOWNLOAD_START, OP_EXTRACT_TEXT,
        OP_FIND_ELEMENTS, OP_INPUT, OP_NAVIGATE, OP_OBSERVE, OP_PERMISSION_QUERY,
        OP_PERMISSION_SET, OP_QUERY_DOCUMENT, OP_RELOAD,
    },
    contracts::{
        CONTRACT_ACT, CONTRACT_CONTEXT, CONTRACT_DOWNLOAD, CONTRACT_NAVIGATE, CONTRACT_OBSERVE,
        CONTRACT_PAGE, CONTRACT_PERMISSION, CONTRACT_QUERY, CreateContextRequest,
        CreateContextResponse, CreatePageRequest, CreatePageResponse, DownloadStatusResponse,
        ElementQueryKind, ExtractTextRequest, ExtractTextResponse, FindElementsRequest,
        FindElementsResponse, NavigateRequest, NavigateResponse, ObservePageRequest,
        PageObservation, PermissionDecision, PermissionResponse, PermissionType,
        QueryDocumentRequest, ReloadRequest, ReloadResponse, SetPermissionRequest,
        StartDownloadRequest,
    },
    identity::{BrowserContextId, ElementRef, PageId, context_resource, page_resource},
    query::{DocumentSnapshot, QueryBounds},
};
use worldline_kernel::{
    GrantLifetime, InvocationRequest, Kernel, PluginId, PrincipalId, PrincipalKind, ResourceId,
    ResourceScope,
};

use crate::{
    engine::ReferenceBrowserSupervisor,
    provider::{EmittedBrowserEvent, SpikeBrowserPlugin, browser_capability},
};

/// Harness orchestrating full end-to-end execution of the browser capability path.
pub struct BrowserSpikeFixture {
    kernel: Kernel,
    plugin_id: PluginId,
    plugin: SpikeBrowserPlugin,
    supervisor: ReferenceBrowserSupervisor,
    caller: PrincipalId,
}

impl BrowserSpikeFixture {
    pub fn boot() -> Result<Self, String> {
        let mut kernel = Kernel::new();
        let supervisor = ReferenceBrowserSupervisor::new();
        let plugin = SpikeBrowserPlugin::new("spike.browser.provider", supervisor.clone());

        let plugin_id = kernel.register(plugin.clone()).map_err(|e| e.to_string())?;
        let caller = PrincipalId::new("spike-caller-agent");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .map_err(|e| e.to_string())?;

        // Grant the caller capability access to browser contracts with Any resource scope
        for (contract, ops) in [
            (CONTRACT_CONTEXT, vec![OP_CREATE_CONTEXT]),
            (CONTRACT_PAGE, vec![OP_CREATE_PAGE]),
            (CONTRACT_NAVIGATE, vec![OP_NAVIGATE, OP_RELOAD]),
            (CONTRACT_OBSERVE, vec![OP_OBSERVE]),
            (
                CONTRACT_QUERY,
                vec![OP_QUERY_DOCUMENT, OP_EXTRACT_TEXT, OP_FIND_ELEMENTS],
            ),
            (CONTRACT_ACT, vec![OP_CLICK, OP_INPUT]),
            (CONTRACT_DOWNLOAD, vec![OP_DOWNLOAD_START]),
            (
                CONTRACT_PERMISSION,
                vec![OP_PERMISSION_QUERY, OP_PERMISSION_SET],
            ),
        ] {
            let cap = browser_capability(contract);
            kernel
                .create_root_grant(
                    caller.clone(),
                    cap.contract(),
                    ops,
                    ResourceScope::Any,
                    false,
                    GrantLifetime::Persistent,
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(Self {
            kernel,
            plugin_id,
            plugin,
            supervisor,
            caller,
        })
    }

    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    pub fn kernel_mut(&mut self) -> &mut Kernel {
        &mut self.kernel
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub fn supervisor(&self) -> &ReferenceBrowserSupervisor {
        &self.supervisor
    }

    pub fn emitted_events(&self) -> Vec<EmittedBrowserEvent> {
        self.plugin.emitted_events().lock().unwrap().clone()
    }

    pub fn create_context(
        &mut self,
        profile_id: Option<String>,
        incognito: bool,
    ) -> Result<BrowserContextId, String> {
        let req = CreateContextRequest {
            profile_id,
            incognito,
            user_agent: None,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_CONTEXT);
        let resource = ResourceId::root("browser-context");

        let invocation = InvocationRequest::new(
            self.caller.clone(),
            cap,
            OP_CREATE_CONTEXT,
            resource,
            payload,
        );

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        let resp: CreateContextResponse =
            serde_json::from_slice(&outcome).map_err(|e| e.to_string())?;
        Ok(resp.context_id)
    }

    pub fn create_page(
        &mut self,
        context_id: &BrowserContextId,
        initial_url: Option<String>,
    ) -> Result<PageId, String> {
        let req = CreatePageRequest {
            context_id: context_id.clone(),
            initial_url,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_PAGE);
        let resource =
            ResourceId::parse(&context_resource(context_id)).map_err(|e| e.to_string())?;

        let invocation =
            InvocationRequest::new(self.caller.clone(), cap, OP_CREATE_PAGE, resource, payload);

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        let resp: CreatePageResponse =
            serde_json::from_slice(&outcome).map_err(|e| e.to_string())?;
        Ok(resp.page_id)
    }

    pub fn navigate(&mut self, page_id: &PageId, url: &str) -> Result<NavigateResponse, String> {
        let req = NavigateRequest {
            page_id: page_id.clone(),
            url: url.to_string(),
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_NAVIGATE);
        let resource = ResourceId::parse(&page_resource(page_id)).map_err(|e| e.to_string())?;

        let invocation =
            InvocationRequest::new(self.caller.clone(), cap, OP_NAVIGATE, resource, payload);

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn reload(&mut self, page_id: &PageId) -> Result<ReloadResponse, String> {
        let req = ReloadRequest {
            page_id: page_id.clone(),
            ignore_cache: true,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_NAVIGATE);
        let resource = ResourceId::parse(&page_resource(page_id)).map_err(|e| e.to_string())?;

        let invocation =
            InvocationRequest::new(self.caller.clone(), cap, OP_RELOAD, resource, payload);

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn observe(&mut self, page_id: &PageId) -> Result<PageObservation, String> {
        let req = ObservePageRequest {
            page_id: page_id.clone(),
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_OBSERVE);
        let resource = ResourceId::parse(&page_resource(page_id)).map_err(|e| e.to_string())?;

        let invocation =
            InvocationRequest::new(self.caller.clone(), cap, OP_OBSERVE, resource, payload);

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn query_document(
        &mut self,
        page_id: &PageId,
        bounds: Option<QueryBounds>,
    ) -> Result<DocumentSnapshot, String> {
        let req = QueryDocumentRequest {
            page_id: page_id.clone(),
            bounds,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_QUERY);
        let resource = ResourceId::parse(&page_resource(page_id)).map_err(|e| e.to_string())?;

        let invocation = InvocationRequest::new(
            self.caller.clone(),
            cap,
            OP_QUERY_DOCUMENT,
            resource,
            payload,
        );

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn extract_text(
        &mut self,
        page_id: &PageId,
        target_element: Option<ElementRef>,
    ) -> Result<ExtractTextResponse, String> {
        let req = ExtractTextRequest {
            page_id: page_id.clone(),
            target_element,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_QUERY);
        let resource = ResourceId::parse(&page_resource(page_id)).map_err(|e| e.to_string())?;

        let invocation =
            InvocationRequest::new(self.caller.clone(), cap, OP_EXTRACT_TEXT, resource, payload);

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn find_elements(
        &mut self,
        page_id: &PageId,
        query: &str,
        kind: ElementQueryKind,
    ) -> Result<FindElementsResponse, String> {
        let req = FindElementsRequest {
            page_id: page_id.clone(),
            query: query.to_string(),
            kind,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_QUERY);
        let resource = ResourceId::parse(&page_resource(page_id)).map_err(|e| e.to_string())?;

        let invocation = InvocationRequest::new(
            self.caller.clone(),
            cap,
            OP_FIND_ELEMENTS,
            resource,
            payload,
        );

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn input_text(
        &mut self,
        element_ref: &ElementRef,
        text: &str,
    ) -> Result<worldline_browser_contract::ActionResult, String> {
        let req = InputActionRequest {
            element_ref: element_ref.clone(),
            text: text.to_string(),
            clear_first: true,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_ACT);
        let resource =
            ResourceId::parse(&page_resource(element_ref.page_id())).map_err(|e| e.to_string())?;

        let invocation =
            InvocationRequest::new(self.caller.clone(), cap, OP_INPUT, resource, payload);

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn click_element(
        &mut self,
        element_ref: &ElementRef,
    ) -> Result<worldline_browser_contract::ActionResult, String> {
        let req = ClickActionRequest {
            element_ref: element_ref.clone(),
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_ACT);
        let resource =
            ResourceId::parse(&page_resource(element_ref.page_id())).map_err(|e| e.to_string())?;

        let invocation =
            InvocationRequest::new(self.caller.clone(), cap, OP_CLICK, resource, payload);

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    /// Invokes an action with a potentially mismatched resource to test confused-deputy protection.
    pub fn invoke_confused_deputy_act(
        &mut self,
        admitted_page_id: &PageId,
        targeted_element_ref: &ElementRef,
    ) -> Result<worldline_browser_contract::ActionResult, String> {
        let req = ClickActionRequest {
            element_ref: targeted_element_ref.clone(),
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_ACT);
        let admitted_resource =
            ResourceId::parse(&page_resource(admitted_page_id)).map_err(|e| e.to_string())?;

        let invocation = InvocationRequest::new(
            self.caller.clone(),
            cap,
            OP_CLICK,
            admitted_resource,
            payload,
        );

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn start_download(
        &mut self,
        page_id: &PageId,
        url: &str,
    ) -> Result<DownloadStatusResponse, String> {
        let req = StartDownloadRequest {
            page_id: page_id.clone(),
            url: url.to_string(),
            destination_path: None,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_DOWNLOAD);
        let resource = ResourceId::parse(&page_resource(page_id)).map_err(|e| e.to_string())?;

        let invocation = InvocationRequest::new(
            self.caller.clone(),
            cap,
            OP_DOWNLOAD_START,
            resource,
            payload,
        );

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }

    pub fn set_permission(
        &mut self,
        context_id: &BrowserContextId,
        origin: &str,
        perm_type: PermissionType,
        decision: PermissionDecision,
    ) -> Result<PermissionResponse, String> {
        let req = SetPermissionRequest {
            context_id: context_id.clone(),
            origin: origin.to_string(),
            permission_type: perm_type,
            decision,
        };
        let payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let cap = browser_capability(CONTRACT_PERMISSION);
        let resource =
            ResourceId::parse(&context_resource(context_id)).map_err(|e| e.to_string())?;

        let invocation = InvocationRequest::new(
            self.caller.clone(),
            cap,
            OP_PERMISSION_SET,
            resource,
            payload,
        );

        let outcome = self.kernel.invoke(invocation).map_err(|e| e.to_string())?;
        serde_json::from_slice(&outcome).map_err(|e| e.to_string())
    }
}
