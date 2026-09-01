//! Supervised native-process execution adapter for the Worldline external
//! plugin boundary.
//!
//! This crate is the physical native-process translation of the boundary in
//! `docs/adr/ADR-EXTERNAL-PLUGIN-BOUNDARY-V1.md`: length-prefixed versioned
//! envelopes over the child's stdio pipes, an explicit handshake, bounded
//! in-flight requests, bounded stderr draining, deadline-based graceful
//! shutdown, and deterministic failure classification.
//!
//! Authority rules implemented here:
//!
//! - The child never supplies authoritative identity. The handshake only
//!   validates that the child matches the identity the host already
//!   expects; `RuntimeId` and `PrincipalId` are assigned by the host after
//!   admission and never travel on the wire.
//! - Trust never bypasses authorization: capability calls made through
//!   this transport are still admitted by the kernel's default-deny broker
//!   in the calling adapter.
//! - A chatty child cannot deadlock the host: stderr is drained by a
//!   bounded thread, in-flight requests and frame sizes are bounded before
//!   allocation, and malformed bytes classify as protocol violations that
//!   break only this connection.

mod codec;
mod connection;
pub mod containment;
mod error;
mod handshake;
mod supervisor;

pub use codec::{read_frame, read_json_frame, write_frame, write_json_frame};
pub use connection::{HostRequestSink, NativeProviderConnection};
pub use containment::{ProcessTreeContainment, ProcessTreeJob};
pub use error::NativeHostError;
pub use handshake::{
    ChildAck, ExpectedIdentity, HostHello, NATIVE_ABI_BASELINE, perform_host_handshake,
};
pub use supervisor::{NativeChild, NativeChildSpec};
