//! S3B: live browser services proving slice (Downloads and Cookies/Site-Data).
//!
//! The production proving path is intentionally an out-of-process Windows
//! run. It starts `worldline-browser-provider-process --backend cef`, talks to
//! it through the native framed IPC boundary, and admits provider-originated
//! blobs and events through a host sink. The deterministic reference backend
//! is covered by lower-level unit fixtures; it is not substituted here.

/// Proving report emitted by slice S3B.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3BReport {
    pub download_record_id: String,
    pub artifact_id: String,
    pub artifact_bytes_verified: bool,
    pub download_survived_restart: bool,
    pub metadata_only_isolation_ok: bool,
    pub cross_context_cookies_isolated: bool,
    pub cookies_survived_restart: bool,
    pub site_data_clear_isolated: bool,
    pub service_failure_isolation_ok: bool,
}

/// Executes the deterministic reference fixture for lower-level service
/// coverage. This path is intentionally separate from [`run`]: it is not
/// production CEF evidence and must not be confused with the hosted native
/// proving slice.
pub fn run_reference() -> Result<S3BReport, String> {
    reference::run()
}

mod reference {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use worldline_browser_contract::{
        authority::*,
        contracts::{
            CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
            NavigateRequest, NavigateResponse, ObservePageRequest, PageObservation,
        },
        identity::{DownloadId, NavigationId},
        primitives::StorageType,
    };
    use worldline_browser_cookies::{CookieEngineBackend, CookiesService, InMemoryCookieEngine};
    use worldline_browser_downloads::{
        AUTH_BLOB_READ, ArtifactStore, BlobReadBroker, DownloadsService, EngineDownloadStarted,
    };
    use worldline_browser_history::HistoryService;
    use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};
    use worldline_browser_services_contract::{
        ClearSiteDataRequest, CreateTabRequest, DownloadLifecycleStatus, GetCookieMetadataRequest,
        GetCookieValueRequest, GetDownloadRecordRequest, SetCookieServiceRequest,
        StartDownloadRequest,
    };
    use worldline_browser_tabs::TabsService;

    use super::S3BReport;

    const DETERMINISTIC_BYTES: &[u8] = b"WORLDLINE_DETERMINISTIC_DOWNLOAD_FIXTURE_BYTES_S3B";

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn create() -> Result<Self, String> {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("clock before Unix epoch: {error}"))?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "worldline-reference-s3b-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path)
                .map_err(|error| format!("create reference S3B root: {error}"))?;
            Ok(Self(path))
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn call<Req, Res>(
        core: &BrowserProviderCore<ReferenceBrowserBackend>,
        operation: &str,
        request: Req,
    ) -> Result<Res, String>
    where
        Req: Serialize,
        Res: DeserializeOwned,
    {
        let payload = serde_json::to_value(request).map_err(|error| error.to_string())?;
        let value = core
            .dispatch(operation, payload)
            .map_err(|error| error.to_string())?;
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    pub fn run() -> Result<S3BReport, String> {
        let temp_root = TempRoot::create()?;
        let core = BrowserProviderCore::new(ReferenceBrowserBackend::new());
        let ctx_a: CreateContextResponse = call(
            &core,
            OP_CREATE_CONTEXT,
            CreateContextRequest {
                profile_id: Some("s3b-reference-a".to_string()),
                incognito: false,
                user_agent: Some("Worldline-S3B-Reference/1.0".to_string()),
            },
        )?;
        let ctx_b: CreateContextResponse = call(
            &core,
            OP_CREATE_CONTEXT,
            CreateContextRequest {
                profile_id: Some("s3b-reference-b".to_string()),
                incognito: false,
                user_agent: Some("Worldline-S3B-Reference/1.0".to_string()),
            },
        )?;
        let page: CreatePageResponse = call(
            &core,
            OP_CREATE_PAGE,
            CreatePageRequest {
                context_id: ctx_a.context_id.clone(),
                initial_url: Some("http://127.0.0.1:8080/index.html".to_string()),
            },
        )?;

        let artifact_store = Arc::new(ArtifactStore::open(temp_root.path().join("blobs"))?);
        let downloads_service = DownloadsService::new(
            Arc::clone(&artifact_store),
            temp_root.path().join("staging"),
        );
        let download_url = "http://127.0.0.1:8080/package-v1.tar.gz".to_string();
        let start = downloads_service.start_download(StartDownloadRequest {
            context_id: ctx_a.context_id.clone(),
            page_id: Some(page.page_id.clone()),
            url: download_url.clone(),
            suggested_filename: Some("package-v1.tar.gz".to_string()),
        });
        let engine_download_id = DownloadId::new("engine-s3b-reference-1");
        downloads_service.on_engine_download_started(EngineDownloadStarted {
            engine_download_id: engine_download_id.clone(),
            context_id: ctx_a.context_id.clone(),
            page_id: page.page_id.clone(),
            url: download_url,
            suggested_filename: "package-v1.tar.gz".to_string(),
            total_bytes: Some(DETERMINISTIC_BYTES.len() as u64),
            media_type: Some("application/gzip".to_string()),
        });
        downloads_service.on_engine_download_completed(
            &engine_download_id,
            DETERMINISTIC_BYTES,
            Some("application/gzip".to_string()),
        );
        let record = downloads_service
            .get_download_record(GetDownloadRecordRequest {
                record_id: start.record_id.clone(),
            })
            .record
            .ok_or_else(|| "reference S3B download record is missing".to_string())?;
        let artifact = record
            .artifact_ref
            .clone()
            .ok_or_else(|| "reference S3B artifact reference is missing".to_string())?;
        let blob_broker = BlobReadBroker::new();
        let metadata_only_isolation_ok = blob_broker
            .issue(
                "reference-metadata-reader",
                "browser.downloads.read",
                artifact.artifact_id.clone(),
            )
            .is_err()
            && record.status == DownloadLifecycleStatus::Completed
            && record.total_bytes == Some(DETERMINISTIC_BYTES.len() as u64);
        let blob_grant = blob_broker.issue(
            "reference-blob-reader",
            AUTH_BLOB_READ,
            artifact.artifact_id.clone(),
        )?;
        let artifact_bytes_verified = artifact_store
            .read_bytes_with_authority(&artifact.artifact_id, &blob_grant)
            .map_err(|error| error.to_string())?
            == DETERMINISTIC_BYTES;
        let restarted_downloads = DownloadsService::from_snapshot(
            downloads_service.export_snapshot(),
            Arc::clone(&artifact_store),
            temp_root.path().join("staging"),
        );
        let restarted_record = restarted_downloads
            .get_download_record(GetDownloadRecordRequest {
                record_id: start.record_id,
            })
            .record
            .ok_or_else(|| "reference S3B download did not survive restart".to_string())?;
        let download_survived_restart = restarted_record.status
            == DownloadLifecycleStatus::Completed
            && restarted_record.artifact_ref.as_ref() == Some(&artifact);

        let cookie_engine = Arc::new(InMemoryCookieEngine::new());
        let cookies =
            CookiesService::new(Arc::clone(&cookie_engine) as Arc<dyn CookieEngineBackend>);
        let domain = "127.0.0.1".to_string();
        for (context_id, value) in [
            (ctx_a.context_id.clone(), "reference-context-a"),
            (ctx_b.context_id.clone(), "reference-context-b"),
        ] {
            cookies.set_cookie(SetCookieServiceRequest {
                context_id,
                name: "auth_token".to_string(),
                value: value.to_string(),
                domain: domain.clone(),
                path: Some("/".to_string()),
                secure: Some(false),
                http_only: Some(true),
                same_site: Some("Lax".to_string()),
                expires_epoch_sec: Some(1_850_000_000),
            })?;
        }
        let metadata = cookies.get_cookie_metadata(GetCookieMetadataRequest {
            context_id: ctx_a.context_id.clone(),
            url: None,
            domain: Some(domain.clone()),
        })?;
        let cookie_a = cookies
            .get_cookie_value(GetCookieValueRequest {
                context_id: ctx_a.context_id.clone(),
                domain: domain.clone(),
                name: "auth_token".to_string(),
                path: Some("/".to_string()),
                url: None,
            })?
            .cookie
            .ok_or_else(|| "reference context A cookie is missing".to_string())?;
        let cookie_b = cookies
            .get_cookie_value(GetCookieValueRequest {
                context_id: ctx_b.context_id.clone(),
                domain: domain.clone(),
                name: "auth_token".to_string(),
                path: Some("/".to_string()),
                url: None,
            })?
            .cookie
            .ok_or_else(|| "reference context B cookie is missing".to_string())?;
        let cross_context_cookies_isolated = metadata.cookies.len() == 1
            && cookie_a.expose_value() == "reference-context-a"
            && cookie_b.expose_value() == "reference-context-b";
        let restarted_cookies = CookiesService::from_policy(
            cookies.export_policy(),
            Arc::clone(&cookie_engine) as Arc<dyn CookieEngineBackend>,
        );
        let cookies_survived_restart = restarted_cookies
            .get_cookie_value(GetCookieValueRequest {
                context_id: ctx_a.context_id.clone(),
                domain: domain.clone(),
                name: "auth_token".to_string(),
                path: Some("/".to_string()),
                url: None,
            })?
            .cookie
            .is_some_and(|cookie| cookie.expose_value() == "reference-context-a");

        let origin = "http://127.0.0.1:8080".to_string();
        cookie_engine.insert_storage_item(
            &ctx_a.context_id,
            &origin,
            StorageType::LocalStorage,
            "theme".to_string(),
            "dark".to_string(),
        );
        cookie_engine.insert_storage_item(
            &ctx_b.context_id,
            &origin,
            StorageType::LocalStorage,
            "theme".to_string(),
            "light".to_string(),
        );
        let clear = restarted_cookies.clear_site_data(ClearSiteDataRequest {
            context_id: ctx_a.context_id.clone(),
            origin: origin.clone(),
            storage_type: StorageType::LocalStorage,
        })?;
        let site_data_clear_isolated = clear.cleared
            && cookie_engine
                .get_storage_item(
                    &ctx_a.context_id,
                    &origin,
                    StorageType::LocalStorage,
                    "theme",
                )
                .is_none()
            && cookie_engine
                .get_storage_item(
                    &ctx_b.context_id,
                    &origin,
                    StorageType::LocalStorage,
                    "theme",
                )
                .as_deref()
                == Some("light");

        drop(restarted_downloads);
        drop(restarted_cookies);
        drop(cookies);
        let tabs = TabsService::new();
        let tab = tabs.create_tab(CreateTabRequest {
            page_id: page.page_id.clone(),
            group_id: None,
            pinned: Some(false),
            select: Some(true),
        });
        let target_url = "http://127.0.0.1:8080/docs.html".to_string();
        let navigation: NavigateResponse = call(
            &core,
            OP_NAVIGATE,
            NavigateRequest {
                page_id: page.page_id.clone(),
                url: target_url.clone(),
            },
        )?;
        let history = HistoryService::new();
        let history_entry = history
            .record_navigation(
                page.page_id.clone(),
                NavigationId::new("reference-s3b-navigation-1"),
                navigation.document_revision,
                target_url.clone(),
                1_725_186_000_000,
            )
            .map_err(|error| error.to_string())?;
        let observation: PageObservation = call(
            &core,
            OP_OBSERVE,
            ObservePageRequest {
                page_id: page.page_id,
            },
        )?;
        let service_failure_isolation_ok = tab.tab.id.as_str().starts_with("tab-")
            && history_entry.url == target_url
            && observation.url == target_url;

        Ok(S3BReport {
            download_record_id: record.record_id.to_string(),
            artifact_id: artifact.artifact_id,
            artifact_bytes_verified,
            download_survived_restart,
            metadata_only_isolation_ok,
            cross_context_cookies_isolated,
            cookies_survived_restart,
            site_data_clear_isolated,
            service_failure_isolation_ok,
        })
    }
}

/// Executes the real native-provider proving path on its supported target.
#[cfg(windows)]
pub fn run() -> Result<S3BReport, String> {
    real::run()
}

/// S3B-real evidence is a hosted Windows gate because the production CEF
/// runtime and native subprocess boundary are Windows-only in this milestone.
#[cfg(not(windows))]
pub fn run() -> Result<S3BReport, String> {
    Err("S3B-real requires the hosted Windows CEF runtime".to_string())
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

    use serde::Deserialize;
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use worldline_browser_contract::authority::{
        OP_COOKIE_DELETE, OP_COOKIE_GET, OP_COOKIE_GET_V0_2, OP_COOKIE_SET, OP_COOKIE_SET_V0_2,
        OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_DOWNLOAD_START, OP_LIST_CONTEXTS, OP_NAVIGATE,
        OP_OBSERVE, OP_STORAGE_GET_V0_2, OP_STORAGE_SET_V0_2,
    };
    use worldline_browser_contract::contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        DownloadStatusResponse, LoadingState, NavigateRequest, NavigateResponse,
        ObservePageRequest, PageObservation, StartDownloadRequest as EngineStartDownloadRequest,
    };
    use worldline_browser_contract::identity::{
        BrowserContextId, DownloadId, NavigationId, PageId,
    };
    use worldline_browser_contract::primitives::{
        ClearStorageRequest, GetCookiesRequest, GetCookiesResponse, GetCookiesResponseV0_2,
        SetCookieRequest, SetCookieRequestV0_2, SetCookieResponse, StorageItemRequestV0_2,
        StorageItemResponseV0_2, StorageType,
    };
    use worldline_browser_cookies::{CookieEngineBackend, CookiesService};
    use worldline_browser_downloads::{
        AUTH_BLOB_READ, ArtifactStore, BlobReadBroker, DownloadsService, EngineDownloadStarted,
    };
    use worldline_browser_history::HistoryService;
    use worldline_browser_services_contract::{
        AUTH_DOWNLOADS_READ, ClearSiteDataRequest, CreateTabRequest, DownloadLifecycleStatus,
        GetCookieMetadataRequest, GetCookieValueRequest, GetDownloadRecordRequest,
        SetCookieServiceRequestV0_2, StartDownloadRequest as ServiceStartDownloadRequest,
    };
    use worldline_browser_tabs::TabsService;
    use worldline_native_host::{
        ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError,
        NativeProviderConnection,
    };
    use worldline_plugin_protocol::{BlobAction, BlobRequest, BlobResult, MessageKind};

    use crate::real_cef_lock::RealCefRunGuard;

    use super::S3BReport;

    const DOWNLOAD_BODY: &[u8] =
        b"WORLDLINE_S3B_REAL_DOWNLOAD_BODY_v1\nsha256-is-host-authoritative\n";
    const INDEX_BODY: &[u8] = br#"<!doctype html>
<html><head><meta charset="utf-8"><title>Worldline S3B</title></head>
<body><h1>Worldline S3B</h1><p>native CEF proving fixture</p></body></html>
"#;
    const DOCS_BODY: &[u8] = br#"<!doctype html>
<html><head><meta charset="utf-8"><title>Worldline S3B Docs</title></head>
<body><h1>Worldline S3B Docs</h1></body></html>
"#;

    #[derive(Debug)]
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn create(label: &str) -> Result<Self, String> {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("clock before Unix epoch: {error}"))?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "worldline-s3b-{label}-{}-{stamp}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path)
                .map_err(|error| format!("create S3B temporary root: {error}"))?;
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

    /// A loopback-only deterministic origin. No public network is involved in
    /// the real engine proof, but CEF still performs an actual HTTP load.
    struct LoopbackServer {
        base_url: String,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
    }

    impl LoopbackServer {
        fn start() -> Result<Self, String> {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .map_err(|error| format!("bind S3B loopback server: {error}"))?;
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("configure S3B loopback server: {error}"))?;
            let address = listener
                .local_addr()
                .map_err(|error| format!("read S3B loopback address: {error}"))?;
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let connections = Arc::new(Mutex::new(Vec::new()));
            let worker_connections = Arc::clone(&connections);
            let worker = thread::Builder::new()
                .name("worldline-s3b-loopback".to_string())
                .spawn(move || {
                    while !worker_stop.load(Ordering::SeqCst) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let connection = thread::Builder::new()
                                    .name("worldline-s3b-http".to_string())
                                    .spawn(move || serve_connection(stream));
                                if let Ok(connection) = connection {
                                    worker_connections
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                                        .push(connection);
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| format!("start S3B loopback server: {error}"))?;

            Ok(Self {
                base_url: format!("http://{}", address),
                stop,
                worker: Some(worker),
                connections,
            })
        }
    }

    impl Drop for LoopbackServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let connections = std::mem::take(
                &mut *self
                    .connections
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
            for connection in connections {
                let _ = connection.join();
            }
        }
    }

    fn serve_connection(mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut request = Vec::with_capacity(4096);
        let mut chunk = [0_u8; 1024];
        while request.len() < 32 * 1024 {
            let Ok(read) = stream.read(&mut chunk) else {
                return;
            };
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        if request.is_empty() {
            return;
        }
        let request = String::from_utf8_lossy(&request);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|target| target.split('?').next())
            .unwrap_or("/");
        let (status, content_type, body, disposition): (&str, &str, &[u8], Option<&str>) =
            match path {
                "/index.html" => ("200 OK", "text/html; charset=utf-8", INDEX_BODY, None),
                "/docs.html" => ("200 OK", "text/html; charset=utf-8", DOCS_BODY, None),
                "/package-v1.tar.gz" => (
                    "200 OK",
                    "application/gzip",
                    DOWNLOAD_BODY,
                    Some("attachment; filename=package-v1.tar.gz"),
                ),
                _ => ("404 Not Found", "text/plain", b"not found", None),
            };
        let disposition_header = disposition
            .map(|value| format!("Content-Disposition: {value}\r\n"))
            .unwrap_or_default();
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{disposition_header}\r\n",
            body.len()
        );
        let _ = stream.write_all(headers.as_bytes());
        let _ = stream.write_all(body);
    }

    /// Host-side sink for child-initiated generic blob puts and event
    /// publication. It deliberately does not expose a public byte-read path.
    struct RealS3BHostSink {
        artifact_store: Arc<ArtifactStore>,
        events: Mutex<Vec<Value>>,
    }

    impl RealS3BHostSink {
        fn new(artifact_store: Arc<ArtifactStore>) -> Self {
            Self {
                artifact_store,
                events: Mutex::new(Vec::new()),
            }
        }

        fn take_event(&self, event_name: &str) -> Option<Value> {
            let mut events = self
                .events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let index = events
                .iter()
                .position(|event| event.get("event").and_then(Value::as_str) == Some(event_name))?;
            Some(events.remove(index))
        }
    }

    impl HostRequestSink for RealS3BHostSink {
        fn on_child_request(
            &self,
            kind: MessageKind,
            _correlation_id: u64,
            payload: Value,
        ) -> Result<Option<Value>, NativeHostError> {
            match kind {
                MessageKind::BlobRequest => {
                    let request: BlobRequest =
                        serde_json::from_value(payload).map_err(|error| {
                            NativeHostError::ProtocolViolation {
                                reason: format!("invalid native blob request: {error}"),
                            }
                        })?;
                    let result = match request.action {
                        BlobAction::Put { blob_id, bytes } => {
                            let byte_len = bytes.len();
                            match self.artifact_store.put_blob(&blob_id, &bytes) {
                                Ok(()) => BlobResult::PutSuccess { blob_id, byte_len },
                                Err(reason) => BlobResult::Error { reason },
                            }
                        }
                        BlobAction::Verify { blob_id } => BlobResult::VerifySuccess {
                            exists: self.artifact_store.contains(&blob_id),
                            blob_id,
                            byte_len: None,
                        },
                        BlobAction::Get { blob_id, .. } => BlobResult::Error {
                            reason: format!(
                                "native S3B proof does not admit blob reads for '{blob_id}'"
                            ),
                        },
                    };
                    serde_json::to_value(result).map(Some).map_err(|error| {
                        NativeHostError::ProtocolViolation {
                            reason: format!("encode native blob result: {error}"),
                        }
                    })
                }
                MessageKind::EventPublishRequest => {
                    if payload.get("event").and_then(Value::as_str).is_none() {
                        return Err(NativeHostError::ProtocolViolation {
                            reason: "native event publication has no event name".to_string(),
                        });
                    }
                    self.events
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(payload);
                    Ok(None)
                }
                other => Err(NativeHostError::ProtocolViolation {
                    reason: format!("unexpected child request in S3B host sink: {other:?}"),
                }),
            }
        }
    }

    /// Cookie service adapter whose every primitive call is a native IPC
    /// capability request to the CEF provider.
    #[derive(Clone)]
    struct NativeCookieEngine {
        connection: Arc<NativeProviderConnection>,
    }

    impl NativeCookieEngine {
        fn new(connection: Arc<NativeProviderConnection>) -> Self {
            Self { connection }
        }
    }

    impl CookieEngineBackend for NativeCookieEngine {
        fn get_cookies(&self, req: GetCookiesRequest) -> Result<GetCookiesResponse, String> {
            decode(call_op(
                &self.connection,
                OP_COOKIE_GET,
                serde_json::to_value(req).map_err(|error| error.to_string())?,
            )?)
        }

        fn set_cookie(&self, req: SetCookieRequest) -> Result<SetCookieResponse, String> {
            decode(call_op(
                &self.connection,
                OP_COOKIE_SET,
                serde_json::to_value(req).map_err(|error| error.to_string())?,
            )?)
        }

        fn get_cookies_v0_2(
            &self,
            req: GetCookiesRequest,
        ) -> Result<GetCookiesResponseV0_2, String> {
            decode(call_op(
                &self.connection,
                OP_COOKIE_GET_V0_2,
                serde_json::to_value(req).map_err(|error| error.to_string())?,
            )?)
        }

        fn set_cookie_v0_2(&self, req: SetCookieRequestV0_2) -> Result<SetCookieResponse, String> {
            decode(call_op(
                &self.connection,
                OP_COOKIE_SET_V0_2,
                serde_json::to_value(req).map_err(|error| error.to_string())?,
            )?)
        }

        fn delete_cookies(
            &self,
            req: worldline_browser_contract::primitives::DeleteCookiesRequest,
        ) -> Result<worldline_browser_contract::primitives::DeleteCookiesResponse, String> {
            decode(call_op(
                &self.connection,
                OP_COOKIE_DELETE,
                serde_json::to_value(req).map_err(|error| error.to_string())?,
            )?)
        }

        fn clear_storage(
            &self,
            req: ClearStorageRequest,
        ) -> Result<worldline_browser_contract::primitives::ClearStorageResponse, String> {
            decode(call_op(
                &self.connection,
                worldline_browser_contract::authority::OP_STORAGE_CLEAR,
                serde_json::to_value(req).map_err(|error| error.to_string())?,
            )?)
        }
    }

    fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    fn call_op(
        connection: &NativeProviderConnection,
        operation: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let response = connection
            .call_with_deadline(
                serde_json::json!({"operation": operation, "payload": payload}),
                Duration::from_secs(5),
            )
            .map_err(|error| {
                let stderr = connection.stderr_text();
                let status = connection
                    .try_status()
                    .map(|status| format!("; provider exit status: {status}"))
                    .unwrap_or_default();
                if stderr.trim().is_empty() && status.is_empty() {
                    format!("native S3B IPC call '{operation}' failed: {error}")
                } else {
                    format!(
                        "native S3B IPC call '{operation}' failed: {error}{status}; provider stderr:\n{stderr}"
                    )
                }
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!(
                "native provider operation '{operation}' failed: {error}"
            ));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("native provider operation '{operation}' omitted result"))
    }

    fn wait_for_event(
        connection: &NativeProviderConnection,
        sink: &RealS3BHostSink,
        event_name: &str,
        timeout: Duration,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = sink.take_event(event_name) {
                return Ok(event);
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for native event '{event_name}'"));
            }
            let _ = call_op(connection, OP_LIST_CONTEXTS, serde_json::json!({}))?;
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_page(
        connection: &NativeProviderConnection,
        page_id: &PageId,
        timeout: Duration,
    ) -> Result<PageObservation, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let observation: PageObservation = decode(call_op(
                connection,
                OP_OBSERVE,
                serde_json::to_value(ObservePageRequest {
                    page_id: page_id.clone(),
                })
                .map_err(|error| error.to_string())?,
            )?)?;
            match observation.loading_state {
                LoadingState::Complete => return Ok(observation),
                LoadingState::Failed => {
                    return Err(format!(
                        "CEF page '{}' failed to load '{}'; provider stderr:\n{}",
                        page_id,
                        observation.url,
                        connection.stderr_text()
                    ));
                }
                LoadingState::Unloaded | LoadingState::Loading | LoadingState::Interactive => {}
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for page '{page_id}'"));
            }
            thread::sleep(Duration::from_millis(25));
        }
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
        candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                "pinned CEF bootstrapc.exe is missing; stage the verified CEF runtime before S3B-real".to_string()
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
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                "worldline_browser_provider_client.dll is missing; build and stage the CEF bootstrap client before S3B-real".to_string()
            })?;
        if client.parent() != bootstrap.parent() {
            return Err(format!(
                "CEF bootstrap client '{}' must be staged beside '{}'",
                client.display(),
                bootstrap.display()
            ));
        }
        Ok(client)
    }

    #[derive(Debug, Deserialize)]
    struct StartedDownloadEvent {
        download_id: DownloadId,
        context_id: BrowserContextId,
        page_id: PageId,
        url: String,
        suggested_filename: String,
        total_bytes: Option<u64>,
        mime_type: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct CompletedDownloadEvent {
        download_id: DownloadId,
        blob_id: String,
        mime_type: Option<String>,
    }

    fn storage_item(
        connection: &NativeProviderConnection,
        operation: &str,
        request: StorageItemRequestV0_2,
    ) -> Result<StorageItemResponseV0_2, String> {
        decode(call_op(
            connection,
            operation,
            serde_json::to_value(request).map_err(|error| error.to_string())?,
        )?)
    }

    /// Runs one isolated real S3B scenario with an explicitly selected native
    /// provider executable. This function is kept separate from discovery so
    /// hosted runners can audit the exact binary and runtime staging.
    pub fn run_with_provider(program: impl Into<PathBuf>) -> Result<S3BReport, String> {
        let _real_cef_guard = RealCefRunGuard::acquire()?;
        let program = program.into();
        let server = LoopbackServer::start()?;
        let temp_root = TempRoot::create("run")?;
        let artifact_root = temp_root.path().join("host-blobs");
        let staging_root = temp_root.path().join("service-staging");
        let downloads_state_path = temp_root
            .path()
            .join("downloads-state")
            .join("records.json");
        let cef_root = temp_root.path().join("cef-runtime");
        std::fs::create_dir_all(&staging_root)
            .map_err(|error| format!("create S3B service staging root: {error}"))?;
        let artifact_store = Arc::new(ArtifactStore::open(&artifact_root)?);
        let sink = Arc::new(RealS3BHostSink::new(Arc::clone(&artifact_store)));
        let identity = ExpectedIdentity {
            package_id: "worldline.browser.pkg".to_string(),
            plugin_definition_id: "worldline.browser.provider".to_string(),
        };
        let client = discover_provider_client(&program)?;
        let client_name = client.file_name().ok_or_else(|| {
            format!(
                "CEF bootstrap client has no file name: {}",
                client.display()
            )
        })?;
        let child_args = vec![
            format!("--module={}", client_name.to_string_lossy()),
            // Hosted Windows runners may not expose a usable GPU adapter.
            // Keep the browser headful while forcing Chromium's software
            // compositing path; this is still the real CEF renderer and does
            // not change the native provider or IPC boundary.
            "--disable-gpu".to_string(),
            "--in-process-gpu".to_string(),
            "--package-id".to_string(),
            identity.package_id.clone(),
            "--definition-id".to_string(),
            identity.plugin_definition_id.clone(),
            "--backend".to_string(),
            "cef".to_string(),
            "--cache-root".to_string(),
            cef_root.to_string_lossy().into_owned(),
        ];
        // The real S3B launcher is the pinned CEF console bootstrap. Keeping
        // the module argument explicit makes the sandbox/client boundary
        // auditable and prevents accidental execution of the provider EXE.
        let spec = NativeChildSpec::new(program, child_args, 4 * 1024 * 1024, 64 * 1024);
        let (connection, ack) = NativeProviderConnection::connect(
            spec,
            &identity,
            Arc::clone(&sink) as Arc<dyn HostRequestSink>,
            16,
        )
        .map_err(|error| format!("connect S3B native provider: {error}"))?;
        if !ack
            .declared_interfaces
            .iter()
            .any(|interface| interface == "browser.engine.cookies/v0.2")
            || !ack
                .declared_interfaces
                .iter()
                .any(|interface| interface == "browser.engine.storage/v0.2")
        {
            return Err(
                "native provider did not declare additive engine primitive contracts".to_string(),
            );
        }
        let connection = Arc::new(connection);

        let run_token = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos()
        );
        let ctx_a: CreateContextResponse = decode(call_op(
            &connection,
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some(format!("s3b-{run_token}-a")),
                incognito: false,
                user_agent: None,
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let ctx_b: CreateContextResponse = decode(call_op(
            &connection,
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some(format!("s3b-{run_token}-b")),
                incognito: false,
                user_agent: None,
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let page_a: CreatePageResponse = decode(call_op(
            &connection,
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_a.context_id.clone(),
                initial_url: Some(format!("{}/index.html", server.base_url)),
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let page_b: CreatePageResponse = decode(call_op(
            &connection,
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_b.context_id.clone(),
                initial_url: Some(format!("{}/index.html", server.base_url)),
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        wait_for_page(&connection, &page_a.page_id, Duration::from_secs(15))?;
        wait_for_page(&connection, &page_b.page_id, Duration::from_secs(15))?;

        // Part A: the product service establishes intent, then the real CEF
        // download primitive emits started/completed events through IPC.
        let downloads_service = DownloadsService::open_persistent(
            Arc::clone(&artifact_store),
            staging_root.clone(),
            downloads_state_path.clone(),
        )?;
        let download_url = format!("{}/package-v1.tar.gz", server.base_url);
        let service_start = downloads_service.start_download(ServiceStartDownloadRequest {
            context_id: ctx_a.context_id.clone(),
            page_id: Some(page_a.page_id.clone()),
            url: download_url.clone(),
            suggested_filename: Some("package-v1.tar.gz".to_string()),
        });
        let download_record_id = service_start.record_id.clone();
        let engine_start: DownloadStatusResponse = decode(call_op(
            &connection,
            OP_DOWNLOAD_START,
            serde_json::to_value(EngineStartDownloadRequest {
                page_id: page_a.page_id.clone(),
                url: download_url.clone(),
                destination_path: None,
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let started: StartedDownloadEvent = decode(wait_for_event(
            &connection,
            &sink,
            "browser.download.started",
            Duration::from_secs(30),
        )?)?;
        if started.download_id != engine_start.download_id
            || started.context_id != ctx_a.context_id
            || started.page_id != page_a.page_id
            || started.url != download_url
        {
            return Err(
                "native CEF download-start event does not match engine response".to_string(),
            );
        }
        downloads_service.on_engine_download_started(EngineDownloadStarted {
            engine_download_id: started.download_id.clone(),
            context_id: started.context_id,
            page_id: started.page_id,
            url: started.url,
            suggested_filename: started.suggested_filename,
            total_bytes: started.total_bytes,
            media_type: started.mime_type,
        });
        let completed: CompletedDownloadEvent = decode(wait_for_event(
            &connection,
            &sink,
            "browser.download.completed",
            Duration::from_secs(30),
        )?)?;
        if completed.download_id != engine_start.download_id {
            return Err(
                "native CEF download-completed event has the wrong download identity".to_string(),
            );
        }
        if !artifact_store.contains(&completed.blob_id) {
            return Err("host generic blob store did not admit the CEF download blob".to_string());
        }
        let service_blob_grant = BlobReadBroker::new().issue(
            "s3b-service-reader",
            AUTH_BLOB_READ,
            completed.blob_id.clone(),
        )?;
        let provider_bytes = artifact_store
            .read_bytes_with_authority(&completed.blob_id, &service_blob_grant)
            .map_err(|error| error.to_string())?;
        if provider_bytes != DOWNLOAD_BODY {
            return Err(
                "host blob bytes differ from the deterministic loopback download".to_string(),
            );
        }
        downloads_service.on_engine_download_completed(
            &engine_start.download_id,
            &provider_bytes,
            completed.mime_type,
        );
        downloads_service.check_persistence()?;
        let record = downloads_service
            .get_download_record(GetDownloadRecordRequest {
                record_id: download_record_id.clone(),
            })
            .record
            .ok_or_else(|| "native download record was not materialized".to_string())?;
        let artifact_ref = record
            .artifact_ref
            .clone()
            .ok_or_else(|| "native download did not produce ArtifactRef".to_string())?;
        if artifact_ref.artifact_id != completed.blob_id
            || artifact_ref.sha256_hash.as_deref() != completed.blob_id.strip_prefix("sha256-v1-")
        {
            return Err(
                "download artifact does not carry the provider blob's real SHA-256 identity"
                    .to_string(),
            );
        }
        let blob_broker = BlobReadBroker::new();
        let metadata_only_isolation_ok = blob_broker
            .issue(
                "metadata-reader",
                AUTH_DOWNLOADS_READ,
                artifact_ref.artifact_id.clone(),
            )
            .is_err()
            && record.suggested_filename == "package-v1.tar.gz"
            && record.total_bytes == Some(DOWNLOAD_BODY.len() as u64)
            && record.status == DownloadLifecycleStatus::Completed;

        // Reopen the generic host store separately before reconstructing the
        // service snapshot: service restart must not rely on an in-memory map
        // or on the original ArtifactStore object.
        let reopened_store = Arc::new(ArtifactStore::open(&artifact_root)?);
        let restarted_downloads = DownloadsService::open_persistent(
            Arc::clone(&reopened_store),
            staging_root,
            downloads_state_path,
        );
        let restarted_downloads = restarted_downloads?;
        restarted_downloads.check_persistence()?;
        let restarted_record = restarted_downloads
            .get_download_record(GetDownloadRecordRequest {
                record_id: download_record_id.clone(),
            })
            .record
            .ok_or_else(|| "download record did not survive service restart".to_string())?;
        let restart_grant = blob_broker.issue(
            "restarted-service-reader",
            AUTH_BLOB_READ,
            artifact_ref.artifact_id.clone(),
        )?;
        let restarted_bytes = reopened_store
            .read_bytes_with_authority(&artifact_ref.artifact_id, &restart_grant)
            .map_err(|error| error.to_string())?;
        let download_survived_restart = restarted_record.status
            == DownloadLifecycleStatus::Completed
            && restarted_record.artifact_ref.as_ref() == Some(&artifact_ref)
            && restarted_bytes == DOWNLOAD_BODY;

        // Part B: CookiesService is restarted around a native CEF cookie
        // adapter; values never pass through an in-memory cookie engine.
        let origin = server.base_url.clone();
        let page_url = format!("{origin}/index.html");
        let domain = "127.0.0.1".to_string();
        let cookie_expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock before Unix epoch: {error}"))?
            .as_secs()
            .saturating_add(30 * 24 * 60 * 60);
        let cookie_engine = Arc::new(NativeCookieEngine::new(Arc::clone(&connection)));
        let cookies_service = CookiesService::new(cookie_engine.clone());
        let cookie_a_set = cookies_service
            .set_cookie_v0_2(SetCookieServiceRequestV0_2 {
                context_id: ctx_a.context_id.clone(),
                name: "auth_token".to_string(),
                value: "secret_context_A_session_token".to_string(),
                domain: domain.clone(),
                path: Some("/".to_string()),
                secure: Some(false),
                http_only: Some(true),
                same_site: Some("Lax".to_string()),
                expires_epoch_sec: Some(cookie_expiry),
                host_only: true,
            })
            .map_err(|error| error.to_string())?;
        if !cookie_a_set.success {
            return Err("native CEF rejected context A cookie".to_string());
        }
        let cookie_b_set = cookies_service
            .set_cookie_v0_2(SetCookieServiceRequestV0_2 {
                context_id: ctx_b.context_id.clone(),
                name: "auth_token".to_string(),
                value: "secret_context_B_session_token".to_string(),
                domain: domain.clone(),
                path: Some("/".to_string()),
                secure: Some(false),
                http_only: Some(true),
                same_site: Some("Lax".to_string()),
                expires_epoch_sec: Some(cookie_expiry),
                host_only: true,
            })
            .map_err(|error| error.to_string())?;
        if !cookie_b_set.success {
            return Err("native CEF rejected context B cookie".to_string());
        }
        let metadata = cookies_service
            .get_cookie_metadata_v0_2(GetCookieMetadataRequest {
                context_id: ctx_a.context_id.clone(),
                url: Some(page_url.clone()),
                domain: None,
            })
            .map_err(|error| error.to_string())?;
        let metadata_auth = match metadata.cookies.as_slice() {
            [cookie] if cookie.name == "auth_token" => cookie,
            cookies => {
                return Err(format!(
                    "native CEF cookie metadata was not the exact context A cookie: {:?}",
                    cookies
                        .iter()
                        .map(|cookie| (&cookie.name, &cookie.domain, cookie.host_only))
                        .collect::<Vec<_>>()
                ));
            }
        };
        if !metadata_auth.host_only
            || metadata_auth.domain != domain
            || metadata_auth.path != "/"
            || !metadata_auth.http_only
            || metadata_auth.same_site.as_deref() != Some("Lax")
            || metadata_auth.expires_epoch_sec != Some(cookie_expiry)
        {
            return Err(format!(
                "native CEF cookie metadata did not preserve host-only attributes: {metadata_auth:?}"
            ));
        }
        let cookie_value = |service: &CookiesService, context_id: BrowserContextId| {
            service
                .get_cookie_value_v0_2(GetCookieValueRequest {
                    context_id,
                    domain: domain.clone(),
                    name: "auth_token".to_string(),
                    path: Some("/".to_string()),
                    url: Some(page_url.clone()),
                })
                .map_err(|error| error.to_string())?
                .cookie
                .ok_or_else(|| "native CEF cookie value is missing".to_string())
        };
        let cookie_a = cookie_value(&cookies_service, ctx_a.context_id.clone())?;
        let cookie_b = cookie_value(&cookies_service, ctx_b.context_id.clone())?;
        let cross_context_cookies_isolated = cookie_a.expose_value()
            == "secret_context_A_session_token"
            && cookie_b.expose_value() == "secret_context_B_session_token"
            && cookie_a.expose_value() != cookie_b.expose_value();
        let cookie_policy = cookies_service.export_policy();
        let restarted_cookie_engine = Arc::new(NativeCookieEngine::new(Arc::clone(&connection)));
        let restarted_cookies = CookiesService::from_policy(cookie_policy, restarted_cookie_engine);
        let cookie_a_after_restart = cookie_value(&restarted_cookies, ctx_a.context_id.clone())?;
        let cookies_survived_restart =
            cookie_a_after_restart.expose_value() == "secret_context_A_session_token";

        // Additive engine.storage/0.2 set/get calls prove real origin-scoped
        // storage, while the service clear operation remains the 0.1 boundary.
        storage_item(
            &connection,
            OP_STORAGE_SET_V0_2,
            StorageItemRequestV0_2 {
                context_id: ctx_a.context_id.clone(),
                origin: origin.clone(),
                storage_type: StorageType::LocalStorage,
                key: "theme".to_string(),
                value: Some("dark".to_string()),
            },
        )?;
        storage_item(
            &connection,
            OP_STORAGE_SET_V0_2,
            StorageItemRequestV0_2 {
                context_id: ctx_b.context_id.clone(),
                origin: origin.clone(),
                storage_type: StorageType::LocalStorage,
                key: "theme".to_string(),
                value: Some("light".to_string()),
            },
        )?;
        let item_a_before_clear = storage_item(
            &connection,
            OP_STORAGE_GET_V0_2,
            StorageItemRequestV0_2 {
                context_id: ctx_a.context_id.clone(),
                origin: origin.clone(),
                storage_type: StorageType::LocalStorage,
                key: "theme".to_string(),
                value: None,
            },
        )?;
        if item_a_before_clear.value.as_deref() != Some("dark") {
            return Err("native CEF storage set/get did not preserve context A value".to_string());
        }
        let clear_response = restarted_cookies
            .clear_site_data(ClearSiteDataRequest {
                context_id: ctx_a.context_id.clone(),
                origin: origin.clone(),
                storage_type: StorageType::LocalStorage,
            })
            .map_err(|error| error.to_string())?;
        let item_a_after_first_clear = storage_item(
            &connection,
            OP_STORAGE_GET_V0_2,
            StorageItemRequestV0_2 {
                context_id: ctx_a.context_id.clone(),
                origin: origin.clone(),
                storage_type: StorageType::LocalStorage,
                key: "theme".to_string(),
                value: None,
            },
        )?;
        let empty_clear_response = restarted_cookies
            .clear_site_data(ClearSiteDataRequest {
                context_id: ctx_a.context_id.clone(),
                origin: origin.clone(),
                storage_type: StorageType::LocalStorage,
            })
            .map_err(|error| error.to_string())?;
        if !clear_response.cleared || empty_clear_response.cleared {
            return Err(format!(
                "native CEF storage clear result did not report whether data existed: first={}, second={}, value_after_first={:?}",
                clear_response.cleared,
                empty_clear_response.cleared,
                item_a_after_first_clear.value
            ));
        }
        let item_a = storage_item(
            &connection,
            OP_STORAGE_GET_V0_2,
            StorageItemRequestV0_2 {
                context_id: ctx_a.context_id.clone(),
                origin: origin.clone(),
                storage_type: StorageType::LocalStorage,
                key: "theme".to_string(),
                value: None,
            },
        )?;
        let item_b = storage_item(
            &connection,
            OP_STORAGE_GET_V0_2,
            StorageItemRequestV0_2 {
                context_id: ctx_b.context_id.clone(),
                origin,
                storage_type: StorageType::LocalStorage,
                key: "theme".to_string(),
                value: None,
            },
        )?;
        let site_data_clear_isolated =
            item_a.value.is_none() && item_b.value.as_deref() == Some("light");

        // Part C: service instances can disappear while the native provider
        // remains live; direct navigation, tabs, and history still work.
        drop(restarted_downloads);
        drop(restarted_cookies);
        drop(cookies_service);
        drop(cookie_engine);
        let tabs_service = TabsService::new();
        let tab = tabs_service.create_tab(CreateTabRequest {
            page_id: page_a.page_id.clone(),
            group_id: None,
            pinned: Some(false),
            select: Some(true),
        });
        let history_service = HistoryService::new();
        let navigation_url = format!("{}/docs.html", server.base_url);
        let navigation: NavigateResponse = decode(call_op(
            &connection,
            OP_NAVIGATE,
            serde_json::to_value(NavigateRequest {
                page_id: page_a.page_id.clone(),
                url: navigation_url.clone(),
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let observation = wait_for_page(&connection, &page_a.page_id, Duration::from_secs(15))?;
        let history_entry = history_service
            .record_navigation(
                page_a.page_id.clone(),
                NavigationId::new("s3b-real-navigation-1"),
                navigation.document_revision,
                navigation_url.clone(),
                1_725_186_000_000,
            )
            .map_err(|error| error.to_string())?;
        let service_failure_isolation_ok = tab.tab.id.as_str().starts_with("tab-")
            && navigation.committed
            && history_entry.url == navigation_url
            && observation.url == navigation_url;

        connection
            .close(Duration::from_secs(10))
            .map_err(|error| format!("close native CEF provider: {error}"))?;

        Ok(S3BReport {
            download_record_id: download_record_id.to_string(),
            artifact_id: artifact_ref.artifact_id,
            artifact_bytes_verified: provider_bytes == DOWNLOAD_BODY,
            download_survived_restart,
            metadata_only_isolation_ok,
            cross_context_cookies_isolated,
            cookies_survived_restart,
            site_data_clear_isolated,
            service_failure_isolation_ok,
        })
    }

    pub fn run() -> Result<S3BReport, String> {
        run_with_provider(discover_provider_process()?)
    }
}
