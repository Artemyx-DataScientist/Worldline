use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
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
        let temp_dir =
            std::env::temp_dir().join(format!("worldline_chromium_spike_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        // Find an open port for debugging
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("failed to bind temporary listener: {e}"))?;
        let debug_port = listener.local_addr().map_err(|e| e.to_string())?.port();
        drop(listener);

        let boot_start = Instant::now();
        let child = Command::new(&binary.executable_path)
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
            .map_err(|e| {
                format!(
                    "failed to spawn browser process '{}': {e}",
                    binary.executable_path.display()
                )
            })?;

        // Poll CDP endpoint until ready
        let poll_start = Instant::now();
        let mut ready = false;
        while poll_start.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(50));
            if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{debug_port}")) {
                stream
                    .set_read_timeout(Some(Duration::from_millis(200)))
                    .ok();
                let req = format!(
                    "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{debug_port}\r\nConnection: close\r\n\r\n"
                );
                if stream.write_all(req.as_bytes()).is_ok() {
                    let mut buf = [0u8; 256];
                    if let Ok(n @ 1..) = stream.read(&mut buf) {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        if text.contains("200 OK") {
                            let _ = stream.shutdown(std::net::Shutdown::Both);
                            ready = true;
                            break;
                        }
                    }
                }
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }

        if !ready {
            return Err(format!(
                "timed out waiting for Chromium process on port {debug_port}"
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

    /// Extracts real Blink Accessibility tree via CDP and bounds it.
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

        // Map Chromium AX nodes to Worldline AccessibilityTree
        let mut root_node = AccessibilityNode::new("blink-root", AccessibilityRole::Root)
            .with_name(page.title.clone());

        if let Some(nodes) = ax_raw.get("nodes").and_then(|n| n.as_array()) {
            for (idx, ax_node) in nodes.iter().enumerate() {
                let role_str = ax_node
                    .pointer("/role/value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("generic");
                let name = ax_node
                    .pointer("/name/value")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let value = ax_node
                    .pointer("/value/value")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let role = match role_str {
                    "heading" => AccessibilityRole::Heading,
                    "button" => AccessibilityRole::Button,
                    "link" => AccessibilityRole::Link,
                    "textField" => AccessibilityRole::TextInput,
                    "StaticText" => AccessibilityRole::StaticText,
                    "form" => AccessibilityRole::Form,
                    _ => AccessibilityRole::Generic,
                };

                let elem_ref = ElementRef::new(
                    page_id.clone(),
                    page.document_revision,
                    format!("ax-node-{idx}"),
                );

                let mut child_node = AccessibilityNode::new(format!("ax-node-{idx}"), role)
                    .with_element_ref(elem_ref);
                if let Some(n) = name {
                    child_node = child_node.with_name(n);
                }
                if let Some(v) = value {
                    child_node = child_node.with_value(v);
                }

                root_node = root_node.with_child(child_node);
            }
        }

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

    /// Dispatches real click/input action via JS evaluation / CDP.
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

        match kind {
            InteractionKind::Input => {
                let text = text_payload.unwrap_or_default();
                // Set value in the first input field on the page
                page.cdp_session
                    .evaluate_string(&format!(
                        "(() => {{ const el = document.querySelector('input') || document.body; el.value = '{text}'; el.dispatchEvent(new Event('input', {{ bubbles: true }})); return el.value; }})()"
                    ))
                    .map_err(BrowserError::InvalidRequest)?;

                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some(format!("input set to '{text}'")),
                })
            }
            InteractionKind::Click | InteractionKind::Submit => {
                page.cdp_session
                    .evaluate_string(
                        "(() => { const btn = document.querySelector('button') || document.body; btn.click(); return 'clicked'; })()",
                    )
                    .map_err(BrowserError::InvalidRequest)?;

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
                    message: Some("clicked element".to_string()),
                })
            }
            InteractionKind::Focus => {
                page.cdp_session
                    .evaluate_string(
                        "(() => { const el = document.querySelector('input, button'); if (el) el.focus(); return 'focused'; })()",
                    )
                    .map_err(BrowserError::InvalidRequest)?;
                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some("focused element".to_string()),
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
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", self.debug_port))
            .map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(3))).ok();
        let port = self.debug_port;
        let req =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(req.as_bytes())
            .map_err(|e| e.to_string())?;

        let mut header_bytes = Vec::new();
        let mut byte = [0u8; 1];
        while header_bytes.len() < 4096 {
            stream
                .read_exact(&mut byte)
                .map_err(|e| format!("header read failed: {e}"))?;
            header_bytes.push(byte[0]);
            if header_bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }

        let header_str = String::from_utf8_lossy(&header_bytes);
        let content_length = header_str
            .lines()
            .find_map(|line| {
                if line.to_ascii_lowercase().starts_with("content-length:") {
                    line.split(':')
                        .nth(1)
                        .and_then(|v| v.trim().parse::<usize>().ok())
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            stream
                .read_exact(&mut body)
                .map_err(|e| format!("body read failed: {e}"))?;
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);

        serde_json::from_slice(&body).map_err(|e| format!("failed to parse JSON response: {e}"))
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
