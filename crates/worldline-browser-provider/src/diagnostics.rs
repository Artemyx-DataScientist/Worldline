//! Engine-neutral browser provider diagnostics observation types.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::{BrowserContextId, DocumentRevision, PageId};
use worldline_browser_contract::request_policy::RequestResourceType;

/// Normalized console severity level.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConsoleLogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl fmt::Display for ProviderConsoleLogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Debug => write!(f, "debug"),
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Normalized network request status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNetworkRequestStatus {
    Completed,
    Failed,
    Blocked,
}

impl fmt::Display for ProviderNetworkRequestStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Blocked => write!(f, "blocked"),
        }
    }
}

/// Normalized diagnostic observation emitted by a browser engine backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderDiagnosticEvent {
    Console {
        context_id: BrowserContextId,
        page_id: PageId,
        document_revision: DocumentRevision,
        level: ProviderConsoleLogLevel,
        message: String,
        source: Option<String>,
        line: Option<u32>,
        timestamp_epoch_ms: u64,
    },
    Network {
        context_id: BrowserContextId,
        page_id: PageId,
        document_revision: DocumentRevision,
        request_id: String,
        method: String,
        resource_type: RequestResourceType,
        url: String,
        status: ProviderNetworkRequestStatus,
        http_status: Option<u16>,
        mime_type: Option<String>,
        received_bytes: Option<u64>,
        duration_ms: Option<u64>,
        timestamp_epoch_ms: u64,
    },
}

/// Thread-safe sink for consuming provider diagnostic events.
pub trait DiagnosticSink: Send + Sync + std::fmt::Debug {
    fn on_diagnostic_event(&self, event: ProviderDiagnosticEvent);
}

/// In-memory diagnostic sink storing drained events.
#[derive(Default, Debug)]
pub struct MemoryDiagnosticSink {
    events: std::sync::Mutex<Vec<ProviderDiagnosticEvent>>,
}

impl MemoryDiagnosticSink {
    pub fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn drain(&self) -> Vec<ProviderDiagnosticEvent> {
        let mut guard = self.events.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

impl DiagnosticSink for MemoryDiagnosticSink {
    fn on_diagnostic_event(&self, event: ProviderDiagnosticEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// Shared reference to a diagnostic sink.
pub type SharedDiagnosticSink = Arc<dyn DiagnosticSink>;
