//! Sandboxed WASM Component execution adapter for the Worldline external
//! plugin boundary.
//!
//! This crate is the physical WASM translation of the boundary described in
//! `docs/adr/ADR-EXTERNAL-PLUGIN-BOUNDARY-V1.md` (change
//! C-KERNEL-STABLE-IPC-WASM-COMPONENT-BOUNDARY-20260831). The logical
//! vocabulary (identities, manifest, envelopes) lives in
//! `worldline-plugin-protocol`; this crate adapts it onto wasmtime's
//! Component Model runtime and exposes exactly one export/import surface,
//! the `external-plugin` world under `crates/worldline-plugin-protocol/wit/`.
//!
//! Boundary rules implemented here:
//!
//! - Nothing above the kernel is special: this adapter is a plugin-side
//!   executor and grants no authority of its own. Capability, state, and
//!   event semantics stay in the kernel behind [`WasmHostBroker`].
//! - Least authority by construction: the host registers no `wasi:*`
//!   bindings ([`WasiPermissionSet`]); a component importing WASI fails to
//!   link with [`WasmHostError::UnsupportedExternalAbi`].
//! - Resource isolation is bounded and typed: binary size, linear memory,
//!   table entries, fuel, and host-call payload bytes
//!   ([`WasmResourceLimits`]); exhaustion maps to typed failures and never
//!   permanently consumes a provider registry slot.
//! - Isolation is per instance: a trap poisons only the trapping instance's
//!   store; the host and other instances remain usable.
//! - The event bus is not RPC: `event-publish` forwards to the broker's
//!   publication side and can never substitute for an invoke result.

#![forbid(unsafe_code)]

mod adapter;
mod error;
mod limits;
mod wasi;

pub use adapter::{WasmComponent, WasmHostBroker, WasmPluginHost, WasmPluginInstance};
pub use error::WasmHostError;
pub use limits::{
    DEFAULT_FUEL_BUDGET_PER_CALL, DEFAULT_MAX_COMPONENT_BYTES, DEFAULT_MAX_HOST_CALL_PAYLOAD_BYTES,
    DEFAULT_MAX_MEMORY_BYTES, DEFAULT_MAX_TABLE_ENTRIES, WasmResourceLimits,
};
pub use wasi::{WasiPermissionClass, WasiPermissionSet};
