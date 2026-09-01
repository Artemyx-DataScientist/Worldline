//! CEF adapter implementation for Worldline browser provider.

pub mod backend;
pub mod ffi;
pub mod loop_runner;

pub use backend::CefBrowserBackend;
pub use ffi::{CefMainArgs, CefSettings, CefWindowInfo, early_subprocess_dispatch};
pub use loop_runner::CefLoopRunner;
