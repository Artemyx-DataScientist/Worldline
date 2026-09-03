//! Experimental browser.devtools v0.1 contract definitions.

use std::fmt;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::LoadingState;
use worldline_browser_contract::identity::{BrowserContextId, DocumentRevision, PageId};
use worldline_browser_contract::request_policy::RequestResourceType;

pub const CONTRACT_BROWSER_DEVTOOLS: &str = "browser.devtools";
pub const CONTRACT_BROWSER_DEVTOOLS_VERSION: &str = "0.1";

pub const OP_QUERY_CONSOLE_RECORDS: &str = "browser.devtools.query_console";
pub const OP_QUERY_NETWORK_RECORDS: &str = "browser.devtools.query_network";
pub const OP_GET_RUNTIME_SNAPSHOT: &str = "browser.devtools.get_runtime_snapshot";
pub const OP_CLEAR_DIAGNOSTICS: &str = "browser.devtools.clear";
pub const OP_SHOW_NATIVE_DEVTOOLS: &str = "browser.devtools.show_native";

pub const AUTH_DEVTOOLS_OBSERVE: &str = "browser.devtools.observe";
pub const AUTH_DEVTOOLS_CONTROL: &str = "browser.devtools.control";
pub const AUTH_DEVTOOLS_NATIVE: &str = "browser.devtools.native";

pub const MAX_CONSOLE_MESSAGE_LENGTH: usize = 2048;
pub const MAX_DIAGNOSTIC_URL_LENGTH: usize = 2048;
pub const MAX_SOURCE_LENGTH: usize = 1024;
pub const DEFAULT_BUFFER_CAPACITY: usize = 500;

/// Normalized console severity level.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleLogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl fmt::Display for ConsoleLogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Debug => write!(f, "debug"),
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Normalized network request completion status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkRequestStatus {
    Completed,
    Failed,
    Blocked,
}

impl fmt::Display for NetworkRequestStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Blocked => write!(f, "blocked"),
        }
    }
}

/// A normalized, length-bounded console diagnostic observation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConsoleDiagnosticRecord {
    pub record_id: u64,
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub level: ConsoleLogLevel,
    pub message: String,
    pub source: Option<String>,
    pub line: Option<u32>,
    pub timestamp_epoch_ms: u64,
    pub truncated: bool,
}

impl ConsoleDiagnosticRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_id: u64,
        context_id: BrowserContextId,
        page_id: PageId,
        document_revision: DocumentRevision,
        level: ConsoleLogLevel,
        message: &str,
        source: Option<&str>,
        line: Option<u32>,
        timestamp_epoch_ms: u64,
    ) -> Self {
        let (bounded_message, msg_truncated) = truncate_string(message, MAX_CONSOLE_MESSAGE_LENGTH);
        let (bounded_source, src_truncated) = match source {
            Some(src) => {
                let (s, t) = truncate_string(src, MAX_SOURCE_LENGTH);
                (Some(s), t)
            }
            None => (None, false),
        };

        Self {
            record_id,
            context_id,
            page_id,
            document_revision,
            level,
            message: bounded_message,
            source: bounded_source,
            line,
            timestamp_epoch_ms,
            truncated: msg_truncated || src_truncated,
        }
    }
}

/// A normalized, privacy-preserving network request observation.
/// Excludes cookies, authorization headers, request/response bodies, client certs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkDiagnosticRecord {
    pub record_id: u64,
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub request_id: String,
    pub method: String,
    pub resource_type: RequestResourceType,
    pub url: String,
    pub status: NetworkRequestStatus,
    pub http_status: Option<u16>,
    pub mime_type: Option<String>,
    pub received_bytes: Option<u64>,
    pub duration_ms: Option<u64>,
    pub timestamp_epoch_ms: u64,
    pub truncated: bool,
}

impl NetworkDiagnosticRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_id: u64,
        context_id: BrowserContextId,
        page_id: PageId,
        document_revision: DocumentRevision,
        request_id: impl Into<String>,
        method: impl Into<String>,
        resource_type: RequestResourceType,
        url: &str,
        status: NetworkRequestStatus,
        http_status: Option<u16>,
        mime_type: Option<String>,
        received_bytes: Option<u64>,
        duration_ms: Option<u64>,
        timestamp_epoch_ms: u64,
    ) -> Self {
        let (bounded_url, truncated) = truncate_string(url, MAX_DIAGNOSTIC_URL_LENGTH);
        Self {
            record_id,
            context_id,
            page_id,
            document_revision,
            request_id: request_id.into(),
            method: method.into(),
            resource_type,
            url: bounded_url,
            status,
            http_status,
            mime_type,
            received_bytes,
            duration_ms,
            timestamp_epoch_ms,
            truncated,
        }
    }
}

/// Diagnostic snapshot of page runtime state without platform handles.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageRuntimeDiagnosticSnapshot {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub url: String,
    pub title: String,
    pub loading_state: LoadingState,
    pub status_code: u16,
    pub crashed: bool,
    pub timestamp_epoch_ms: u64,
}

/// Diagnostic buffer operational metrics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticBufferStats {
    pub retained_console_records: usize,
    pub dropped_console_records: usize,
    pub truncated_console_records: usize,
    pub retained_network_records: usize,
    pub dropped_network_records: usize,
    pub truncated_network_records: usize,
}

/// Request to query console diagnostic records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryConsoleRecordsRequest {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub document_revision: Option<DocumentRevision>,
    pub min_level: Option<ConsoleLogLevel>,
    pub limit: Option<usize>,
    pub since_record_id: Option<u64>,
}

/// Response containing queried console diagnostic records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryConsoleRecordsResponse {
    pub records: Vec<ConsoleDiagnosticRecord>,
    pub stats: DiagnosticBufferStats,
}

/// Request to query network diagnostic records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryNetworkRecordsRequest {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub document_revision: Option<DocumentRevision>,
    pub resource_type: Option<RequestResourceType>,
    pub status: Option<NetworkRequestStatus>,
    pub limit: Option<usize>,
    pub since_record_id: Option<u64>,
}

/// Response containing queried network diagnostic records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryNetworkRecordsResponse {
    pub records: Vec<NetworkDiagnosticRecord>,
    pub stats: DiagnosticBufferStats,
}

/// Request for a page runtime diagnostic snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetRuntimeSnapshotRequest {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
}

/// Response containing the page runtime diagnostic snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetRuntimeSnapshotResponse {
    pub snapshot: PageRuntimeDiagnosticSnapshot,
}

/// Request to clear diagnostic buffers for a page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearDiagnosticsRequest {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
}

/// Response for clear diagnostics operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearDiagnosticsResponse {
    pub cleared: bool,
}

/// Request to show native provider DevTools window.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShowNativeDevToolsRequest {
    pub context_id: BrowserContextId,
    pub page_id: PageId,
}

/// Response from showing native provider DevTools window.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShowNativeDevToolsResponse {
    pub supported: bool,
    pub opened: bool,
}

/// Truncate string to max_len bytes safely on UTF-8 character boundary.
pub fn truncate_string(input: &str, max_len: usize) -> (String, bool) {
    if input.len() <= max_len {
        return (input.to_string(), false);
    }
    let mut boundary = max_len;
    while boundary > 0 && !input.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (input[..boundary].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_utf8_boundaries() {
        let text = "Привет, мир! 🚀 Antigravity Diagnostics";
        let (truncated, is_trunc) = truncate_string(text, 10);
        assert!(is_trunc);
        assert!(truncated.len() <= 10);
        // Valid utf-8
        std::str::from_utf8(truncated.as_bytes()).unwrap();
    }

    #[test]
    fn console_record_truncation_sets_flag() {
        let long_msg = "A".repeat(MAX_CONSOLE_MESSAGE_LENGTH + 100);
        let rec = ConsoleDiagnosticRecord::new(
            1,
            BrowserContextId::new("ctx-1"),
            PageId::new("page-1"),
            DocumentRevision::new(1),
            ConsoleLogLevel::Error,
            &long_msg,
            Some("test.js"),
            Some(42),
            1000,
        );
        assert!(rec.truncated);
        assert_eq!(rec.message.len(), MAX_CONSOLE_MESSAGE_LENGTH);
    }

    #[test]
    fn network_record_url_truncation_sets_flag() {
        let long_url = format!(
            "https://example.com/{}",
            "x".repeat(MAX_DIAGNOSTIC_URL_LENGTH)
        );
        let rec = NetworkDiagnosticRecord::new(
            1,
            BrowserContextId::new("ctx-1"),
            PageId::new("page-1"),
            DocumentRevision::new(1),
            "req-1",
            "GET",
            RequestResourceType::Xhr,
            &long_url,
            NetworkRequestStatus::Completed,
            Some(200),
            Some("application/json".to_string()),
            Some(1234),
            Some(45),
            1000,
        );
        assert!(rec.truncated);
        assert_eq!(rec.url.len(), MAX_DIAGNOSTIC_URL_LENGTH);
    }

    #[test]
    fn serialization_roundtrip() {
        let rec = ConsoleDiagnosticRecord::new(
            1,
            BrowserContextId::new("ctx-1"),
            PageId::new("page-1"),
            DocumentRevision::new(1),
            ConsoleLogLevel::Warning,
            "warn msg",
            Some("index.html"),
            Some(10),
            1000,
        );
        let json = serde_json::to_string(&rec).unwrap();
        let deserialized: ConsoleDiagnosticRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, deserialized);
    }
}
