//! Supervision of native provider child processes.
//!
//! One child process is owned by exactly one runtime. Stderr is drained by
//! a bounded background thread so a chatty child can never deadlock the
//! host by filling an inherited pipe. Graceful shutdown closes stdin and
//! waits under a deadline; after the deadline the host kills the child.
//! Killing a child never affects the host.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::NativeHostError;

/// Spawn specification for one native provider child.
#[derive(Clone, Debug)]
pub struct NativeChildSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Maximum accepted frame size in bytes (inbound and outbound).
    pub max_frame_bytes: usize,
    /// Maximum retained stderr bytes; excess is counted and dropped.
    pub stderr_max_bytes: usize,
}

/// Bounded stderr retention: keeps the tail of the child's stderr plus an
/// overflow counter for diagnostics.
#[derive(Default)]
struct LimitedStderr {
    tail: Vec<u8>,
    overflow_bytes: u64,
}

impl LimitedStderr {
    fn new(capacity: usize) -> Self {
        Self {
            tail: Vec::with_capacity(capacity.min(4096)),
            overflow_bytes: 0,
        }
    }

    fn append(&mut self, capacity: usize, chunk: &[u8]) {
        let room = capacity.saturating_sub(self.tail.len());
        let kept = &chunk[..chunk.len().min(room)];
        self.tail.extend_from_slice(kept);
        self.overflow_bytes += (chunk.len() - kept.len()) as u64;
    }
}

/// One supervised native provider child process.
pub struct NativeChild {
    child: Mutex<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr_tail: Arc<Mutex<LimitedStderr>>,
    stderr_capacity: usize,
}

impl NativeChild {
    /// Spawns the child with fully piped stdio and starts the bounded
    /// stderr drain thread.
    pub fn spawn(spec: &NativeChildSpec) -> Result<Self, NativeHostError> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| NativeHostError::SpawnFailed {
                reason: format!("{}: {error}", spec.program.display()),
            })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stderr_capacity = spec.stderr_max_bytes;
        let stderr_tail = Arc::new(Mutex::new(LimitedStderr::new(stderr_capacity)));
        if let Some(mut stderr) = stderr {
            let tail = Arc::clone(&stderr_tail);
            std::thread::spawn(move || {
                // Bounded drain: chunks are appended into the limited tail;
                // the child can never block the host by filling this pipe.
                let mut buffer = [0u8; 4096];
                loop {
                    match stderr.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if let Ok(mut guard) = tail.lock() {
                                guard.append(stderr_capacity, &buffer[..read]);
                            }
                        }
                    }
                }
            });
        }
        Ok(Self {
            child: Mutex::new(child),
            stdin,
            stdout,
            stderr_tail,
            stderr_capacity,
        })
    }

    /// Takes the child's stdout for the connection reader thread.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.stdout.take()
    }

    /// Takes the child's stdin for the connection writer.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.stdin.take()
    }

    /// Non-blocking exit probe.
    pub fn try_status(&self) -> Option<ExitStatus> {
        self.child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_wait()
            .unwrap_or(None)
    }

    /// Stderr diagnostics tail.
    pub fn stderr_text(&self) -> String {
        let guard = self
            .stderr_tail
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let overflow = guard.overflow_bytes;
        let mut text = String::from_utf8_lossy(&guard.tail).to_string();
        drop(guard);
        if overflow > 0 {
            text.push_str(&format!(
                "\n[stderr truncated: {overflow} overflow bytes dropped]"
            ));
        }
        let _ = self.stderr_capacity;
        text
    }

    /// Closes stdin (signalling EOF), waits up to `deadline` for exit, and
    /// kills the child when the deadline passes. Killing never affects the
    /// host.
    pub fn shutdown(&mut self, deadline: Duration) -> Result<ExitStatus, NativeHostError> {
        // Dropping stdin closes the pipe: an orderly child exits on EOF.
        self.stdin.take();
        let started = Instant::now();
        loop {
            if let Some(status) = self.try_status() {
                return Ok(status);
            }
            if started.elapsed() > deadline {
                self.kill();
                let _ = self
                    .child
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .wait();
                return Err(NativeHostError::ShutdownTimeout {
                    deadline_ms: deadline.as_millis() as u64,
                });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Unconditionally terminates the child. The host is unaffected.
    pub fn kill(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
