//! Versioned envelope protocol for the native IPC transport.
//!
//! The native process transport carries length-prefixed, JSON-encoded
//! envelopes over stdio pipes. Every envelope carries an explicit protocol
//! version: representation without an explicit protocol version contract is
//! forbidden at this boundary.
//!
//! Compatibility rule: unknown protocol versions fail closed. Payload-internal
//! unknown fields are ignored by `serde_json::Value` semantics, but the
//! envelope's own wire fields are fail-closed (unknown or missing wire fields
//! are rejected).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ProtocolError;

/// Native IPC envelope protocol version understood by this build. Any other
/// version fails closed with [`ProtocolError::UnsupportedProtocolVersion`].
pub const PROTOCOL_VERSION: u32 = 1;

/// The nine message classes of the native IPC protocol.
///
/// These are the spec's message classes, not product messages: capability
/// invocation stays distinct from event publication (`EVENT BUS IS NOT RPC`),
/// and results are always correlated to requests by [`Envelope::correlation_id`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Lifecycle request (activate/deactivate).
    LifecycleRequest,
    /// Lifecycle result for a correlated lifecycle request.
    LifecycleResult,
    /// Capability invocation request.
    CapabilityRequest,
    /// Capability invocation result for a correlated request.
    CapabilityResult,
    /// Cancellation notice for a correlated in-flight request.
    Cancellation,
    /// Event publication request. Delivery never substitutes for an RPC
    /// result.
    EventPublishRequest,
    /// Installation-state access request.
    StateRequest,
    /// State access result for a correlated request.
    StateResult,
    /// Protocol-level error report (e.g. in reply to a malformed frame).
    ProtocolError,
}

/// One versioned frame of the native IPC protocol.
///
/// Wire shape (JSON):
///
/// ```json
/// {
///   "protocol_version": 1,
///   "message_kind": "capability_request",
///   "correlation_id": 42,
///   "payload": {}
/// }
/// ```
///
/// `correlation_id` is the explicit bounded protocol identity correlating
/// every response to its request; it never carries authority by itself.
/// `payload` is protocol-opaque JSON owned by the message class's contract.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// Wire protocol version. Must equal [`PROTOCOL_VERSION`] on decode.
    pub protocol_version: u32,
    /// Message class of this frame.
    pub message_kind: MessageKind,
    /// Explicit bounded identity correlating responses with requests.
    pub correlation_id: u64,
    /// Protocol-opaque JSON payload owned by the message contract.
    pub payload: Value,
}

impl Envelope {
    /// Builds an envelope speaking the current [`PROTOCOL_VERSION`].
    #[must_use]
    pub fn new(message_kind: MessageKind, correlation_id: u64, payload: Value) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            message_kind,
            correlation_id,
            payload,
        }
    }

    /// Encodes the envelope as JSON bytes, ready for length-prefixed framing.
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        serde_json::to_vec(self).map_err(|error| ProtocolError::MalformedEnvelope {
            reason: error.to_string(),
        })
    }

    /// Decodes one frame.
    ///
    /// Order of checks, by contract:
    ///
    /// 1. Frame size is checked against `max_frame_bytes` BEFORE any parsing
    ///    or allocation, yielding [`ProtocolError::PayloadTooLarge`].
    /// 2. The frame is parsed as JSON, yielding
    ///    [`ProtocolError::MalformedEnvelope`] for invalid JSON or unknown or
    ///    missing wire fields.
    /// 3. `protocol_version != PROTOCOL_VERSION` yields
    ///    [`ProtocolError::UnsupportedProtocolVersion`].
    pub fn decode(bytes: &[u8], max_frame_bytes: usize) -> Result<Self, ProtocolError> {
        // Size check happens before any parsing or allocation.
        if bytes.len() > max_frame_bytes {
            return Err(ProtocolError::PayloadTooLarge {
                limit: max_frame_bytes,
                actual: bytes.len(),
            });
        }
        let envelope: Self =
            serde_json::from_slice(bytes).map_err(|error| ProtocolError::MalformedEnvelope {
                reason: error.to_string(),
            })?;
        if envelope.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedProtocolVersion {
                found: envelope.protocol_version,
            });
        }
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn roundtrips_encode_decode() {
        let envelope = Envelope::new(
            MessageKind::CapabilityRequest,
            42,
            json!({"operation": "lookup", "args": {"key": "value"}}),
        );
        let bytes = envelope.encode().expect("encode");
        let decoded = Envelope::decode(&bytes, 4096).expect("decode");
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.protocol_version, PROTOCOL_VERSION);
        assert_eq!(decoded.message_kind, MessageKind::CapabilityRequest);
        assert_eq!(decoded.correlation_id, 42);
    }

    #[test]
    fn message_kinds_use_snake_case_names() {
        let kinds = [
            (MessageKind::LifecycleRequest, "lifecycle_request"),
            (MessageKind::LifecycleResult, "lifecycle_result"),
            (MessageKind::CapabilityRequest, "capability_request"),
            (MessageKind::CapabilityResult, "capability_result"),
            (MessageKind::Cancellation, "cancellation"),
            (MessageKind::EventPublishRequest, "event_publish_request"),
            (MessageKind::StateRequest, "state_request"),
            (MessageKind::StateResult, "state_result"),
            (MessageKind::ProtocolError, "protocol_error"),
        ];
        assert_eq!(kinds.len(), 9, "the protocol has exactly nine classes");
        for (kind, name) in kinds {
            let json = serde_json::to_string(&kind).expect("serialize");
            assert_eq!(json, format!(r#""{name}""#));
        }
    }

    #[test]
    fn rejects_oversized_frame_before_parsing() {
        // Not even valid JSON: proves the size gate fires before parsing.
        let bytes = vec![b'a'; 8192];
        let error = Envelope::decode(&bytes, 1024).expect_err("must reject");
        assert_eq!(
            error,
            ProtocolError::PayloadTooLarge {
                limit: 1024,
                actual: 8192
            }
        );
    }

    #[test]
    fn accepts_frame_exactly_at_limit() {
        let bytes = Envelope::new(MessageKind::StateRequest, 1, json!({}))
            .encode()
            .expect("encode");
        let limit = bytes.len();
        Envelope::decode(&bytes, limit).expect("frame at the limit must decode");
        let error = Envelope::decode(&bytes, limit - 1).expect_err("one over must fail");
        assert!(matches!(error, ProtocolError::PayloadTooLarge { .. }));
    }

    #[test]
    fn unsupported_protocol_version_fails_closed() {
        let json = json!({
            "protocol_version": 99,
            "message_kind": "lifecycle_request",
            "correlation_id": 7,
            "payload": {}
        });
        let error = Envelope::decode(json.to_string().as_bytes(), 4096)
            .expect_err("unknown version must fail closed");
        assert_eq!(
            error,
            ProtocolError::UnsupportedProtocolVersion { found: 99 }
        );
    }

    #[test]
    fn rejects_unknown_and_missing_wire_fields() {
        let unknown_field = json!({
            "protocol_version": 1,
            "message_kind": "cancellation",
            "correlation_id": 1,
            "payload": {},
            "extra_wire_field": true
        });
        let error = Envelope::decode(unknown_field.to_string().as_bytes(), 4096)
            .expect_err("unknown wire field must fail closed");
        assert!(matches!(error, ProtocolError::MalformedEnvelope { .. }));

        let missing_field = json!({
            "protocol_version": 1,
            "message_kind": "cancellation",
            "payload": {}
        });
        let error = Envelope::decode(missing_field.to_string().as_bytes(), 4096)
            .expect_err("missing wire field must fail closed");
        assert!(matches!(error, ProtocolError::MalformedEnvelope { .. }));
    }

    #[test]
    fn payload_unknown_fields_pass_through() {
        let envelope = Envelope::new(
            MessageKind::CapabilityResult,
            5,
            json!({"outcome": "ok", "brand_new_future_field": {"a": 1}}),
        );
        let bytes = envelope.encode().expect("encode");
        let decoded = Envelope::decode(&bytes, 4096).expect("payload fields are opaque");
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn rejects_non_json_frames() {
        let error =
            Envelope::decode(b"not json at all", 4096).expect_err("garbage must fail closed");
        assert!(matches!(error, ProtocolError::MalformedEnvelope { .. }));
    }
}
