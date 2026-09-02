//! Engine-neutral identities shared by reference request-policy fixtures.
//!
//! This module is deliberately limited to deterministic reference evidence.
//! It does not select a production failure mode and is never used as hosted
//! CEF evidence.

/// Stable evaluator identity used by the bounded reference feasibility fixture.
pub const REFERENCE_REQUEST_POLICY_PROVIDER_ID: &str = "worldline.reference.request-policy";

/// Human-readable topology label for local reference-only evidence.
pub const REFERENCE_REQUEST_POLICY_TOPOLOGY: &str = "reference broker only; no CEF or native IPC";
