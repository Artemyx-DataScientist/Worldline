use std::sync::Arc;

use worldline_browser_contract::{
    action::{ClickActionRequest, InputActionRequest, InteractionKind},
    authority::{
        OP_CLICK, OP_CLOSE_CONTEXT, OP_CLOSE_PAGE, OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_GET_TITLE,
        OP_GET_URL, OP_INPUT, OP_NAVIGATE, OP_OBSERVE, OP_QUERY_ACCESSIBILITY, OP_QUERY_DOCUMENT,
        OP_SUBMIT,
    },
    contracts::{
        BROWSER_NAMESPACE, CONTRACT_ACT, CONTRACT_CONTEXT, CONTRACT_DOWNLOAD, CONTRACT_NAVIGATE,
        CONTRACT_OBSERVE, CONTRACT_PAGE, CONTRACT_PERMISSION, CONTRACT_QUERY, CloseContextRequest,
        CloseContextResponse, ClosePageRequest, ClosePageResponse, CreateContextRequest,
        CreateContextResponse, CreatePageRequest, CreatePageResponse, INTERFACE_MAJOR_V1,
        INTERFACE_MINOR_V1, NavigateRequest, NavigateResponse, ObservePageRequest,
        QueryDocumentRequest,
    },
};
use worldline_kernel::{
    ActivationContext, CapabilityId, CapabilityService, InterfaceVersion, InvocationContext,
    NoopRuntime, Plugin, PluginDefinition, PluginError, PluginRuntime,
};

use crate::engine::SpikeEngineSupervisor;

pub fn browser_capability(name: &str) -> CapabilityId {
    CapabilityId::new(
        BROWSER_NAMESPACE,
        name,
        InterfaceVersion::new(INTERFACE_MAJOR_V1, INTERFACE_MINOR_V1),
    )
}

/// Browser plugin exposing engine-neutral browser capabilities to Worldline kernel.
pub struct SpikeBrowserPlugin {
    definition: PluginDefinition,
    supervisor: SpikeEngineSupervisor,
}

impl SpikeBrowserPlugin {
    pub fn new(plugin_id: impl Into<String>, supervisor: SpikeEngineSupervisor) -> Self {
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
        }
    }

    pub fn supervisor(&self) -> &SpikeEngineSupervisor {
        &self.supervisor
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
    supervisor: SpikeEngineSupervisor,
}

impl CapabilityService for SpikeBrowserCapabilityService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.dispatch("", operation, payload)
    }

    fn invoke_with_context(
        &self,
        context: &InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.dispatch(
            context.capability().name(),
            context.operation().as_str(),
            payload,
        )
    }
}

impl SpikeBrowserCapabilityService {
    fn dispatch(&self, contract: &str, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        match (contract, operation) {
            (CONTRACT_CONTEXT, OP_CREATE_CONTEXT) | ("", OP_CREATE_CONTEXT) => {
                let req: CreateContextRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid CreateContextRequest: {e}"))?;
                let created = self
                    .supervisor
                    .create_context(None, req.profile_storage_path.clone(), req.incognito)
                    .map_err(|e| e.to_string())?;
                let resp = CreateContextResponse {
                    context_id: created,
                    profile_storage_path: req.profile_storage_path,
                    incognito: req.incognito,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_CONTEXT, OP_CLOSE_CONTEXT) => {
                let req: CloseContextRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid CloseContextRequest: {e}"))?;
                self.supervisor
                    .close_context(&req.context_id)
                    .map_err(|e| e.to_string())?;
                let resp = CloseContextResponse {
                    context_id: req.context_id,
                    closed: true,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_PAGE, OP_CREATE_PAGE) => {
                let req: CreatePageRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid CreatePageRequest: {e}"))?;
                let (page_id, rev) = self
                    .supervisor
                    .create_page(&req.context_id, None, req.initial_url)
                    .map_err(|e| e.to_string())?;
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
                self.supervisor
                    .close_page(&req.page_id)
                    .map_err(|e| e.to_string())?;
                let resp = ClosePageResponse {
                    page_id: req.page_id,
                    closed: true,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_NAVIGATE, OP_NAVIGATE) | ("", OP_NAVIGATE) => {
                let req: NavigateRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid NavigateRequest: {e}"))?;
                let (nav_id, rev) = self
                    .supervisor
                    .navigate(&req.page_id, &req.url)
                    .map_err(|e| e.to_string())?;
                let resp = NavigateResponse {
                    page_id: req.page_id,
                    navigation_id: nav_id,
                    committed: true,
                    document_revision: rev,
                };
                serde_json::to_vec(&resp).map_err(|e| e.to_string())
            }
            (CONTRACT_OBSERVE, OP_OBSERVE)
            | (CONTRACT_OBSERVE, OP_GET_TITLE)
            | (CONTRACT_OBSERVE, OP_GET_URL)
            | ("", OP_OBSERVE) => {
                let req: ObservePageRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ObservePageRequest: {e}"))?;
                let obs = self
                    .supervisor
                    .observe(&req.page_id)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&obs).map_err(|e| e.to_string())
            }
            (CONTRACT_QUERY, OP_QUERY_DOCUMENT)
            | (CONTRACT_QUERY, OP_QUERY_ACCESSIBILITY)
            | ("", OP_QUERY_DOCUMENT) => {
                let req: QueryDocumentRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid QueryDocumentRequest: {e}"))?;
                let doc = self
                    .supervisor
                    .query_document(&req.page_id)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&doc).map_err(|e| e.to_string())
            }
            (CONTRACT_ACT, OP_CLICK) | (CONTRACT_ACT, OP_SUBMIT) | ("", OP_CLICK) => {
                let req: ClickActionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid ClickActionRequest: {e}"))?;
                let res = self
                    .supervisor
                    .execute_action(&req.element_ref, InteractionKind::Click, None)
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&res).map_err(|e| e.to_string())
            }
            (CONTRACT_ACT, OP_INPUT) | ("", OP_INPUT) => {
                let req: InputActionRequest = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid InputActionRequest: {e}"))?;
                let res = self
                    .supervisor
                    .execute_action(&req.element_ref, InteractionKind::Input, Some(&req.text))
                    .map_err(|e| e.to_string())?;
                serde_json::to_vec(&res).map_err(|e| e.to_string())
            }
            (c, o) => Err(format!("unsupported browser operation: {c}/{o}")),
        }
    }
}
