//! Experimental browser.history v0.1 contract definitions.

use std::fmt;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::{DocumentRevision, NavigationId, PageId};

pub const CONTRACT_BROWSER_HISTORY: &str = "browser.history";
pub const CONTRACT_BROWSER_HISTORY_VERSION: &str = "0.1";

pub const OP_QUERY_HISTORY: &str = "browser.history.query";
pub const OP_GET_HISTORY_ENTRY: &str = "browser.history.get";
pub const OP_DELETE_HISTORY_ENTRY: &str = "browser.history.delete";
pub const OP_CLEAR_HISTORY: &str = "browser.history.clear";

pub const AUTH_HISTORY_READ: &str = "browser.history.read";
pub const AUTH_HISTORY_DELETE: &str = "browser.history.delete";

/// Opaque identity of a durable history entry.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct HistoryEntryId(String);

impl HistoryEntryId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HistoryEntryId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for HistoryEntryId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for HistoryEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Durable record of a committed navigation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub entry_id: HistoryEntryId,
    pub page_id: PageId,
    pub navigation_id: NavigationId,
    pub document_revision: DocumentRevision,
    pub url: String,
    pub title: Option<String>,
    pub committed_at_unix_ms: u64,
    pub visit_count: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryHistoryRequest {
    pub query: Option<String>,
    pub max_results: Option<usize>,
    pub start_time_unix_ms: Option<u64>,
    pub end_time_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryHistoryResponse {
    pub entries: Vec<HistoryEntry>,
    pub total_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetHistoryEntryRequest {
    pub entry_id: HistoryEntryId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetHistoryEntryResponse {
    pub entry: HistoryEntry,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteHistoryEntryRequest {
    pub entry_id: HistoryEntryId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteHistoryEntryResponse {
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearHistoryRequest {
    pub start_time_unix_ms: Option<u64>,
    pub end_time_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearHistoryResponse {
    pub deleted_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_entry_id_roundtrip() {
        let id = HistoryEntryId::new("hist-abc");
        assert_eq!(id.as_str(), "hist-abc");
        assert_eq!(id.to_string(), "hist-abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: HistoryEntryId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }
}
