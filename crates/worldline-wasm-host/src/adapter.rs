//! The wasmtime-backed WASM Component Model execution adapter.
//!
//! This is the physical translation of the external plugin boundary onto the
//! Component Model: a [`WasmPluginHost`] owns one shared
//! [`wasmtime::Engine`] (fuel metering on, epoch interruption off) plus the
//! resource limits, and turns component binaries into isolated
//! [`WasmPluginInstance`]s.
//!
//! Authority properties encoded here (ADR "WASM Component Model selection",
//! "Supported WASI surface", "Failure mapping"):
//!
//! - **Least authority by construction.** The linker registers exactly two
//!   host imports (`state-access`, `event-publish`) that delegate to the
//!   [`WasmHostBroker`]. No WASI binding exists. Before instantiation the
//!   component's imports are inspected; any `wasi:*` import (or any other
//!   import the host does not provide) fails closed with
//!   [`WasmHostError::UnsupportedExternalAbi`] instead of instantiating.
//! - **The event bus is not RPC.** `event-publish` only forwards to the
//!   broker's publication side; delivery can never stand in for an RPC
//!   result of `plugin-operations.invoke`.
//! - **Isolation.** Every instance owns a private [`wasmtime::Store`]. A
//!   trap poisons only that store; the host object holds no per-component
//!   state, so other instances and fresh instantiations keep working.
//! - **Typed resource failures.** Fuel is charged per call; memory and table
//!   growth pass through a recording store limiter. Out-of-fuel maps to
//!   [`WasmHostError::WasmResourceLimitExceeded`] with dimension `"fuel"`,
//!   memory/table denials to dimensions `"memory"`/`"table"`. Exhaustion
//!   never permanently consumes a provider registry slot.
//! - **Payload gates before handoff.** Host-call payloads are checked
//!   against `max_host_call_payload_bytes` before they reach the broker or
//!   re-enter the component ABI; oversized payloads fail with
//!   [`WasmHostError::ExternalPayloadTooLarge`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, Trap};

use crate::error::WasmHostError;
use crate::limits::WasmResourceLimits;

wasmtime::component::bindgen!({
    path: "../worldline-plugin-protocol/wit",
    world: "external-plugin",
});

/// Host-side delegation surface behind the two imported WIT interfaces.
///
/// Implementations connect the sandbox to the existing kernel semantics:
/// `state_*` to the installation-owned state contract and `event_publish`
/// to the typed event transport. The adapter never builds a second state or
/// event mechanism; it only forwards. Authorization stays where it belongs —
/// with the kernel's default-deny broker behind the implementation.
pub trait WasmHostBroker: Send + Sync {
    /// Reads the installation-scoped value at `key`, or `none` when absent.
    fn state_get(&self, key: String) -> Option<Vec<u8>>;
    /// Writes the installation-scoped value at `key`.
    fn state_set(&self, key: String, value: Vec<u8>);
    /// Publishes one event into the host's typed event transport.
    fn event_publish(
        &self,
        namespace: String,
        name: String,
        payload: Vec<u8>,
    ) -> Result<(), String>;
}

/// Recorded payload-gate denial inside a host import.
///
/// v1 host imports are non-trapping, so the gate degrades the observable
/// return value (`none` / skip / guest-visible `err`) and records the typed
/// denial; `invoke` re-classifies the call outcome as the typed
/// [`WasmHostError::ExternalPayloadTooLarge`] instead of the degraded value.
#[derive(Debug, Default)]
struct PayloadGate {
    denial: std::sync::Mutex<Option<WasmHostError>>,
}

impl PayloadGate {
    fn record(&self, failure: WasmHostError) {
        *self
            .denial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(failure);
    }

    fn take(&self) -> Option<WasmHostError> {
        self.denial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// The store data bridging guest calls to the broker with the payload gate
/// in front of every handoff.
struct BrokerBridge {
    broker: Arc<dyn WasmHostBroker>,
    limits: WasmResourceLimits,
    gate: PayloadGate,
}

impl BrokerBridge {
    /// Gate for one payload handoff; records the typed denial and reports
    /// that the payload is inadmissible.
    fn gate(&self, len: usize) -> bool {
        match self.limits.check_host_call_payload(len) {
            Ok(()) => true,
            Err(failure) => {
                self.gate.record(failure);
                false
            }
        }
    }
}

impl worldline::plugin::state_access::Host for BrokerBridge {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        let value = self.broker.state_get(key);
        match &value {
            // Gate before the broker's buffer re-enters the component ABI; a
            // denied read degrades to `none` and is reported as the typed
            // payload error by `invoke`.
            Some(bytes) if !self.gate(bytes.len()) => None,
            _ => value,
        }
    }

    fn set(&mut self, key: String, value: Vec<u8>) {
        // Gate before the broker sees the buffer.
        if self.gate(value.len()) {
            self.broker.state_set(key, value);
        }
    }
}

impl worldline::plugin::event_publish::Host for BrokerBridge {
    fn publish(&mut self, namespace: String, name: String, payload: Vec<u8>) -> Result<(), String> {
        // Gate before the broker sees the buffer.
        if !self.gate(payload.len()) {
            return Err("event payload exceeds the declared host-call payload limit".to_owned());
        }
        self.broker.event_publish(namespace, name, payload)
    }
}

/// Store limiter that applies the configured memory/table limits and records
/// which dimension was denied, so failures can be classified as typed
/// resource-limit errors instead of generic traps.
struct RecordingLimiter {
    inner: StoreLimits,
    memory_denied: AtomicBool,
    table_denied: AtomicBool,
}

impl RecordingLimiter {
    fn new(limits: &WasmResourceLimits) -> Self {
        Self {
            inner: StoreLimitsBuilder::new()
                .memory_size(limits.max_memory_bytes)
                .table_elements(limits.max_table_entries)
                .build(),
            memory_denied: AtomicBool::new(false),
            table_denied: AtomicBool::new(false),
        }
    }

    fn take_denial(&self) -> Option<WasmHostError> {
        if self.memory_denied.load(Ordering::Relaxed) {
            Some(WasmHostError::WasmResourceLimitExceeded {
                dimension: "memory".to_string(),
            })
        } else if self.table_denied.load(Ordering::Relaxed) {
            Some(WasmHostError::WasmResourceLimitExceeded {
                dimension: "table".to_string(),
            })
        } else {
            None
        }
    }
}

impl ResourceLimiter for RecordingLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        let allowed = self.inner.memory_growing(current, desired, maximum)?;
        if !allowed {
            self.memory_denied.store(true, Ordering::Relaxed);
        }
        Ok(allowed)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        let allowed = self.inner.table_growing(current, desired, maximum)?;
        if !allowed {
            self.table_denied.store(true, Ordering::Relaxed);
        }
        Ok(allowed)
    }
}

/// Per-instance store data: the broker bridge plus the recording limiter.
struct InstanceState {
    bridge: BrokerBridge,
    limiter: RecordingLimiter,
}

impl wasmtime::component::HasData for InstanceState {
    type Data<'a> = &'a mut BrokerBridge;
}

impl InstanceState {
    /// Returns and clears the most precise recorded denial: an explicit
    /// payload gate denial first, then memory/table limiter denials.
    fn first_recorded_denial(&self) -> Option<WasmHostError> {
        self.bridge
            .gate
            .take()
            .or_else(|| self.limiter.take_denial())
    }
}

/// A component binary that passed the size gate and compiled against the
/// shared engine.
#[derive(Clone, Debug)]
pub struct WasmComponent {
    pub(crate) component: Component,
}

/// The host object for the WASM Component execution mode.
///
/// One host owns one shared wasmtime engine and the resource limits. It is
/// `Clone` (the engine is internally reference-counted) and holds no
/// per-component state: loading and instantiation failures, traps, and limit
/// exhaustion never affect the usability of the host or of other instances.
#[derive(Clone, Debug)]
pub struct WasmPluginHost {
    engine: Engine,
    limits: WasmResourceLimits,
}

impl WasmPluginHost {
    /// Creates a host with the default [`WasmResourceLimits`].
    pub fn new() -> Self {
        Self::with_limits(WasmResourceLimits::default())
    }

    /// Creates a host with explicit resource limits.
    ///
    /// The engine enables fuel metering (the CPU/execution budget) and keeps
    /// epoch interruption off: v1 uses cooperative fuel metering only.
    pub fn with_limits(limits: WasmResourceLimits) -> Self {
        let mut config = Config::new();
        config.consume_fuel(true);
        // Epoch interruption stays off (default); the fuel budget is the v1
        // CPU/execution dimension.
        let engine =
            Engine::new(&config).expect("wasmtime engine config is a compile-time constant");
        Self { engine, limits }
    }

    /// The resource limits this host enforces.
    pub fn limits(&self) -> &WasmResourceLimits {
        &self.limits
    }

    /// Compiles a component binary.
    ///
    /// The size gate runs **before** compilation, so an oversized binary
    /// never reaches the wasmtime compiler
    /// ([`WasmHostError::ComponentTooLarge`]). Binaries that do not decode
    /// as components at all fail with
    /// [`WasmHostError::UnsupportedExternalAbi`]: they do not speak the
    /// supported external ABI.
    pub fn load_component(&self, bytes: &[u8]) -> Result<WasmComponent, WasmHostError> {
        if bytes.len() > self.limits.max_component_bytes {
            return Err(WasmHostError::ComponentTooLarge {
                limit: self.limits.max_component_bytes,
                actual: bytes.len(),
            });
        }
        let component = Component::from_binary(&self.engine, bytes).map_err(|error| {
            WasmHostError::UnsupportedExternalAbi {
                reason: format!(
                    "binary is not a Component Model component for the external-plugin ABI: {error}"
                ),
            }
        })?;
        Ok(WasmComponent { component })
    }

    /// Instantiates a component with `broker` behind its host imports.
    ///
    /// Fails closed before instantiation when the component demands ambient
    /// authority: any `wasi:*` import fails with
    /// [`WasmHostError::UnsupportedExternalAbi`] because the host registers
    /// no WASI bindings (see [`crate::WasiPermissionSet`]). Instantiation
    /// runs under the configured fuel budget, so a hostile start function
    /// cannot spin forever.
    pub fn make_instance(
        &self,
        component: &WasmComponent,
        broker: Arc<dyn WasmHostBroker>,
    ) -> Result<WasmPluginInstance, WasmHostError> {
        if let Some(reason) = self.forbidden_import_reason(component) {
            return Err(WasmHostError::UnsupportedExternalAbi { reason });
        }

        let mut linker: Linker<InstanceState> = Linker::new(&self.engine);
        worldline::plugin::state_access::add_to_linker::<InstanceState, InstanceState>(
            &mut linker,
            |state: &mut InstanceState| &mut state.bridge,
        )
        .map_err(|error| WasmHostError::InstantiationFailed {
            reason: format!("failed to register state-access import: {error}"),
        })?;
        worldline::plugin::event_publish::add_to_linker::<InstanceState, InstanceState>(
            &mut linker,
            |state: &mut InstanceState| &mut state.bridge,
        )
        .map_err(|error| WasmHostError::InstantiationFailed {
            reason: format!("failed to register event-publish import: {error}"),
        })?;

        let mut store = Store::new(
            &self.engine,
            InstanceState {
                bridge: BrokerBridge {
                    broker,
                    limits: self.limits,
                    gate: PayloadGate::default(),
                },
                limiter: RecordingLimiter::new(&self.limits),
            },
        );
        store.limiter(|state: &mut InstanceState| -> &mut dyn ResourceLimiter {
            &mut state.limiter
        });
        store
            .set_fuel(self.limits.fuel_budget_per_call)
            .map_err(|error| WasmHostError::InstantiationFailed {
                reason: format!("failed to charge the instantiation fuel budget: {error}"),
            })?;

        let bindings = ExternalPlugin::instantiate(&mut store, &component.component, &linker)
            .map_err(|error| classify_setup_error(&store, error))?;

        Ok(WasmPluginInstance {
            store,
            bindings,
            limits: self.limits,
        })
    }

    /// Invokes one exported operation of `instance`.
    ///
    /// A fresh fuel budget is charged per call. Outcomes map exactly:
    /// guest-returned `err(s)` to [`WasmHostError::GuestReturnedError`],
    /// traps to [`WasmHostError::WasmTrap`], out-of-fuel to
    /// [`WasmHostError::WasmResourceLimitExceeded`] with dimension `"fuel"`,
    /// and recorded memory/table denials to the matching dimension.
    pub fn invoke(
        &self,
        instance: &mut WasmPluginInstance,
        operation: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, WasmHostError> {
        instance.invoke(operation, payload)
    }

    /// Inspects the component's imports and returns the denial reason for
    /// the first import the host does not provide, naming `wasi:*` imports
    /// explicitly.
    fn forbidden_import_reason(&self, component: &WasmComponent) -> Option<String> {
        let component_type = component.component.component_type();
        for (name, _) in component_type.imports(&self.engine) {
            if name.starts_with("wasi:") {
                return Some(format!(
                    "component imports '{name}', but the host provides no WASI bindings; \
                     components run with zero ambient authority"
                ));
            }
        }
        for (name, _) in component_type.imports(&self.engine) {
            if name != "worldline:plugin/state-access@0.3.0"
                && name != "worldline:plugin/event-publish@0.3.0"
            {
                return Some(format!(
                    "component imports '{name}', which the external-plugin world does not provide"
                ));
            }
        }
        None
    }
}

impl Default for WasmPluginHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Classifies an instantiation/setup error: recorded limiter denials and
/// out-of-fuel are typed resource failures; everything else is an
/// instantiation failure.
fn classify_setup_error(store: &Store<InstanceState>, error: wasmtime::Error) -> WasmHostError {
    if let Some(denial) = store.data().limiter.take_denial() {
        return denial;
    }
    if is_out_of_fuel(&error) {
        return WasmHostError::WasmResourceLimitExceeded {
            dimension: "fuel".to_string(),
        };
    }
    WasmHostError::InstantiationFailed {
        reason: error.to_string(),
    }
}

/// One instantiated, isolated plugin component.
///
/// The instance owns a private [`wasmtime::Store`]; a trap poisons only this
/// store. The host object and other instances stay usable.
pub struct WasmPluginInstance {
    store: Store<InstanceState>,
    bindings: ExternalPlugin,
    limits: WasmResourceLimits,
}

impl std::fmt::Debug for WasmPluginInstance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WasmPluginInstance")
            .finish_non_exhaustive()
    }
}

impl WasmPluginInstance {
    /// Invokes one exported operation with a fresh per-call fuel budget.
    pub fn invoke(&mut self, operation: &str, payload: Vec<u8>) -> Result<Vec<u8>, WasmHostError> {
        self.store
            .set_fuel(self.limits.fuel_budget_per_call)
            .map_err(|error| {
                // Not a guest failure: the host could not charge its own budget.
                // There is no host-adapter failure class in the v1 error set, so
                // this lands in the trap class with an explicit message.
                WasmHostError::WasmTrap {
                    message: format!("host failed to charge the per-call fuel budget: {error}"),
                }
            })?;

        let operations = self.bindings.worldline_plugin_plugin_operations();

        match operations.call_invoke(&mut self.store, operation, &payload) {
            Ok(Ok(bytes)) => {
                // A recorded denial outranks a successful reply: the call hit
                // a declared resource limit even if the guest recovered.
                match self.store.data().first_recorded_denial() {
                    Some(denial) => Err(denial),
                    None => Ok(bytes),
                }
            }
            Ok(Err(message)) => {
                // Guest-returned errors are ordinary RPC error outcomes, but
                // a recorded host-side denial is still the more precise
                // typed class.
                match self.store.data().first_recorded_denial() {
                    Some(denial) => Err(denial),
                    None => Err(WasmHostError::GuestReturnedError { message }),
                }
            }
            Err(error) => Err(self.classify_call_error(error)),
        }
    }

    /// Classifies one failed guest call into the typed failure classes.
    fn classify_call_error(&self, error: wasmtime::Error) -> WasmHostError {
        // 1. Recorded memory/table/payload denials are the most precise.
        if let Some(denial) = self.store.data().first_recorded_denial() {
            return denial;
        }
        // 2. Fuel exhaustion is a resource failure, not a plain trap.
        if is_out_of_fuel(&error) {
            return WasmHostError::WasmResourceLimitExceeded {
                dimension: "fuel".to_string(),
            };
        }
        // 3. Everything else is a guest trap isolated to this store.
        WasmHostError::WasmTrap {
            message: error.to_string(),
        }
    }
}

/// Detects wasmtime's out-of-fuel condition, preferring the typed trap code
/// with the message text as a fallback.
fn is_out_of_fuel(error: &wasmtime::Error) -> bool {
    if matches!(error.downcast_ref::<Trap>(), Some(Trap::OutOfFuel)) {
        return true;
    }
    error.to_string().contains("fuel consumed")
}
