use worldline_browser_contract::{
    action::{ClickActionRequest, InputActionRequest},
    authority::{
        OP_CLICK, OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_INPUT, OP_NAVIGATE, OP_OBSERVE,
        OP_QUERY_DOCUMENT,
    },
    contracts::{
        CONTRACT_ACT, CONTRACT_CONTEXT, CONTRACT_NAVIGATE, CONTRACT_OBSERVE, CONTRACT_PAGE,
        CONTRACT_QUERY, CreateContextRequest, CreateContextResponse, CreatePageRequest,
        CreatePageResponse, NavigateRequest, NavigateResponse, ObservePageRequest, PageObservation,
        QueryDocumentRequest,
    },
    identity::{BrowserContextId, ElementRef, PageId, context_resource, page_resource},
    query::DocumentSnapshot,
};
use worldline_kernel::{
    GrantLifetime, InvocationRequest, Kernel, PluginId, PrincipalId, PrincipalKind, ResourceId,
    ResourceScope,
};

use crate::{
    engine::SpikeEngineSupervisor,
    provider::{SpikeBrowserPlugin, browser_capability},
};

/// Harness orchestrating full end-to-end execution of the browser capability path.
pub struct BrowserSpikeFixture {
    kernel: Kernel,
    plugin_id: PluginId,
    supervisor: SpikeEngineSupervisor,
    caller: PrincipalId,
}

impl BrowserSpikeFixture {
    pub fn boot() -> Result<Self, String> {
        let mut kernel = Kernel::new();
        let supervisor = SpikeEngineSupervisor::new();
        let plugin = SpikeBrowserPlugin::new("spike.browser.provider", supervisor.clone());

        let plugin_id = kernel.register(plugin).map_err(|e| e.to_string())?;
        let caller = PrincipalId::new("spike-caller-agent");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .map_err(|e| e.to_string())?;

        // Grant the caller capability access to browser contracts
        for (contract, ops) in [
            (CONTRACT_CONTEXT, vec![OP_CREATE_CONTEXT]),
            (CONTRACT_PAGE, vec![OP_CREATE_PAGE]),
            (CONTRACT_NAVIGATE, vec![OP_NAVIGATE]),
            (CONTRACT_OBSERVE, vec![OP_OBSERVE]),
            (CONTRACT_QUERY, vec![OP_QUERY_DOCUMENT]),
            (CONTRACT_ACT, vec![OP_CLICK, OP_INPUT]),
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

    pub fn supervisor(&self) -> &SpikeEngineSupervisor {
        &self.supervisor
    }

    pub fn create_context(
        &mut self,
        storage_path: Option<String>,
        incognito: bool,
    ) -> Result<BrowserContextId, String> {
        let req = CreateContextRequest {
            profile_storage_path: storage_path,
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

    pub fn query_document(&mut self, page_id: &PageId) -> Result<DocumentSnapshot, String> {
        let req = QueryDocumentRequest {
            page_id: page_id.clone(),
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
}
