use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::{DocumentRevision, NavigationId, PageId};
use worldline_browser_services_contract::{HistoryEntry, HistoryEntryId};

use std::fmt;

/// Error returned when a redelivered navigation event carries conflicting immutable data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsistencyError {
    pub navigation_id: NavigationId,
    pub existing_url: String,
    pub conflicting_url: String,
}

impl fmt::Display for ConsistencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Conflicting navigation commit for navigation {}: existing url='{}', conflicting url='{}'",
            self.navigation_id, self.existing_url, self.conflicting_url
        )
    }
}

impl std::error::Error for ConsistencyError {}

/// Transactional snapshot of the history store.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoryStoreSnapshot {
    pub entries: BTreeMap<HistoryEntryId, HistoryEntry>,
    pub navigation_to_entry: BTreeMap<NavigationId, HistoryEntryId>,
    pub next_entry_index: u64,
}

impl HistoryStoreSnapshot {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            navigation_to_entry: BTreeMap::new(),
            next_entry_index: 1,
        }
    }

    pub fn generate_entry_id(&mut self) -> HistoryEntryId {
        let id = HistoryEntryId::new(format!("hist-{}", self.next_entry_index));
        self.next_entry_index += 1;
        id
    }

    /// Records a committed navigation event idempotently.
    pub fn record_navigation(
        &mut self,
        page_id: PageId,
        navigation_id: NavigationId,
        document_revision: DocumentRevision,
        url: String,
        committed_at_unix_ms: u64,
    ) -> Result<HistoryEntry, ConsistencyError> {
        if let Some(existing) = self
            .navigation_to_entry
            .get(&navigation_id)
            .and_then(|id| self.entries.get_mut(id))
        {
            // Check if immutable fields match
            if existing.url == url
                && existing.page_id == page_id
                && existing.document_revision == document_revision
            {
                // Idempotent duplicate: return existing without modifying
                return Ok(existing.clone());
            } else {
                return Err(ConsistencyError {
                    navigation_id,
                    existing_url: existing.url.clone(),
                    conflicting_url: url,
                });
            }
        }

        let entry_id = self.generate_entry_id();
        let entry = HistoryEntry {
            entry_id: entry_id.clone(),
            page_id,
            navigation_id: navigation_id.clone(),
            document_revision,
            url,
            title: None,
            committed_at_unix_ms,
            visit_count: 1,
        };

        self.entries.insert(entry_id.clone(), entry.clone());
        self.navigation_to_entry.insert(navigation_id, entry_id);

        Ok(entry)
    }

    /// Enriches the title of an entry matching (page_id, document_revision).
    pub fn enrich_title(
        &mut self,
        page_id: &PageId,
        document_revision: DocumentRevision,
        title: String,
    ) -> Option<HistoryEntry> {
        for entry in self.entries.values_mut().rev() {
            if &entry.page_id == page_id && entry.document_revision == document_revision {
                entry.title = Some(title);
                return Some(entry.clone());
            }
        }
        None
    }

    pub fn get_entry(&self, entry_id: &HistoryEntryId) -> Option<&HistoryEntry> {
        self.entries.get(entry_id)
    }

    pub fn query_history(
        &self,
        query: Option<&str>,
        max_results: Option<usize>,
        start_time_unix_ms: Option<u64>,
        end_time_unix_ms: Option<u64>,
    ) -> (Vec<HistoryEntry>, usize) {
        let query_lower = query.map(|q| q.to_lowercase());

        let mut matched: Vec<HistoryEntry> = self
            .entries
            .values()
            .filter(|e| {
                if let Some(start) = start_time_unix_ms
                    && e.committed_at_unix_ms < start
                {
                    return false;
                }
                if let Some(end) = end_time_unix_ms
                    && e.committed_at_unix_ms > end
                {
                    return false;
                }
                if let Some(ref q) = query_lower {
                    let url_match = e.url.to_lowercase().contains(q);
                    let title_match = e
                        .title
                        .as_ref()
                        .is_some_and(|t| t.to_lowercase().contains(q));
                    if !url_match && !title_match {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        // Sort descending by commit timestamp
        matched.sort_by_key(|a| std::cmp::Reverse(a.committed_at_unix_ms));

        let total_count = matched.len();
        if let Some(limit) = max_results {
            matched.truncate(limit);
        }

        (matched, total_count)
    }

    pub fn delete_entry(&mut self, entry_id: &HistoryEntryId) -> bool {
        if let Some(entry) = self.entries.remove(entry_id) {
            self.navigation_to_entry.remove(&entry.navigation_id);
            true
        } else {
            false
        }
    }

    pub fn clear_history(
        &mut self,
        start_time_unix_ms: Option<u64>,
        end_time_unix_ms: Option<u64>,
    ) -> usize {
        let to_remove: Vec<HistoryEntryId> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                if let Some(start) = start_time_unix_ms
                    && e.committed_at_unix_ms < start
                {
                    return false;
                }
                if let Some(end) = end_time_unix_ms
                    && e.committed_at_unix_ms > end
                {
                    return false;
                }
                true
            })
            .map(|(id, _)| id.clone())
            .collect();

        let count = to_remove.len();
        for id in to_remove {
            self.delete_entry(&id);
        }
        count
    }
}
