use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

use worldline_browser_contract::{
    action::{ActionResult, InteractionKind},
    error::BrowserError,
    identity::{DocumentRevision, ElementRef, NavigationId, PageId},
    query::{
        AccessibilityNode, AccessibilityRole, AccessibilityTree, DocumentMetadata,
        DocumentSnapshot, QueryBounds,
    },
};

use crate::cdp::CdpSession;

const STARTUP_DEADLINE: Duration = Duration::from_secs(30);
const STARTUP_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const BACKEND_NODE_KEY_PREFIX: &str = "backend-dom-node-";

const INPUT_FUNCTION: &str = r#"function(text) {
    if (!(this instanceof Element) || !("value" in this)) {
        return { found: false };
    }
    this.focus();
    this.value = text;
    this.dispatchEvent(new Event("input", { bubbles: true }));
    this.dispatchEvent(new Event("change", { bubbles: true }));
    return { found: true, value: String(this.value) };
}"#;

const CLICK_FUNCTION: &str = r#"function() {
    if (!(this instanceof Element) || typeof this.click !== "function") {
        return { found: false };
    }
    this.focus();
    this.click();
    return { found: true };
}"#;

const SUBMIT_FUNCTION: &str = r#"function() {
    if (!(this instanceof Element)) {
        return { found: false };
    }
    this.focus();
    if (this instanceof HTMLFormElement && typeof this.requestSubmit === "function") {
        this.requestSubmit();
    } else if (typeof this.click === "function") {
        this.click();
    } else {
        return { found: false };
    }
    return { found: true };
}"#;

const FOCUS_FUNCTION: &str = r#"function() {
    if (!(this instanceof Element)) {
        return { found: false };
    }
    this.focus();
    return { found: document.activeElement === this };
}"#;

#[derive(Clone, Debug)]
struct AxNodeRecord {
    node_id: String,
    parent_id: Option<String>,
    child_ids: Vec<String>,
    role: AccessibilityRole,
    name: Option<String>,
    value: Option<String>,
    description: Option<String>,
    backend_dom_node_id: Option<i64>,
    ignored: bool,
}

fn backend_node_key(backend_dom_node_id: i64) -> String {
    format!("{BACKEND_NODE_KEY_PREFIX}{backend_dom_node_id}")
}

fn backend_dom_node_id(node_key: &str) -> Option<i64> {
    node_key
        .strip_prefix(BACKEND_NODE_KEY_PREFIX)
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
}

fn ax_property_string(node: &serde_json::Value, property_name: &str) -> Option<String> {
    node.get(property_name)
        .and_then(|property| property.get("value"))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn map_ax_role(node: &serde_json::Value) -> AccessibilityRole {
    let role = ax_property_string(node, "role")
        .unwrap_or_else(|| "generic".to_string())
        .to_ascii_lowercase();

    match role.as_str() {
        "rootwebarea" | "webarea" => AccessibilityRole::Root,
        "heading" => AccessibilityRole::Heading,
        "button" => AccessibilityRole::Button,
        "link" => AccessibilityRole::Link,
        "textfield" | "textbox" | "searchbox" | "combobox" | "spinbutton" => {
            AccessibilityRole::TextInput
        }
        "statictext" => AccessibilityRole::StaticText,
        "checkbox" => AccessibilityRole::Checkbox,
        "radiobutton" => AccessibilityRole::Radio,
        "form" => AccessibilityRole::Form,
        "group" => AccessibilityRole::Group,
        "dialog" => AccessibilityRole::Dialog,
        "list" => AccessibilityRole::List,
        "listitem" => AccessibilityRole::ListItem,
        "image" => AccessibilityRole::Image,
        _ => AccessibilityRole::Generic,
    }
}

fn parse_ax_node(node: &serde_json::Value) -> Option<AxNodeRecord> {
    let node_id = node.get("nodeId")?.as_str()?.to_string();
    let child_ids = node
        .get("childIds")
        .and_then(|children| children.as_array())
        .map(|children| {
            children
                .iter()
                .filter_map(|child| child.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let backend_dom_node_id = node
        .get("backendDOMNodeId")
        .and_then(|value| value.as_i64())
        .or_else(|| {
            node.get("backendDOMNodeId")
                .and_then(|value| value.as_u64())
                .and_then(|value| i64::try_from(value).ok())
        })
        .filter(|value| *value > 0);

    Some(AxNodeRecord {
        node_id,
        parent_id: node
            .get("parentId")
            .and_then(|parent| parent.as_str())
            .map(str::to_owned),
        child_ids,
        role: map_ax_role(node),
        name: ax_property_string(node, "name"),
        value: ax_property_string(node, "value"),
        description: ax_property_string(node, "description"),
        backend_dom_node_id,
        ignored: node
            .get("ignored")
            .and_then(|ignored| ignored.as_bool())
            .unwrap_or(false),
    })
}

fn probe_remaining(deadline: Instant) -> Result<Duration, String> {
    let now = Instant::now();
    if now >= deadline {
        return Err("CDP HTTP probe deadline exceeded".to_string());
    }
    Ok(deadline.duration_since(now))
}

fn read_exact_with_deadline(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < buffer.len() {
        let remaining = probe_remaining(deadline)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| format!("failed to set CDP read timeout: {error}"))?;
        let read = stream
            .read(&mut buffer[offset..])
            .map_err(|error| format!("CDP HTTP body read failed: {error}"))?;
        if read == 0 {
            return Err("CDP HTTP response ended before the complete body arrived".to_string());
        }
        offset += read;
    }
    Ok(())
}

fn read_to_end_with_deadline(
    stream: &mut TcpStream,
    max_bytes: usize,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let remaining = probe_remaining(deadline)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| format!("failed to set CDP read timeout: {error}"))?;
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("CDP HTTP body read failed: {error}"))?;
        if read == 0 {
            return Ok(body);
        }
        if body.len().saturating_add(read) > max_bytes {
            return Err("CDP HTTP response exceeded the readiness probe limit".to_string());
        }
        body.extend_from_slice(&chunk[..read]);
    }
}

fn build_ax_node(
    node_id: &str,
    records: &HashMap<String, AxNodeRecord>,
    visiting: &mut HashSet<String>,
    page_id: &PageId,
    document_revision: DocumentRevision,
) -> Option<AccessibilityNode> {
    let record = records.get(node_id)?.clone();
    if !visiting.insert(node_id.to_string()) {
        return None;
    }

    let mut node = AccessibilityNode::new(record.node_id.clone(), record.role);
    if let Some(name) = record.name {
        node = node.with_name(name);
    }
    if let Some(value) = record.value {
        node = node.with_value(value);
    }
    if let Some(description) = record.description {
        node = node.with_description(description);
    }
    if !record.ignored
        && record.role != AccessibilityRole::Root
        && let Some(backend_dom_node_id) = record.backend_dom_node_id
    {
        node = node.with_element_ref(ElementRef::new(
            page_id.clone(),
            document_revision,
            backend_node_key(backend_dom_node_id),
        ));
    }

    for child_id in &record.child_ids {
        if let Some(child) = build_ax_node(child_id, records, visiting, page_id, document_revision)
        {
            node = node.with_child(child);
        }
    }

    visiting.remove(node_id);
    Some(node)
}

/// Information about a discovered local Chromium-compatible browser binary.
#[derive(Clone, Debug)]
pub struct ChromiumBinaryInfo {
    pub executable_path: PathBuf,
    pub browser_name: String,
}

/// Discovers available Chromium or Edge browser executables on the host.
pub fn discover_chromium_binary() -> Result<ChromiumBinaryInfo, String> {
    if let Ok(path) = std::env::var("CHROME_BIN") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(ChromiumBinaryInfo {
                executable_path: p,
                browser_name: "Custom Chrome".to_string(),
            });
        }
    }

    let candidates = [
        (
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "Google Chrome",
        ),
        (
            "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
            "Microsoft Edge",
        ),
        (
            "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
            "Microsoft Edge",
        ),
        (
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
            "Google Chrome",
        ),
    ];

    for (path_str, name) in candidates {
        let p = PathBuf::from(path_str);
        if p.exists() {
            return Ok(ChromiumBinaryInfo {
                executable_path: p,
                browser_name: name.to_string(),
            });
        }
    }

    Err("No Chromium or Edge browser executable found on the host system".to_string())
}

/// Real out-of-process Chromium engine supervisor managing child browser process.
pub struct ChromiumEngineSupervisor {
    binary: ChromiumBinaryInfo,
    child_process: Option<Child>,
    user_data_dir: PathBuf,
    debug_port: u16,
    active_pages: HashMap<PageId, RealPageState>,
    startup_duration: Duration,
}

struct RealPageState {
    page_id: PageId,
    #[allow(dead_code)]
    target_id: String,
    cdp_session: CdpSession,
    current_url: String,
    title: String,
    document_revision: DocumentRevision,
    is_crashed: bool,
}

impl ChromiumEngineSupervisor {
    /// Launches a real out-of-process headless Chromium browser process.
    pub fn spawn() -> Result<Self, String> {
        let binary = discover_chromium_binary()?;
        // Find an open port for debugging
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("failed to bind temporary listener: {e}"))?;
        let debug_port = listener.local_addr().map_err(|e| e.to_string())?.port();
        drop(listener);

        let temp_dir =
            std::env::temp_dir().join(format!("worldline_chromium_spike_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).map_err(|error| {
            format!(
                "failed to create Chromium user-data directory '{}': {error}",
                temp_dir.display()
            )
        })?;

        let boot_start = Instant::now();
        let mut child = match Command::new(&binary.executable_path)
            .arg("--headless=new")
            .arg(format!("--remote-debugging-port={debug_port}"))
            .arg(format!("--user-data-dir={}", temp_dir.display()))
            .arg("--disable-gpu")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-extensions")
            .arg("about:blank")
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_dir_all(&temp_dir);
                return Err(format!(
                    "failed to spawn browser process '{}': {error}",
                    binary.executable_path.display()
                ));
            }
        };

        // Poll the CDP endpoint until the browser has fully published its
        // version document. Hosted runners can take several seconds before
        // the debugging server accepts a complete HTTP response.
        let poll_start = Instant::now();
        let mut ready = false;
        let mut last_probe_error = None;
        while poll_start.elapsed() < STARTUP_DEADLINE {
            if let Ok(Some(status)) = child.try_wait() {
                let error = format!("Chromium exited before CDP readiness with status {status}");
                Self::cleanup_failed_spawn(&mut child, &temp_dir);
                return Err(error);
            }

            match Self::probe_cdp_ready(debug_port) {
                Ok(()) => {
                    ready = true;
                    break;
                }
                Err(error) => last_probe_error = Some(error),
            }

            std::thread::sleep(STARTUP_POLL_INTERVAL);
        }

        if !ready {
            let detail = last_probe_error
                .map(|error| format!("; last readiness probe: {error}"))
                .unwrap_or_default();
            Self::cleanup_failed_spawn(&mut child, &temp_dir);
            return Err(format!(
                "timed out waiting {STARTUP_DEADLINE:?} for Chromium CDP readiness on port {debug_port}{detail}"
            ));
        }

        let startup_duration = boot_start.elapsed();

        Ok(Self {
            binary,
            child_process: Some(child),
            user_data_dir: temp_dir,
            debug_port,
            active_pages: HashMap::new(),
            startup_duration,
        })
    }

    fn cleanup_failed_spawn(child: &mut Child, temp_dir: &Path) {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(temp_dir);
    }

    fn probe_cdp_ready(debug_port: u16) -> Result<(), String> {
        let version =
            Self::http_get_json_from_port(debug_port, "/json/version", STARTUP_PROBE_TIMEOUT)?;
        let browser = version
            .get("Browser")
            .or_else(|| version.get("browser"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "CDP version response has no Browser field".to_string())?;
        let websocket_url = version
            .get("webSocketDebuggerUrl")
            .and_then(|value| value.as_str())
            .filter(|value| value.starts_with("ws://") || value.starts_with("wss://"))
            .ok_or_else(|| "CDP version response has no websocketDebuggerUrl field".to_string())?;

        if browser.is_empty() || websocket_url.is_empty() {
            return Err("CDP version response contains empty readiness fields".to_string());
        }
        Ok(())
    }

    pub fn startup_duration(&self) -> Duration {
        self.startup_duration
    }

    pub fn browser_name(&self) -> &str {
        &self.binary.browser_name
    }

    pub fn is_host_alive(&mut self) -> bool {
        if let Some(child) = &mut self.child_process {
            matches!(child.try_wait(), Ok(None))
        } else {
            false
        }
    }

    /// Measures working set memory footprint (RAM) in bytes of the browser process.
    pub fn measure_memory_bytes(&self) -> Option<u64> {
        if let Some(child) = &self.child_process {
            let pid = child.id();
            // Query memory via powershell Get-Process
            let output = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!("(Get-Process -Id {pid}).WorkingSet64"),
                ])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            text.parse::<u64>().ok()
        } else {
            None
        }
    }

    /// Creates a real browser page attached to CDP.
    pub fn create_page(&mut self, page_id: PageId) -> Result<(), BrowserError> {
        // Query target list from HTTP endpoint
        let targets = self
            .http_get_json("/json")
            .or_else(|_| self.http_get_json("/json/list"))
            .map_err(BrowserError::InvalidRequest)?;
        let page_target = targets
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
                    .or_else(|| arr.first())
            })
            .ok_or_else(|| BrowserError::InvalidRequest("no page targets found".to_string()))?;

        let ws_url = page_target
            .get("webSocketDebuggerUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let target_id = page_target
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("page-target")
            .to_string();
        let ws_path = if let Some(idx) = ws_url.find("/devtools/") {
            ws_url[idx..].to_string()
        } else {
            format!("/devtools/page/{target_id}")
        };

        let mut session =
            CdpSession::connect(self.debug_port, &ws_path).map_err(BrowserError::EngineHung)?;
        session.enable_domains().map_err(BrowserError::EngineHung)?;

        self.active_pages.insert(
            page_id.clone(),
            RealPageState {
                page_id,
                target_id,
                cdp_session: session,
                current_url: "about:blank".to_string(),
                title: "".to_string(),
                document_revision: DocumentRevision::initial(),
                is_crashed: false,
            },
        );
        Ok(())
    }

    /// Navigates a page to local HTML fixture or data URL.
    pub fn navigate(
        &mut self,
        page_id: &PageId,
        url: &str,
    ) -> Result<(NavigationId, DocumentRevision), BrowserError> {
        let page = self
            .active_pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "renderer process has crashed".to_string(),
            ));
        }

        page.cdp_session
            .navigate(url)
            .map_err(BrowserError::NavigationFailed)?;

        // Wait brief duration for document load and title to populate
        let start = Instant::now();
        let mut title = String::new();
        while start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(50));
            match page.cdp_session.evaluate_string("document.title") {
                Ok(t) if !t.is_empty() => {
                    title = t;
                    break;
                }
                _ => {}
            }
        }

        if title.is_empty() {
            title = page
                .cdp_session
                .evaluate_string("document.title")
                .unwrap_or_else(|_| "Untitled".to_string());
        }

        page.current_url = url.to_string();
        page.title = title;
        page.document_revision = page.document_revision.next();

        let nav_id = NavigationId::new(format!("nav-real-{}", page.document_revision.value()));
        Ok((nav_id, page.document_revision))
    }

    /// Extracts real DOM and Blink Accessibility tree via CDP and bounds it.
    pub fn query_document(
        &mut self,
        page_id: &PageId,
        bounds: Option<&QueryBounds>,
    ) -> Result<DocumentSnapshot, BrowserError> {
        let page = self
            .active_pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "renderer process has crashed".to_string(),
            ));
        }

        let ax_raw = page
            .cdp_session
            .get_full_ax_tree()
            .map_err(BrowserError::EngineHung)?;

        let mut records = HashMap::new();
        if let Some(nodes) = ax_raw.get("nodes").and_then(|nodes| nodes.as_array()) {
            for node in nodes {
                if let Some(record) = parse_ax_node(node) {
                    records.insert(record.node_id.clone(), record);
                }
            }
        }

        let root_id = records
            .values()
            .find(|record| record.role == AccessibilityRole::Root)
            .map(|record| record.node_id.clone())
            .or_else(|| {
                records
                    .values()
                    .find(|record| record.parent_id.is_none())
                    .map(|record| record.node_id.clone())
            });

        let mut visiting = HashSet::new();
        let root_node = root_id
            .as_deref()
            .and_then(|id| {
                build_ax_node(id, &records, &mut visiting, page_id, page.document_revision)
            })
            .unwrap_or_else(|| {
                AccessibilityNode::new("blink-root", AccessibilityRole::Root)
                    .with_name(page.title.clone())
            });

        let raw_tree = AccessibilityTree::new(page_id.clone(), page.document_revision, root_node);
        let default_bounds = QueryBounds::default();
        let active_bounds = bounds.unwrap_or(&default_bounds);
        let bounded_tree = raw_tree.to_bounded(active_bounds);

        Ok(DocumentSnapshot::new(
            DocumentMetadata {
                page_id: page_id.clone(),
                url: page.current_url.clone(),
                title: page.title.clone(),
                document_revision: page.document_revision,
                status_code: 200,
            },
            bounded_tree,
        ))
    }

    fn execute_targeted_function(
        page: &mut RealPageState,
        node_key: &str,
        function_declaration: &str,
        arguments: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, BrowserError> {
        let backend_dom_node_id = backend_dom_node_id(node_key)
            .ok_or_else(|| BrowserError::ElementNotFound(node_key.to_string()))?;
        let object_id = page
            .cdp_session
            .resolve_backend_node(backend_dom_node_id)
            .map_err(|_| BrowserError::ElementNotFound(node_key.to_string()))?;
        let result = page
            .cdp_session
            .call_function_on(&object_id, function_declaration, arguments);
        let _ = page.cdp_session.release_object(&object_id);
        result.map_err(BrowserError::InvalidRequest)
    }

    fn require_target_found(
        result: &serde_json::Value,
        node_key: &str,
    ) -> Result<(), BrowserError> {
        if result.get("found").and_then(|value| value.as_bool()) == Some(true) {
            Ok(())
        } else {
            Err(BrowserError::ElementNotFound(node_key.to_string()))
        }
    }

    /// Dispatches an action to the exact DOM node referenced by ElementRef.node_key.
    pub fn execute_action(
        &mut self,
        element_ref: &ElementRef,
        kind: InteractionKind,
        text_payload: Option<&str>,
    ) -> Result<ActionResult, BrowserError> {
        let page = self
            .active_pages
            .get_mut(element_ref.page_id())
            .ok_or_else(|| BrowserError::PageNotFound(element_ref.page_id().clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "renderer process has crashed".to_string(),
            ));
        }

        if element_ref.document_revision() != page.document_revision {
            return Err(BrowserError::StaleElementReference {
                expected_revision: element_ref.document_revision(),
                actual_revision: page.document_revision,
            });
        }

        let node_key = element_ref.node_key();
        match kind {
            InteractionKind::Input => {
                let text = text_payload.unwrap_or_default();
                let result = Self::execute_targeted_function(
                    page,
                    node_key,
                    INPUT_FUNCTION,
                    vec![serde_json::json!({ "value": text })],
                )?;
                Self::require_target_found(&result, node_key)?;

                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some("input set on targeted element".to_string()),
                })
            }
            InteractionKind::Click | InteractionKind::Submit => {
                let function_declaration = match kind {
                    InteractionKind::Click => CLICK_FUNCTION,
                    InteractionKind::Submit => SUBMIT_FUNCTION,
                    _ => unreachable!("the match arm only contains click or submit"),
                };
                let result = Self::execute_targeted_function(
                    page,
                    node_key,
                    function_declaration,
                    Vec::new(),
                )?;
                Self::require_target_found(&result, node_key)?;

                page.document_revision = page.document_revision.next();
                page.title = page
                    .cdp_session
                    .evaluate_string("document.title")
                    .unwrap_or_else(|_| "Updated".to_string());

                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some("action executed on targeted element".to_string()),
                })
            }
            InteractionKind::Focus => {
                let result =
                    Self::execute_targeted_function(page, node_key, FOCUS_FUNCTION, Vec::new())?;
                Self::require_target_found(&result, node_key)?;

                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some("focused targeted element".to_string()),
                })
            }
            InteractionKind::Scroll => {
                page.cdp_session
                    .evaluate_string("window.scrollBy(0, 100); 'scrolled'")
                    .map_err(BrowserError::InvalidRequest)?;
                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some("scrolled viewport".to_string()),
                })
            }
        }
    }

    /// Deliberately crashes the page renderer process via CDP Page.crash.
    pub fn crash_renderer(&mut self, page_id: &PageId) -> Result<(), BrowserError> {
        let page = self
            .active_pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        let _ = page.cdp_session.crash_renderer();
        page.is_crashed = true;
        Ok(())
    }

    fn http_get_json(&self, path: &str) -> Result<serde_json::Value, String> {
        Self::http_get_json_from_port(self.debug_port, path, Duration::from_secs(3))
    }

    fn http_get_json_from_port(
        port: u16,
        path: &str,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let probe_deadline = Instant::now() + timeout;
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&address, timeout)
            .map_err(|error| format!("failed to connect to CDP HTTP endpoint: {error}"))?;
        let remaining = probe_remaining(probe_deadline)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| format!("failed to set CDP read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(remaining))
            .map_err(|error| format!("failed to set CDP write timeout: {error}"))?;

        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .map_err(|error| format!("failed to write CDP HTTP request: {error}"))?;

        let mut header_bytes = Vec::with_capacity(1024);
        let mut byte = [0u8; 1];
        while !header_bytes.ends_with(b"\r\n\r\n") {
            if header_bytes.len() >= MAX_HTTP_HEADER_BYTES {
                return Err("CDP HTTP headers exceeded the readiness probe limit".to_string());
            }
            read_exact_with_deadline(&mut stream, &mut byte, probe_deadline)?;
            header_bytes.push(byte[0]);
        }

        let header_str = String::from_utf8_lossy(&header_bytes);
        let status_line = header_str.lines().next().unwrap_or_default();
        let status_code = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| format!("invalid CDP HTTP status line: {status_line}"))?;
        if status_code != 200 {
            return Err(format!("CDP HTTP endpoint returned status {status_code}"));
        }

        let content_length = header_str.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|error| format!("invalid CDP Content-Length header: {error}")),
                )
            } else {
                None
            }
        });
        let content_length = match content_length {
            Some(result) => Some(result?),
            None => None,
        };

        let body = if let Some(content_length) = content_length {
            if content_length > MAX_HTTP_RESPONSE_BYTES {
                return Err("CDP HTTP response exceeded the readiness probe limit".to_string());
            }
            let mut body = vec![0u8; content_length];
            read_exact_with_deadline(&mut stream, &mut body, probe_deadline)?;
            body
        } else {
            read_to_end_with_deadline(&mut stream, MAX_HTTP_RESPONSE_BYTES, probe_deadline)?
        };

        let _ = stream.shutdown(std::net::Shutdown::Both);
        serde_json::from_slice(&body).map_err(|error| format!("failed to parse CDP JSON: {error}"))
    }
}

impl Drop for ChromiumEngineSupervisor {
    fn drop(&mut self) {
        if let Some(mut child) = self.child_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.user_data_dir);
    }
}
