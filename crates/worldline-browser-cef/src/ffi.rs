//! CEF lifecycle bindings and process-boundary helpers.
//!
//! The adapter deliberately keeps the CEF objects on the thread that owns the
//! CEF message pump. Worldline identifiers and capabilities never cross this
//! boundary as native CEF pointers or handles.

use std::ffi::{c_int, c_void};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
use windows_sys::Win32::Foundation::HWND;

#[cfg(windows)]
use cef::rc::Rc;
use cef::{
    App, BrowserProcessHandler, CefString, CommandLine, ImplApp, ImplBrowserProcessHandler,
    ImplCommandLine, WrapApp, WrapBrowserProcessHandler, wrap_app, wrap_browser_process_handler,
};

#[cfg(windows)]
static CEF_CONTEXT_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
wrap_browser_process_handler! {
    struct WorldlineBrowserProcessHandler;

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            CEF_CONTEXT_INITIALIZED.store(true, Ordering::SeqCst);
        }

        fn on_before_child_process_launch(&self, command_line: Option<&mut CommandLine>) {
            // Hosted Windows runners do not provide a stable hardware GPU.
            // Keep the CEF sandbox enabled while selecting Chromium's
            // software/headful path for the real child process.
            if let Some(command_line) = command_line {
                let disable_gpu = CefString::from("disable-gpu");
                let in_process_gpu = CefString::from("in-process-gpu");
                command_line.append_switch(Some(&disable_gpu));
                command_line.append_switch(Some(&in_process_gpu));
            }
        }

        fn on_schedule_message_pump_work(&self, _delay_ms: i64) {
            // CefLoopRunner polls cef_do_message_loop_work on the owning UI
            // thread every few milliseconds, so there is no blocking wait to
            // wake here. Registering the callback still completes the
            // external-message-pump contract instead of leaving it null.
        }
    }
}

#[cfg(windows)]
wrap_app! {
    struct WorldlineCefApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            // Keep the browser headful while selecting Chromium's software
            // rendering path on hosted runners. This is a rendering switch,
            // not a sandbox-disabling switch.
            if let Some(command_line) = command_line {
                let disable_gpu = CefString::from("disable-gpu");
                let in_process_gpu = CefString::from("in-process-gpu");
                command_line.append_switch(Some(&disable_gpu));
                command_line.append_switch(Some(&in_process_gpu));
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(WorldlineBrowserProcessHandler::new())
        }
    }
}

/// Windows CEF main args structure matching `cef_main_args_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CefMainArgs {
    pub instance: *mut c_void,
}

/// Settings used to initialize CEF.
#[derive(Clone, Debug)]
pub struct CefSettings {
    pub browser_subprocess_path: Option<String>,
    pub cache_path: Option<String>,
    pub root_cache_path: Option<String>,
    pub user_data_path: Option<String>,
    /// Opaque sandbox context supplied by the Windows CEF bootstrap. It is
    /// represented as an integer so the settings remain movable to the CEF UI
    /// thread without transferring ownership of a foreign pointer in Rust.
    pub windows_sandbox_info: usize,
    pub windowless_rendering_enabled: bool,
    pub multi_threaded_message_loop: bool,
    pub external_message_pump: bool,
    pub locale: Option<String>,
    pub log_severity: i32,
}

impl Default for CefSettings {
    fn default() -> Self {
        Self {
            browser_subprocess_path: None,
            cache_path: None,
            root_cache_path: None,
            user_data_path: None,
            windows_sandbox_info: 0,
            windowless_rendering_enabled: false,
            multi_threaded_message_loop: false,
            external_message_pump: true,
            locale: Some("en-US".to_string()),
            log_severity: 0,
        }
    }
}

/// Window information retained for the public adapter surface.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CefWindowInfo {
    #[cfg(windows)]
    pub parent_window: HWND,
    #[cfg(windows)]
    pub window: HWND,
    #[cfg(not(windows))]
    pub parent_window: *mut c_void,
    #[cfg(not(windows))]
    pub window: *mut c_void,
    pub windowless_rendering_enabled: c_int,
    pub transparent_painting_enabled: c_int,
}

impl Default for CefWindowInfo {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            parent_window: std::ptr::null_mut(),
            #[cfg(windows)]
            window: std::ptr::null_mut(),
            #[cfg(not(windows))]
            parent_window: std::ptr::null_mut(),
            #[cfg(not(windows))]
            window: std::ptr::null_mut(),
            windowless_rendering_enabled: 0,
            transparent_painting_enabled: 0,
        }
    }
}

/// Owns the CEF application object and lifecycle on the CEF UI thread.
#[cfg(windows)]
pub struct CefRuntime {
    initialized: bool,
}

#[cfg(windows)]
impl CefRuntime {
    /// Initializes the pinned CEF runtime with the production safety policy.
    pub fn initialize(settings: &CefSettings) -> Result<Self, String> {
        if settings.windows_sandbox_info == 0 {
            return Err(
                "CEF production initialization requires the bootstrap-owned sandbox context"
                    .to_string(),
            );
        }
        CEF_CONTEXT_INITIALIZED.store(false, Ordering::SeqCst);
        let args = cef::args::Args::new();
        let mut native = cef::Settings::default();

        // The CEF C API wrapper is built for the pinned last API version. Set
        // the matching runtime hash before constructing any wrapped object;
        // otherwise the wrapper rejects CefApp with an invalid version during
        // bootstrap initialization.
        let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        let mut app = WorldlineCefApp::new();

        // The production path must remain sandboxed. A caller cannot opt out
        // through the adapter settings.
        native.no_sandbox = 0;
        native.multi_threaded_message_loop = i32::from(settings.multi_threaded_message_loop);
        native.external_message_pump = i32::from(settings.external_message_pump);
        native.windowless_rendering_enabled = i32::from(settings.windowless_rendering_enabled);
        native.persist_session_cookies = 1;

        if let Some(path) = settings
            .browser_subprocess_path
            .as_deref()
            .filter(|path| !path.is_empty())
        {
            native.browser_subprocess_path = cef::CefString::from(path);
        }

        if let Some(path) = settings
            .cache_path
            .as_deref()
            .filter(|path| !path.is_empty())
        {
            native.cache_path = cef::CefString::from(path);
        } else if let Some(path) = settings
            .user_data_path
            .as_deref()
            .filter(|path| !path.is_empty())
        {
            native.cache_path = cef::CefString::from(path);
        }

        if let Some(path) = settings
            .root_cache_path
            .as_deref()
            .filter(|path| !path.is_empty())
        {
            native.root_cache_path = cef::CefString::from(path);
        }

        if let Some(locale) = settings
            .locale
            .as_deref()
            .filter(|locale| !locale.is_empty())
        {
            native.locale = cef::CefString::from(locale);
        }

        // `cef_initialize` returns 1 on success and 0 on failure. The
        // subprocess dispatch is intentionally performed by the provider
        // executable before this function is reached.
        let initialized = cef::initialize(
            Some(args.as_main_args()),
            Some(&native),
            Some(&mut app),
            settings.windows_sandbox_info as *mut u8,
        );
        if initialized != 1 {
            return Err("cef_initialize returned failure".to_string());
        }

        Ok(Self { initialized: true })
    }

    /// Waits until CEF has completed browser-process context initialization.
    /// Browser creation before this callback is not a valid production
    /// lifecycle, even when the CEF UI task queue is already accepting work.
    pub fn wait_for_context_initialized() -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if CEF_CONTEXT_INITIALIZED.load(Ordering::SeqCst) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        CEF_CONTEXT_INITIALIZED.load(Ordering::SeqCst)
    }

    /// Runs one external message-pump tick.
    pub fn do_message_loop_work(&self) {
        cef::do_message_loop_work();
    }

    /// Shuts down CEF from the application thread that initialized it.
    pub fn shutdown_global() {
        cef::shutdown();
    }

    /// Shuts CEF down on the same thread that initialized it.
    pub fn shutdown(&mut self) {
        if self.initialized {
            cef::shutdown();
            self.initialized = false;
        }
    }
}

#[cfg(windows)]
impl Drop for CefRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Non-Windows build surface. The real CEF runtime is currently pinned to the
/// hosted Windows target; keeping this no-op type preserves cross-target
/// compilation for contract and static-analysis jobs.
#[cfg(not(windows))]
pub struct CefRuntime;

#[cfg(not(windows))]
impl CefRuntime {
    pub fn initialize(_settings: &CefSettings) -> Result<Self, String> {
        Ok(Self)
    }

    pub fn do_message_loop_work(&self) {}

    pub fn shutdown(&mut self) {}
}

/// Calls the actual CEF subprocess entry point before provider startup.
///
/// CEF returns `-1` for the browser process; child processes return their exit
/// code and must terminate without entering the Worldline provider handshake.
pub fn early_subprocess_dispatch(sandbox_info: usize) -> Option<i32> {
    #[cfg(windows)]
    {
        let args = cef::args::Args::new();
        let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        let is_subprocess = std::env::args().any(|arg| arg.starts_with("--type="));
        let sandbox_info = sandbox_info as *mut u8;
        // Keep subprocess dispatch aligned with the cef-rs entrypoint. The
        // browser-process app is supplied to `cef_initialize`; child-process
        // dispatch must not construct a second wrapped app before CEF has
        // established the subprocess ABI context.
        let exit_code = cef::execute_process(Some(args.as_main_args()), None, sandbox_info);
        if is_subprocess {
            // A CEF child must never fall through into the Worldline host
            // handshake. Some Chromium utility processes are handled by CEF
            // with a non-negative exit code; if an unhandled child reports
            // -1, terminate it here rather than returning provider protocol
            // status 2 from a process that has no host transport.
            return Some(exit_code.max(0));
        }
        (exit_code >= 0).then_some(exit_code)
    }

    #[cfg(not(windows))]
    {
        let is_subprocess = std::env::args().any(|arg| arg.starts_with("--type="));
        is_subprocess.then_some(0)
    }
}
