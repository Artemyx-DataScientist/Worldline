//! Explicit native IPC handshake.
//!
//! The handshake is the first exchange on the framed transport. It carries
//! the protocol version, a host-generated session nonce, the package and
//! plugin definition identities the host expects, and the supported ABI
//! range. The child's acknowledgment is *validated against the host's
//! expectation*: child-supplied identity is never trusted or recorded as
//! authority. The host assigns the authoritative `RuntimeId` and security
//! identity only after a successful handshake and admission.

use serde::{Deserialize, Serialize};

use worldline_plugin_protocol::PROTOCOL_VERSION;

use crate::codec::{read_json_frame, write_json_frame};
use crate::error::NativeHostError;

/// The external ABI baseline this host build speaks (ADR v1).
pub const NATIVE_ABI_BASELINE: &str = "worldline-native-ipc/1";

/// First message from host to child.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostHello {
    pub protocol_version: u32,
    pub host_nonce: String,
    pub package_id: String,
    pub plugin_definition_id: String,
    pub abi_min: String,
    pub abi_max: String,
}

/// Reply from child to host. All fields are validated against the host's
/// own expectation; none of them grant or describe authority.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChildAck {
    pub protocol_version: u32,
    pub package_id: String,
    pub plugin_definition_id: String,
    pub abi: String,
    pub declared_interfaces: Vec<String>,
}

/// The identity the host expects the child to acknowledge.
#[derive(Clone, Debug)]
pub struct ExpectedIdentity {
    pub package_id: String,
    pub plugin_definition_id: String,
}

impl HostHello {
    /// Builds a hello with a session nonce derived from process and clock
    /// entropy. The nonce is a session correlation token, not a secret.
    pub fn new(expected: &ExpectedIdentity) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        Self {
            protocol_version: PROTOCOL_VERSION,
            host_nonce: format!("session-{}-{}", std::process::id(), nanos),
            package_id: expected.package_id.clone(),
            plugin_definition_id: expected.plugin_definition_id.clone(),
            abi_min: NATIVE_ABI_BASELINE.to_owned(),
            abi_max: NATIVE_ABI_BASELINE.to_owned(),
        }
    }
}

/// Writes the hello and validates the child's acknowledgment.
pub fn perform_host_handshake<W: std::io::Write, R: std::io::Read>(
    writer: &mut W,
    reader: &mut R,
    expected: &ExpectedIdentity,
    max_frame_bytes: usize,
) -> Result<ChildAck, NativeHostError> {
    let hello = HostHello::new(expected);
    write_json_frame(writer, &hello)?;
    let ack: ChildAck = read_json_frame(reader, max_frame_bytes)?;
    if ack.protocol_version != PROTOCOL_VERSION {
        return Err(NativeHostError::UnsupportedProtocolVersion {
            found: ack.protocol_version,
        });
    }
    if ack.package_id != expected.package_id {
        return Err(NativeHostError::HandshakeFailed {
            reason: format!(
                "child claims package '{}' but the host expects '{}'; child-supplied identity is never authoritative",
                ack.package_id, expected.package_id
            ),
        });
    }
    if ack.plugin_definition_id != expected.plugin_definition_id {
        return Err(NativeHostError::HandshakeFailed {
            reason: format!(
                "child claims plugin definition '{}' but the host expects '{}'",
                ack.plugin_definition_id, expected.plugin_definition_id
            ),
        });
    }
    Ok(ack)
}
