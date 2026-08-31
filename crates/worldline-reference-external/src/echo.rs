//! Cross-mode reference plugin `reference.echo/v1`.
//!
//! The same logical capability contract is implemented by builtin Rust,
//! native-process, and WASM Component providers. The consumer never changes
//! when the execution mode changes. All implementations must produce
//! byte-identical observable results and persist `stateful_increment`
//! through the same installation-owned state contract.
//!
//! This module owns the mode-independent vocabulary and the builtin
//! implementation. External adapters (native process, WASM component) live
//! in their host crates and are wired by the conformance suites.

use std::sync::{Arc, Mutex};

use worldline_kernel::{
    CapabilityId, CapabilityService, EventContract, InterfaceVersion, InvocationContext,
    NoopRuntime, Plugin, PluginDefinition, PluginError, PluginId, PluginRuntime, ResourceId,
    RuntimeStateHandle,
};

/// Logical capability identity. Fixed across execution modes.
pub fn echo_capability() -> CapabilityId {
    CapabilityId::new("reference.echo", "v1", InterfaceVersion::new(1, 0))
}

/// Observation contract published by `publish_observation`.
pub fn observation_contract() -> EventContract {
    EventContract::new("reference.echo", "observation", InterfaceVersion::new(1, 0))
}

/// Resource addressed by every echo operation.
pub fn echo_resource() -> ResourceId {
    ResourceId::new("reference.echo", ["v1"])
}

pub const OPERATION_ECHO: &str = "echo";
pub const OPERATION_STATEFUL_INCREMENT: &str = "stateful_increment";
pub const OPERATION_PUBLISH_OBSERVATION: &str = "publish_observation";

/// Installation state key holding the persisted increment counter.
pub const STATE_KEY: &str = "reference-echo-count";

/// Byte-identical result for `echo` across all execution modes.
pub fn format_echo_result(payload: &[u8]) -> Vec<u8> {
    format!("echo:{}", String::from_utf8_lossy(payload)).into_bytes()
}

/// Byte-identical result for `stateful_increment` across all execution
/// modes: `incremented:<count>:<payload>`.
pub fn format_increment_result(count: u64, payload: &[u8]) -> Vec<u8> {
    format!("incremented:{count}:{}", String::from_utf8_lossy(payload)).into_bytes()
}

/// Byte-identical result for `publish_observation`.
pub fn format_observation_result(payload: &[u8]) -> Vec<u8> {
    format!("observed:{}", String::from_utf8_lossy(payload)).into_bytes()
}

/// Builtin execution of the three operations against an installation-bound
/// state handle and an invocation context. Native and WASM adapters must
/// produce the same observable behavior through their host round-trips.
pub mod semantics {
    use super::*;

    pub fn echo(payload: &[u8]) -> Vec<u8> {
        format_echo_result(payload)
    }

    /// Reads the persisted counter, increments it, commits through the
    /// installation state contract, and returns the new count.
    pub fn stateful_increment(
        state: &RuntimeStateHandle,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let current = state
            .get(STATE_KEY)
            .map_err(|error| error.to_string())?
            .and_then(|value| String::from_utf8(value).ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_default();
        let next = current
            .checked_add(1)
            .ok_or_else(|| "echo count exhausted".to_owned())?;
        let mut transaction = state.transaction().map_err(|error| error.to_string())?;
        transaction
            .put(STATE_KEY, next.to_string().as_bytes())
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(format_increment_result(next, payload))
    }

    pub fn publish_observation(
        context: &InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        context
            .publish_event(
                observation_contract(),
                payload,
                worldline_kernel::EventPublishOptions::default(),
            )
            .map_err(|error| error.to_string())?;
        Ok(format_observation_result(payload))
    }
}

/// Builtin `reference.echo/v1` provider. Trusted Rust, statically linked,
/// but using the ordinary runtime identity, authority, state lease, and
/// capability contracts — no special kernel path.
pub struct BuiltinEchoPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    state: Arc<Mutex<Option<RuntimeStateHandle>>>,
}

impl BuiltinEchoPlugin {
    pub fn new(plugin: impl Into<PluginId>) -> Self {
        let capability = echo_capability();
        Self {
            definition: PluginDefinition::new(plugin).provides(capability.clone()),
            capability,
            state: Arc::new(Mutex::new(None)),
        }
    }
}

impl Plugin for BuiltinEchoPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut worldline_kernel::ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        *self
            .state
            .lock()
            .map_err(|_| PluginError::new("echo state slot is poisoned"))? =
            Some(context.state().clone());
        let state = self.state.clone();
        let service: Arc<dyn CapabilityService> = Arc::new(EchoBuiltinService { state });
        context.publish_capability(self.capability.clone(), service)?;
        Ok(Box::new(NoopRuntime))
    }
}

struct EchoBuiltinService {
    state: Arc<Mutex<Option<RuntimeStateHandle>>>,
}

impl CapabilityService for EchoBuiltinService {
    fn invoke(&self, _operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(semantics::echo(payload))
    }

    fn invoke_with_context(
        &self,
        context: &InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        match context.operation().as_str() {
            OPERATION_ECHO => Ok(semantics::echo(payload)),
            OPERATION_STATEFUL_INCREMENT => {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| "echo state slot is poisoned".to_owned())?
                    .clone()
                    .ok_or_else(|| "echo state handle is not initialized".to_owned())?;
                semantics::stateful_increment(&state, payload)
            }
            OPERATION_PUBLISH_OBSERVATION => semantics::publish_observation(context, payload),
            other => Err(format!("unsupported echo operation '{other}'")),
        }
    }
}
