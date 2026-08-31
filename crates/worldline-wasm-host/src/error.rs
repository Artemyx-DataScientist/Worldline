//! Typed failure classes for the WASM component boundary.
//!
//! Every failure crossing this boundary is one explicit variant of
//! [`WasmHostError`]; nothing degrades into a generic plugin error. The
//! mapping follows the "Failure mapping" table of
//! `docs/adr/ADR-EXTERNAL-PLUGIN-BOUNDARY-V1.md`:
//!
//! - Guest traps isolate to the exact instance and its own store; the host
//!   object and other instances keep working.
//! - Guest-returned `err(...)` values are ordinary RPC error outcomes.
//! - Resource-limit exhaustion is a typed class carrying the exhausted
//!   dimension, never a permanent loss of a provider registry slot: the host
//!   object stays usable for fresh instantiations.
//! - Authority failures (a component demanding interfaces the host does not
//!   provide, notably any `wasi:*` import) fail closed before instantiation.

use std::fmt;

/// Typed failure classes of the WASM component boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WasmHostError {
    /// The component imports interfaces the host linker does not provide.
    ///
    /// Most importantly: any component importing `wasi:*` fails here, because
    /// the host registers no WASI bindings at all in v1 (least authority by
    /// construction; see [`crate::WasiPermissionSet`]). Binaries that do not
    /// decode as Component Model components at all also land here, since they
    /// do not speak the supported external ABI.
    UnsupportedExternalAbi {
        /// Human-readable explanation of which import or decode step failed.
        reason: String,
    },
    /// The guest trapped (unreachable, out-of-bounds, stack exhaustion, ...).
    ///
    /// The trap is isolated to this instance's store; the host object and all
    /// other instances remain usable.
    WasmTrap {
        /// Wasmtime's trap description.
        message: String,
    },
    /// A declared resource limit was exhausted.
    ///
    /// `dimension` is the exhausted resource, for example `"fuel"`,
    /// `"memory"`, or `"table"`. Exhaustion is a typed per-call failure and
    /// never permanently consumes a provider registry slot: fresh
    /// instantiations through the same [`crate::WasmPluginHost`] are
    /// unaffected.
    WasmResourceLimitExceeded {
        /// Name of the exhausted resource dimension.
        dimension: String,
    },
    /// The guest returned `err(message)` from its exported operation.
    ///
    /// This is an ordinary RPC error outcome, not a sandbox failure.
    GuestReturnedError {
        /// Message returned by the guest.
        message: String,
    },
    /// A host-call payload exceeded `max_host_call_payload_bytes`.
    ///
    /// Enforced before any oversized buffer is handed to the broker or back
    /// into the component ABI.
    ExternalPayloadTooLarge {
        /// Configured limit in bytes.
        limit: usize,
        /// Observed payload size in bytes.
        actual: usize,
    },
    /// The component binary exceeded `max_component_bytes`.
    ///
    /// Checked before compilation, so an oversized binary never reaches the
    /// wasmtime compiler.
    ComponentTooLarge {
        /// Configured limit in bytes.
        limit: usize,
        /// Observed binary size in bytes.
        actual: usize,
    },
    /// Instantiation (or link) of the component failed for a reason that is
    /// not an authority failure or a resource-limit denial: missing exports,
    /// mismatched interface types, or a failing start function.
    InstantiationFailed {
        /// Wasmtime's instantiation failure description.
        reason: String,
    },
}

impl fmt::Display for WasmHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedExternalAbi { reason } => {
                write!(f, "unsupported external ABI: {reason}")
            }
            Self::WasmTrap { message } => write!(f, "guest trapped: {message}"),
            Self::WasmResourceLimitExceeded { dimension } => {
                write!(f, "resource limit exceeded: {dimension}")
            }
            Self::GuestReturnedError { message } => write!(f, "guest returned error: {message}"),
            Self::ExternalPayloadTooLarge { limit, actual } => {
                write!(
                    f,
                    "host-call payload too large: {actual} bytes (limit {limit})"
                )
            }
            Self::ComponentTooLarge { limit, actual } => {
                write!(f, "component too large: {actual} bytes (limit {limit})")
            }
            Self::InstantiationFailed { reason } => {
                write!(f, "instantiation failed: {reason}")
            }
        }
    }
}

impl std::error::Error for WasmHostError {}
