//! Thread-affine UI message pump and Win32 window lifetime management.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type UiTask = Box<dyn FnOnce() + Send + 'static>;

/// Coordinates work on a dedicated, thread-affine UI message loop thread.
pub struct CefLoopRunner {
    task_sender: Sender<UiTask>,
    running: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
}

impl CefLoopRunner {
    /// Spawns the dedicated UI message loop thread.
    pub fn spawn() -> Result<Self, String> {
        let (task_sender, task_receiver) = channel::<UiTask>();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        let thread_handle = thread::Builder::new()
            .name("worldline-cef-ui-thread".to_string())
            .spawn(move || {
                Self::run_ui_loop(task_receiver, running_clone);
            })
            .map_err(|e| format!("Failed to spawn UI loop thread: {e}"))?;

        Ok(Self {
            task_sender,
            running,
            thread_handle: Some(thread_handle),
        })
    }

    /// Dispatches a closure to be executed synchronously on the UI thread.
    pub fn dispatch_sync<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        if !self.running.load(Ordering::SeqCst) {
            return Err("UI loop runner is stopped".to_string());
        }

        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<R>(1);
        let task: UiTask = Box::new(move || {
            let res = f();
            let _ = reply_tx.send(res);
        });

        self.task_sender
            .send(task)
            .map_err(|e| format!("Failed to send task to UI thread: {e}"))?;

        reply_rx
            .recv()
            .map_err(|e| format!("Failed to receive task result from UI thread: {e}"))
    }

    /// UI thread loop: drains tasks and runs message pump work.
    fn run_ui_loop(task_rx: Receiver<UiTask>, running: Arc<AtomicBool>) {
        while running.load(Ordering::SeqCst) {
            // Drain all pending tasks
            while let Ok(task) = task_rx.try_recv() {
                task();
            }

            // Perform platform message pump ticks
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
                };
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Shuts down the UI loop thread cleanly.
    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for CefLoopRunner {
    fn drop(&mut self) {
        self.shutdown();
    }
}
