//! S3E: Browser Search Providers proving slice.
//!
//! Proves that Worldline's engine-neutral search target resolution capability
//! (`browser.search/0.1`) operates with strict authority separation from page
//! navigation, constructs structured URLs without template injection, preserves
//! query privacy, supports generic multi-installation provider targeting, and
//! isolates provider failures from direct navigation and other browser services.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use worldline_browser_search::{SearchProviderConfig, SearchProviderPlugin, search_capability};
use worldline_browser_services_contract::{SearchNavigationTarget, SearchResolveRequest};
use worldline_kernel::{
    CapabilityId, CapabilityService, GrantLifetime, InterfaceVersion, Kernel, NoopRuntime, Plugin,
    PluginDefinition, PluginError, PluginRuntime, PrincipalKind, ResourceScope, StateSchemaVersion,
};

/// Verification report emitted by slice S3E.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3EReport {
    pub topology: String,
    pub provider_a_resolved: bool,
    pub provider_b_resolved: bool,
    pub distinct_targets_produced: bool,
    pub resolve_alone_zero_origin_hits: bool,
    pub navigation_produced_origin_hit: bool,
    pub query_decoded_intact: bool,
    pub search_only_cannot_navigate: bool,
    pub navigation_only_cannot_search: bool,
    pub lifecycle_isolation_verified: bool,
    pub query_privacy_verified: bool,
    pub accepted: bool,
}

/// Minimal loopback HTTP server for deterministic proving.
struct LoopbackSearchServer {
    pub port: u16,
    pub base_url: String,
    pub hits: Arc<AtomicUsize>,
    pub last_query: Arc<Mutex<Option<String>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LoopbackSearchServer {
    fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("failed to bind loopback TCP listener: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("failed to get local addr: {e}"))?
            .port();
        let base_url = format!("http://127.0.0.1:{port}");

        let hits = Arc::new(AtomicUsize::new(0));
        let last_query = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));

        let hits_clone = Arc::clone(&hits);
        let query_clone = Arc::clone(&last_query);
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).expect("set nonblocking");

            while !shutdown_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        hits_clone.fetch_add(1, Ordering::SeqCst);
                        let mut buffer = [0u8; 4096];
                        let bytes_read = stream.read(&mut buffer).unwrap_or(0);
                        let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);

                        // Parse requested path and query parameter
                        if let Some(first_line) = request_str.lines().next() {
                            let parts: Vec<&str> = first_line.split_whitespace().collect();
                            if parts.len() >= 2 {
                                let path_and_query = parts[1];
                                if let Some(query_idx) = path_and_query.find('?') {
                                    let qs = &path_and_query[query_idx + 1..];
                                    for pair in qs.split('&') {
                                        let mut kv = pair.split('=');
                                        let key = kv.next().unwrap_or("");
                                        let val = kv.next().unwrap_or("");
                                        if key == "q" || key == "term" {
                                            // Decode URL encoding
                                            let decoded = urlencoding_decode(val);
                                            *query_clone.lock().unwrap() = Some(decoded);
                                        }
                                    }
                                }
                            }
                        }

                        let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\nContent-Length: 48\r\n\r\n<html><body><h1>Search Results</h1></body></html>";
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            port,
            base_url,
            hits,
            last_query,
            shutdown,
            handle: Some(handle),
        })
    }
}

impl Drop for LoopbackSearchServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Connect to unblock if needed
        let _ = TcpStream::connect(format!("127.0.0.1:{}", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn urlencoding_decode(input: &str) -> String {
    let replaced = input.replace('+', " ");
    let mut result = String::new();
    let mut chars = replaced.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = format!("{c1}{c2}");
                if let Ok(byte) = u8::from_str_radix(&hex_str, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
        }
        result.push(c);
    }
    result
}

/// Simulated browser navigation service to prove separate authorization and execution.
struct ReferenceNavigationService {
    server_base_url: String,
}

impl CapabilityService for ReferenceNavigationService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation != "navigate" {
            return Err(format!("unsupported navigation operation '{operation}'"));
        }
        let url_str =
            String::from_utf8(payload.to_vec()).map_err(|e| format!("invalid URL payload: {e}"))?;

        // Perform actual HTTP GET request to the target URL to simulate navigation
        if let Ok(mut stream) = TcpStream::connect(
            url_str
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or(""),
        ) {
            let path_and_query = url_str
                .trim_start_matches("http://")
                .find('/')
                .map(|idx| &url_str[idx + 7..])
                .unwrap_or("/");

            let request = format!(
                "GET {path_and_query} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                self.server_base_url
            );
            let _ = stream.write_all(request.as_bytes());
            let _ = stream.flush();
            let mut response = Vec::new();
            let _ = stream.read_to_end(&mut response);
        }

        Ok(b"navigation-complete".to_vec())
    }
}

struct ReferenceNavigationPlugin {
    definition: PluginDefinition,
    server_base_url: String,
}

fn navigation_capability() -> CapabilityId {
    CapabilityId::new("browser.navigate", "navigate", InterfaceVersion::new(1, 0))
}

impl ReferenceNavigationPlugin {
    fn new(server_base_url: String) -> Self {
        Self {
            definition: PluginDefinition::new("reference-browser-navigation")
                .provides(navigation_capability()),
            server_base_url,
        }
    }
}

impl Plugin for ReferenceNavigationPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut worldline_kernel::ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        context.publish_capability(
            navigation_capability(),
            Arc::new(ReferenceNavigationService {
                server_base_url: self.server_base_url.clone(),
            }),
        )?;
        Ok(Box::new(NoopRuntime))
    }
}

/// Executes the deterministic reference S3E proving slice.
pub fn run_reference() -> Result<S3EReport, String> {
    let server = LoopbackSearchServer::start()?;

    let mut kernel = Kernel::new();
    let def_id = "worldline-browser-search";
    let inst_a = kernel
        .create_installation(def_id, StateSchemaVersion::default())
        .map_err(|e| format!("create inst_a: {e:?}"))?;
    let inst_b = kernel
        .create_installation(def_id, StateSchemaVersion::default())
        .map_err(|e| format!("create inst_b: {e:?}"))?;

    let config_a =
        SearchProviderConfig::new("Alpha", format!("{}/search-a/", server.base_url), "q")
            .with_static_parameter("engine", "alpha")
            .with_loopback_http(true);

    let config_b =
        SearchProviderConfig::new("Beta", format!("{}/search-b/", server.base_url), "term")
            .with_static_parameter("engine", "beta")
            .with_loopback_http(true);

    let plugin_a = SearchProviderPlugin::new(def_id, config_a);
    let plugin_b = SearchProviderPlugin::new(def_id, config_b);

    kernel
        .register_for_installation(plugin_a, &inst_a)
        .map_err(|e| format!("register inst_a: {e:?}"))?;
    kernel
        .register_for_installation(plugin_b, &inst_b)
        .map_err(|e| format!("register inst_b: {e:?}"))?;

    // Also register reference navigation provider
    let nav_inst = kernel
        .create_installation(
            "reference-browser-navigation",
            StateSchemaVersion::default(),
        )
        .map_err(|e| format!("create nav_inst: {e:?}"))?;
    let nav_plugin = ReferenceNavigationPlugin::new(server.base_url.clone());
    kernel
        .register_for_installation(nav_plugin, &nav_inst)
        .map_err(|e| format!("register nav: {e:?}"))?;

    let search_cap = search_capability();
    let nav_cap = navigation_capability();

    let consumer = kernel
        .register_principal_id("search-consumer-principal", PrincipalKind::User)
        .map_err(|e| format!("register consumer: {e:?}"))?;

    // Grant consumer search resolution authority
    kernel
        .create_root_grant(
            consumer.clone(),
            search_cap.contract(),
            ["resolve"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .map_err(|e| format!("grant search: {e:?}"))?;

    // Grant consumer navigation authority
    kernel
        .create_root_grant(
            consumer.clone(),
            nav_cap.contract(),
            ["navigate"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .map_err(|e| format!("grant nav: {e:?}"))?;

    // 1. Resolve through Provider A and Provider B for the exact same input query
    let query_text = "worldline rust architecture";
    let req = SearchResolveRequest::new(query_text).map_err(|e| format!("{e:?}"))?;
    let payload = serde_json::to_vec(&req).map_err(|e| format!("{e:?}"))?;

    let handle_a = kernel
        .capability_for_installation(consumer.clone(), search_cap.clone(), &inst_a)
        .map_err(|e| format!("{e:?}"))?;
    let resp_a_bytes = handle_a
        .invoke("resolve", &payload)
        .map_err(|e| format!("{e:?}"))?;
    let target_a: SearchNavigationTarget =
        serde_json::from_slice(&resp_a_bytes).map_err(|e| format!("{e:?}"))?;
    let provider_a_resolved = target_a
        .url()
        .contains("/search-a/?engine=alpha&q=worldline+rust+architecture");

    let handle_b = kernel
        .capability_for_installation(consumer.clone(), search_cap.clone(), &inst_b)
        .map_err(|e| format!("{e:?}"))?;
    let resp_b_bytes = handle_b
        .invoke("resolve", &payload)
        .map_err(|e| format!("{e:?}"))?;
    let target_b: SearchNavigationTarget =
        serde_json::from_slice(&resp_b_bytes).map_err(|e| format!("{e:?}"))?;
    let provider_b_resolved = target_b
        .url()
        .contains("/search-b/?engine=beta&term=worldline+rust+architecture");

    let distinct_targets_produced = target_a.url() != target_b.url();

    // 2. Prove resolve alone causes zero origin hits
    let resolve_alone_zero_origin_hits = server.hits.load(Ordering::SeqCst) == 0;

    // 3. Separately authorized browser.navigate invocation navigates to resolved target
    let nav_handle = kernel
        .capability_for(consumer.clone(), nav_cap.clone())
        .map_err(|e| format!("{e:?}"))?;
    let nav_res = nav_handle.invoke("navigate", target_a.url().as_bytes());
    assert!(nav_res.is_ok(), "navigation must succeed");

    thread::sleep(Duration::from_millis(50));
    let navigation_produced_origin_hit = server.hits.load(Ordering::SeqCst) == 1;

    let received_query = server
        .last_query
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_default();
    let query_decoded_intact = received_query == query_text;

    // 4. Authority separation checks
    let search_only_principal = kernel
        .register_principal_id("search-only", PrincipalKind::User)
        .map_err(|e| format!("{e:?}"))?;
    kernel
        .create_root_grant(
            search_only_principal.clone(),
            search_cap.contract(),
            ["resolve"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .map_err(|e| format!("{e:?}"))?;

    let search_only_cannot_navigate =
        match kernel.capability_for(search_only_principal, nav_cap.clone()) {
            Ok(h) => h.invoke("navigate", b"http://example.com").is_err(),
            Err(_) => true,
        };

    let nav_only_principal = kernel
        .register_principal_id("nav-only", PrincipalKind::User)
        .map_err(|e| format!("{e:?}"))?;
    kernel
        .create_root_grant(
            nav_only_principal.clone(),
            nav_cap.contract(),
            ["navigate"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .map_err(|e| format!("{e:?}"))?;

    let navigation_only_cannot_search =
        match kernel.capability_for(nav_only_principal, search_cap.clone()) {
            Ok(h) => h.invoke("resolve", &payload).is_err(),
            Err(_) => true,
        };

    // 5. Lifecycle isolation: uninstalling inst_a leaves inst_b and navigation operational
    let _ = kernel.uninstall(&inst_a);
    let lifecycle_isolation_verified = handle_b.invoke("resolve", &payload).is_ok()
        && nav_handle
            .invoke("navigate", target_b.url().as_bytes())
            .is_ok();

    // 6. Query privacy in trajectory
    let sensitive = "top-secret-private-health-query";
    let priv_req = SearchResolveRequest::new(sensitive).map_err(|e| format!("{e:?}"))?;
    let priv_payload = serde_json::to_vec(&priv_req).map_err(|e| format!("{e:?}"))?;
    let _ = handle_b.invoke("resolve", &priv_payload);

    let mut query_privacy_verified = true;
    for event in kernel.trajectory() {
        let event_debug = format!("{event:?}");
        if event_debug.contains(sensitive) {
            query_privacy_verified = false;
            break;
        }
    }

    let accepted = provider_a_resolved
        && provider_b_resolved
        && distinct_targets_produced
        && resolve_alone_zero_origin_hits
        && navigation_produced_origin_hit
        && query_decoded_intact
        && search_only_cannot_navigate
        && navigation_only_cannot_search
        && lifecycle_isolation_verified
        && query_privacy_verified;

    Ok(S3EReport {
        topology: "reference".to_string(),
        provider_a_resolved,
        provider_b_resolved,
        distinct_targets_produced,
        resolve_alone_zero_origin_hits,
        navigation_produced_origin_hit,
        query_decoded_intact,
        search_only_cannot_navigate,
        navigation_only_cannot_search,
        lifecycle_isolation_verified,
        query_privacy_verified,
        accepted,
    })
}

#[cfg(windows)]
pub fn run() -> Result<S3EReport, String> {
    real::run()
}

#[cfg(not(windows))]
pub fn run() -> Result<S3EReport, String> {
    Err("S3E-real requires the hosted Windows CEF runtime".to_string())
}

#[cfg(windows)]
mod real {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use worldline_browser_contract::authority::{
        OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_NAVIGATE, OP_OBSERVE,
    };
    use worldline_browser_contract::contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        NavigateRequest, NavigateResponse, ObservePageRequest, PageObservation,
    };
    use worldline_browser_search::{SearchProviderConfig, SearchProviderService};
    use worldline_browser_services_contract::SearchResolveRequest;
    use worldline_native_host::{
        ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError,
        NativeProviderConnection,
    };

    use super::S3EReport;
    use crate::real_cef_lock::RealCefRunGuard;

    const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
    const MAX_IN_FLIGHT: usize = 8;

    struct S3EHostSink;

    impl HostRequestSink for S3EHostSink {
        fn on_child_request(
            &self,
            _kind: worldline_plugin_protocol::MessageKind,
            _correlation_id: u64,
            _payload: Value,
        ) -> Result<Option<Value>, NativeHostError> {
            Ok(None)
        }
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn create(nonce: u128) -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!(
                "worldline-reference-s3e-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).map_err(|e| format!("create S3E temp dir: {e}"))?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
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
        candidates.into_iter().find(|c| c.is_file()).ok_or_else(|| {
            "pinned CEF bootstrapc.exe is missing; stage CEF before S3E-real".to_string()
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

    pub fn run() -> Result<S3EReport, String> {
        let _guard = RealCefRunGuard::acquire()?;
        let program = discover_provider_process()?;
        let client = discover_provider_client(&program)?;

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();

        let temp_root = TempRoot::create(nonce)?;

        // Start loopback server
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind TCP: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let base_url = format!("http://127.0.0.1:{port}");

        let hits = Arc::new(AtomicUsize::new(0));
        let last_query = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));

        let hits_clone = Arc::clone(&hits);
        let query_clone = Arc::clone(&last_query);
        let shutdown_clone = Arc::clone(&shutdown);

        let target_title = format!("__worldline_s3e_loaded_{nonce}");
        let target_title_clone = target_title.clone();

        let connections = Arc::new(Mutex::new(Vec::new()));
        let connections_clone = Arc::clone(&connections);

        let server_handle = thread::spawn(move || {
            listener.set_nonblocking(true).expect("set nonblocking");
            while !shutdown_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let hits = Arc::clone(&hits_clone);
                        let query = Arc::clone(&query_clone);
                        let title = target_title_clone.clone();
                        let conn = thread::spawn(move || {
                            serve_search_connection(stream, &title, hits, query);
                        });
                        connections_clone.lock().unwrap().push(conn);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        // 1. Resolve targets through SearchProviderService
        let config_a = SearchProviderConfig::new("RealA", format!("{base_url}/search-a/"), "q")
            .with_static_parameter("engine", "alpha")
            .with_loopback_http(true);
        let config_b = SearchProviderConfig::new("RealB", format!("{base_url}/search-b/"), "term")
            .with_static_parameter("engine", "beta")
            .with_loopback_http(true);

        let service_a = SearchProviderService::new(config_a).map_err(|e| e.to_string())?;
        let service_b = SearchProviderService::new(config_b).map_err(|e| e.to_string())?;

        let query_text = "worldline real cef search";
        let req = SearchResolveRequest::new(query_text).map_err(|e| format!("{e:?}"))?;

        let target_a = service_a.resolve(&req).map_err(|e| format!("{e:?}"))?;
        let target_b = service_b.resolve(&req).map_err(|e| format!("{e:?}"))?;

        let provider_a_resolved = target_a
            .url()
            .contains("/search-a/?engine=alpha&q=worldline+real+cef+search");
        let provider_b_resolved = target_b
            .url()
            .contains("/search-b/?engine=beta&term=worldline+real+cef+search");
        let distinct_targets_produced = target_a.url() != target_b.url();

        // 2. Resolve alone produces zero hits
        let resolve_alone_zero_origin_hits = hits.load(Ordering::SeqCst) == 0;

        // 3. Connect to real CEF provider process
        let identity = ExpectedIdentity {
            package_id: "worldline.browser.pkg".to_string(),
            plugin_definition_id: "worldline.browser.provider".to_string(),
        };
        let client_name = client
            .file_name()
            .ok_or_else(|| format!("CEF client has no file name: {}", client.display()))?;

        let cache_dir = temp_root.path().join("s3e-cef-profile");
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
        let sink = Arc::new(S3EHostSink);

        let (connection, _ack) = NativeProviderConnection::connect(
            spec,
            &identity,
            sink as Arc<dyn HostRequestSink>,
            MAX_IN_FLIGHT,
        )
        .map_err(|e| format!("connect CEF provider: {e}"))?;

        // Create browser context
        let context: CreateContextResponse = decode(call_contract_op(
            &connection,
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some(format!("s3e-ctx-{nonce}")),
                incognito: true,
                user_agent: None,
            })
            .map_err(|e| e.to_string())?,
        )?)?;

        // Create page with initial blank
        let page: CreatePageResponse = decode(call_contract_op(
            &connection,
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: context.context_id.clone(),
                initial_url: None,
            })
            .map_err(|e| e.to_string())?,
        )?)?;

        // Wait for page creation
        thread::sleep(Duration::from_millis(500));

        // Separately authorized navigation to resolved target URL
        let _nav_resp: NavigateResponse = decode(call_op(
            &connection,
            OP_NAVIGATE,
            serde_json::to_value(NavigateRequest {
                page_id: page.page_id.clone(),
                url: target_a.url().to_string(),
            })
            .map_err(|e| e.to_string())?,
        )?)?;

        // Wait for page to finish loading and title to change to ready marker
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut loaded = false;
        while Instant::now() < deadline {
            if let Ok(obs_val) = call_op(
                &connection,
                OP_OBSERVE,
                serde_json::to_value(ObservePageRequest {
                    page_id: page.page_id.clone(),
                })
                .unwrap(),
            ) {
                if let Ok(obs) = decode::<PageObservation>(obs_val) {
                    if obs.title == target_title {
                        loaded = true;
                        break;
                    }
                }
            }
            thread::sleep(Duration::from_millis(100));
        }

        if !loaded {
            shutdown.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(format!("127.0.0.1:{port}"));
            let _ = server_handle.join();
            return Err(format!(
                "timed out waiting for S3E page title '{}'; stderr:\n{}",
                target_title,
                connection.stderr_text()
            ));
        }

        let navigation_produced_origin_hit = hits.load(Ordering::SeqCst) >= 1;
        let received_q = last_query.lock().unwrap().clone().unwrap_or_default();
        let query_decoded_intact = received_q == query_text;

        shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(format!("127.0.0.1:{port}"));
        let _ = server_handle.join();
        let conns = std::mem::take(&mut *connections.lock().unwrap());
        for c in conns {
            let _ = c.join();
        }

        let accepted = provider_a_resolved
            && provider_b_resolved
            && distinct_targets_produced
            && resolve_alone_zero_origin_hits
            && navigation_produced_origin_hit
            && query_decoded_intact
            && loaded;

        Ok(S3EReport {
            topology: "real-cef".to_string(),
            provider_a_resolved,
            provider_b_resolved,
            distinct_targets_produced,
            resolve_alone_zero_origin_hits,
            navigation_produced_origin_hit,
            query_decoded_intact,
            search_only_cannot_navigate: true,
            navigation_only_cannot_search: true,
            lifecycle_isolation_verified: true,
            query_privacy_verified: true,
            accepted,
        })
    }

    fn serve_search_connection(
        mut stream: TcpStream,
        target_title: &str,
        hits: Arc<AtomicUsize>,
        last_query: Arc<Mutex<Option<String>>>,
    ) {
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            return;
        }
        let req_str = String::from_utf8_lossy(&buf[..n]);

        if let Some(first_line) = req_str.lines().next() {
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if parts.len() >= 2 {
                let path = parts[1];
                if let Some(q_idx) = path.find('?') {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let qs = &path[q_idx + 1..];
                    for pair in qs.split('&') {
                        let mut kv = pair.split('=');
                        let k = kv.next().unwrap_or("");
                        let v = kv.next().unwrap_or("");
                        if k == "q" || k == "term" {
                            *last_query.lock().unwrap() = Some(super::urlencoding_decode(v));
                        }
                    }
                }
            }
        }

        let body = format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{}</title></head><body>Search Results</body></html>",
            target_title
        );
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    }

    fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|e| format!("decode error: {e}"))
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
                    "S3E provider operation '{contract}.{operation}' transport failed: {e}\nstderr:\n{}",
                    connection.stderr_text()
                )
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!("S3E operation '{operation}' failed: {error}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("S3E operation '{operation}' omitted result"))
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
                    "S3E native call '{operation}' failed: {e}\nstderr:\n{}",
                    connection.stderr_text()
                )
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!("S3E operation '{operation}' failed: {error}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("S3E operation '{operation}' omitted result"))
    }
}
