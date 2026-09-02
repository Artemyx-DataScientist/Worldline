//! Cross-process serialization for hosted real-CEF proving runs.
//!
//! The real CEF fixtures are separate test processes, but they share one
//! staged runtime and the same desktop/CEF resource budget. A small
//! process-wide lock keeps parallel GRACE command evidence from starting
//! competing CEF sessions and making origin callbacks nondeterministic.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const LOCK_FILE_NAME: &str = "worldline-real-cef.lock";
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(300);
const STALE_AFTER: Duration = Duration::from_secs(600);
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// Owns the shared real-CEF proving lock until the fixture completes.
pub(crate) struct RealCefRunGuard {
    path: PathBuf,
    _file: File,
}

impl RealCefRunGuard {
    /// Waits for exclusive ownership of the hosted real-CEF proving slot.
    pub(crate) fn acquire() -> Result<Self, String> {
        let path = std::env::temp_dir().join(LOCK_FILE_NAME);
        let deadline = Instant::now() + ACQUIRE_TIMEOUT;

        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "pid={}", std::process::id())
                        .map_err(|error| format!("write real-CEF lock owner: {error}"))?;
                    return Ok(Self { path, _file: file });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if is_stale(&path) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "timed out waiting for the shared real-CEF proving lock '{}'",
                            path.display()
                        ));
                    }
                    thread::sleep(RETRY_DELAY);
                }
                Err(error) => {
                    return Err(format!(
                        "create shared real-CEF proving lock '{}': {error}",
                        path.display()
                    ));
                }
            }
        }
    }
}

impl Drop for RealCefRunGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_AFTER)
}
