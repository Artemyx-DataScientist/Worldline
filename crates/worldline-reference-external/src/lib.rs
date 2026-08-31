//! Cross-mode reference plugin host fixtures for the Worldline external
//! plugin boundary (M0.6).
//!
//! One unchanged consumer drives `reference.echo/v1` providers across all
//! three execution modes (builtin, native process, WASM component). The
//! fixture in this module is that consumer: it boots a kernel, registers a
//! provider plugin for the mode under test, wires authority explicitly, and
//! exposes the same observable surface to every conformance suite.

#![forbid(unsafe_code)]

pub mod echo;
pub mod native_echo;
pub mod wasm_echo;

use worldline_kernel::{
    EventEnvelope, InstallationId, Kernel, Plugin, PluginId, PrincipalId, PrincipalKind,
    ResourceScope, RpcCallOptions, SubscriptionHandle, SubscriptionOptions, TraceContext,
};

pub use echo::{
    BuiltinEchoPlugin, OPERATION_ECHO, OPERATION_PUBLISH_OBSERVATION, OPERATION_STATEFUL_INCREMENT,
    STATE_KEY, echo_capability, echo_resource, format_echo_result, format_increment_result,
    format_observation_result, observation_contract, semantics,
};
pub use native_echo::{NativeEchoOptions, NativeEchoPlugin};
pub use wasm_echo::WasmEchoPlugin;

/// Boots the unchanged consumer around one provider implementation.
pub struct EchoFixture {
    kernel: Kernel,
    plugin_id: PluginId,
    installation: InstallationId,
    caller: PrincipalId,
    observer: PrincipalId,
    observation: SubscriptionHandle,
    control: SubscriptionHandle,
}

impl EchoFixture {
    /// Registers `provider`, grants the caller exactly the three echo
    /// operations, grants the provider runtime only the observation publish,
    /// and subscribes the observer to observations and the metadata-only
    /// control stream.
    pub fn boot(provider: impl Plugin + 'static) -> Result<Self, String> {
        let mut kernel = Kernel::new();
        let plugin_id = kernel
            .register(provider)
            .map_err(|error| error.to_string())?;
        let installation = kernel
            .installation_id_for_plugin(&plugin_id)
            .ok_or_else(|| "echo installation is missing".to_owned())?;
        let runtime_principal = kernel
            .principal_for_plugin(&plugin_id)
            .ok_or_else(|| "echo runtime principal is missing".to_owned())?;
        let caller = PrincipalId::new("echo-caller");
        let observer = PrincipalId::new("echo-observer");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .map_err(|error| error.to_string())?;
        kernel
            .register_principal_id(observer.clone(), PrincipalKind::Agent)
            .map_err(|error| error.to_string())?;

        let capability = echo_capability();
        let event = observation_contract();
        let control = worldline_kernel::invocation_completed_event_contract();
        for operation in [
            OPERATION_ECHO,
            OPERATION_STATEFUL_INCREMENT,
            OPERATION_PUBLISH_OBSERVATION,
        ] {
            kernel
                .create_root_grant(
                    caller.clone(),
                    capability.contract(),
                    [operation],
                    ResourceScope::Any,
                    false,
                    worldline_kernel::GrantLifetime::Persistent,
                )
                .map_err(|error| error.to_string())?;
        }
        kernel
            .create_root_grant(
                runtime_principal,
                event.capability_id(),
                ["publish"],
                ResourceScope::Any,
                false,
                worldline_kernel::GrantLifetime::Persistent,
            )
            .map_err(|error| error.to_string())?;
        kernel
            .create_root_grant(
                observer.clone(),
                event.capability_id(),
                ["subscribe"],
                ResourceScope::Any,
                false,
                worldline_kernel::GrantLifetime::Persistent,
            )
            .map_err(|error| error.to_string())?;
        kernel
            .create_root_grant(
                observer.clone(),
                control.capability_id(),
                ["subscribe"],
                ResourceScope::Any,
                false,
                worldline_kernel::GrantLifetime::Persistent,
            )
            .map_err(|error| error.to_string())?;

        let observation = kernel
            .subscribe(observer.clone(), event, SubscriptionOptions::default())
            .map_err(|error| error.to_string())?;
        let control = kernel
            .subscribe(observer.clone(), control, SubscriptionOptions::default())
            .map_err(|error| error.to_string())?;

        Ok(Self {
            kernel,
            plugin_id,
            installation,
            caller,
            observer,
            observation,
            control,
        })
    }

    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    pub fn installation(&self) -> &InstallationId {
        &self.installation
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub fn observer(&self) -> &PrincipalId {
        &self.observer
    }

    /// Registers a principal that holds no echo grants. Authorization
    /// denials must be identical across execution modes.
    pub fn register_unauthorized_subject(&self, name: &str) -> Result<PrincipalId, String> {
        let subject = PrincipalId::new(name);
        self.kernel
            .register_principal_id(subject.clone(), PrincipalKind::Agent)
            .map_err(|error| error.to_string())?;
        Ok(subject)
    }

    /// Invokes one echo operation as the authorized caller with an explicit
    /// request identity.
    pub fn call(
        &self,
        operation: &str,
        payload: &[u8],
        request_id: &str,
    ) -> Result<Vec<u8>, String> {
        self.kernel
            .capability_for(self.caller.clone(), echo_capability())
            .map_err(|error| error.to_string())?
            .invoke_with_options(
                operation,
                payload,
                RpcCallOptions::new()
                    .with_request_id(request_id)
                    .with_trace_context(TraceContext::new("echo-conformance")),
            )
            .map_err(|error| error.to_string())
    }

    /// Attempts one echo operation from a principal without grants. The
    /// outer result covers handle acquisition; the inner result is the RPC
    /// outcome. The adapter must not change the denial semantics.
    pub fn call_unauthorized(
        &self,
        subject: &PrincipalId,
        operation: &str,
        payload: &[u8],
    ) -> Result<Result<Vec<u8>, worldline_kernel::CapabilityError>, String> {
        let handle = self
            .kernel
            .capability_for(subject.clone(), echo_capability())
            .map_err(|error| error.to_string())?;
        Ok(handle.invoke(operation, payload))
    }

    /// Reads the persisted increment counter through the installation state
    /// contract, independently of the provider execution mode.
    pub fn committed_count(&self) -> Result<u64, String> {
        let value = self
            .kernel
            .state_handle(&self.installation)
            .map_err(|error| error.to_string())?
            .get(STATE_KEY)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "echo counter is missing".to_owned())?;
        String::from_utf8(value)
            .map_err(|error| error.to_string())?
            .parse::<u64>()
            .map_err(|error| error.to_string())
    }

    /// Receives the next provider observation.
    pub fn next_observation(&self) -> Result<EventEnvelope, String> {
        self.observation
            .try_recv()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "echo observation is missing".to_owned())
    }

    /// Receives the next metadata-only control observation.
    pub fn next_control(&self) -> Result<EventEnvelope, String> {
        self.control
            .try_recv()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "echo control observation is missing".to_owned())
    }

    /// Drops and re-subscribes the observation subscriptions.
    pub fn resubscribe(&mut self) -> Result<(), String> {
        let event = observation_contract();
        let control = worldline_kernel::invocation_completed_event_contract();
        self.observation = self
            .kernel
            .subscribe(self.observer.clone(), event, SubscriptionOptions::default())
            .map_err(|error| error.to_string())?;
        self.control = self
            .kernel
            .subscribe(
                self.observer.clone(),
                control,
                SubscriptionOptions::default(),
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
