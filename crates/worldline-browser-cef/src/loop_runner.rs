//! CEF UI dispatch and lifecycle coordination.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
use crate::ffi::CefRuntime;
use crate::ffi::CefSettings;

#[cfg(windows)]
use std::sync::Mutex;

#[cfg(windows)]
use cef::rc::Rc;
#[cfg(windows)]
use cef::{ImplTask, Task, ThreadId, WrapTask, currently_on, post_task, wrap_task};

#[cfg(windows)]
type UiAction = Box<dyn FnOnce() + Send + 'static>;

#[cfg(windows)]
wrap_task! {
    struct DispatchTask {
        action: Arc<Mutex<Option<UiAction>>>,
    }

    impl Task {
        fn execute(&self) {
            let action = self
                .action
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(action) = action {
                action();
            }
        }
    }
}

/// Coordinates synchronous Worldline operations with the CEF UI thread.
///
/// On Windows CEF is initialized on the application main thread and owns its
/// UI thread through `multi_threaded_message_loop`. Calls are posted to that
/// CEF UI thread with `CefPostTask`; this type does not create a competing Rust
/// thread and never pumps CEF from a non-main thread.
pub struct CefLoopRunner {
    running: Arc<AtomicBool>,
    #[cfg(windows)]
    runtime: Option<CefRuntime>,
    #[cfg(windows)]
    owner_thread: std::thread::ThreadId,
}

impl CefLoopRunner {
    /// Initializes the production CEF runtime on the caller's application
    /// thread. The hosted Windows provider calls this from its process entry
    /// thread, as required by the CEF lifecycle contract.
    pub fn spawn() -> Result<Self, String> {
        Self::spawn_with_settings(CefSettings::default())
    }

    /// Initializes CEF and prepares synchronous UI dispatch.
    pub fn spawn_with_settings(settings: CefSettings) -> Result<Self, String> {
        #[cfg(windows)]
        {
            let mut settings = settings;
            // `CefInitialize` and `CefShutdown` must stay on the application
            // main thread. CEF's own multi-threaded loop then owns the UI
            // thread, while `dispatch_sync` below posts work to it.
            settings.multi_threaded_message_loop = true;
            settings.external_message_pump = false;
            let runtime = CefRuntime::initialize(&settings)?;
            if !CefRuntime::wait_for_context_initialized() {
                return Err("CEF browser context initialization timed out".to_string());
            }
            Ok(Self {
                running: Arc::new(AtomicBool::new(true)),
                runtime: Some(runtime),
                owner_thread: std::thread::current().id(),
            })
        }

        #[cfg(not(windows))]
        {
            let _ = settings;
            Err("CEF production runtime is only available on Windows".to_string())
        }
    }

    /// Dispatches a closure synchronously on the CEF UI thread.
    pub fn dispatch_sync<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        if !self.running.load(Ordering::SeqCst) {
            return Err("CEF UI loop runner is stopped".to_string());
        }

        #[cfg(windows)]
        {
            use std::sync::mpsc::sync_channel;
            use std::time::Duration;

            if currently_on(ThreadId::UI) != 0 {
                return Ok(f());
            }

            let (reply_tx, reply_rx) = sync_channel::<R>(1);
            let action = Arc::new(Mutex::new(Some(Box::new(move || {
                let _ = reply_tx.send(f());
            }) as UiAction)));
            let mut task = DispatchTask::new(action);
            if post_task(ThreadId::UI, Some(&mut task)) == 0 {
                return Err("failed to post task to CEF UI thread".to_string());
            }
            reply_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| format!("CEF UI task did not complete: {error}"))
        }

        #[cfg(not(windows))]
        {
            let _ = f;
            Err("CEF UI dispatch is only available on Windows".to_string())
        }
    }

    /// Shuts down CEF on the application thread that initialized it.
    pub fn shutdown(&mut self) {
        if self.running.swap(false, Ordering::SeqCst) {
            #[cfg(windows)]
            {
                debug_assert_eq!(
                    std::thread::current().id(),
                    self.owner_thread,
                    "CEF shutdown must run on the application thread"
                );
                if let Some(mut runtime) = self.runtime.take() {
                    runtime.shutdown();
                }
            }
        }
    }
}

impl Drop for CefLoopRunner {
    fn drop(&mut self) {
        self.shutdown();
    }
}
