use std::sync::Mutex;

use serde_json::Value;
use worldline_browser_contract::identity::{DocumentRevision, NavigationId, PageId};
use worldline_browser_services_contract::{
    ClearHistoryRequest, ClearHistoryResponse, DeleteHistoryEntryRequest,
    DeleteHistoryEntryResponse, GetHistoryEntryRequest, GetHistoryEntryResponse, HistoryEntry,
    OP_CLEAR_HISTORY, OP_DELETE_HISTORY_ENTRY, OP_GET_HISTORY_ENTRY, OP_QUERY_HISTORY,
    QueryHistoryRequest, QueryHistoryResponse,
};

use crate::store::{ConsistencyError, HistoryStoreSnapshot};

/// History service managing durable user navigation records and deduplication above the engine provider.
pub struct HistoryService {
    state: Mutex<HistoryStoreSnapshot>,
}

impl Default for HistoryService {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HistoryStoreSnapshot::new()),
        }
    }

    pub fn from_snapshot(snapshot: HistoryStoreSnapshot) -> Self {
        Self {
            state: Mutex::new(snapshot),
        }
    }

    pub fn export_snapshot(&self) -> HistoryStoreSnapshot {
        self.state.lock().unwrap().clone()
    }

    /// Records committed navigation fact observed from browser.navigation.committed event.
    pub fn record_navigation(
        &self,
        page_id: PageId,
        navigation_id: NavigationId,
        document_revision: DocumentRevision,
        url: String,
        committed_at_unix_ms: u64,
    ) -> Result<HistoryEntry, ConsistencyError> {
        let mut state = self.state.lock().unwrap();
        state.record_navigation(
            page_id,
            navigation_id,
            document_revision,
            url,
            committed_at_unix_ms,
        )
    }

    /// Enriches page title observed from browser.page.ready event.
    pub fn enrich_title(
        &self,
        page_id: &PageId,
        document_revision: DocumentRevision,
        title: String,
    ) -> Option<HistoryEntry> {
        let mut state = self.state.lock().unwrap();
        state.enrich_title(page_id, document_revision, title)
    }

    pub fn query_history(&self, req: QueryHistoryRequest) -> QueryHistoryResponse {
        let state = self.state.lock().unwrap();
        let (entries, total_count) = state.query_history(
            req.query.as_deref(),
            req.max_results,
            req.start_time_unix_ms,
            req.end_time_unix_ms,
        );
        QueryHistoryResponse {
            entries,
            total_count,
        }
    }

    pub fn get_history_entry(
        &self,
        req: GetHistoryEntryRequest,
    ) -> Result<GetHistoryEntryResponse, String> {
        let state = self.state.lock().unwrap();
        state
            .get_entry(&req.entry_id)
            .cloned()
            .map(|entry| GetHistoryEntryResponse { entry })
            .ok_or_else(|| format!("History entry '{}' not found", req.entry_id))
    }

    pub fn delete_history_entry(
        &self,
        req: DeleteHistoryEntryRequest,
    ) -> DeleteHistoryEntryResponse {
        let mut state = self.state.lock().unwrap();
        let deleted = state.delete_entry(&req.entry_id);
        DeleteHistoryEntryResponse { deleted }
    }

    pub fn clear_history(&self, req: ClearHistoryRequest) -> ClearHistoryResponse {
        let mut state = self.state.lock().unwrap();
        let deleted_count = state.clear_history(req.start_time_unix_ms, req.end_time_unix_ms);
        ClearHistoryResponse { deleted_count }
    }

    /// Dispatches RPC operations to the corresponding handler.
    pub fn dispatch(&self, operation: &str, payload: Value) -> Result<Value, String> {
        match operation {
            OP_QUERY_HISTORY => {
                let req: QueryHistoryRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.query_history(req);
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_GET_HISTORY_ENTRY => {
                let req: GetHistoryEntryRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.get_history_entry(req)?;
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_DELETE_HISTORY_ENTRY => {
                let req: DeleteHistoryEntryRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.delete_history_entry(req);
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_CLEAR_HISTORY => {
                let req: ClearHistoryRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.clear_history(req);
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            unknown => Err(format!("Unsupported history operation '{unknown}'")),
        }
    }
}
