//! Production CEF implementation of the engine-neutral `BrowserBackend`.
//!
//! Only logical Worldline identifiers and serialized contract values leave
//! this module. CEF reference-counted objects are created, used, and released
//! on the CEF UI thread through [`CefLoopRunner`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use url::Url;
use worldline_browser_contract::action::ActionResult;
use worldline_browser_contract::capture::{
    CapturePageRequest, CapturePageResponse, ReadCaptureArtifactRequest,
    ReadCaptureArtifactResponse,
};
use worldline_browser_contract::contracts::{
    ActRequest, CloseContextRequest, CloseContextResponse, ClosePageRequest, ClosePageResponse,
    ControlDownloadRequest, CreateContextRequest, CreateContextResponse, CreatePageRequest,
    CreatePageResponse, DownloadState, DownloadStatusResponse, HistoryNavRequest,
    HistoryNavResponse, ListContextsResponse, ListPagesRequest, ListPagesResponse, LoadingState,
    NavigateRequest, NavigateResponse, ObservePageRequest, PageObservation, PermissionResponse,
    QueryDocumentRequest, QueryPermissionRequest, ReloadRequest, ReloadResponse,
    SetPermissionRequest, StartDownloadRequest, StopRequest, StopResponse, ViewportInfo,
};
use worldline_browser_contract::error::BrowserError;
use worldline_browser_contract::identity::{
    BrowserContextId, DocumentRevision, DownloadId, NavigationId, PageId,
};
use worldline_browser_contract::primitives::{
    ClearStorageRequest, ClearStorageResponse, CookieV0_2, DeleteCookiesRequest,
    DeleteCookiesResponse, GetCookiesRequest, GetCookiesResponse, GetCookiesResponseV0_2,
    SetCookieRequest, SetCookieRequestV0_2, SetCookieResponse, StorageItemRequestV0_2,
    StorageItemResponseV0_2, StorageType,
};
use worldline_browser_contract::query::DocumentSnapshot;
use worldline_browser_provider::BrowserBackend;

use crate::ffi::CefSettings;
use crate::loop_runner::CefLoopRunner;

const CEF_CALLBACK_TIMEOUT: Duration = Duration::from_secs(5);
const WINDOWS_EPOCH_OFFSET_SECONDS: u64 = 11_644_473_600;
const MICROSECONDS_PER_SECOND: i64 = 1_000_000;
static STORAGE_MARKER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
struct ContextState {
    incognito: bool,
    user_agent: Option<String>,
    cache_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PageState {
    context_id: BrowserContextId,
    url: String,
    title: String,
    revision: DocumentRevision,
    loading_state: LoadingState,
    status_code: u16,
    browser_id: i32,
    history: Vec<String>,
    history_idx: usize,
    crashed: bool,
}

#[derive(Clone, Debug)]
struct PendingDownload {
    context_id: BrowserContextId,
    page_id: PageId,
    url: String,
    destination_path: PathBuf,
    suggested_filename: String,
}

#[derive(Clone, Debug)]
struct ActiveDownload {
    download_id: DownloadId,
    destination_path: PathBuf,
}

/// Engine event emitted by the actual CEF download handler.
///
/// The content is retained only until the provider integration transfers it
/// to the generic host blob boundary. It is never used as a service-local
/// authoritative artifact store.
#[derive(Clone, Debug)]
pub enum CefDownloadEvent {
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

struct DownloadShared {
    pages: Arc<Mutex<BTreeMap<PageId, PageState>>>,
    pending: Mutex<BTreeMap<DownloadId, PendingDownload>>,
    active: Mutex<BTreeMap<u32, ActiveDownload>>,
    events: Mutex<Vec<CefDownloadEvent>>,
    download_root: PathBuf,
}

impl DownloadShared {
    fn new(pages: Arc<Mutex<BTreeMap<PageId, PageState>>>, download_root: PathBuf) -> Self {
        Self {
            pages,
            pending: Mutex::new(BTreeMap::new()),
            active: Mutex::new(BTreeMap::new()),
            events: Mutex::new(Vec::new()),
            download_root,
        }
    }

    fn page_id_for_browser(&self, browser_id: i32) -> Option<PageId> {
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find_map(|(page_id, page)| (page.browser_id == browser_id).then(|| page_id.clone()))
    }

    fn push_event(&self, event: CefDownloadEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
    }

    fn drain_events(&self) -> Vec<CefDownloadEvent> {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *events)
    }
}

#[cfg(windows)]
#[derive(Clone)]
struct PageCallbackState {
    page_id: PageId,
    pages: Arc<Mutex<BTreeMap<PageId, PageState>>>,
}

#[cfg(windows)]
use cef::rc::Rc;
#[cfg(windows)]
use cef::*;
#[cfg(windows)]
use cef::{
    wrap_client, wrap_cookie_visitor, wrap_delete_cookies_callback, wrap_display_handler,
    wrap_download_handler, wrap_life_span_handler, wrap_load_handler, wrap_set_cookie_callback,
};

#[cfg(windows)]
fn cef_string(value: &CefStringUserfree) -> String {
    CefString::from(value).to_string()
}

#[cfg(windows)]
fn browser_id(browser: Option<&mut Browser>) -> Option<i32> {
    browser.map(|browser| browser.identifier())
}

#[cfg(windows)]
fn frame_url(frame: Option<&mut Frame>) -> Option<String> {
    frame.map(|frame| cef_string(&frame.url()))
}

#[cfg(windows)]
fn update_page<F>(state: &PageCallbackState, update: F)
where
    F: FnOnce(&mut PageState),
{
    if let Some(page) = state
        .pages
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_mut(&state.page_id)
    {
        update(page);
    }
}

#[cfg(windows)]
wrap_life_span_handler! {
    struct WorldlineLifeSpanHandler { state: PageCallbackState }
    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(id) = browser_id(browser) {
                update_page(&self.state, |page| page.browser_id = id);
            }
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            if browser_id(browser).is_some() {
                update_page(&self.state, |page| page.browser_id = -1);
            }
        }
    }
}

#[cfg(windows)]
wrap_load_handler! {
    struct WorldlineLoadHandler { state: PageCallbackState }
    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            browser: Option<&mut Browser>,
            is_loading: i32,
            _can_go_back: i32,
            _can_go_forward: i32,
        ) {
            if browser_id(browser).is_some() {
                update_page(&self.state, |page| {
                    page.loading_state = if is_loading != 0 {
                        LoadingState::Loading
                    } else {
                        LoadingState::Complete
                    };
                });
            }
        }

        fn on_load_start(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _transition_type: TransitionType,
        ) {
            if browser_id(browser).is_some() {
                let url = frame_url(frame);
                update_page(&self.state, |page| {
                    page.loading_state = LoadingState::Loading;
                    if let Some(url) = url {
                        page.url = url;
                    }
                });
            }
        }

        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            http_status_code: i32,
        ) {
            if browser_id(browser).is_some() {
                let url = frame_url(frame);
                update_page(&self.state, |page| {
                    page.loading_state = LoadingState::Complete;
                    page.status_code = http_status_code.clamp(0, u16::MAX as i32) as u16;
                    if let Some(url) = url {
                        page.url = url;
                    }
                });
            }
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            if browser_id(browser).is_some() {
                let url = failed_url.map(ToString::to_string).or_else(|| frame_url(frame));
                update_page(&self.state, |page| {
                    page.loading_state = LoadingState::Failed;
                    page.status_code = 0;
                    if let Some(url) = url {
                        page.url = url;
                    }
                    page.crashed = false;
                });
                let _ = (error_text, _error_code);
            }
        }
    }
}

#[cfg(windows)]
wrap_display_handler! {
    struct WorldlineDisplayHandler { state: PageCallbackState }
    impl DisplayHandler {
        fn on_address_change(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if browser_id(browser).is_some()
                && let Some(url) = url
            {
                let url = url.to_string();
                update_page(&self.state, |page| page.url = url);
            }
        }

        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            if browser_id(browser).is_some()
                && let Some(title) = title
            {
                let title = title.to_string();
                update_page(&self.state, |page| page.title = title);
            }
        }
    }
}

#[cfg(windows)]
wrap_client! {
    struct WorldlineClient {
        state: PageCallbackState,
        downloads: Arc<DownloadShared>,
    }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(WorldlineLifeSpanHandler::new(self.state.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(WorldlineLoadHandler::new(self.state.clone()))
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(WorldlineDisplayHandler::new(self.state.clone()))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(WorldlineDownloadHandler::new(self.downloads.clone()))
        }
    }
}

#[cfg(windows)]
fn suggested_filename(value: Option<&CefString>, fallback: &str) -> String {
    value
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(windows)]
fn safe_filename(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .unwrap_or("download.bin")
        .to_string()
}

#[cfg(windows)]
wrap_download_handler! {
    struct WorldlineDownloadHandler { downloads: Arc<DownloadShared> }
    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> i32 {
            1
        }

        fn on_before_download(
            &self,
            browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            let Some(item) = download_item else { return 0 };
            let item_id = item.id();
            let Some(page_id) = browser_id(browser) else { return 0 };
            let Some(page_id) = self.downloads.page_id_for_browser(page_id) else { return 0 };
            let mut pending = self
                .downloads
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some((download_id, request)) = pending
                .iter()
                .find(|(_, request)| request.page_id == page_id)
                .map(|(id, request)| (id.clone(), request.clone()))
            else {
                return 0;
            };
            pending.remove(&download_id);
            drop(pending);

            let name = safe_filename(&suggested_filename(
                suggested_name,
                &request.suggested_filename,
            ));
            let destination = if request.destination_path.as_os_str().is_empty() {
                self.downloads
                    .download_root
                    .join(format!("{}-{name}", download_id.as_str()))
            } else {
                request.destination_path.clone()
            };
            let destination_string = destination.to_string_lossy().to_string();
            if let Some(callback) = callback {
                let destination = CefString::from(destination_string.as_str());
                callback.cont(Some(&destination), 0);
            }

            self.downloads
                .active
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    item_id,
                    ActiveDownload {
                        download_id: download_id.clone(),
                        destination_path: destination,
                    },
                );
            self.downloads.push_event(CefDownloadEvent::Started {
                download_id,
                context_id: request.context_id,
                page_id,
                url: request.url,
                suggested_filename: name,
                total_bytes: positive_u64(item.total_bytes()),
                mime_type: None,
            });
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(item) = download_item else { return };
            if let Some(callback) = callback
                && item.is_paused() != 0
            {
                callback.pause();
            }
            let item_id = item.id();
            let active = self
                .downloads
                .active
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&item_id)
                .cloned();
            let Some(active) = active else { return };
            let received_bytes = positive_u64(item.received_bytes()).unwrap_or_default();
            let total_bytes = positive_u64(item.total_bytes());

            if item.is_complete() != 0 {
                let path = {
                    let full_path = item.full_path();
                    let actual_path = cef_string(&full_path);
                    if actual_path.is_empty() {
                        active.destination_path.clone()
                    } else {
                        PathBuf::from(actual_path)
                    }
                };
                match std::fs::read(&path) {
                    Ok(content) => self.downloads.push_event(CefDownloadEvent::Completed {
                        download_id: active.download_id.clone(),
                        content,
                        mime_type: {
                            let mime = cef_string(&item.mime_type());
                            (!mime.is_empty()).then_some(mime)
                        },
                    }),
                    Err(error) => self.downloads.push_event(CefDownloadEvent::Failed {
                        download_id: active.download_id.clone(),
                        error: format!("CEF completed download could not be read: {error}"),
                    }),
                }
                self.downloads
                    .active
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&item_id);
            } else if item.is_canceled() != 0 || item.is_interrupted() != 0 {
                self.downloads.push_event(CefDownloadEvent::Failed {
                    download_id: active.download_id.clone(),
                    error: "CEF download was cancelled or interrupted".to_string(),
                });
                self.downloads
                    .active
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&item_id);
            } else {
                self.downloads.push_event(CefDownloadEvent::Progress {
                    download_id: active.download_id,
                    received_bytes,
                    total_bytes,
                });
            }
        }
    }
}

#[cfg(windows)]
fn positive_u64(value: i64) -> Option<u64> {
    (value >= 0).then_some(value as u64)
}

#[cfg(windows)]
fn native_request_context(context: &ContextState) -> Result<RequestContext, String> {
    let mut settings = RequestContextSettings::default();
    if !context.incognito
        && let Some(path) = &context.cache_path
    {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("create CEF profile directory: {error}"))?;
        settings.cache_path = CefString::from(path.to_string_lossy().as_ref());
    }
    settings.persist_session_cookies = 1;
    let request_context = request_context_create_context(Some(&settings), None)
        .ok_or_else(|| "cef_request_context_create_context returned null".to_string())?;
    if let Some(user_agent) = &context.user_agent {
        let preference_name = CefString::from("general.useragent");
        let user_agent = CefString::from(user_agent.as_str());
        let mut preference_value = value_create()
            .ok_or_else(|| "cef_value_create returned null for user agent".to_string())?;
        if preference_value.set_string(Some(&user_agent)) == 0 {
            return Err("CEF rejected the user-agent preference value".to_string());
        }
        let mut error = CefString::from("");
        if request_context.set_preference(
            Some(&preference_name),
            Some(&mut preference_value),
            Some(&mut error),
        ) == 0
        {
            return Err(format!("CEF rejected the user-agent preference: {error}"));
        }
    }
    Ok(request_context)
}

#[cfg(windows)]
fn native_context(
    contexts: &Arc<Mutex<BTreeMap<BrowserContextId, RequestContext>>>,
    context_id: &BrowserContextId,
) -> Result<RequestContext, String> {
    contexts
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(context_id)
        .cloned()
        .ok_or_else(|| format!("CEF request context '{context_id}' is no longer available"))
}

#[cfg(windows)]
fn native_page_context(
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    context_id: &BrowserContextId,
) -> Result<RequestContext, String> {
    let browser_id = pages
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .find(|page| page.context_id == *context_id && page.browser_id >= 0)
        .map(|page| page.browser_id)
        .ok_or_else(|| format!("CEF context '{context_id}' has no live browser page"))?;
    native_browser(browser_id)?
        .host()
        .ok_or_else(|| "CEF browser has no host".to_string())?
        .request_context()
        .ok_or_else(|| "CEF browser host has no request context".to_string())
}

#[cfg(windows)]
fn native_cookie_context(
    contexts: &Arc<Mutex<BTreeMap<BrowserContextId, RequestContext>>>,
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    context_id: &BrowserContextId,
) -> Result<RequestContext, String> {
    native_page_context(pages, context_id).or_else(|_| native_context(contexts, context_id))
}

#[cfg(windows)]
fn native_browser(browser_id: i32) -> Result<Browser, String> {
    browser_host_get_browser_by_identifier(browser_id)
        .ok_or_else(|| format!("CEF browser {browser_id} is no longer available"))
}

#[cfg(windows)]
fn native_create_browser(
    contexts: &Arc<Mutex<BTreeMap<BrowserContextId, RequestContext>>>,
    context_id: &BrowserContextId,
    initial_url: &str,
    callback_state: PageCallbackState,
    downloads: Arc<DownloadShared>,
) -> Result<(), String> {
    let mut request_context = native_context(contexts, context_id)?;
    let mut client = WorldlineClient::new(callback_state, downloads);
    // S3B is deliberately a native headful CEF path.  The browser is created
    // by CEF itself; this adapter does not substitute an off-screen or
    // reference browser when the hosted proving slice exercises it.
    let window_info = WindowInfo::default().set_as_popup(
        cef::sys::HWND(std::ptr::null_mut()),
        "Worldline hosted proving browser",
    );
    let browser_settings = BrowserSettings::default();
    let url = CefString::from(initial_url);
    let accepted = browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&url),
        Some(&browser_settings),
        None,
        Some(&mut request_context),
    );
    if accepted == 0 {
        return Err("cef_browser_host_create_browser rejected the browser request".to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn wait_for_browser_id(
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    page_id: &PageId,
) -> Result<i32, String> {
    let deadline = Instant::now() + CEF_CALLBACK_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(browser_id) = pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(page_id)
            .map(|page| page.browser_id)
            .filter(|browser_id| *browser_id >= 0)
        {
            return Ok(browser_id);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err("timed out waiting for CEF browser creation callback".to_string())
}

#[cfg(not(windows))]
fn wait_for_browser_id(
    _pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    _page_id: &PageId,
) -> Result<i32, String> {
    Err("CEF browser creation is only available on Windows".to_string())
}

#[cfg(windows)]
fn native_close_browser(browser_id: i32) -> Result<(), String> {
    let browser = native_browser(browser_id)?;
    let host = browser
        .host()
        .ok_or_else(|| "CEF browser has no host".to_string())?;
    host.close_browser(1);
    Ok(())
}

#[cfg(windows)]
fn native_load_url(browser_id: i32, url: &str) -> Result<(), String> {
    let browser = native_browser(browser_id)?;
    let frame = browser
        .main_frame()
        .ok_or_else(|| "CEF browser has no main frame".to_string())?;
    let url = CefString::from(url);
    frame.load_url(Some(&url));
    Ok(())
}

#[cfg(windows)]
fn wait_for_flag(flag: &AtomicBool, deadline: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    flag.load(Ordering::SeqCst)
}

#[cfg(windows)]
fn native_cookie_from_contract(cookie: &CookieV0_2) -> Result<Cookie, String> {
    let mut native = Cookie {
        name: CefString::from(cookie.name.as_str()),
        value: CefString::from(cookie.value.as_str()),
        ..Cookie::default()
    };
    let domain = cookie
        .domain
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.');
    if domain.is_empty() {
        return Err("cookie domain is empty".to_string());
    }
    // CEF represents a host-only cookie with an empty domain. Domain cookies
    // use a leading dot, which Chromium preserves when visiting the store.
    let native_domain = if cookie.host_only {
        String::new()
    } else {
        format!(".{domain}")
    };
    native.domain = CefString::from(native_domain.as_str());
    native.path = CefString::from(cookie.path.as_str());
    native.secure = i32::from(cookie.secure);
    native.httponly = i32::from(cookie.http_only);
    if let Some(expires) = cookie.expires_epoch_sec {
        let windows_epoch_seconds = expires.saturating_add(WINDOWS_EPOCH_OFFSET_SECONDS);
        if let Ok(windows_epoch_seconds) = i64::try_from(windows_epoch_seconds) {
            native.has_expires = 1;
            native.expires = Basetime {
                val: windows_epoch_seconds.saturating_mul(MICROSECONDS_PER_SECOND),
            };
        }
    }
    native.same_site = match cookie.same_site.as_deref() {
        Some(value) if value.eq_ignore_ascii_case("strict") => CookieSameSite::STRICT_MODE,
        Some(value) if value.eq_ignore_ascii_case("lax") => CookieSameSite::LAX_MODE,
        Some(value)
            if value.eq_ignore_ascii_case("none")
                || value.eq_ignore_ascii_case("no_restriction") =>
        {
            CookieSameSite::NO_RESTRICTION
        }
        _ => CookieSameSite::UNSPECIFIED,
    };
    Ok(native)
}

#[cfg(windows)]
fn contract_cookie_from_native(cookie: &Cookie) -> CookieV0_2 {
    let domain = cookie.domain.to_string();
    let host_only = !domain.starts_with('.');
    CookieV0_2 {
        name: cookie.name.to_string(),
        value: cookie.value.to_string(),
        domain,
        path: cookie.path.to_string(),
        secure: cookie.secure != 0,
        http_only: cookie.httponly != 0,
        same_site: match cookie.same_site.get_raw() {
            value if value == CookieSameSite::STRICT_MODE.get_raw() => Some("Strict".to_string()),
            value if value == CookieSameSite::LAX_MODE.get_raw() => Some("Lax".to_string()),
            value if value == CookieSameSite::NO_RESTRICTION.get_raw() => Some("None".to_string()),
            _ => None,
        },
        expires_epoch_sec: (cookie.has_expires != 0 && cookie.expires.val > 0)
            .then_some(cookie.expires.val / MICROSECONDS_PER_SECOND)
            .and_then(|windows_epoch_seconds| {
                (windows_epoch_seconds >= WINDOWS_EPOCH_OFFSET_SECONDS as i64)
                    .then_some((windows_epoch_seconds - WINDOWS_EPOCH_OFFSET_SECONDS as i64) as u64)
            }),
        host_only,
    }
}

#[cfg(windows)]
wrap_cookie_visitor! {
    struct WorldlineCookieVisitor {
        cookies: Arc<Mutex<Vec<CookieV0_2>>>,
        completed: Arc<AtomicBool>,
    }
    impl CookieVisitor {
        fn visit(
            &self,
            cookie: Option<&Cookie>,
            count: i32,
            total: i32,
            _delete_cookie: Option<&mut i32>,
        ) -> i32 {
            if let Some(cookie) = cookie {
                self.cookies
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(contract_cookie_from_native(cookie));
            }
            if cookie.is_none() || count.saturating_add(1) >= total {
                self.completed.store(true, Ordering::SeqCst);
            }
            1
        }
    }
}

#[cfg(windows)]
wrap_set_cookie_callback! {
    struct WorldlineSetCookieCallback {
        completed: Arc<AtomicBool>,
        success: Arc<AtomicBool>,
    }
    impl SetCookieCallback {
        fn on_complete(&self, success: i32) {
            self.success.store(success != 0, Ordering::SeqCst);
            self.completed.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(windows)]
wrap_completion_callback! {
    struct WorldlineCookieManagerReadyCallback {
        completed: Arc<AtomicBool>,
    }
    impl CompletionCallback {
        fn on_complete(&self) {
            self.completed.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(windows)]
wrap_delete_cookies_callback! {
    struct WorldlineDeleteCookiesCallback {
        completed: Arc<AtomicBool>,
        deleted: Arc<AtomicI32>,
    }
    impl DeleteCookiesCallback {
        fn on_complete(&self, deleted: i32) {
            self.deleted.store(deleted, Ordering::SeqCst);
            self.completed.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(windows)]
fn native_prepare_cookie_manager(
    context: &ContextState,
) -> Result<(RequestContext, Arc<AtomicBool>), String> {
    let context = native_request_context(context)?;
    let completed = Arc::new(AtomicBool::new(false));
    let mut callback = WorldlineCookieManagerReadyCallback::new(Arc::clone(&completed));
    context
        .cookie_manager(Some(&mut callback))
        .ok_or_else(|| "CEF request context has no cookie manager".to_string())?;
    Ok((context, completed))
}

#[cfg(windows)]
fn cookie_scope_url(url: Option<&str>, domain: Option<&str>) -> Result<Option<String>, String> {
    if let Some(url) = url {
        let parsed = Url::parse(url).map_err(|error| format!("invalid cookie URL: {error}"))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err("cookie URL must use an HTTP(S) host target".to_string());
        }
        return Ok(Some(url.to_string()));
    }
    domain
        .map(|domain| {
            let host = domain.trim().trim_start_matches('.').trim_end_matches('.');
            if host.is_empty() || host.contains('/') || host.contains(char::is_whitespace) {
                return Err("cookie domain is not a valid host target".to_string());
            }
            Ok(format!("https://{host}/"))
        })
        .transpose()
}

#[cfg(windows)]
type NativeCookieReadState = (
    Arc<Mutex<Vec<CookieV0_2>>>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
);

#[cfg(windows)]
fn native_get_cookies_start(
    contexts: &Arc<Mutex<BTreeMap<BrowserContextId, RequestContext>>>,
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    context_id: &BrowserContextId,
    request: &GetCookiesRequest,
) -> Result<NativeCookieReadState, String> {
    let context = native_cookie_context(contexts, pages, context_id)?;
    let manager = context
        .cookie_manager(None)
        .ok_or_else(|| "CEF request context has no cookie manager".to_string())?;
    let scope = cookie_scope_url(request.url.as_deref(), request.domain.as_deref())?;
    let cookies = Arc::new(Mutex::new(Vec::new()));
    let completed = Arc::new(AtomicBool::new(false));
    let mut visitor = WorldlineCookieVisitor::new(Arc::clone(&cookies), Arc::clone(&completed));
    let accepted = if let Some(scope) = scope.filter(|_| request.url.is_some()) {
        let scope = CefString::from(scope.as_str());
        manager.visit_url_cookies(Some(&scope), 1, Some(&mut visitor))
    } else {
        manager.visit_all_cookies(Some(&mut visitor))
    };
    if accepted == 0 {
        return Err("CEF rejected cookie visitor request".to_string());
    }
    // CEF explicitly permits a cookie visitor not to be called when no
    // matching cookies exist. A flush completion gives the adapter a native
    // store-readiness signal so that this documented empty result is not
    // confused with an uninitialized or dead engine.
    let store_ready = Arc::new(AtomicBool::new(false));
    let mut ready_callback = WorldlineCookieManagerReadyCallback::new(Arc::clone(&store_ready));
    if manager.flush_store(Some(&mut ready_callback)) == 0 {
        return Err("CEF rejected cookie store readiness request".to_string());
    }
    Ok((cookies, completed, store_ready))
}

#[cfg(windows)]
fn native_set_cookie_start(
    contexts: &Arc<Mutex<BTreeMap<BrowserContextId, RequestContext>>>,
    context_id: &BrowserContextId,
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    request: &SetCookieRequestV0_2,
) -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), String> {
    let context = native_cookie_context(contexts, pages, context_id)?;
    let manager = context
        .cookie_manager(None)
        .ok_or_else(|| "CEF request context has no cookie manager".to_string())?;
    let host = request
        .cookie
        .domain
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.');
    if host.is_empty() {
        return Err("missing cookie scope".to_string());
    }
    // The engine-neutral SetCookieRequest carries a domain but no URL. Prefer
    // the live page origin for this context so the native primitive receives
    // the exact scheme/host/port that owns the cookie. Fall back to a plain
    // HTTP host URL only when the context has no live page.
    let scope = pages
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .find(|page| {
            page.context_id == *context_id
                && Url::parse(&page.url)
                    .ok()
                    .and_then(|url| url.host_str().map(|value| value.eq_ignore_ascii_case(host)))
                    .unwrap_or(false)
        })
        .map(|page| page.url.clone())
        .unwrap_or_else(|| format!("http://{host}/"));
    let scope = CefString::from(scope.as_str());
    let native_cookie = native_cookie_from_contract(&request.cookie)?;
    let completed = Arc::new(AtomicBool::new(false));
    let success = Arc::new(AtomicBool::new(false));
    let mut callback =
        WorldlineSetCookieCallback::new(Arc::clone(&completed), Arc::clone(&success));
    let accepted = manager.set_cookie(Some(&scope), Some(&native_cookie), Some(&mut callback));
    if accepted == 0 {
        return Err("CEF rejected setting cookie".to_string());
    }
    manager.flush_store(None);
    Ok((completed, success))
}

#[cfg(windows)]
fn native_delete_cookies_start(
    contexts: &Arc<Mutex<BTreeMap<BrowserContextId, RequestContext>>>,
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    context_id: &BrowserContextId,
    request: &DeleteCookiesRequest,
) -> Result<(Arc<AtomicBool>, Arc<AtomicI32>), String> {
    let context = native_cookie_context(contexts, pages, context_id)?;
    let manager = context
        .cookie_manager(None)
        .ok_or_else(|| "CEF request context has no cookie manager".to_string())?;
    let scope = cookie_scope_url(request.url.as_deref(), request.domain.as_deref())?;
    let scope = scope.map(|scope| CefString::from(scope.as_str()));
    let name = request.name.as_deref().map(CefString::from);
    let completed = Arc::new(AtomicBool::new(false));
    let deleted = Arc::new(AtomicI32::new(0));
    let mut callback =
        WorldlineDeleteCookiesCallback::new(Arc::clone(&completed), Arc::clone(&deleted));
    let accepted = manager.delete_cookies(scope.as_ref(), name.as_ref(), Some(&mut callback));
    if accepted == 0 {
        return Err("CEF rejected deleting cookies".to_string());
    }
    manager.flush_store(None);
    Ok((completed, deleted))
}

#[cfg(windows)]
fn native_clear_storage_start(
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    request: &ClearStorageRequest,
) -> Result<(PageId, String), String> {
    if matches!(request.storage_type, StorageType::IndexedDb) {
        return Err("CEF storage primitive does not expose bounded IndexedDB clearing".to_string());
    }
    let target_origin = canonical_origin(&request.origin)?;
    let page = native_storage_page(pages, request.context_id.clone(), &target_origin)?;
    let page_id = page.0.clone();
    let page = page.1;
    let browser = native_browser(page.browser_id)?;
    let frame = browser
        .main_frame()
        .ok_or_else(|| "CEF browser has no main frame".to_string())?;
    let marker = next_storage_marker(&format!("clear:{}:{}", request.context_id, request.origin));
    let marker_json = serde_json::to_string(&marker).map_err(|error| error.to_string())?;
    let script = match request.storage_type {
        StorageType::LocalStorage => {
            format!(
                "(() => {{ const before = window.localStorage.length; window.localStorage.clear(); const remaining = window.localStorage.length; document.title = {marker_json} + JSON.stringify({{had_items: before > 0, before, remaining}}); }})();"
            )
        }
        StorageType::SessionStorage => {
            format!(
                "(() => {{ const before = window.sessionStorage.length; window.sessionStorage.clear(); const remaining = window.sessionStorage.length; document.title = {marker_json} + JSON.stringify({{had_items: before > 0, before, remaining}}); }})();"
            )
        }
        StorageType::All => format!(
            "(() => {{ const before = window.localStorage.length + window.sessionStorage.length; window.localStorage.clear(); window.sessionStorage.clear(); const remaining = window.localStorage.length + window.sessionStorage.length; document.title = {marker_json} + JSON.stringify({{had_items: before > 0, before, remaining}}); }})();"
        ),
        StorageType::IndexedDb => unreachable!(),
    };
    let script = CefString::from(script.as_str());
    frame.execute_java_script(Some(&script), None, 1);
    Ok((page_id, marker))
}

#[cfg(windows)]
fn native_storage_page(
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    context_id: BrowserContextId,
    target_origin: &str,
) -> Result<(PageId, PageState), String> {
    pages
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|(_, page)| {
            page.context_id == context_id
                && canonical_origin(&page.url).ok().as_deref() == Some(target_origin)
        })
        .map(|(page_id, page)| (page_id.clone(), page.clone()))
        .ok_or_else(|| "no live CEF page for requested storage origin".to_string())
}

#[cfg(windows)]
fn storage_namespace(storage_type: StorageType) -> Result<&'static str, String> {
    match storage_type {
        StorageType::LocalStorage => Ok("localStorage"),
        StorageType::SessionStorage => Ok("sessionStorage"),
        StorageType::IndexedDb | StorageType::All => Err(
            "CEF storage item primitive supports only localStorage or sessionStorage".to_string(),
        ),
    }
}

#[cfg(windows)]
fn storage_marker(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let mut marker = String::from("__worldline_storage_");
    for byte in digest.iter().take(8) {
        use std::fmt::Write as _;
        let _ = write!(marker, "{byte:02x}");
    }
    marker
}

#[cfg(windows)]
fn next_storage_marker(operation: &str) -> String {
    let sequence = STORAGE_MARKER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    storage_marker(&format!("{operation}:{sequence}"))
}

#[cfg(windows)]
fn wait_for_page_title(
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    page_id: &PageId,
    marker: &str,
) -> Result<String, String> {
    let started = Instant::now();
    while started.elapsed() < CEF_CALLBACK_TIMEOUT {
        if let Some(title) = pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(page_id)
            .map(|page| page.title.clone())
            .filter(|title| title.starts_with(marker))
        {
            return Ok(title);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err("timed out waiting for CEF storage JavaScript bridge".to_string())
}

#[cfg(windows)]
fn native_set_storage_item(
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    request: &StorageItemRequestV0_2,
) -> Result<(PageId, String), String> {
    let namespace = storage_namespace(request.storage_type)?;
    let value = request
        .value
        .as_deref()
        .ok_or_else(|| "storage set requires a value".to_string())?;
    let target_origin = canonical_origin(&request.origin)?;
    let (page_id, page) = native_storage_page(pages, request.context_id.clone(), &target_origin)?;
    let browser = native_browser(page.browser_id)?;
    let frame = browser
        .main_frame()
        .ok_or_else(|| "CEF browser has no main frame".to_string())?;
    let key = serde_json::to_string(&request.key).map_err(|error| error.to_string())?;
    let value = serde_json::to_string(value).map_err(|error| error.to_string())?;
    let marker = next_storage_marker(&format!(
        "set:{}:{}:{}",
        request.context_id, request.origin, request.key
    ));
    let script =
        format!("window.{namespace}.setItem({key}, {value}); document.title = {marker:?};");
    let script = CefString::from(script.as_str());
    frame.execute_java_script(Some(&script), None, 1);
    Ok((page_id, marker))
}

#[cfg(windows)]
fn native_get_storage_item_start(
    pages: &Arc<Mutex<BTreeMap<PageId, PageState>>>,
    request: &StorageItemRequestV0_2,
) -> Result<(PageId, String), String> {
    let namespace = storage_namespace(request.storage_type)?;
    let target_origin = canonical_origin(&request.origin)?;
    let (page_id, page) = native_storage_page(pages, request.context_id.clone(), &target_origin)?;
    let browser = native_browser(page.browser_id)?;
    let frame = browser
        .main_frame()
        .ok_or_else(|| "CEF browser has no main frame".to_string())?;
    let key = serde_json::to_string(&request.key).map_err(|error| error.to_string())?;
    let marker = next_storage_marker(&format!(
        "get:{}:{}:{}",
        request.context_id, request.origin, request.key
    ));
    let marker_json = serde_json::to_string(&marker).map_err(|error| error.to_string())?;
    let script = format!(
        "document.title = {marker_json} + JSON.stringify(window.{namespace}.getItem({key}));"
    );
    let script = CefString::from(script.as_str());
    frame.execute_java_script(Some(&script), None, 1);
    Ok((page_id, marker))
}

#[cfg(not(windows))]
fn canonical_origin(value: &str) -> Result<String, String> {
    let parsed = Url::parse(value).map_err(|error| format!("invalid origin: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("origin must be an HTTP(S) origin".to_string());
    }
    Ok(format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default().to_ascii_lowercase(),
        parsed
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    ))
}

#[cfg(windows)]
fn canonical_origin(value: &str) -> Result<String, String> {
    let parsed = Url::parse(value).map_err(|error| format!("invalid origin: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("origin must be an HTTP(S) origin".to_string());
    }
    Ok(format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default().to_ascii_lowercase(),
        parsed
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    ))
}

/// CEF-backed browser provider. No reference backend is retained here.
pub struct CefBrowserBackend {
    contexts: Mutex<BTreeMap<BrowserContextId, ContextState>>,
    #[cfg(windows)]
    native_contexts: Arc<Mutex<BTreeMap<BrowserContextId, RequestContext>>>,
    pages: Arc<Mutex<BTreeMap<PageId, PageState>>>,
    permissions: Mutex<BTreeMap<(BrowserContextId, String, String), PermissionResponse>>,
    next_id: Mutex<u64>,
    loop_runner: Option<Arc<CefLoopRunner>>,
    startup_error: Option<String>,
    cache_root: PathBuf,
    download_shared: Arc<DownloadShared>,
}

impl Default for CefBrowserBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CefBrowserBackend {
    pub fn new() -> Self {
        let cache_root = std::env::var_os("WORLDLINE_CEF_CACHE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join("target")
                    .join("worldline-cef-profiles")
            });
        Self::new_with_cache_root_and_sandbox(cache_root, 0)
    }

    /// Creates a backend for a direct executable caller without a bootstrap
    /// sandbox context. The production Windows bootstrap entrypoint uses
    /// [`Self::new_with_sandbox`] instead.
    pub fn new_with_cache_root(cache_root: impl Into<PathBuf>) -> Self {
        Self::new_with_cache_root_and_sandbox(cache_root, 0)
    }

    /// Creates a backend with the opaque sandbox context supplied by the
    /// Windows CEF bootstrap. Each independently supervised provider instance
    /// should receive its own root so CEF's process singleton and profile
    /// locks cannot couple test or runtime instances.
    pub fn new_with_cache_root_and_sandbox(
        cache_root: impl Into<PathBuf>,
        windows_sandbox_info: usize,
    ) -> Self {
        let cache_root = cache_root.into();
        let download_root = cache_root.join("downloads");
        let setup_error = std::fs::create_dir_all(&cache_root)
            .and_then(|_| std::fs::create_dir_all(&download_root))
            .err()
            .map(|error| format!("cannot prepare CEF runtime directories: {error}"));
        let (loop_runner, startup_error) = if setup_error.is_none() {
            let settings = CefSettings {
                // With the Windows sandbox policy the browser and CEF child
                // processes must use the same executable.  The provider
                // process performs early `cef_execute_process` dispatch
                // before entering the Worldline handshake, so configuring a
                // separate subprocess executable would violate that CEF
                // boundary and make the native path unstable.
                browser_subprocess_path: None,
                windows_sandbox_info,
                // The hosted proving contract requires a real headful CEF
                // browser.  Keep this false so CEF creates its native popup
                // window rather than silently selecting the OSR path.
                windowless_rendering_enabled: false,
                root_cache_path: Some(cache_root.to_string_lossy().to_string()),
                ..CefSettings::default()
            };
            match CefLoopRunner::spawn_with_settings(settings) {
                Ok(runner) => (Some(Arc::new(runner)), None),
                Err(error) => (None, Some(error)),
            }
        } else {
            (None, setup_error)
        };
        #[cfg(windows)]
        let native_contexts = Arc::new(Mutex::new(BTreeMap::new()));
        let pages = Arc::new(Mutex::new(BTreeMap::new()));
        let download_shared = Arc::new(DownloadShared::new(pages.clone(), download_root));
        Self {
            contexts: Mutex::new(BTreeMap::new()),
            #[cfg(windows)]
            native_contexts,
            pages,
            permissions: Mutex::new(BTreeMap::new()),
            next_id: Mutex::new(1),
            loop_runner,
            startup_error,
            cache_root,
            download_shared,
        }
    }

    /// Creates a backend using the default cache root and the bootstrap-owned
    /// Windows sandbox context.
    pub fn new_with_sandbox(windows_sandbox_info: usize) -> Self {
        let cache_root = std::env::var_os("WORLDLINE_CEF_CACHE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join("target")
                    .join("worldline-cef-profiles")
            });
        Self::new_with_cache_root_and_sandbox(cache_root, windows_sandbox_info)
    }

    /// Returns engine events generated by the real CEF download handler.
    pub fn drain_download_events(&self) -> Vec<CefDownloadEvent> {
        self.download_shared.drain_events()
    }

    /// Access the thread-affine loop runner if CEF startup succeeded.
    pub fn loop_runner(&self) -> Option<&Arc<CefLoopRunner>> {
        self.loop_runner.as_ref()
    }

    fn next_id_str(&self, prefix: &str) -> String {
        let mut next = self
            .next_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = *next;
        *next = next.saturating_add(1);
        format!("{prefix}-{id}")
    }

    fn runner(&self) -> Result<Arc<CefLoopRunner>, BrowserError> {
        if let Some(error) = &self.startup_error {
            return Err(BrowserError::EngineCrashed(error.clone()));
        }
        self.loop_runner
            .as_ref()
            .cloned()
            .ok_or_else(|| BrowserError::EngineCrashed("CEF runtime is unavailable".to_string()))
    }

    fn context(&self, id: &BrowserContextId) -> Result<ContextState, BrowserError> {
        self.contexts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| BrowserError::ContextNotFound(id.clone()))
    }

    fn page(&self, id: &PageId) -> Result<PageState, BrowserError> {
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| BrowserError::PageNotFound(id.clone()))
    }

    fn profile_path(&self, context_id: &BrowserContextId, profile_id: Option<&str>) -> PathBuf {
        let key = profile_id.unwrap_or(context_id.as_str());
        let digest = Sha256::digest(key.as_bytes());
        let mut encoded = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(encoded, "{byte:02x}");
        }
        self.cache_root.join(encoded)
    }

    fn validate_url(url: &str, allow_about_blank: bool) -> Result<(), BrowserError> {
        if allow_about_blank && url == "about:blank" {
            return Ok(());
        }
        let parsed = Url::parse(url).map_err(|error| {
            BrowserError::InvalidRequest(format!("invalid browser URL: {error}"))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(BrowserError::InvalidRequest(
                "CEF production navigation permits only HTTP(S) host URLs (and about:blank for page creation)".to_string(),
            ));
        }
        Ok(())
    }

    fn shutdown_internal(&mut self) {
        let browser_ids: Vec<i32> = self
            .pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .map(|page| page.browser_id)
            .filter(|id| *id >= 0)
            .collect();
        if let Some(runner) = self.loop_runner.as_ref() {
            #[cfg(windows)]
            let native_contexts = Arc::clone(&self.native_contexts);
            let _ = runner.dispatch_sync(move || {
                #[cfg(windows)]
                {
                    for browser_id in browser_ids {
                        let _ = native_close_browser(browser_id);
                    }
                    native_contexts
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clear();
                }
            });
        }
        if let Some(runner) = self.loop_runner.as_mut().and_then(Arc::get_mut) {
            runner.shutdown();
        }
        self.loop_runner = None;
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.contexts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

impl Drop for CefBrowserBackend {
    fn drop(&mut self) {
        self.shutdown_internal();
    }
}

impl BrowserBackend for CefBrowserBackend {
    fn initialize(&mut self) -> Result<(), BrowserError> {
        self.runner().map(|_| ())
    }

    fn shutdown(&mut self) -> Result<(), BrowserError> {
        self.shutdown_internal();
        Ok(())
    }

    fn create_context(
        &mut self,
        req: &CreateContextRequest,
    ) -> Result<CreateContextResponse, BrowserError> {
        let id = BrowserContextId::new(self.next_id_str("ctx"));
        let state = ContextState {
            incognito: req.incognito,
            user_agent: req.user_agent.clone(),
            cache_path: (!req.incognito).then(|| self.profile_path(&id, req.profile_id.as_deref())),
        };
        let context_for_cef = state.clone();
        let runner = self.runner()?;
        #[cfg(windows)]
        {
            let native_contexts = Arc::clone(&self.native_contexts);
            let native_context_id = id.clone();
            let cookie_manager_ready = runner
                .dispatch_sync(move || {
                    let (cookie_context, ready) = native_prepare_cookie_manager(&context_for_cef)?;
                    native_contexts
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(native_context_id, cookie_context);
                    Ok(ready)
                })
                .map_err(BrowserError::EngineCrashed)?
                .map_err(BrowserError::EngineCrashed)?;
            if !wait_for_flag(&cookie_manager_ready, CEF_CALLBACK_TIMEOUT) {
                self.native_contexts
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id);
                return Err(BrowserError::EngineCrashed(
                    "timed out waiting for CEF cookie manager initialization".to_string(),
                ));
            }
        }
        #[cfg(not(windows))]
        runner
            .dispatch_sync(move || {
                let _ = context_for_cef;
                Ok(())
            })
            .map_err(BrowserError::EngineCrashed)?
            .map_err(BrowserError::EngineCrashed)?;
        self.contexts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.clone(), state);
        Ok(CreateContextResponse {
            context_id: id,
            profile_id: req.profile_id.clone(),
            incognito: req.incognito,
        })
    }

    fn close_context(
        &mut self,
        req: &CloseContextRequest,
    ) -> Result<CloseContextResponse, BrowserError> {
        let _context = self.context(&req.context_id)?;
        let browser_ids: Vec<i32> = self
            .pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .filter(|page| page.context_id == req.context_id && page.browser_id >= 0)
            .map(|page| page.browser_id)
            .collect();
        let runner = self.runner()?;
        #[cfg(windows)]
        let native_contexts = Arc::clone(&self.native_contexts);
        let context_id = req.context_id.clone();
        runner
            .dispatch_sync(move || {
                #[cfg(windows)]
                {
                    for browser_id in browser_ids {
                        let _ = native_close_browser(browser_id);
                    }
                    native_contexts
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&context_id);
                }
            })
            .map_err(BrowserError::EngineCrashed)?;
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, page| page.context_id != req.context_id);
        self.contexts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&req.context_id);
        Ok(CloseContextResponse {
            context_id: req.context_id.clone(),
            closed: true,
        })
    }

    fn list_contexts(&self) -> Result<ListContextsResponse, BrowserError> {
        let contexts = self
            .contexts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(ListContextsResponse {
            contexts: contexts.keys().cloned().collect(),
        })
    }

    fn create_page(&mut self, req: &CreatePageRequest) -> Result<CreatePageResponse, BrowserError> {
        let context = self.context(&req.context_id)?;
        let initial_url = req
            .initial_url
            .clone()
            .unwrap_or_else(|| "about:blank".to_string());
        Self::validate_url(&initial_url, true)?;
        let page_id = PageId::new(self.next_id_str("page"));
        let page_state = PageState {
            context_id: req.context_id.clone(),
            url: initial_url.clone(),
            title: "Worldline Browser".to_string(),
            revision: DocumentRevision::initial(),
            loading_state: LoadingState::Loading,
            status_code: 0,
            browser_id: -1,
            history: vec![initial_url.clone()],
            history_idx: 0,
            crashed: false,
        };
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(page_id.clone(), page_state);
        let callback_state = PageCallbackState {
            page_id: page_id.clone(),
            pages: Arc::clone(&self.pages),
        };
        let downloads = Arc::clone(&self.download_shared);
        let runner = self.runner()?;
        #[cfg(windows)]
        let native_contexts = Arc::clone(&self.native_contexts);
        let context_id = req.context_id.clone();
        runner
            .dispatch_sync(move || {
                #[cfg(windows)]
                {
                    let _ = context;
                    native_create_browser(
                        &native_contexts,
                        &context_id,
                        &initial_url,
                        callback_state,
                        downloads,
                    )
                }
                #[cfg(not(windows))]
                {
                    let _ = (context, context_id, initial_url, callback_state, downloads);
                    Err("CEF production backend is only available on Windows".to_string())
                }
            })
            .map_err(BrowserError::EngineCrashed)?
            .map_err(BrowserError::EngineCrashed)?;
        let browser_id =
            wait_for_browser_id(&self.pages, &page_id).map_err(BrowserError::EngineCrashed)?;
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(&page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?
            .browser_id = browser_id;
        Ok(CreatePageResponse {
            context_id: req.context_id.clone(),
            page_id,
            initial_revision: DocumentRevision::initial(),
        })
    }

    fn close_page(&mut self, req: &ClosePageRequest) -> Result<ClosePageResponse, BrowserError> {
        let page = self.page(&req.page_id)?;
        let runner = self.runner()?;
        runner
            .dispatch_sync(move || {
                #[cfg(windows)]
                if page.browser_id >= 0 {
                    native_close_browser(page.browser_id)?;
                }
                Ok::<(), String>(())
            })
            .map_err(BrowserError::EngineCrashed)?
            .map_err(BrowserError::EngineCrashed)?;
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&req.page_id);
        Ok(ClosePageResponse {
            page_id: req.page_id.clone(),
            closed: true,
        })
    }

    fn list_pages(&self, req: &ListPagesRequest) -> Result<ListPagesResponse, BrowserError> {
        let pages = self
            .pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(ListPagesResponse {
            pages: pages
                .iter()
                .filter(|(_, page)| page.context_id == req.context_id)
                .map(
                    |(page_id, page)| worldline_browser_contract::contracts::PageSummary {
                        page_id: page_id.clone(),
                        url: page.url.clone(),
                        title: page.title.clone(),
                        document_revision: page.revision,
                    },
                )
                .collect(),
        })
    }

    fn navigate(&mut self, req: &NavigateRequest) -> Result<NavigateResponse, BrowserError> {
        Self::validate_url(&req.url, false)?;
        let page = self.page(&req.page_id)?;
        let runner = self.runner()?;
        let url = req.url.clone();
        runner
            .dispatch_sync(move || {
                #[cfg(windows)]
                {
                    native_load_url(page.browser_id, &url)
                }
                #[cfg(not(windows))]
                {
                    let _ = (page, url);
                    Err("CEF production backend is only available on Windows".to_string())
                }
            })
            .map_err(BrowserError::EngineCrashed)?
            .map_err(BrowserError::EngineCrashed)?;
        let mut pages = self
            .pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let page = pages
            .get_mut(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        page.crashed = false;
        page.url = req.url.clone();
        page.loading_state = LoadingState::Loading;
        page.status_code = 0;
        page.revision = page.revision.next();
        page.history.truncate(page.history_idx.saturating_add(1));
        page.history.push(req.url.clone());
        page.history_idx = page.history.len().saturating_sub(1);
        Ok(NavigateResponse {
            page_id: req.page_id.clone(),
            navigation_id: NavigationId::new(self.next_id_str("nav")),
            committed: true,
            document_revision: page.revision,
        })
    }

    fn reload(&mut self, req: &ReloadRequest) -> Result<ReloadResponse, BrowserError> {
        let page = self.page(&req.page_id)?;
        let ignore_cache = req.ignore_cache;
        let runner = self.runner()?;
        runner
            .dispatch_sync(move || {
                #[cfg(windows)]
                {
                    let browser = native_browser(page.browser_id)?;
                    if let Some(host) = browser.host() {
                        if ignore_cache {
                            browser.reload_ignore_cache();
                        } else {
                            browser.reload();
                        }
                        let _ = host;
                        Ok(())
                    } else {
                        Err("CEF browser has no host".to_string())
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = page;
                    Err("CEF production backend is only available on Windows".to_string())
                }
            })
            .map_err(BrowserError::EngineCrashed)?
            .map_err(BrowserError::EngineCrashed)?;
        let mut pages = self
            .pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let page = pages
            .get_mut(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        page.loading_state = LoadingState::Loading;
        page.revision = page.revision.next();
        Ok(ReloadResponse {
            page_id: req.page_id.clone(),
            reloaded: true,
            document_revision: page.revision,
        })
    }

    fn stop(&mut self, req: &StopRequest) -> Result<StopResponse, BrowserError> {
        let page = self.page(&req.page_id)?;
        let runner = self.runner()?;
        runner
            .dispatch_sync(move || {
                #[cfg(windows)]
                {
                    let browser = native_browser(page.browser_id)?;
                    browser.stop_load();
                    Ok(())
                }
                #[cfg(not(windows))]
                {
                    let _ = page;
                    Err("CEF production backend is only available on Windows".to_string())
                }
            })
            .map_err(BrowserError::EngineCrashed)?
            .map_err(BrowserError::EngineCrashed)?;
        if let Some(page) = self
            .pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(&req.page_id)
        {
            page.loading_state = LoadingState::Complete;
        }
        Ok(StopResponse {
            page_id: req.page_id.clone(),
            stopped: true,
        })
    }

    fn history_nav(&mut self, req: &HistoryNavRequest) -> Result<HistoryNavResponse, BrowserError> {
        let page = self.page(&req.page_id)?;
        let delta = req.delta;
        let runner = self.runner()?;
        runner
            .dispatch_sync(move || {
                #[cfg(windows)]
                {
                    let browser = native_browser(page.browser_id)?;
                    if delta < 0 {
                        browser.go_back();
                    } else if delta > 0 {
                        browser.go_forward();
                    }
                    Ok(())
                }
                #[cfg(not(windows))]
                {
                    let _ = page;
                    Err("CEF production backend is only available on Windows".to_string())
                }
            })
            .map_err(BrowserError::EngineCrashed)?
            .map_err(BrowserError::EngineCrashed)?;
        let mut pages = self
            .pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let page = pages
            .get_mut(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        if req.delta < 0 && page.history_idx > 0 {
            page.history_idx -= 1;
            page.url = page.history[page.history_idx].clone();
        } else if req.delta > 0 && page.history_idx + 1 < page.history.len() {
            page.history_idx += 1;
            page.url = page.history[page.history_idx].clone();
        }
        page.revision = page.revision.next();
        Ok(HistoryNavResponse {
            page_id: req.page_id.clone(),
            success: true,
            document_revision: page.revision,
        })
    }

    fn observe(&self, req: &ObservePageRequest) -> Result<PageObservation, BrowserError> {
        let page = self.page(&req.page_id)?;
        if page.crashed {
            return Err(BrowserError::EngineCrashed(
                "CEF renderer process crashed".to_string(),
            ));
        }
        let is_secure = page.url.starts_with("https://");
        Ok(PageObservation {
            page_id: req.page_id.clone(),
            url: page.url,
            title: page.title,
            loading_state: page.loading_state,
            document_revision: page.revision,
            status_code: page.status_code,
            is_secure,
            viewport: Some(ViewportInfo {
                width: 1280,
                height: 720,
                device_scale_factor: 1,
            }),
        })
    }

    fn query(&self, req: &QueryDocumentRequest) -> Result<DocumentSnapshot, BrowserError> {
        let _ = self.page(&req.page_id)?;
        Err(BrowserError::UnsupportedOperation(
            "CEF semantic query requires an explicit DOM/accessibility bridge and is not silently emulated".to_string(),
        ))
    }

    fn act(&mut self, req: &ActRequest) -> Result<ActionResult, BrowserError> {
        let page_id = match req {
            ActRequest::Click(action) => action.element_ref.page_id().clone(),
            ActRequest::Input(action) => action.element_ref.page_id().clone(),
            ActRequest::Focus(action) => action.element_ref.page_id().clone(),
            ActRequest::Submit(action) => action.element_ref.page_id().clone(),
            ActRequest::Scroll(action) => action.page_id.clone(),
        };
        let _ = self.page(&page_id)?;
        Err(BrowserError::UnsupportedOperation(
            "CEF semantic actions require an explicit DOM/input bridge and are not silently emulated".to_string(),
        ))
    }

    fn start_download(
        &mut self,
        req: &StartDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        Self::validate_url(&req.url, false)?;
        let page = self.page(&req.page_id)?;
        let context_id = page.context_id.clone();
        let download_id = DownloadId::new(self.next_id_str("dl"));
        let suggested = req
            .url
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("download.bin")
            .to_string();
        let destination = req
            .destination_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_default();
        if let Some(path) = req.destination_path.as_deref() {
            let root = std::fs::canonicalize(&self.download_shared.download_root)
                .unwrap_or_else(|_| self.download_shared.download_root.clone());
            let candidate = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                root.join(path)
            };
            if candidate
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
                || !candidate.starts_with(&root)
            {
                return Err(BrowserError::PermissionDenied(
                    "CEF download destination is outside the provider staging root".to_string(),
                ));
            }
        }
        self.download_shared
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                download_id.clone(),
                PendingDownload {
                    context_id: context_id.clone(),
                    page_id: req.page_id.clone(),
                    url: req.url.clone(),
                    destination_path: destination.clone(),
                    suggested_filename: suggested.clone(),
                },
            );
        let runner = self.runner()?;
        let url = req.url.clone();
        let browser_id = page.browser_id;
        if let Err(error) = runner.dispatch_sync(move || {
            #[cfg(windows)]
            {
                let browser = native_browser(browser_id)?;
                let host = browser
                    .host()
                    .ok_or_else(|| "CEF browser has no host".to_string())?;
                let url = CefString::from(url.as_str());
                host.start_download(Some(&url));
                Ok::<(), String>(())
            }
            #[cfg(not(windows))]
            {
                let _ = (browser_id, url);
                Err("CEF production backend is only available on Windows".to_string())
            }
        }) {
            self.download_shared
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&download_id);
            return Err(BrowserError::EngineCrashed(error));
        }
        Ok(DownloadStatusResponse {
            download_id,
            page_id: req.page_id.clone(),
            url: req.url.clone(),
            destination_path: destination.to_string_lossy().to_string(),
            state: DownloadState::InProgress,
            received_bytes: 0,
            total_bytes: 0,
        })
    }

    fn control_download(
        &mut self,
        req: &ControlDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        let _ = req;
        Err(BrowserError::UnsupportedOperation(
            "CEF download control is not exposed until a stable brokered download-handle mapping is available".to_string(),
        ))
    }

    fn query_permission(
        &self,
        req: &QueryPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError> {
        let _ = self.context(&req.context_id)?;
        self.permissions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(
                req.context_id.clone(),
                req.origin.clone(),
                format!("{:?}", req.permission_type),
            ))
            .cloned()
            .ok_or_else(|| {
                BrowserError::UnsupportedOperation(
                    "CEF permission handler is not enabled for this production adapter".to_string(),
                )
            })
    }

    fn set_permission(
        &mut self,
        req: &SetPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError> {
        let _ = req;
        Err(BrowserError::UnsupportedOperation(
            "CEF permission handler is not enabled for this production adapter".to_string(),
        ))
    }

    fn capture(&mut self, req: &CapturePageRequest) -> Result<CapturePageResponse, BrowserError> {
        let _ = self.page(&req.page_id)?;
        Err(BrowserError::UnsupportedOperation(
            "CEF capture requires an explicit native render/capture bridge".to_string(),
        ))
    }

    fn read_capture(
        &self,
        req: &ReadCaptureArtifactRequest,
    ) -> Result<ReadCaptureArtifactResponse, BrowserError> {
        let _ = req;
        Err(BrowserError::UnsupportedOperation(
            "CEF capture artifacts are unavailable until the generic blob bridge is active"
                .to_string(),
        ))
    }

    fn get_cookies(&self, req: &GetCookiesRequest) -> Result<GetCookiesResponse, BrowserError> {
        self.get_cookies_v0_2(req)
            .map(|response| GetCookiesResponse {
                cookies: response.cookies.into_iter().map(Into::into).collect(),
            })
    }

    fn get_cookies_v0_2(
        &self,
        req: &GetCookiesRequest,
    ) -> Result<GetCookiesResponseV0_2, BrowserError> {
        #[cfg(not(windows))]
        {
            let _ = req;
            return Err(BrowserError::EngineCrashed(
                "CEF production backend is only available on Windows".to_string(),
            ));
        }

        #[cfg(windows)]
        {
            let _context = self.context(&req.context_id)?;
            let request = req.clone();
            let runner = self.runner()?;
            let native_contexts = Arc::clone(&self.native_contexts);
            let pages = Arc::clone(&self.pages);
            let context_id = req.context_id.clone();
            let (cookies, completed, store_ready) = runner
                .dispatch_sync(move || {
                    native_get_cookies_start(&native_contexts, &pages, &context_id, &request)
                })
                .map_err(BrowserError::EngineCrashed)?
                .map_err(BrowserError::EngineCrashed)?;
            if !wait_for_flag(&completed, CEF_CALLBACK_TIMEOUT) {
                if !wait_for_flag(&store_ready, CEF_CALLBACK_TIMEOUT) {
                    return Err(BrowserError::EngineCrashed(
                        "timed out waiting for CEF cookie visitor and store readiness".to_string(),
                    ));
                }
                // Visit*Cookies is documented not to invoke the visitor for
                // an empty result. The successful native flush above makes
                // that empty result distinguishable from an unavailable
                // cookie store.
                return Ok(GetCookiesResponseV0_2 {
                    cookies: Vec::new(),
                });
            }
            let cookies = cookies
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            Ok(GetCookiesResponseV0_2 { cookies })
        }
    }

    fn set_cookie(&mut self, req: &SetCookieRequest) -> Result<SetCookieResponse, BrowserError> {
        let request = SetCookieRequestV0_2 {
            context_id: req.context_id.clone(),
            cookie: req.cookie.clone().into(),
        };
        self.set_cookie_v0_2(&request)
    }

    fn set_cookie_v0_2(
        &mut self,
        req: &SetCookieRequestV0_2,
    ) -> Result<SetCookieResponse, BrowserError> {
        #[cfg(not(windows))]
        {
            let _ = req;
            return Err(BrowserError::EngineCrashed(
                "CEF production backend is only available on Windows".to_string(),
            ));
        }

        #[cfg(windows)]
        {
            let _context = self.context(&req.context_id)?;
            let runner = self.runner()?;
            let request = req.clone();
            let native_contexts = Arc::clone(&self.native_contexts);
            let pages = Arc::clone(&self.pages);
            let context_id = req.context_id.clone();
            let (completed, success) = runner
                .dispatch_sync(move || {
                    native_set_cookie_start(&native_contexts, &context_id, &pages, &request)
                })
                .map_err(BrowserError::EngineCrashed)?
                .map_err(BrowserError::EngineCrashed)?;
            if !wait_for_flag(&completed, CEF_CALLBACK_TIMEOUT) {
                return Err(BrowserError::EngineCrashed(
                    "timed out waiting for CEF cookie set callback".to_string(),
                ));
            }
            Ok(SetCookieResponse {
                success: success.load(Ordering::SeqCst),
            })
        }
    }

    fn delete_cookies(
        &mut self,
        req: &DeleteCookiesRequest,
    ) -> Result<DeleteCookiesResponse, BrowserError> {
        #[cfg(not(windows))]
        {
            let _ = req;
            return Err(BrowserError::EngineCrashed(
                "CEF production backend is only available on Windows".to_string(),
            ));
        }

        #[cfg(windows)]
        {
            let _context = self.context(&req.context_id)?;
            let runner = self.runner()?;
            let request = req.clone();
            let native_contexts = Arc::clone(&self.native_contexts);
            let pages = Arc::clone(&self.pages);
            let context_id = req.context_id.clone();
            let (completed, deleted) = runner
                .dispatch_sync(move || {
                    native_delete_cookies_start(&native_contexts, &pages, &context_id, &request)
                })
                .map_err(BrowserError::EngineCrashed)?
                .map_err(BrowserError::EngineCrashed)?;
            if !wait_for_flag(&completed, CEF_CALLBACK_TIMEOUT) {
                return Err(BrowserError::EngineCrashed(
                    "timed out waiting for CEF cookie delete callback".to_string(),
                ));
            }
            Ok(DeleteCookiesResponse {
                deleted_count: deleted.load(Ordering::SeqCst).max(0) as u32,
            })
        }
    }

    fn clear_storage(
        &mut self,
        req: &ClearStorageRequest,
    ) -> Result<ClearStorageResponse, BrowserError> {
        #[cfg(not(windows))]
        {
            let _ = req;
            return Err(BrowserError::EngineCrashed(
                "CEF production backend is only available on Windows".to_string(),
            ));
        }

        #[cfg(windows)]
        {
            let _ = canonical_origin(&req.origin).map_err(BrowserError::InvalidRequest)?;
            let runner = self.runner()?;
            let pages = Arc::clone(&self.pages);
            let request = req.clone();
            let (page_id, marker) = runner
                .dispatch_sync(move || native_clear_storage_start(&pages, &request))
                .map_err(BrowserError::EngineCrashed)?
                .map_err(BrowserError::EngineCrashed)?;
            let title = wait_for_page_title(&self.pages, &page_id, &marker)
                .map_err(BrowserError::EngineCrashed)?;
            let bridge_result = title.strip_prefix(&marker).ok_or_else(|| {
                BrowserError::EngineCrashed(
                    "CEF storage clear JavaScript bridge returned an invalid marker".to_string(),
                )
            })?;
            let bridge_result: serde_json::Value =
                serde_json::from_str(bridge_result).map_err(|error| {
                    BrowserError::EngineCrashed(format!(
                        "invalid CEF storage clear result: {error}"
                    ))
                })?;
            let had_items = bridge_result
                .get("had_items")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| {
                    BrowserError::EngineCrashed(
                        "CEF storage clear result omitted had_items".to_string(),
                    )
                })?;
            let remaining = bridge_result
                .get("remaining")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    BrowserError::EngineCrashed(
                        "CEF storage clear result omitted remaining count".to_string(),
                    )
                })?;
            if remaining != 0 {
                return Err(BrowserError::EngineCrashed(format!(
                    "CEF storage clear left {remaining} item(s) in the requested scope"
                )));
            }
            Ok(ClearStorageResponse { cleared: had_items })
        }
    }

    fn set_storage_item(
        &mut self,
        req: &StorageItemRequestV0_2,
    ) -> Result<StorageItemResponseV0_2, BrowserError> {
        #[cfg(not(windows))]
        {
            let _ = req;
            return Err(BrowserError::EngineCrashed(
                "CEF production backend is only available on Windows".to_string(),
            ));
        }

        #[cfg(windows)]
        {
            let _ = canonical_origin(&req.origin).map_err(BrowserError::InvalidRequest)?;
            let runner = self.runner()?;
            let pages = Arc::clone(&self.pages);
            let request = req.clone();
            let (page_id, marker) = runner
                .dispatch_sync(move || native_set_storage_item(&pages, &request))
                .map_err(BrowserError::EngineCrashed)?
                .map_err(BrowserError::EngineCrashed)?;
            let _ = wait_for_page_title(&self.pages, &page_id, &marker)
                .map_err(BrowserError::EngineCrashed)?;
            Ok(StorageItemResponseV0_2 {
                value: None,
                changed: true,
            })
        }
    }

    fn get_storage_item(
        &self,
        req: &StorageItemRequestV0_2,
    ) -> Result<StorageItemResponseV0_2, BrowserError> {
        #[cfg(not(windows))]
        {
            let _ = req;
            return Err(BrowserError::EngineCrashed(
                "CEF production backend is only available on Windows".to_string(),
            ));
        }

        #[cfg(windows)]
        {
            let _ = canonical_origin(&req.origin).map_err(BrowserError::InvalidRequest)?;
            let runner = self.runner()?;
            let pages = Arc::clone(&self.pages);
            let request = req.clone();
            let (page_id, marker) = runner
                .dispatch_sync(move || native_get_storage_item_start(&pages, &request))
                .map_err(BrowserError::EngineCrashed)?
                .map_err(BrowserError::EngineCrashed)?;
            let title = wait_for_page_title(&self.pages, &page_id, &marker)
                .map_err(BrowserError::EngineCrashed)?;
            let encoded = title.strip_prefix(&marker).ok_or_else(|| {
                BrowserError::EngineCrashed(
                    "CEF storage JavaScript bridge returned an invalid marker".to_string(),
                )
            })?;
            let value = serde_json::from_str(encoded).map_err(|error| {
                BrowserError::EngineCrashed(format!("invalid CEF storage value: {error}"))
            })?;
            Ok(StorageItemResponseV0_2 {
                value,
                changed: false,
            })
        }
    }
}
