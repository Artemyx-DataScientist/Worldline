//! M0.6 native execution mode acceptance: framed codec bounds, handshake
//! identity validation, full-loop provider semantics with state round-trips
//! through a host sink, protocol-violation containment, and deadline-based
//! supervision.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use worldline_native_host::{
    ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError, NativeProviderConnection,
    read_frame, write_frame,
};
use worldline_plugin_protocol::{Envelope, MessageKind, REQUEST_POLICY_INTERFACE};

fn identity() -> ExpectedIdentity {
    ExpectedIdentity {
        package_id: "reference.echo.pkg".to_owned(),
        plugin_definition_id: "reference.echo.native".to_owned(),
    }
}

fn roundtrip(envelope: &Envelope) -> Result<Envelope, NativeHostError> {
    let max = 4096;
    let mut buffer = Vec::new();
    write_frame(&mut buffer, envelope)?;
    let mut cursor = Cursor::new(buffer);
    read_frame(&mut cursor, max)
}

#[test]
fn envelopes_roundtrip_through_the_framed_transport() {
    let envelope = Envelope::new(
        MessageKind::CapabilityRequest,
        7,
        json!({"operation": "echo", "bytes": [104, 105]}),
    );
    let decoded = roundtrip(&envelope).expect("framed roundtrip must succeed");
    assert_eq!(decoded, envelope);
}

#[test]
fn oversized_frames_are_rejected_before_allocation() {
    let huge = Envelope::new(MessageKind::CapabilityRequest, 1, Value::Null);
    let mut buffer = Vec::new();
    write_frame(&mut buffer, &huge).expect("encode must succeed");
    // Push the declared length far beyond the limit without writing bytes:
    // only the header is needed to trigger the gate.
    let mut hostile = vec![0xFF, 0xFF, 0xFF, 0xFF];
    hostile.extend_from_slice(&buffer[4..]);

    let error = read_frame(&mut Cursor::new(hostile), 1024)
        .expect_err("an oversized frame must be rejected");
    assert!(
        matches!(
            error,
            NativeHostError::PayloadTooLarge {
                actual: 0xFFFF_FFFF,
                limit: 1024
            }
        ),
        "expected PayloadTooLarge, got {error:?}"
    );
}

#[test]
fn truncated_frames_close_the_transport_deterministically() {
    let envelope = Envelope::new(MessageKind::CapabilityRequest, 1, json!({"a": 1}));
    let mut buffer = Vec::new();
    write_frame(&mut buffer, &envelope).expect("encode must succeed");
    buffer.truncate(6);

    let error =
        read_frame(&mut Cursor::new(buffer), 4096).expect_err("a truncated frame must not decode");
    assert!(matches!(error, NativeHostError::TransportClosed));
}

#[test]
fn handshake_rejects_identity_mismatch_and_unknown_versions() {
    let expected = identity();

    use worldline_native_host::{ChildAck, HostHello};

    let _hello: HostHello = HostHello::new(&expected);

    // Unknown protocol version: the ack is framed exactly like a real child
    // would frame it.
    let mut framed = Vec::new();
    worldline_native_host::write_json_frame(
        &mut framed,
        &ChildAck {
            protocol_version: 99,
            package_id: expected.package_id.clone(),
            plugin_definition_id: expected.plugin_definition_id.clone(),
            abi: "worldline-native-ipc/1".to_owned(),
            declared_interfaces: Vec::new(),
        },
    )
    .expect("framing must succeed");
    let error = worldline_native_host::perform_host_handshake(
        &mut Vec::new(),
        &mut Cursor::new(framed),
        &expected,
        4096,
    )
    .expect_err("unknown protocol versions must fail closed");
    assert!(
        matches!(
            error,
            NativeHostError::UnsupportedProtocolVersion { found: 99 }
        ),
        "actual error: {error:?}"
    );

    // Wrong identity: the child's claim must match the host expectation.
    let mut framed = Vec::new();
    worldline_native_host::write_json_frame(
        &mut framed,
        &ChildAck {
            protocol_version: 1,
            package_id: "some.other.pkg".to_owned(),
            plugin_definition_id: expected.plugin_definition_id.clone(),
            abi: "worldline-native-ipc/1".to_owned(),
            declared_interfaces: Vec::new(),
        },
    )
    .expect("framing must succeed");
    let error = worldline_native_host::perform_host_handshake(
        &mut Vec::new(),
        &mut Cursor::new(framed),
        &expected,
        4096,
    )
    .expect_err("child identity claims must be validated, never trusted");
    assert!(matches!(error, NativeHostError::HandshakeFailed { .. }));
}

/// Host sink backing child state requests with a map and recording event
/// publications.
#[derive(Default)]
struct MapSink {
    state: Mutex<BTreeMap<String, Vec<u8>>>,
    publications: Mutex<Vec<(String, String, Vec<u8>)>>,
}

impl HostRequestSink for MapSink {
    fn on_child_request(
        &self,
        kind: MessageKind,
        _correlation_id: u64,
        payload: Value,
    ) -> Result<Option<Value>, NativeHostError> {
        match kind {
            MessageKind::StateRequest => {
                let key = payload
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                match payload.get("action").and_then(Value::as_str) {
                    Some("get") => {
                        let value = self.state.lock().expect("state lock").get(&key).cloned();
                        Ok(Some(json!({ "value": value })))
                    }
                    Some("set") => {
                        let value = payload
                            .get("value")
                            .and_then(Value::as_array)
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(Value::as_u64)
                                    .map(|value| value as u8)
                                    .collect::<Vec<u8>>()
                            })
                            .unwrap_or_default();
                        self.state.lock().expect("state lock").insert(key, value);
                        Ok(None)
                    }
                    other => Err(NativeHostError::ProtocolViolation {
                        reason: format!("unknown state action {other:?}"),
                    }),
                }
            }
            MessageKind::EventPublishRequest => {
                let namespace = payload
                    .get("namespace")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let bytes = payload
                    .get("bytes")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_u64)
                            .map(|value| value as u8)
                            .collect::<Vec<u8>>()
                    })
                    .unwrap_or_default();
                self.publications
                    .lock()
                    .expect("publications lock")
                    .push((namespace, name, bytes));
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

fn provider_spec() -> NativeChildSpec {
    NativeChildSpec {
        program: PathBuf::from(env!("CARGO_BIN_EXE_reference-native-provider")),
        args: vec![
            "--package-id".to_owned(),
            "reference.echo.pkg".to_owned(),
            "--definition-id".to_owned(),
            "reference.echo.native".to_owned(),
        ],
        max_frame_bytes: 4 * 1024 * 1024,
        stderr_max_bytes: 64 * 1024,
        enable_process_tree_containment: false,
    }
}

#[test]
fn unnegotiated_request_policy_interface_fails_closed() {
    let error = NativeProviderConnection::connect_with_required_interface(
        provider_spec(),
        &identity(),
        Arc::new(MapSink::default()),
        4,
        REQUEST_POLICY_INTERFACE,
    )
    .expect_err("reference echo provider must not receive an unnegotiated policy plane");
    assert!(matches!(error, NativeHostError::HandshakeFailed { .. }));
}

#[test]
fn native_provider_implements_the_echo_semantics_with_host_state() {
    let sink = Arc::new(MapSink::default());
    let (connection, ack) =
        NativeProviderConnection::connect(provider_spec(), &identity(), sink.clone(), 16)
            .expect("the native provider must connect");
    assert_eq!(ack.protocol_version, 1);

    let bytes = |payload: &Value| -> Vec<u8> {
        payload
            .get("bytes")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|value| value as u8)
                    .collect::<Vec<u8>>()
            })
            .unwrap_or_default()
    };

    let echoed = connection
        .call(json!({"operation": "echo", "bytes": [104, 105]}))
        .expect("echo must succeed");
    assert_eq!(bytes(&echoed), b"echo:hi");

    let first = connection
        .call(json!({"operation": "stateful_increment", "bytes": [97]}))
        .expect("first increment must succeed");
    let second = connection
        .call(json!({"operation": "stateful_increment", "bytes": [98]}))
        .expect("second increment must succeed");
    assert_eq!(bytes(&first), b"incremented:1:a");
    assert_eq!(bytes(&second), b"incremented:2:b");

    // State authority stayed with the host sink, never the child.
    assert_eq!(
        sink.state
            .lock()
            .expect("state lock")
            .get("reference-echo-count")
            .cloned()
            .unwrap_or_default(),
        b"2".to_vec()
    );

    let observed = connection
        .call(json!({"operation": "publish_observation", "bytes": [110]}))
        .expect("observation publish must succeed");
    assert_eq!(bytes(&observed), b"observed:n");
    assert_eq!(
        sink.publications
            .lock()
            .expect("publications lock")
            .as_slice(),
        &[(
            "reference.echo".to_owned(),
            "observation".to_owned(),
            b"n".to_vec()
        )]
    );

    connection
        .close(Duration::from_secs(5))
        .expect("graceful close must succeed");
}

#[test]
fn native_provider_honors_deadlines_with_cancellation() {
    let sink = Arc::new(MapSink::default());
    let mut spec = provider_spec();
    // After the handshake the hung child never answers, so the deadline is
    // guaranteed to elapse.
    spec.args.push("--hang".to_owned());
    let (connection, _ack) = NativeProviderConnection::connect(spec, &identity(), sink, 16)
        .expect("the hung child completes its handshake");

    let error = connection
        .call_with_deadline(
            json!({"operation": "echo", "bytes": [120]}),
            Duration::from_millis(150),
        )
        .expect_err("an impossible deadline must produce a typed timeout");
    assert!(matches!(
        error,
        NativeHostError::DeadlineExceeded { deadline_ms: 150 }
    ));

    connection.kill();
}

#[test]
fn protocol_violation_is_contained_without_harming_the_host() {
    let sink = Arc::new(MapSink::default());
    let violator = NativeChildSpec {
        program: PathBuf::from(env!("CARGO_BIN_EXE_test-violator")),
        args: Vec::new(),
        max_frame_bytes: 4 * 1024 * 1024,
        stderr_max_bytes: 64 * 1024,
        enable_process_tree_containment: false,
    };
    let (connection, _ack) = NativeProviderConnection::connect(
        violator,
        &identity(),
        Arc::clone(&sink) as Arc<dyn HostRequestSink>,
        16,
    )
    .expect("the violator completes its handshake");

    let error = connection
        .call(json!({"operation": "echo", "bytes": []}))
        .expect_err("garbage bytes must fail the pending call deterministically");
    assert!(
        matches!(
            error,
            NativeHostError::ProtocolViolation { .. } | NativeHostError::TransportClosed
        ),
        "actual error: {error:?}"
    );

    // The host is unharmed: a fresh connection to the real provider works.
    let (healthy, _ack) = NativeProviderConnection::connect(provider_spec(), &identity(), sink, 16)
        .expect("a new connection must succeed after a violation");
    let echoed = healthy
        .call(json!({"operation": "echo", "bytes": [111, 107]}))
        .expect("echo must succeed on the healthy connection");
    assert_eq!(
        echoed
            .get("bytes")
            .and_then(Value::as_array)
            .map(|v| v.len()),
        Some(7)
    );
    healthy.kill();
}

#[test]
fn a_hung_child_is_killed_after_the_shutdown_deadline() {
    let sink = Arc::new(MapSink::default());
    let mut spec = provider_spec();
    spec.args.push("--hang".to_owned());
    let (connection, _ack) = NativeProviderConnection::connect(spec, &identity(), sink, 16)
        .expect("the hung child completes its handshake");

    let error = connection
        .close(Duration::from_millis(250))
        .expect_err("a hung child must be killed after the deadline");
    assert!(matches!(
        error,
        NativeHostError::ShutdownTimeout { deadline_ms: 250 }
    ));
}
