//! The reference native provider child: one binary implementing
//! `reference.echo/v1` semantics over the framed envelope protocol with
//! byte-identical result formats across execution modes. State round-trips
//! go back to the host as `StateRequest` envelopes, so the authoritative
//! copy always lives in the host's installation-owned state contract.

use std::io::Write;

use serde_json::Value;
use worldline_native_host::{NativeHostError, read_frame, write_frame};
use worldline_plugin_protocol::{Envelope, MessageKind, PROTOCOL_VERSION};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut package_id = String::new();
    let mut definition_id = String::new();
    let mut hang = false;
    let mut cursor = 1;
    while cursor < args.len() {
        match args[cursor].as_str() {
            "--package-id" => {
                cursor += 1;
                package_id = args.get(cursor).cloned().unwrap_or_default();
            }
            "--definition-id" => {
                cursor += 1;
                definition_id = args.get(cursor).cloned().unwrap_or_default();
            }
            "--hang" => hang = true,
            other => {
                eprintln!("unknown argument '{other}'");
                std::process::exit(64);
            }
        }
        cursor += 1;
    }

    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let max_frame_bytes: usize = 4 * 1024 * 1024;

    // Handshake: the child validates the host hello's protocol version and
    // echoes its own identity arguments. The host decides whether that
    // identity matches its expectation; the child never asserts authority.
    let hello: serde_json::Value =
        match worldline_native_host::read_json_frame(&mut stdin, max_frame_bytes) {
            Ok(hello) => hello,
            Err(_) => std::process::exit(2),
        };
    if hello.get("protocol_version").and_then(Value::as_u64) != Some(PROTOCOL_VERSION as u64) {
        std::process::exit(2);
    }
    let ack = serde_json::json!({
        "protocol_version": PROTOCOL_VERSION,
        "package_id": package_id,
        "plugin_definition_id": definition_id,
        "abi": "worldline-native-ipc/1",
        "declared_interfaces": ["reference.echo/v1"],
    });
    if worldline_native_host::write_json_frame(&mut stdout, &ack).is_err() {
        std::process::exit(2);
    }

    if hang {
        // Test mode: stay alive without answering, to exercise the
        // supervisor's shutdown deadline and kill path.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    let mut cancelled: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    loop {
        let envelope = match read_frame(&mut stdin, max_frame_bytes) {
            Ok(envelope) => envelope,
            Err(NativeHostError::TransportClosed) => std::process::exit(0),
            Err(_) => std::process::exit(2),
        };
        match envelope.message_kind {
            MessageKind::CapabilityRequest => {
                let reply = handle_capability(&mut stdin, &mut stdout, &envelope, &cancelled);
                let payload = match reply {
                    Ok(bytes) => serde_json::json!({"bytes": bytes}),
                    Err(message) => serde_json::json!({"error": message}),
                };
                if write_frame(
                    &mut stdout,
                    &Envelope::new(
                        MessageKind::CapabilityResult,
                        envelope.correlation_id,
                        payload,
                    ),
                )
                .is_err()
                {
                    std::process::exit(2);
                }
            }
            MessageKind::Cancellation => {
                cancelled.insert(envelope.correlation_id);
            }
            MessageKind::LifecycleRequest => {
                let _ = write_frame(
                    &mut stdout,
                    &Envelope::new(
                        MessageKind::LifecycleResult,
                        envelope.correlation_id,
                        serde_json::json!({"ok": true}),
                    ),
                );
                std::process::exit(0);
            }
            MessageKind::ProtocolError => std::process::exit(3),
            _ => std::process::exit(2),
        }
    }
}

fn handle_capability(
    stdin: &mut impl std::io::Read,
    stdout: &mut impl Write,
    request: &Envelope,
    cancelled: &std::collections::BTreeSet<u64>,
) -> Result<Vec<u8>, String> {
    let operation = request
        .payload
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let payload = request
        .payload
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

    if cancelled.contains(&request.correlation_id) {
        return Err("cancelled".to_owned());
    }

    match operation.as_str() {
        "echo" => Ok(format!("echo:{}", String::from_utf8_lossy(&payload)).into_bytes()),
        "stateful_increment" => {
            // Load through the host's installation state contract.
            let get_request = Envelope::new(
                MessageKind::StateRequest,
                request.correlation_id.wrapping_add(0x0001_0000),
                serde_json::json!({"action": "get", "key": "reference-echo-count"}),
            );
            write_frame(stdout, &get_request).map_err(|_| "state transport closed".to_owned())?;
            let result = read_frame(stdin, 4 * 1024 * 1024)
                .map_err(|_| "state transport closed".to_owned())?;
            let current = result
                .payload
                .get("value")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_u64)
                        .map(|value| value as u8)
                        .collect::<Vec<u8>>()
                })
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|text| text.parse::<u64>().ok())
                .unwrap_or_default();
            let next = current.checked_add(1).ok_or("echo count exhausted")?;

            let set_request = Envelope::new(
                MessageKind::StateRequest,
                request.correlation_id.wrapping_add(0x0002_0000),
                serde_json::json!({
                    "action": "set",
                    "key": "reference-echo-count",
                    "value": next.to_string().into_bytes(),
                }),
            );
            write_frame(stdout, &set_request).map_err(|_| "state transport closed".to_owned())?;
            let committed = read_frame(stdin, 4 * 1024 * 1024)
                .map_err(|_| "state transport closed".to_owned())?;
            if committed.message_kind != MessageKind::StateResult {
                return Err("unexpected reply while committing state".to_owned());
            }
            Ok(format!("incremented:{next}:{}", String::from_utf8_lossy(&payload)).into_bytes())
        }
        "publish_observation" => {
            let publish = Envelope::new(
                MessageKind::EventPublishRequest,
                request.correlation_id.wrapping_add(0x0003_0000),
                serde_json::json!({
                    "namespace": "reference.echo",
                    "name": "observation",
                    "bytes": payload,
                }),
            );
            write_frame(stdout, &publish).map_err(|_| "event transport closed".to_owned())?;
            Ok(format!("observed:{}", String::from_utf8_lossy(&payload)).into_bytes())
        }
        other => Err(format!("unsupported echo operation '{other}'")),
    }
}
