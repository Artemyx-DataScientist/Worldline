//! Supervised native process for the browser provider boundary.
//!
//! The reference backend remains available for deterministic protocol and
//! provider tests. Production proof must opt into `--backend cef`; that path
//! performs CEF subprocess dispatch before the Worldline handshake and never
//! silently substitutes the reference backend.

#[cfg(windows)]
use std::ffi::{CStr, c_char, c_int, c_void};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use worldline_browser_contract::identity::{BrowserContextId, DownloadId, PageId};
use worldline_browser_provider::{BrowserBackend, BrowserProviderCore, ReferenceBrowserBackend};
use worldline_native_host::{
    NativeHostError, read_frame, read_json_frame, write_frame, write_json_frame,
};
use worldline_plugin_protocol::{BlobAction, BlobRequest, Envelope, MessageKind, PROTOCOL_VERSION};

#[cfg(windows)]
use worldline_browser_cef::CefBrowserBackend;
#[cfg(windows)]
use worldline_browser_cef::early_subprocess_dispatch;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CapabilityPayload {
    pub operation: String,
    pub payload: Value,
}

#[derive(Clone, Debug)]
enum NativeDownloadEvent {
    Started {
        download_id: DownloadId,
        context_id: BrowserContextId,
        page_id: PageId,
        url: String,
        suggested_filename: String,
        total_bytes: Option<u64>,
        mime_type: Option<String>,
    },
    Progress {
        download_id: DownloadId,
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    Completed {
        download_id: DownloadId,
        content: Vec<u8>,
        mime_type: Option<String>,
    },
    Failed {
        download_id: DownloadId,
        error: String,
    },
}

trait DownloadEventSource {
    fn drain_download_events(&self) -> Vec<NativeDownloadEvent>;
}

impl DownloadEventSource for ReferenceBrowserBackend {
    fn drain_download_events(&self) -> Vec<NativeDownloadEvent> {
        Vec::new()
    }
}

#[cfg(windows)]
impl DownloadEventSource for CefBrowserBackend {
    fn drain_download_events(&self) -> Vec<NativeDownloadEvent> {
        CefBrowserBackend::drain_download_events(self)
            .into_iter()
            .map(|event| match event {
                worldline_browser_cef::backend::CefDownloadEvent::Started {
                    download_id,
                    context_id,
                    page_id,
                    url,
                    suggested_filename,
                    total_bytes,
                    mime_type,
                } => NativeDownloadEvent::Started {
                    download_id,
                    context_id,
                    page_id,
                    url,
                    suggested_filename,
                    total_bytes,
                    mime_type,
                },
                worldline_browser_cef::backend::CefDownloadEvent::Progress {
                    download_id,
                    received_bytes,
                    total_bytes,
                } => NativeDownloadEvent::Progress {
                    download_id,
                    received_bytes,
                    total_bytes,
                },
                worldline_browser_cef::backend::CefDownloadEvent::Completed {
                    download_id,
                    content,
                    mime_type,
                } => NativeDownloadEvent::Completed {
                    download_id,
                    content,
                    mime_type,
                },
                worldline_browser_cef::backend::CefDownloadEvent::Failed { download_id, error } => {
                    NativeDownloadEvent::Failed { download_id, error }
                }
            })
            .collect()
    }
}

/// Runs the provider entrypoint and returns the process status expected by the
/// native executable or CEF bootstrap client.
pub fn run_main(args: Vec<String>, sandbox_info: usize) -> i32 {
    #[cfg(windows)]
    if args.iter().any(|argument| argument.starts_with("--type=")) {
        // CEF child command lines do not carry Worldline's `--backend cef`
        // selector. Dispatch them before parsing provider arguments so they
        // can never fall through into the reference backend or host IPC.
        return early_subprocess_dispatch(sandbox_info).unwrap_or(3);
    }

    let mut package_id = "worldline.browser.pkg".to_string();
    let mut definition_id = "worldline.browser.provider".to_string();
    let mut backend_kind = "reference".to_string();
    let mut cache_root = None;

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
                backend_kind = args[i + 1].clone();
                i += 1;
            }
            "--cache-root" if i + 1 < args.len() => {
                cache_root = Some(std::path::PathBuf::from(&args[i + 1]));
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    if backend_kind == "cef"
        && args.iter().any(|argument| {
            matches!(
                argument.as_str(),
                "--no-sandbox" | "--disable-sandbox" | "--disable-gpu-sandbox"
            )
        })
    {
        eprintln!("CEF sandbox-disabling command-line arguments are forbidden");
        return 64;
    }

    match backend_kind.as_str() {
        "reference" => run_provider(package_id, definition_id, ReferenceBrowserBackend::new()),
        "cef" => {
            #[cfg(windows)]
            {
                // CEF child processes must terminate here and must never
                // enter the Worldline handshake or receive host authority.
                if let Some(exit_code) = early_subprocess_dispatch(sandbox_info) {
                    return exit_code;
                }
                let backend = cache_root
                    .map(|root| {
                        CefBrowserBackend::new_with_cache_root_and_sandbox(root, sandbox_info)
                    })
                    .unwrap_or_else(|| CefBrowserBackend::new_with_sandbox(sandbox_info));
                run_provider(package_id, definition_id, backend)
            }
            #[cfg(not(windows))]
            {
                eprintln!("--backend cef is only supported on the hosted Windows target");
                3
            }
        }
        other => {
            eprintln!("unknown browser backend '{other}'; expected 'reference' or 'cef'");
            64
        }
    }
}

fn run_provider<B: BrowserBackend + DownloadEventSource>(
    package_id: String,
    definition_id: String,
    mut backend: B,
) -> i32 {
    if let Err(error) = backend.initialize() {
        eprintln!("browser provider backend initialization failed: {error}");
        return 3;
    }
    let core = BrowserProviderCore::new(backend);
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let max_frame_bytes = 4 * 1024 * 1024;

    let hello: Value = match read_json_frame(&mut stdin, max_frame_bytes) {
        Ok(hello) => hello,
        Err(_) => return 2,
    };
    if hello.get("protocol_version").and_then(Value::as_u64) != Some(PROTOCOL_VERSION as u64) {
        return 2;
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
            "browser.engine.cookies/v0.2",
            "browser.engine.storage/v0.1",
            "browser.engine.storage/v0.2",
        ],
    });
    if write_json_frame(&mut stdout, &ack).is_err() {
        return 2;
    }

    let mut next_event_correlation = 1_u64;
    loop {
        let envelope = match read_frame(&mut stdin, max_frame_bytes) {
            Ok(env) => env,
            Err(NativeHostError::TransportClosed) => return 0,
            Err(_) => return 2,
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
                    return 2;
                }
                if publish_download_events(&core, &mut stdout, &mut next_event_correlation).is_err()
                {
                    return 2;
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
                    let _ = core.shutdown_backend();
                    return 0;
                }
            }
            _ => {
                // Ignore other messages; the host is the only sender of
                // capability and lifecycle requests in this direction.
            }
        }
    }
}

/// CEF's Windows bootstrap invokes this symbol from the client DLL.
///
/// # Safety
///
/// The bootstrap contract supplies a valid `argv` array for the duration of
/// the call. The sandbox pointer is owned by the bootstrap and is only copied
/// as an opaque process-lifetime value into the CEF initialization path.
#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RunConsoleMain(
    argc: c_int,
    argv: *mut *mut c_char,
    sandbox_info: *mut c_void,
    _version_info: *mut c_void,
) -> c_int {
    if argc < 0 || argv.is_null() {
        return 64;
    }

    let raw_args = unsafe { std::slice::from_raw_parts(argv, argc as usize) };
    let mut args = Vec::with_capacity(raw_args.len());
    for raw_arg in raw_args {
        if raw_arg.is_null() {
            return 64;
        }
        let bytes = unsafe { CStr::from_ptr(*raw_arg).to_bytes() };
        args.push(String::from_utf8_lossy(bytes).into_owned());
    }
    run_main(args, sandbox_info as usize)
}

fn publish_download_events<B: BrowserBackend + DownloadEventSource, W: std::io::Write>(
    core: &BrowserProviderCore<B>,
    stdout: &mut W,
    next_correlation: &mut u64,
) -> Result<(), NativeHostError> {
    let events = core.with_backend(DownloadEventSource::drain_download_events);
    for event in events {
        let event_correlation = *next_correlation;
        *next_correlation = (*next_correlation).saturating_add(1);
        let payload = match event {
            NativeDownloadEvent::Started {
                download_id,
                context_id,
                page_id,
                url,
                suggested_filename,
                total_bytes,
                mime_type,
            } => serde_json::json!({
                "event": "browser.download.started",
                "download_id": download_id,
                "context_id": context_id,
                "page_id": page_id,
                "url": url,
                "suggested_filename": suggested_filename,
                "total_bytes": total_bytes,
                "mime_type": mime_type,
            }),
            NativeDownloadEvent::Progress {
                download_id,
                received_bytes,
                total_bytes,
            } => serde_json::json!({
                "event": "browser.download.progress",
                "download_id": download_id,
                "received_bytes": received_bytes,
                "total_bytes": total_bytes,
            }),
            NativeDownloadEvent::Failed { download_id, error } => serde_json::json!({
                "event": "browser.download.failed",
                "download_id": download_id,
                "error": error,
            }),
            NativeDownloadEvent::Completed {
                download_id,
                content,
                mime_type,
            } => {
                let blob_id = content_blob_id(&content);
                let blob_request = BlobRequest {
                    action: BlobAction::Put {
                        blob_id: blob_id.clone(),
                        bytes: content,
                    },
                };
                let blob_envelope = Envelope::new(
                    MessageKind::BlobRequest,
                    event_correlation,
                    serde_json::to_value(blob_request).map_err(|error| {
                        NativeHostError::ProtocolViolation {
                            reason: format!("encode download blob request: {error}"),
                        }
                    })?,
                );
                write_frame(stdout, &blob_envelope)?;
                serde_json::json!({
                    "event": "browser.download.completed",
                    "download_id": download_id,
                    "blob_id": blob_id,
                    "mime_type": mime_type,
                })
            }
        };
        let event_envelope =
            Envelope::new(MessageKind::EventPublishRequest, event_correlation, payload);
        write_frame(stdout, &event_envelope)?;
    }
    Ok(())
}

fn content_blob_id(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut encoded = String::with_capacity("sha256-v1-".len() + digest.len() * 2);
    encoded.push_str("sha256-v1-");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}
