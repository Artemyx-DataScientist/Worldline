//! S3D diagnostics proving slice.
//!
//! S3D proves that Worldline's engine-neutral diagnostic boundary captures
//! console messages and network/resource observations into bounded, ephemeral
//! buffers, produces accurate page runtime snapshots, maintains strict context
//! and page isolation, enforces drop counters upon buffer overflow, and cleans
//! up upon page/context lifecycle closure.

use worldline_browser_contract::contracts::LoadingState;
use worldline_browser_contract::identity::{BrowserContextId, DocumentRevision, PageId};
use worldline_browser_contract::request_policy::RequestResourceType;
use worldline_browser_devtools::{BrowserDevToolsService, ConsoleLogLevel, NetworkRequestStatus};
use worldline_browser_services_contract::{
    GetRuntimeSnapshotRequest, PageRuntimeDiagnosticSnapshot, QueryConsoleRecordsRequest,
    QueryNetworkRecordsRequest,
};

/// Report for the S3D deterministic or hosted proving slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3DReport {
    pub topology: String,
    pub console_log_captured: bool,
    pub console_warn_captured: bool,
    pub console_error_captured: bool,
    pub network_ok_captured: bool,
    pub network_404_captured: bool,
    pub runtime_snapshot_valid: bool,
    pub context_isolation_enforced: bool,
    pub overflow_drops_counted: bool,
    pub lifecycle_cleanup_verified: bool,
    pub accepted: bool,
}

/// Executes the deterministic reference S3D proving slice.
pub fn run_reference() -> Result<S3DReport, String> {
    let service = BrowserDevToolsService::new(10);
    let ctx_a = BrowserContextId::new("ctx-ref-a");
    let page_a = PageId::new("page-ref-a");
    let rev_a = DocumentRevision::initial();

    let now_ms = 1_700_000_000_000u64;

    // 1. Record console observations
    service.record_console(
        ctx_a.clone(),
        page_a.clone(),
        rev_a,
        ConsoleLogLevel::Info,
        "diagnostic-log-ref",
        Some("http://127.0.0.1/index.html"),
        Some(10),
        now_ms,
    );
    service.record_console(
        ctx_a.clone(),
        page_a.clone(),
        rev_a,
        ConsoleLogLevel::Warning,
        "diagnostic-warn-ref",
        Some("http://127.0.0.1/index.html"),
        Some(11),
        now_ms + 1,
    );
    service.record_console(
        ctx_a.clone(),
        page_a.clone(),
        rev_a,
        ConsoleLogLevel::Error,
        "diagnostic-error-ref",
        Some("http://127.0.0.1/index.html"),
        Some(12),
        now_ms + 2,
    );

    // 2. Record network observations
    service.record_network(
        ctx_a.clone(),
        page_a.clone(),
        rev_a,
        "req-1",
        "GET",
        RequestResourceType::Script,
        "http://127.0.0.1/asset.js",
        NetworkRequestStatus::Completed,
        Some(200),
        Some("application/javascript".to_string()),
        Some(128),
        Some(15),
        now_ms + 5,
    );
    service.record_network(
        ctx_a.clone(),
        page_a.clone(),
        rev_a,
        "req-2",
        "GET",
        RequestResourceType::Stylesheet,
        "http://127.0.0.1/missing.css",
        NetworkRequestStatus::Failed,
        Some(404),
        Some("text/plain".to_string()),
        Some(32),
        Some(10),
        now_ms + 8,
    );

    // 3. Update runtime snapshot
    service.update_runtime_snapshot(PageRuntimeDiagnosticSnapshot {
        context_id: ctx_a.clone(),
        page_id: page_a.clone(),
        document_revision: rev_a,
        url: "http://127.0.0.1/index.html".to_string(),
        title: "__worldline_s3d_ready_ref".to_string(),
        loading_state: LoadingState::Complete,
        status_code: 200,
        crashed: false,
        timestamp_epoch_ms: now_ms + 10,
    });

    // Verify console queries
    let console_res = service
        .query_console(&QueryConsoleRecordsRequest {
            context_id: ctx_a.clone(),
            page_id: page_a.clone(),
            document_revision: Some(rev_a),
            min_level: None,
            limit: None,
            since_record_id: None,
        })
        .map_err(|e| e.to_string())?;

    let console_log_captured = console_res
        .records
        .iter()
        .any(|r| r.level == ConsoleLogLevel::Info && r.message == "diagnostic-log-ref");
    let console_warn_captured = console_res
        .records
        .iter()
        .any(|r| r.level == ConsoleLogLevel::Warning && r.message == "diagnostic-warn-ref");
    let console_error_captured = console_res
        .records
        .iter()
        .any(|r| r.level == ConsoleLogLevel::Error && r.message == "diagnostic-error-ref");

    // Verify network queries
    let network_res = service
        .query_network(&QueryNetworkRecordsRequest {
            context_id: ctx_a.clone(),
            page_id: page_a.clone(),
            document_revision: Some(rev_a),
            resource_type: None,
            status: None,
            limit: None,
            since_record_id: None,
        })
        .map_err(|e| e.to_string())?;

    let network_ok_captured = network_res.records.iter().any(|r| {
        r.status == NetworkRequestStatus::Completed
            && r.http_status == Some(200)
            && r.url.contains("asset.js")
    });
    let network_404_captured = network_res.records.iter().any(|r| {
        r.status == NetworkRequestStatus::Failed
            && r.http_status == Some(404)
            && r.url.contains("missing.css")
    });

    // Verify runtime snapshot
    let snapshot_res = service
        .get_runtime_snapshot(&GetRuntimeSnapshotRequest {
            context_id: ctx_a.clone(),
            page_id: page_a.clone(),
        })
        .map_err(|e| e.to_string())?;
    let runtime_snapshot_valid = snapshot_res.snapshot.url == "http://127.0.0.1/index.html"
        && snapshot_res.snapshot.title == "__worldline_s3d_ready_ref"
        && snapshot_res.snapshot.loading_state == LoadingState::Complete
        && snapshot_res.snapshot.status_code == 200;

    // Verify context isolation
    let ctx_b = BrowserContextId::new("ctx-ref-b");
    let cross_ctx_res = service
        .query_console(&QueryConsoleRecordsRequest {
            context_id: ctx_b.clone(),
            page_id: page_a.clone(),
            document_revision: None,
            min_level: None,
            limit: None,
            since_record_id: None,
        })
        .map_err(|e| e.to_string())?;
    let context_isolation_enforced = cross_ctx_res.records.is_empty();

    // Verify overflow drops counted:
    // Capacity is 10. We have 3 console records. Ingest 12 more -> total 15, capacity 10 -> 5 dropped.
    for i in 0..12 {
        service.record_console(
            ctx_a.clone(),
            page_a.clone(),
            rev_a,
            ConsoleLogLevel::Info,
            &format!("overflow-msg-{i}"),
            None,
            None,
            now_ms + 100 + i as u64,
        );
    }
    let overflow_res = service
        .query_console(&QueryConsoleRecordsRequest {
            context_id: ctx_a.clone(),
            page_id: page_a.clone(),
            document_revision: None,
            min_level: None,
            limit: None,
            since_record_id: None,
        })
        .map_err(|e| e.to_string())?;
    let overflow_drops_counted = overflow_res.stats.dropped_console_records > 0
        && overflow_res.stats.retained_console_records == 10;

    // Verify lifecycle cleanup
    service.close_page(&ctx_a, &page_a);
    let post_cleanup = service
        .query_console(&QueryConsoleRecordsRequest {
            context_id: ctx_a.clone(),
            page_id: page_a.clone(),
            document_revision: None,
            min_level: None,
            limit: None,
            since_record_id: None,
        })
        .map_err(|e| e.to_string())?;
    let lifecycle_cleanup_verified =
        post_cleanup.records.is_empty() && post_cleanup.stats.retained_console_records == 0;

    let accepted = console_log_captured
        && console_warn_captured
        && console_error_captured
        && network_ok_captured
        && network_404_captured
        && runtime_snapshot_valid
        && context_isolation_enforced
        && overflow_drops_counted
        && lifecycle_cleanup_verified;

    Ok(S3DReport {
        topology: "reference".to_string(),
        console_log_captured,
        console_warn_captured,
        console_error_captured,
        network_ok_captured,
        network_404_captured,
        runtime_snapshot_valid,
        context_isolation_enforced,
        overflow_drops_counted,
        lifecycle_cleanup_verified,
        accepted,
    })
}

#[cfg(windows)]
pub fn run() -> Result<S3DReport, String> {
    real::run()
}

#[cfg(not(windows))]
pub fn run() -> Result<S3DReport, String> {
    Err("S3D-real requires the hosted Windows CEF runtime".to_string())
}

#[cfg(windows)]
mod real {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use worldline_browser_contract::authority::{OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_OBSERVE};
    use worldline_browser_contract::contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        LoadingState, ObservePageRequest, PageObservation,
    };
    use worldline_browser_contract::identity::{BrowserContextId, DocumentRevision, PageId};
    use worldline_browser_contract::request_policy::RequestResourceType;
    use worldline_browser_devtools::{
        BrowserDevToolsService, ConsoleLogLevel, NetworkRequestStatus,
    };
    use worldline_browser_services_contract::{
        GetRuntimeSnapshotRequest, PageRuntimeDiagnosticSnapshot, QueryConsoleRecordsRequest,
        QueryNetworkRecordsRequest,
    };
    use worldline_native_host::{
        ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError,
        NativeProviderConnection,
    };
    use worldline_plugin_protocol::MessageKind;

    use super::S3DReport;
    use crate::real_cef_lock::RealCefRunGuard;

    const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
    const MAX_DEVTOOLS_IN_FLIGHT: usize = 8;

    struct S3DHostSink {
        service: Arc<BrowserDevToolsService>,
    }

    impl HostRequestSink for S3DHostSink {
        fn on_child_request(
            &self,
            kind: MessageKind,
            _correlation_id: u64,
            payload: Value,
        ) -> Result<Option<Value>, NativeHostError> {
            if kind == MessageKind::EventPublishRequest {
                let event = payload
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match event {
                    "browser.diagnostics.console" => {
                        let context_id = payload
                            .get("context_id")
                            .and_then(Value::as_str)
                            .map(BrowserContextId::new)
                            .unwrap_or_else(|| BrowserContextId::new("unknown"));
                        let page_id = payload
                            .get("page_id")
                            .and_then(Value::as_str)
                            .map(PageId::new)
                            .unwrap_or_else(|| PageId::new("unknown"));
                        let rev = payload
                            .get("document_revision")
                            .and_then(Value::as_u64)
                            .map(DocumentRevision::new)
                            .unwrap_or_else(DocumentRevision::initial);
                        let level_str = payload
                            .get("level")
                            .and_then(Value::as_str)
                            .unwrap_or("Info");
                        let level = match level_str.to_lowercase().as_str() {
                            "debug" => ConsoleLogLevel::Debug,
                            "warning" | "warn" => ConsoleLogLevel::Warning,
                            "error" => ConsoleLogLevel::Error,
                            _ => ConsoleLogLevel::Info,
                        };
                        let message = payload
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let source = payload.get("source").and_then(Value::as_str);
                        let line = payload
                            .get("line")
                            .and_then(Value::as_u64)
                            .map(|l| l as u32);
                        let timestamp = payload
                            .get("timestamp_epoch_ms")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);

                        self.service.record_console(
                            context_id, page_id, rev, level, message, source, line, timestamp,
                        );
                    }
                    "browser.diagnostics.network" => {
                        let context_id = payload
                            .get("context_id")
                            .and_then(Value::as_str)
                            .map(BrowserContextId::new)
                            .unwrap_or_else(|| BrowserContextId::new("unknown"));
                        let page_id = payload
                            .get("page_id")
                            .and_then(Value::as_str)
                            .map(PageId::new)
                            .unwrap_or_else(|| PageId::new("unknown"));
                        let rev = payload
                            .get("document_revision")
                            .and_then(Value::as_u64)
                            .map(DocumentRevision::new)
                            .unwrap_or_else(DocumentRevision::initial);
                        let request_id = payload
                            .get("request_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let method = payload
                            .get("method")
                            .and_then(Value::as_str)
                            .unwrap_or("GET");
                        let resource_type = payload
                            .get("resource_type")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or(RequestResourceType::Other);
                        let url = payload
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let status_str = payload
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("Completed");
                        let status = match status_str.to_lowercase().as_str() {
                            "failed" => NetworkRequestStatus::Failed,
                            "blocked" => NetworkRequestStatus::Blocked,
                            _ => NetworkRequestStatus::Completed,
                        };
                        let http_status = payload
                            .get("http_status")
                            .and_then(Value::as_u64)
                            .map(|s| s as u16);
                        let mime_type = payload
                            .get("mime_type")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let received_bytes = payload.get("received_bytes").and_then(Value::as_u64);
                        let duration_ms = payload.get("duration_ms").and_then(Value::as_u64);
                        let timestamp = payload
                            .get("timestamp_epoch_ms")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);

                        self.service.record_network(
                            context_id,
                            page_id,
                            rev,
                            request_id,
                            method,
                            resource_type,
                            url,
                            status,
                            http_status,
                            mime_type,
                            received_bytes,
                            duration_ms,
                            timestamp,
                        );
                    }
                    _ => {}
                }
            }
            Ok(None)
        }
    }

    struct LoopbackServer {
        base_url: String,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
        _hits: Arc<Mutex<Vec<String>>>,
    }

    impl LoopbackServer {
        fn start(nonce: &str) -> Result<Self, String> {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .map_err(|e| format!("bind S3D loopback server: {e}"))?;
            listener
                .set_nonblocking(true)
                .map_err(|e| format!("configure S3D loopback server: {e}"))?;
            let address = listener
                .local_addr()
                .map_err(|e| format!("read S3D loopback address: {e}"))?;
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let connections = Arc::new(Mutex::new(Vec::new()));
            let worker_connections = Arc::clone(&connections);
            let hits = Arc::new(Mutex::new(Vec::new()));
            let worker_hits = Arc::clone(&hits);
            let index_body = Arc::new(index_body(nonce));
            let worker_index_body = Arc::clone(&index_body);
            let asset_nonce = nonce.to_string();

            let worker = thread::Builder::new()
                .name("worldline-s3d-loopback".to_string())
                .spawn(move || {
                    while !worker_stop.load(Ordering::SeqCst) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let body = Arc::clone(&worker_index_body);
                                let hits = Arc::clone(&worker_hits);
                                let nonce = asset_nonce.clone();
                                let connection = thread::Builder::new()
                                    .name("worldline-s3d-http".to_string())
                                    .spawn(move || serve_connection(stream, body, hits, &nonce));
                                if let Ok(connection) = connection {
                                    worker_connections
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .push(connection);
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(e)
                                if e.kind() == std::io::ErrorKind::ConnectionAborted
                                    || e.kind() == std::io::ErrorKind::ConnectionReset =>
                            {
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(e) => {
                                eprintln!("S3D loopback server accept error: {e:?}");
                                break;
                            }
                        }
                    }
                })
                .map_err(|e| format!("start S3D loopback server: {e}"))?;

            Ok(Self {
                base_url: format!("http://{address}"),
                stop,
                worker: Some(worker),
                connections,
                _hits: hits,
            })
        }
    }

    impl Drop for LoopbackServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let connections =
                std::mem::take(&mut *self.connections.lock().unwrap_or_else(|p| p.into_inner()));
            for connection in connections {
                let _ = connection.join();
            }
        }
    }

    fn index_body(nonce: &str) -> String {
        format!(
            "<!DOCTYPE html>\n\
            <html>\n\
            <head>\n\
              <meta charset=\"utf-8\">\n\
              <title>__worldline_s3d_init_{nonce}</title>\n\
              <link rel=\"stylesheet\" href=\"/missing_{nonce}.css\">\n\
            </head>\n\
            <body>\n\
              <h1>Worldline S3D Diagnostics Proving Page</h1>\n\
              <script src=\"/asset_{nonce}.js\"></script>\n\
              <script>\n\
                console.log(\"diagnostic-log-{nonce}\");\n\
                console.warn(\"diagnostic-warn-{nonce}\");\n\
                console.error(\"diagnostic-error-{nonce}\");\n\
                document.title = \"__worldline_s3d_ready_{nonce}\";\n\
              </script>\n\
            </body>\n\
            </html>"
        )
    }

    fn serve_connection(
        mut stream: TcpStream,
        index_body: Arc<String>,
        hits: Arc<Mutex<Vec<String>>>,
        nonce: &str,
    ) {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let mut buffer = [0u8; 4096];
        let mut request = Vec::new();
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(ref error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(_) => return,
            }
        }

        let request_text = String::from_utf8_lossy(&request);
        let path = request_text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");

        let path_only = path.split('?').next().unwrap_or(path);
        hits.lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(path_only.to_string());

        let asset_path = format!("/asset_{nonce}.js");

        let response = if path_only == "/" || path_only == "/index.html" {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                index_body.len(),
                *index_body
            )
        } else if path_only == asset_path {
            let body = "console.log(\"asset-loaded\");\n";
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        } else {
            let body = "Not Found\n";
            format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        };

        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn create() -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!(
                "worldline-s3d-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path)
                .map_err(|e| format!("create S3D temp root '{}': {e}", path.display()))?;
            Ok(Self { path })
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|e| format!("decode S3D message: {e}"))
    }

    fn call_contract_op(
        connection: &NativeProviderConnection,
        contract: &str,
        operation: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let request = serde_json::json!({
            "contract": contract,
            "operation": operation,
            "payload": payload,
        });
        let response = connection
            .call_with_deadline(request, Duration::from_secs(10))
            .map_err(|e| {
                format!(
                    "S3D provider operation '{contract}.{operation}' transport failed: {e}\nstderr:\n{}",
                    connection.stderr_text()
                )
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!("S3D operation '{operation}' failed: {error}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("S3D operation '{operation}' omitted result"))
    }

    fn call_op(
        connection: &NativeProviderConnection,
        operation: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let response = connection
            .call_with_deadline(
                serde_json::json!({ "operation": operation, "payload": payload }),
                Duration::from_secs(10),
            )
            .map_err(|e| {
                format!(
                    "S3D native call '{operation}' failed: {e}\nstderr:\n{}",
                    connection.stderr_text()
                )
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!("S3D operation '{operation}' failed: {error}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("S3D operation '{operation}' omitted result"))
    }

    fn discover_provider_process() -> Result<PathBuf, String> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("WORLDLINE_BROWSER_PROVIDER_BOOTSTRAP") {
            candidates.push(PathBuf::from(path));
        }
        if let Ok(current_exe) = std::env::current_exe()
            && let Some(deps) = current_exe.parent()
            && let Some(target_debug) = deps.parent()
        {
            candidates.push(target_debug.join("bootstrapc.exe"));
        }
        if let Some(target_dir) = std::env::var_os("CARGO_TARGET_DIR") {
            candidates.push(
                PathBuf::from(target_dir)
                    .join("debug")
                    .join("bootstrapc.exe"),
            );
        }
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
                .join("debug")
                .join("bootstrapc.exe"),
        );
        candidates.into_iter().find(|c| c.is_file()).ok_or_else(|| {
            "pinned CEF bootstrapc.exe is missing; stage CEF before S3D-real".to_string()
        })
    }

    fn discover_provider_client(bootstrap: &Path) -> Result<PathBuf, String> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("WORLDLINE_BROWSER_PROVIDER_CLIENT") {
            candidates.push(PathBuf::from(path));
        }
        if let Some(parent) = bootstrap.parent() {
            candidates.push(parent.join("worldline_browser_provider_client.dll"));
        }
        let client = candidates
            .into_iter()
            .find(|c| c.is_file())
            .ok_or_else(|| "worldline_browser_provider_client.dll is missing".to_string())?;
        Ok(client)
    }

    pub fn run() -> Result<S3DReport, String> {
        let _real_cef_guard = RealCefRunGuard::acquire()?;
        let program = discover_provider_process()?;
        let client = discover_provider_client(&program)?;
        let temp_root = TempRoot::create()?;
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        );
        let server = LoopbackServer::start(&nonce)?;

        let service = Arc::new(BrowserDevToolsService::new(500));
        let sink = Arc::new(S3DHostSink {
            service: Arc::clone(&service),
        });

        let identity = ExpectedIdentity {
            package_id: "worldline.browser.pkg".to_string(),
            plugin_definition_id: "worldline.browser.provider".to_string(),
        };
        let client_name = client
            .file_name()
            .ok_or_else(|| format!("CEF client has no file name: {}", client.display()))?;

        let cache_dir = temp_root.path().join("s3d-cef-profile");
        let child_args = vec![
            format!("--module={}", client_name.to_string_lossy()),
            "--disable-gpu".to_string(),
            "--in-process-gpu".to_string(),
            "--package-id".to_string(),
            identity.package_id.clone(),
            "--definition-id".to_string(),
            identity.plugin_definition_id.clone(),
            "--backend".to_string(),
            "cef".to_string(),
            "--cache-root".to_string(),
            cache_dir.to_string_lossy().into_owned(),
        ];

        let spec = NativeChildSpec::new(program, child_args, MAX_FRAME_BYTES, 64 * 1024);

        let (connection, _ack) = NativeProviderConnection::connect(
            spec,
            &identity,
            Arc::clone(&sink) as Arc<dyn HostRequestSink>,
            MAX_DEVTOOLS_IN_FLIGHT,
        )
        .map_err(|e| format!("connect S3D native provider: {e}"))?;

        let context: CreateContextResponse = decode(call_contract_op(
            &connection,
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some(format!("s3d-ctx-{nonce}")),
                incognito: true,
                user_agent: None,
            })
            .map_err(|e| e.to_string())?,
        )?)?;

        let initial_url = format!("{}/index.html?run={nonce}", server.base_url);
        let page: CreatePageResponse = decode(call_contract_op(
            &connection,
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: context.context_id.clone(),
                initial_url: Some(initial_url.clone()),
            })
            .map_err(|e| e.to_string())?,
        )?)?;

        // Wait for page to finish loading and title to change to ready marker
        let target_title = format!("__worldline_s3d_ready_{nonce}");
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut last_obs: Option<PageObservation> = None;
        while Instant::now() < deadline {
            let obs: PageObservation = decode(call_op(
                &connection,
                OP_OBSERVE,
                serde_json::to_value(ObservePageRequest {
                    page_id: page.page_id.clone(),
                })
                .map_err(|e| e.to_string())?,
            )?)?;
            if obs.title == target_title {
                last_obs = Some(obs);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        let obs = last_obs.ok_or_else(|| {
            format!(
                "timed out waiting for S3D page readiness '{target_title}'; stderr:\n{}",
                connection.stderr_text()
            )
        })?;

        // Ingest the runtime snapshot into service
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        service.update_runtime_snapshot(PageRuntimeDiagnosticSnapshot {
            context_id: context.context_id.clone(),
            page_id: page.page_id.clone(),
            document_revision: obs.document_revision,
            url: obs.url.clone(),
            title: obs.title.clone(),
            loading_state: obs.loading_state,
            status_code: obs.status_code,
            crashed: false,
            timestamp_epoch_ms: now_ms,
        });

        // Small pause to ensure events in flight across IPC are received
        thread::sleep(Duration::from_millis(200));

        // Query console records
        let console_res = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: context.context_id.clone(),
                page_id: page.page_id.clone(),
                document_revision: None,
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .map_err(|e| e.to_string())?;

        let console_log_captured = console_res.records.iter().any(|r| {
            r.level == ConsoleLogLevel::Info
                && r.message.contains(&format!("diagnostic-log-{nonce}"))
        });
        let console_warn_captured = console_res.records.iter().any(|r| {
            r.level == ConsoleLogLevel::Warning
                && r.message.contains(&format!("diagnostic-warn-{nonce}"))
        });
        let console_error_captured = console_res.records.iter().any(|r| {
            r.level == ConsoleLogLevel::Error
                && r.message.contains(&format!("diagnostic-error-{nonce}"))
        });

        // Query network records
        let network_res = service
            .query_network(&QueryNetworkRecordsRequest {
                context_id: context.context_id.clone(),
                page_id: page.page_id.clone(),
                document_revision: None,
                resource_type: None,
                status: None,
                limit: None,
                since_record_id: None,
            })
            .map_err(|e| e.to_string())?;

        let network_ok_captured = network_res.records.iter().any(|r| {
            r.status == NetworkRequestStatus::Completed
                && (r.http_status == Some(200) || r.http_status.is_none())
                && r.url.contains(&format!("asset_{nonce}.js"))
        });
        let network_404_captured = network_res.records.iter().any(|r| {
            (r.http_status == Some(404) || r.status == NetworkRequestStatus::Failed)
                && r.url.contains(&format!("missing_{nonce}.css"))
        });

        // Query runtime snapshot
        let snapshot_res = service
            .get_runtime_snapshot(&GetRuntimeSnapshotRequest {
                context_id: context.context_id.clone(),
                page_id: page.page_id.clone(),
            })
            .map_err(|e| e.to_string())?;

        let runtime_snapshot_valid = snapshot_res.snapshot.title == target_title
            && snapshot_res.snapshot.loading_state == LoadingState::Complete;

        // Context isolation: querying another context returns empty
        let other_ctx = BrowserContextId::new(format!("other-ctx-{nonce}"));
        let cross_ctx_res = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: other_ctx,
                page_id: page.page_id.clone(),
                document_revision: None,
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .map_err(|e| e.to_string())?;
        let context_isolation_enforced = cross_ctx_res.records.is_empty();

        // Overflow drop counting:
        // Service was created with capacity 500. Push 600 records.
        for i in 0..600 {
            service.record_console(
                context.context_id.clone(),
                page.page_id.clone(),
                obs.document_revision,
                ConsoleLogLevel::Debug,
                &format!("overflow-drop-check-{i}"),
                None,
                None,
                0,
            );
        }
        let overflow_res = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: context.context_id.clone(),
                page_id: page.page_id.clone(),
                document_revision: None,
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .map_err(|e| e.to_string())?;
        let overflow_drops_counted = overflow_res.stats.dropped_console_records > 0
            && overflow_res.stats.retained_console_records == 500;

        // Lifecycle cleanup:
        service.close_page(&context.context_id, &page.page_id);
        let post_cleanup = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: context.context_id.clone(),
                page_id: page.page_id.clone(),
                document_revision: None,
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .map_err(|e| e.to_string())?;
        let lifecycle_cleanup_verified =
            post_cleanup.records.is_empty() && post_cleanup.stats.retained_console_records == 0;

        // Cleanup provider
        let _ = connection.close(Duration::from_secs(10));

        let accepted = console_log_captured
            && console_warn_captured
            && console_error_captured
            && network_ok_captured
            && network_404_captured
            && runtime_snapshot_valid
            && context_isolation_enforced
            && overflow_drops_counted
            && lifecycle_cleanup_verified;

        Ok(S3DReport {
            topology: "cef_hosted".to_string(),
            console_log_captured,
            console_warn_captured,
            console_error_captured,
            network_ok_captured,
            network_404_captured,
            runtime_snapshot_valid,
            context_isolation_enforced,
            overflow_drops_counted,
            lifecycle_cleanup_verified,
            accepted,
        })
    }
}
