//! Resource isolation dimensions for the WASM component adapter.
//!
//! ADR "Resource isolation" requires bounded dimensions with typed failures.
//! For the WASM adapter these are: component binary size (checked before
//! compilation), linear memory, component/table entries, CPU/execution budget
//! (fuel), and host-call payload bytes. Exhaustion of any dimension maps to
//! an explicit typed failure ([`WasmHostError::WasmResourceLimitExceeded`],
//! [`WasmHostError::ExternalPayloadTooLarge`],
//! [`WasmHostError::ComponentTooLarge`]).
//!
//! Exhaustion never permanently consumes a provider registry slot: a
//! component or instance that exhausts a limit is discarded, the host object
//! holds no per-component state, and fresh `load_component` /
//! `make_instance` calls keep working. Wall-clock/deadline and
//! host-call-concurrency dimensions are handled by the shared RPC semantics
//! around the adapter, not by this type.

use worldline_plugin_protocol::ResourceLimitHints;

use crate::error::WasmHostError;

/// Default cap for one component binary: 8 MiB.
pub const DEFAULT_MAX_COMPONENT_BYTES: usize = 8 * 1024 * 1024;
/// Default cap for guest linear memory: 64 MiB.
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
/// Default cap for component/resource table entries: 10 000.
pub const DEFAULT_MAX_TABLE_ENTRIES: usize = 10_000;
/// Default CPU/execution budget per call: 1 000 000 000 wasmtime fuel units.
pub const DEFAULT_FUEL_BUDGET_PER_CALL: u64 = 1_000_000_000;
/// Default cap for one host-call payload (state value, event payload): 1 MiB.
pub const DEFAULT_MAX_HOST_CALL_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Resource limits applied to every component loaded and instantiated
/// through one [`WasmPluginHost`](crate::WasmPluginHost).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WasmResourceLimits {
    /// Maximum component binary size in bytes, enforced before compilation.
    pub max_component_bytes: usize,
    /// Maximum guest linear memory in bytes, enforced as a wasmtime store
    /// limit. Exceeding it denies memory growth (dimension `"memory"`).
    pub max_memory_bytes: usize,
    /// Maximum number of table entries, enforced as a wasmtime store limit.
    /// Exceeding it denies table growth (dimension `"table"`).
    pub max_table_entries: usize,
    /// Execution budget (wasmtime fuel) charged per `invoke` call and per
    /// instantiation. Exhaustion surfaces as dimension `"fuel"`.
    pub fuel_budget_per_call: u64,
    /// Maximum payload bytes of one host call, enforced before any oversized
    /// buffer is handed to the broker or back across the boundary.
    pub max_host_call_payload_bytes: usize,
}

impl Default for WasmResourceLimits {
    fn default() -> Self {
        Self {
            max_component_bytes: DEFAULT_MAX_COMPONENT_BYTES,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_table_entries: DEFAULT_MAX_TABLE_ENTRIES,
            fuel_budget_per_call: DEFAULT_FUEL_BUDGET_PER_CALL,
            max_host_call_payload_bytes: DEFAULT_MAX_HOST_CALL_PAYLOAD_BYTES,
        }
    }
}

impl WasmResourceLimits {
    /// Builds limits from the resource limit hints declared in a package
    /// manifest.
    ///
    /// Hints are inputs to host policy, never authority: a hint below the
    /// default tightens the limit, a hint above (or an absent hint) falls
    /// back to the default. Manifest declarations can only ever restrict,
    /// never extend, the host's baseline policy. There is no manifest hint
    /// for the component size or the fuel budget; both always use the
    /// defaults.
    pub fn from_manifest_hints(hints: &ResourceLimitHints) -> Self {
        Self {
            max_memory_bytes: clamp_hint(hints.memory_bytes, DEFAULT_MAX_MEMORY_BYTES as u64)
                as usize,
            max_table_entries: clamp_hint(
                hints.table_entries.map(u64::from),
                DEFAULT_MAX_TABLE_ENTRIES as u64,
            ) as usize,
            fuel_budget_per_call: DEFAULT_FUEL_BUDGET_PER_CALL,
            max_host_call_payload_bytes: clamp_hint(
                hints.host_call_payload_bytes,
                DEFAULT_MAX_HOST_CALL_PAYLOAD_BYTES as u64,
            ) as usize,
            ..Self::default()
        }
    }

    /// The host-call payload gate.
    ///
    /// Enforced before any buffer is handed around: guest-to-broker payloads
    /// (`state-access.set`, `event-publish.publish`) are checked before the
    /// broker sees them, and broker-to-guest payloads (`state-access.get`
    /// results) are checked before they re-enter the component ABI. The
    /// check is on the reported length only, so rejecting an oversized
    /// payload never allocates or copies it.
    pub fn check_host_call_payload(&self, len: usize) -> Result<(), WasmHostError> {
        if len > self.max_host_call_payload_bytes {
            Err(WasmHostError::ExternalPayloadTooLarge {
                limit: self.max_host_call_payload_bytes,
                actual: len,
            })
        } else {
            Ok(())
        }
    }
}

/// Clamps one optional hint at `default`: `None` and values above `default`
/// yield `default`, smaller values are honored.
fn clamp_hint(hint: Option<u64>, default: u64) -> u64 {
    match hint {
        Some(value) => value.min(default),
        None => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        let limits = WasmResourceLimits::default();
        assert_eq!(limits.max_component_bytes, 8 * 1024 * 1024);
        assert_eq!(limits.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.max_table_entries, 10_000);
        assert_eq!(limits.fuel_budget_per_call, 1_000_000_000);
        assert_eq!(limits.max_host_call_payload_bytes, 1024 * 1024);
    }

    #[test]
    fn payload_gate_rejects_only_above_limit() {
        let limits = WasmResourceLimits::default();
        assert_eq!(limits.check_host_call_payload(0), Ok(()));
        assert_eq!(limits.check_host_call_payload(1024 * 1024), Ok(()));
        assert_eq!(
            limits.check_host_call_payload(1024 * 1024 + 1),
            Err(WasmHostError::ExternalPayloadTooLarge {
                limit: 1024 * 1024,
                actual: 1024 * 1024 + 1
            })
        );
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn manifest_hints_only_tighten() {
        let mut hints = ResourceLimitHints::default();
        hints.memory_bytes = Some(32 * 1024 * 1024);
        hints.table_entries = Some(128);
        hints.host_call_payload_bytes = Some(4096);
        let limits = WasmResourceLimits::from_manifest_hints(&hints);
        assert_eq!(limits.max_memory_bytes, 32 * 1024 * 1024);
        assert_eq!(limits.max_table_entries, 128);
        assert_eq!(limits.max_host_call_payload_bytes, 4096);

        hints.memory_bytes = Some(u64::MAX);
        hints.table_entries = None;
        let limits = WasmResourceLimits::from_manifest_hints(&hints);
        assert_eq!(limits.max_memory_bytes, DEFAULT_MAX_MEMORY_BYTES);
        assert_eq!(limits.max_table_entries, DEFAULT_MAX_TABLE_ENTRIES);
    }
}
