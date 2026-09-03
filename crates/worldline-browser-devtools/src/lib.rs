//! Bounded engine-neutral browser devtools diagnostics service.
//!
//! Owns ephemeral in-memory ring buffers per `(BrowserContextId, PageId)`.
//! Enforces finite entry bounds, drop-oldest overflow policy, field truncation,
//! and strict context/page isolation.

#![forbid(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use worldline_browser_contract::LoadingState;
use worldline_browser_contract::identity::{BrowserContextId, DocumentRevision, PageId};
use worldline_browser_contract::request_policy::RequestResourceType;
pub use worldline_browser_services_contract::devtools::*;

/// Error types for diagnostics service operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DevToolsError {
    AccessDenied(String),
    PageNotFound {
        context_id: BrowserContextId,
        page_id: PageId,
    },
    InvalidQuery(String),
}

impl std::fmt::Display for DevToolsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessDenied(msg) => write!(f, "access denied: {msg}"),
            Self::PageNotFound {
                context_id,
                page_id,
            } => {
                write!(f, "page not found: context={context_id}, page={page_id}")
            }
            Self::InvalidQuery(msg) => write!(f, "invalid query: {msg}"),
        }
    }
}

impl std::error::Error for DevToolsError {}

/// Per-(Context, Page) bounded ring buffer for diagnostic observations.
#[derive(Debug)]
pub struct PageDiagnosticBuffer {
    pub capacity: usize,
    pub console_records: VecDeque<ConsoleDiagnosticRecord>,
    pub network_records: VecDeque<NetworkDiagnosticRecord>,
    pub next_console_id: u64,
    pub next_network_id: u64,
    pub dropped_console_records: usize,
    pub truncated_console_records: usize,
    pub dropped_network_records: usize,
    pub truncated_network_records: usize,
}

impl PageDiagnosticBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            console_records: VecDeque::new(),
            network_records: VecDeque::new(),
            next_console_id: 1,
            next_network_id: 1,
            dropped_console_records: 0,
            truncated_console_records: 0,
            dropped_network_records: 0,
            truncated_network_records: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_console(
        &mut self,
        context_id: BrowserContextId,
        page_id: PageId,
        document_revision: DocumentRevision,
        level: ConsoleLogLevel,
        message: &str,
        source: Option<&str>,
        line: Option<u32>,
        timestamp_epoch_ms: u64,
    ) -> u64 {
        let id = self.next_console_id;
        self.next_console_id = self.next_console_id.wrapping_add(1);

        let record = ConsoleDiagnosticRecord::new(
            id,
            context_id,
            page_id,
            document_revision,
            level,
            message,
            source,
            line,
            timestamp_epoch_ms,
        );

        if record.truncated {
            self.truncated_console_records += 1;
        }

        if self.console_records.len() >= self.capacity {
            self.console_records.pop_front();
            self.dropped_console_records += 1;
        }

        self.console_records.push_back(record);
        id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_network(
        &mut self,
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
    ) -> u64 {
        let id = self.next_network_id;
        self.next_network_id = self.next_network_id.wrapping_add(1);

        let record = NetworkDiagnosticRecord::new(
            id,
            context_id,
            page_id,
            document_revision,
            request_id,
            method,
            resource_type,
            url,
            status,
            http_status,
            mime_type,
            received_bytes,
            duration_ms,
            timestamp_epoch_ms,
        );

        if record.truncated {
            self.truncated_network_records += 1;
        }

        if self.network_records.len() >= self.capacity {
            self.network_records.pop_front();
            self.dropped_network_records += 1;
        }

        self.network_records.push_back(record);
        id
    }

    pub fn stats(&self) -> DiagnosticBufferStats {
        DiagnosticBufferStats {
            retained_console_records: self.console_records.len(),
            dropped_console_records: self.dropped_console_records,
            truncated_console_records: self.truncated_console_records,
            retained_network_records: self.network_records.len(),
            dropped_network_records: self.dropped_network_records,
            truncated_network_records: self.truncated_network_records,
        }
    }

    pub fn clear(&mut self) {
        self.console_records.clear();
        self.network_records.clear();
    }
}

/// Thread-safe service plugin managing page diagnostic buffers.
#[derive(Clone, Debug)]
pub struct BrowserDevToolsService {
    default_capacity: usize,
    buffers: Arc<Mutex<HashMap<(BrowserContextId, PageId), PageDiagnosticBuffer>>>,
    runtime_snapshots:
        Arc<Mutex<HashMap<(BrowserContextId, PageId), PageRuntimeDiagnosticSnapshot>>>,
}

impl Default for BrowserDevToolsService {
    fn default() -> Self {
        Self::new(DEFAULT_BUFFER_CAPACITY)
    }
}

impl BrowserDevToolsService {
    pub fn new(default_capacity: usize) -> Self {
        Self {
            default_capacity: default_capacity.max(1),
            buffers: Arc::new(Mutex::new(HashMap::new())),
            runtime_snapshots: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record a console message observation.
    #[allow(clippy::too_many_arguments)]
    pub fn record_console(
        &self,
        context_id: BrowserContextId,
        page_id: PageId,
        document_revision: DocumentRevision,
        level: ConsoleLogLevel,
        message: &str,
        source: Option<&str>,
        line: Option<u32>,
        timestamp_epoch_ms: u64,
    ) -> u64 {
        let mut buffers = self.buffers.lock().unwrap();
        let buffer = buffers
            .entry((context_id.clone(), page_id.clone()))
            .or_insert_with(|| PageDiagnosticBuffer::new(self.default_capacity));

        buffer.record_console(
            context_id,
            page_id,
            document_revision,
            level,
            message,
            source,
            line,
            timestamp_epoch_ms,
        )
    }

    /// Record a network/resource observation.
    #[allow(clippy::too_many_arguments)]
    pub fn record_network(
        &self,
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
    ) -> u64 {
        let mut buffers = self.buffers.lock().unwrap();
        let buffer = buffers
            .entry((context_id.clone(), page_id.clone()))
            .or_insert_with(|| PageDiagnosticBuffer::new(self.default_capacity));

        buffer.record_network(
            context_id,
            page_id,
            document_revision,
            request_id,
            method,
            resource_type,
            url,
            status,
            http_status,
            mime_type,
            received_bytes,
            duration_ms,
            timestamp_epoch_ms,
        )
    }

    /// Update page runtime snapshot facts.
    pub fn update_runtime_snapshot(&self, snapshot: PageRuntimeDiagnosticSnapshot) {
        let key = (snapshot.context_id.clone(), snapshot.page_id.clone());
        let mut snapshots = self.runtime_snapshots.lock().unwrap();
        snapshots.insert(key, snapshot);
    }

    /// Query console diagnostic records for an admitted (Context, Page) scope.
    pub fn query_console(
        &self,
        req: &QueryConsoleRecordsRequest,
    ) -> Result<QueryConsoleRecordsResponse, DevToolsError> {
        let buffers = self.buffers.lock().unwrap();
        let key = (req.context_id.clone(), req.page_id.clone());
        let buffer = match buffers.get(&key) {
            Some(b) => b,
            None => {
                return Ok(QueryConsoleRecordsResponse {
                    records: Vec::new(),
                    stats: DiagnosticBufferStats::default(),
                });
            }
        };

        let mut filtered: Vec<ConsoleDiagnosticRecord> = buffer
            .console_records
            .iter()
            .filter(|rec| {
                if let Some(rev) = req.document_revision
                    && rec.document_revision != rev
                {
                    return false;
                }
                if let Some(min_lvl) = req.min_level
                    && rec.level < min_lvl
                {
                    return false;
                }
                if let Some(since_id) = req.since_record_id
                    && rec.record_id <= since_id
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        if let Some(limit) = req.limit
            && filtered.len() > limit
        {
            filtered.truncate(limit);
        }

        Ok(QueryConsoleRecordsResponse {
            records: filtered,
            stats: buffer.stats(),
        })
    }

    /// Query network diagnostic records for an admitted (Context, Page) scope.
    pub fn query_network(
        &self,
        req: &QueryNetworkRecordsRequest,
    ) -> Result<QueryNetworkRecordsResponse, DevToolsError> {
        let buffers = self.buffers.lock().unwrap();
        let key = (req.context_id.clone(), req.page_id.clone());
        let buffer = match buffers.get(&key) {
            Some(b) => b,
            None => {
                return Ok(QueryNetworkRecordsResponse {
                    records: Vec::new(),
                    stats: DiagnosticBufferStats::default(),
                });
            }
        };

        let mut filtered: Vec<NetworkDiagnosticRecord> = buffer
            .network_records
            .iter()
            .filter(|rec| {
                if let Some(rev) = req.document_revision
                    && rec.document_revision != rev
                {
                    return false;
                }
                if let Some(rt) = req.resource_type
                    && rec.resource_type != rt
                {
                    return false;
                }
                if let Some(st) = req.status
                    && rec.status != st
                {
                    return false;
                }
                if let Some(since_id) = req.since_record_id
                    && rec.record_id <= since_id
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        if let Some(limit) = req.limit
            && filtered.len() > limit
        {
            filtered.truncate(limit);
        }

        Ok(QueryNetworkRecordsResponse {
            records: filtered,
            stats: buffer.stats(),
        })
    }

    /// Retrieve runtime diagnostic snapshot.
    pub fn get_runtime_snapshot(
        &self,
        req: &GetRuntimeSnapshotRequest,
    ) -> Result<GetRuntimeSnapshotResponse, DevToolsError> {
        let snapshots = self.runtime_snapshots.lock().unwrap();
        let key = (req.context_id.clone(), req.page_id.clone());
        let snapshot =
            snapshots
                .get(&key)
                .cloned()
                .unwrap_or_else(|| PageRuntimeDiagnosticSnapshot {
                    context_id: req.context_id.clone(),
                    page_id: req.page_id.clone(),
                    document_revision: DocumentRevision::new(0),
                    url: String::new(),
                    title: String::new(),
                    loading_state: LoadingState::Complete,
                    status_code: 0,
                    crashed: false,
                    timestamp_epoch_ms: 0,
                });

        Ok(GetRuntimeSnapshotResponse { snapshot })
    }

    /// Clear diagnostics for an admitted Page.
    pub fn clear(
        &self,
        req: &ClearDiagnosticsRequest,
    ) -> Result<ClearDiagnosticsResponse, DevToolsError> {
        let mut buffers = self.buffers.lock().unwrap();
        let key = (req.context_id.clone(), req.page_id.clone());
        if let Some(buffer) = buffers.get_mut(&key) {
            buffer.clear();
        }
        Ok(ClearDiagnosticsResponse { cleared: true })
    }

    /// Lifecycle cleanup: remove buffer and snapshot when a Page is closed.
    pub fn close_page(&self, context_id: &BrowserContextId, page_id: &PageId) {
        let key = (context_id.clone(), page_id.clone());
        self.buffers.lock().unwrap().remove(&key);
        self.runtime_snapshots.lock().unwrap().remove(&key);
    }

    /// Lifecycle cleanup: remove all buffers and snapshots when a Context is closed.
    pub fn close_context(&self, context_id: &BrowserContextId) {
        self.buffers
            .lock()
            .unwrap()
            .retain(|(ctx, _), _| ctx != context_id);
        self.runtime_snapshots
            .lock()
            .unwrap()
            .retain(|(ctx, _), _| ctx != context_id);
    }

    /// Ingest an engine-neutral provider diagnostic event.
    pub fn ingest_provider_event(
        &self,
        event: &worldline_browser_provider::ProviderDiagnosticEvent,
    ) {
        match event {
            worldline_browser_provider::ProviderDiagnosticEvent::Console {
                context_id,
                page_id,
                document_revision,
                level,
                message,
                source,
                line,
                timestamp_epoch_ms,
            } => {
                let lvl = match level {
                    worldline_browser_provider::ProviderConsoleLogLevel::Debug => {
                        ConsoleLogLevel::Debug
                    }
                    worldline_browser_provider::ProviderConsoleLogLevel::Info => {
                        ConsoleLogLevel::Info
                    }
                    worldline_browser_provider::ProviderConsoleLogLevel::Warning => {
                        ConsoleLogLevel::Warning
                    }
                    worldline_browser_provider::ProviderConsoleLogLevel::Error => {
                        ConsoleLogLevel::Error
                    }
                };
                self.record_console(
                    context_id.clone(),
                    page_id.clone(),
                    *document_revision,
                    lvl,
                    message,
                    source.as_deref(),
                    *line,
                    *timestamp_epoch_ms,
                );
            }
            worldline_browser_provider::ProviderDiagnosticEvent::Network {
                context_id,
                page_id,
                document_revision,
                request_id,
                method,
                resource_type,
                url,
                status,
                http_status,
                mime_type,
                received_bytes,
                duration_ms,
                timestamp_epoch_ms,
            } => {
                let st = match status {
                    worldline_browser_provider::ProviderNetworkRequestStatus::Completed => {
                        NetworkRequestStatus::Completed
                    }
                    worldline_browser_provider::ProviderNetworkRequestStatus::Failed => {
                        NetworkRequestStatus::Failed
                    }
                    worldline_browser_provider::ProviderNetworkRequestStatus::Blocked => {
                        NetworkRequestStatus::Blocked
                    }
                };
                self.record_network(
                    context_id.clone(),
                    page_id.clone(),
                    *document_revision,
                    request_id,
                    method,
                    *resource_type,
                    url,
                    st,
                    *http_status,
                    mime_type.clone(),
                    *received_bytes,
                    *duration_ms,
                    *timestamp_epoch_ms,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_overflow_enforces_capacity_and_drop_counters() {
        let mut buffer = PageDiagnosticBuffer::new(5);
        let ctx = BrowserContextId::new("ctx-1");
        let page = PageId::new("page-1");
        let rev = DocumentRevision::new(1);

        for i in 0..15 {
            buffer.record_console(
                ctx.clone(),
                page.clone(),
                rev,
                ConsoleLogLevel::Info,
                &format!("msg {i}"),
                None,
                None,
                1000 + i as u64,
            );
        }

        let stats = buffer.stats();
        assert_eq!(stats.retained_console_records, 5);
        assert_eq!(stats.dropped_console_records, 10);
        assert_eq!(buffer.console_records.len(), 5);
        // Retained records are the newest 5 (10..15)
        assert_eq!(buffer.console_records.front().unwrap().message, "msg 10");
        assert_eq!(buffer.console_records.back().unwrap().message, "msg 14");
    }

    #[test]
    fn exact_context_and_page_isolation() {
        let service = BrowserDevToolsService::new(100);
        let ctx_a = BrowserContextId::new("ctx-a");
        let ctx_b = BrowserContextId::new("ctx-b");
        let page_1 = PageId::new("page-1");
        let rev = DocumentRevision::new(1);

        service.record_console(
            ctx_a.clone(),
            page_1.clone(),
            rev,
            ConsoleLogLevel::Error,
            "Secret Error in Context A",
            None,
            None,
            1000,
        );

        // Querying for Context B returns empty, despite same page ID
        let resp_b = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: ctx_b,
                page_id: page_1.clone(),
                document_revision: None,
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .unwrap();
        assert_eq!(resp_b.records.len(), 0);

        // Querying for Context A returns the record
        let resp_a = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: ctx_a,
                page_id: page_1,
                document_revision: None,
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .unwrap();
        assert_eq!(resp_a.records.len(), 1);
        assert_eq!(resp_a.records[0].message, "Secret Error in Context A");
    }

    #[test]
    fn document_revision_isolation() {
        let service = BrowserDevToolsService::new(100);
        let ctx = BrowserContextId::new("ctx-1");
        let page = PageId::new("page-1");
        let rev_1 = DocumentRevision::new(1);
        let rev_2 = DocumentRevision::new(2);

        service.record_console(
            ctx.clone(),
            page.clone(),
            rev_1,
            ConsoleLogLevel::Info,
            "Page 1 initial log",
            None,
            None,
            1000,
        );
        service.record_console(
            ctx.clone(),
            page.clone(),
            rev_2,
            ConsoleLogLevel::Info,
            "Page 2 navigated log",
            None,
            None,
            2000,
        );

        // Query rev 1
        let resp_1 = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: ctx.clone(),
                page_id: page.clone(),
                document_revision: Some(rev_1),
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .unwrap();
        assert_eq!(resp_1.records.len(), 1);
        assert_eq!(resp_1.records[0].message, "Page 1 initial log");

        // Query rev 2
        let resp_2 = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: ctx,
                page_id: page,
                document_revision: Some(rev_2),
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .unwrap();
        assert_eq!(resp_2.records.len(), 1);
        assert_eq!(resp_2.records[0].message, "Page 2 navigated log");
    }

    #[test]
    fn lifecycle_cleanup_removes_buffers() {
        let service = BrowserDevToolsService::new(100);
        let ctx = BrowserContextId::new("ctx-1");
        let page = PageId::new("page-1");
        let rev = DocumentRevision::new(1);

        service.record_console(
            ctx.clone(),
            page.clone(),
            rev,
            ConsoleLogLevel::Info,
            "test log",
            None,
            None,
            1000,
        );

        service.close_page(&ctx, &page);

        let resp = service
            .query_console(&QueryConsoleRecordsRequest {
                context_id: ctx,
                page_id: page,
                document_revision: None,
                min_level: None,
                limit: None,
                since_record_id: None,
            })
            .unwrap();
        assert_eq!(resp.records.len(), 0);
        assert_eq!(resp.stats.retained_console_records, 0);
    }
}
