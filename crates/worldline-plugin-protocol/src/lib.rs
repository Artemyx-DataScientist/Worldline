//! ABI-neutral vocabulary for the Worldline external plugin boundary.
//!
//! This crate owns the transport-neutral dictionary shared by every external
//! adapter of the kernel: package and plugin identities, the plugin manifest
//! schema, and the versioned envelope protocol for the native IPC transport.
//! It intentionally exposes no kernel Rust types: the public Rust ABI is not
//! an external ABI, and nothing in this crate can grant capability authority.
//!
//! Boundary rules implemented here:
//!
//! - Identities are distribution vocabulary only; they never grant authority.
//! - A manifest describes requested permissions; it never grants them, and
//!   loading one never activates authority.
//! - Unknown manifest fields, unknown manifest schema versions, and unknown
//!   protocol versions fail closed.
//! - Boundary payload size is bounded before parsing or allocation.
//!
//! Physical translations live in adapters: the WIT interfaces under `wit/`
//! for the WASM Component Model, and the JSON envelopes in this crate over
//! stdio pipes for native processes. See
//! `docs/adr/ADR-EXTERNAL-PLUGIN-BOUNDARY-V1.md`.

#![forbid(unsafe_code)]

mod blob;
mod compatibility;
mod envelope;
mod error;
mod identity;
mod manifest;

pub use blob::{BlobAction, BlobRequest, BlobResult, MAX_BLOB_CHUNK_BYTES};
pub use compatibility::{
    ContractCompatibilityOutcome, ContractSpec, ContractStability, SUPPORTED_SDK_VERSIONS,
    evaluate_abi_compatibility, is_supported_sdk_version,
};
pub use envelope::{Envelope, MessageKind, PROTOCOL_VERSION};
pub use error::ProtocolError;
pub use identity::{
    AbiVersion, InstallationRevision, PackageRevisionId, PackageVersion, PluginDefinitionId,
    PluginPackageId,
};
pub use manifest::{
    ExecutionMode, MANIFEST_SCHEMA_VERSION, PermissionClass, PluginManifest, RequestedPermission,
    ResourceLimitHints,
};
