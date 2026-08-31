//! WASM Component execution of `reference.echo/v1`.
//!
//! The in-process plugin proxies capability calls into a sandboxed
//! component. The component holds no authority: its state imports are
//! routed to the installation-owned state contract and its event
//! publications are replayed through the invocation context so the producer
//! identity stays host-stamped.

use std::sync::{Arc, Mutex};

use worldline_kernel::{
    CapabilityId, CapabilityService, EventContract, EventPublishOptions, InterfaceVersion,
    InvocationContext, NoopRuntime, Plugin, PluginDefinition, PluginError, PluginId, PluginRuntime,
    RuntimeStateHandle,
};
use worldline_wasm_host::{WasmHostBroker, WasmHostError, WasmPluginHost, WasmPluginInstance};

use crate::echo::echo_capability;

type Publications = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

/// Host-side broker behind the component's two imported interfaces. State
/// flows to the installation-owned contract; publications are queued for
/// replay through the invocation context.
struct WasmEchoBroker {
    state: Arc<Mutex<Option<RuntimeStateHandle>>>,
    publications: Publications,
}

impl WasmEchoBroker {
    fn locked_state(&self) -> Result<RuntimeStateHandle, String> {
        self.state
            .lock()
            .map_err(|_| "wasm echo state slot is poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "wasm echo state handle is not initialized".to_owned())
    }
}

impl WasmHostBroker for WasmEchoBroker {
    fn state_get(&self, key: String) -> Option<Vec<u8>> {
        self.locked_state()
            .and_then(|state| state.get(key.as_str()).map_err(|error| error.to_string()))
            .ok()
            .flatten()
    }

    fn state_set(&self, key: String, value: Vec<u8>) {
        if let Ok(state) = self.locked_state()
            && let Ok(mut transaction) = state.transaction()
            && transaction.put(key.as_str(), &value).is_ok()
        {
            let _ = transaction.commit();
        }
    }

    fn event_publish(
        &self,
        namespace: String,
        name: String,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        self.publications
            .lock()
            .map_err(|_| "publications queue poisoned".to_owned())?
            .push((namespace, name, payload));
        Ok(())
    }
}

/// The WASM execution mode of `reference.echo/v1`.
pub struct WasmEchoPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<WasmEchoService>,
}

struct WasmEchoService {
    component_bytes: Arc<Vec<u8>>,
    host: WasmPluginHost,
    state: Arc<Mutex<Option<RuntimeStateHandle>>>,
    publications: Publications,
    instance: Mutex<Option<WasmPluginInstance>>,
}

impl WasmEchoService {
    fn instance(&self) -> Result<(), String> {
        let mut guard = self
            .instance
            .lock()
            .map_err(|_| "wasm instance slot poisoned".to_owned())?;
        if guard.is_some() {
            return Ok(());
        }
        let component = self
            .host
            .load_component(&self.component_bytes)
            .map_err(|error| error.to_string())?;
        let broker = Arc::new(WasmEchoBroker {
            state: Arc::clone(&self.state),
            publications: Arc::clone(&self.publications),
        });
        let instance = self
            .host
            .make_instance(&component, broker)
            .map_err(|error| error.to_string())?;
        *guard = Some(instance);
        Ok(())
    }
}

impl CapabilityService for WasmEchoService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Err("wasm echo requires an invocation context".to_owned())
    }

    fn invoke_with_context(
        &self,
        context: &InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.instance()?;
        let mut guard = self
            .instance
            .lock()
            .map_err(|_| "wasm instance slot poisoned".to_owned())?;
        let instance = guard
            .as_mut()
            .ok_or_else(|| "wasm instance is missing".to_owned())?;
        let bytes = instance
            .invoke(context.operation().as_str(), payload.to_vec())
            .map_err(|error| match error {
                WasmHostError::GuestReturnedError { message } => message,
                other => other.to_string(),
            })?;
        // Component-side publications are replayed here so the producer
        // identity stays host-stamped under this runtime's own authority.
        let drained: Vec<(String, String, Vec<u8>)> = self
            .publications
            .lock()
            .map_err(|_| "publications queue poisoned".to_owned())?
            .drain(..)
            .collect();
        for (namespace, name, bytes) in drained {
            context
                .publish_event(
                    EventContract::new(namespace, name, InterfaceVersion::new(1, 0)),
                    &bytes,
                    EventPublishOptions::default(),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(bytes)
    }
}

impl WasmEchoPlugin {
    /// Builds the WASM execution mode over raw component bytes.
    pub fn from_component(plugin: impl Into<String>, component_bytes: Vec<u8>) -> Self {
        let capability = echo_capability();
        Self {
            definition: PluginDefinition::new(PluginId::new(plugin)).provides(capability.clone()),
            capability,
            service: Arc::new(WasmEchoService {
                component_bytes: Arc::new(component_bytes),
                host: WasmPluginHost::new(),
                state: Arc::new(Mutex::new(None)),
                publications: Arc::new(Mutex::new(Vec::new())),
                instance: Mutex::new(None),
            }),
        }
    }
}

impl Plugin for WasmEchoPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut worldline_kernel::ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        *self
            .service
            .state
            .lock()
            .map_err(|_| PluginError::new("wasm echo state slot is poisoned"))? =
            Some(context.state().clone());
        let service: Arc<dyn CapabilityService> = Arc::clone(&self.service) as _;
        context.publish_capability(self.capability.clone(), service)?;
        Ok(Box::new(NoopRuntime))
    }
}
