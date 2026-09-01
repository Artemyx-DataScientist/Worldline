//! Supervised native process binary for browser provider external plugin boundary.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};
use worldline_native_host::{
    NativeHostError, read_frame, read_json_frame, write_frame, write_json_frame,
};
use worldline_plugin_protocol::{Envelope, MessageKind, PROTOCOL_VERSION};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CapabilityPayload {
    pub operation: String,
    pub payload: Value,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut package_id = "worldline.browser.pkg".to_string();
    let mut definition_id = "worldline.browser.provider".to_string();
    let mut _backend_kind = "reference".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--package-id" if i + 1 < args.len() => {
                package_id = args[i + 1].clone();
                i += 1;
            }
            "--definition-id" if i + 1 < args.len() => {
                definition_id = args[i + 1].clone();
                i += 1;
            }
            "--backend" if i + 1 < args.len() => {
                _backend_kind = args[i + 1].clone();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let max_frame_bytes = 4 * 1024 * 1024;

    // 1. Perform Host Handshake
    let hello: Value = match read_json_frame(&mut stdin, max_frame_bytes) {
        Ok(h) => h,
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
        "declared_interfaces": [
            "browser.context/v1",
            "browser.page/v1",
            "browser.navigate/v1",
            "browser.observe/v1",
            "browser.query/v1",
            "browser.act/v1",
            "browser.download/v1",
            "browser.permission/v1",
            "browser.capture/v0.1",
            "browser.engine.cookies/v0.1",
            "browser.engine.storage/v0.1",
        ],
    });
    if write_json_frame(&mut stdout, &ack).is_err() {
        std::process::exit(2);
    }

    // 2. Initialize Backend & Core
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    // 3. Process Request Loop
    loop {
        let envelope = match read_frame(&mut stdin, max_frame_bytes) {
            Ok(env) => env,
            Err(NativeHostError::TransportClosed) => std::process::exit(0),
            Err(_) => std::process::exit(2),
        };

        match envelope.message_kind {
            MessageKind::CapabilityRequest => {
                let call_req: Result<CapabilityPayload, _> =
                    serde_json::from_value(envelope.payload);
                let response_payload = match call_req {
                    Ok(req) => match core.dispatch(&req.operation, req.payload) {
                        Ok(res) => serde_json::json!({ "result": res }),
                        Err(err) => serde_json::json!({ "error": err.to_string() }),
                    },
                    Err(err) => serde_json::json!({ "error": format!("Invalid payload: {err}") }),
                };

                let reply = Envelope::new(
                    MessageKind::CapabilityResult,
                    envelope.correlation_id,
                    response_payload,
                );
                if write_frame(&mut stdout, &reply).is_err() {
                    std::process::exit(2);
                }
            }
            MessageKind::LifecycleRequest => {
                let reply = Envelope::new(
                    MessageKind::LifecycleResult,
                    envelope.correlation_id,
                    serde_json::json!({ "status": "ok" }),
                );
                let _ = write_frame(&mut stdout, &reply);
                let action = envelope
                    .payload
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if action == "deactivate" {
                    std::process::exit(0);
                }
            }
            _ => {
                // Ignore other messages
            }
        }
    }
}
