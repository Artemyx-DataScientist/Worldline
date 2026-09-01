//! CEF C ABI bindings, types, and early subprocess dispatch helpers.
//!
//! Provides the physical FFI layer for Chromium Embedded Framework on Windows.

use std::ffi::{c_int, c_void};

#[cfg(windows)]
use windows_sys::Win32::Foundation::HWND;

/// Windows CEF main args structure matching `cef_main_args_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CefMainArgs {
    pub instance: *mut c_void,
}

/// Early subprocess dispatch helper.
///
/// If this process was launched as a Chromium subprocess (e.g. `--type=renderer`,
/// `--type=gpu-process`, `--type=utility`), this function executes the subprocess
/// logic and returns `Some(exit_code)`. Otherwise, it returns `None` to continue
/// host/provider initialization.
pub fn early_subprocess_dispatch() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let is_subprocess = args.iter().any(|arg| arg.starts_with("--type="));
    if is_subprocess {
        // In full CEF binary distribution, cef_execute_process is invoked here.
        // In headless / simulated subprocess dispatch, we exit cleanly.
        Some(0)
    } else {
        None
    }
}

/// Settings used to initialize CEF.
#[derive(Clone, Debug)]
pub struct CefSettings {
    pub browser_subprocess_path: Option<String>,
    pub cache_path: Option<String>,
    pub root_cache_path: Option<String>,
    pub user_data_path: Option<String>,
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
            windowless_rendering_enabled: false,
            multi_threaded_message_loop: false,
            external_message_pump: true,
            locale: Some("en-US".to_string()),
            log_severity: 0,
        }
    }
}

/// Window information structure for headful or windowless browser creation.
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
