use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::{BrowserContextId, DownloadId, PageId};
use worldline_browser_services_contract::{
    ArtifactRef, DownloadLifecycleStatus, DownloadRecord, DownloadRecordId,
};

/// Persistent transactional snapshot of download service records.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownloadsSnapshot {
    pub records: BTreeMap<DownloadRecordId, DownloadRecord>,
    pub engine_id_to_record_id: BTreeMap<DownloadId, DownloadRecordId>,
    pub next_record_index: u64,
}

/// Native engine download-start notification passed across the service
/// boundary as one coherent event instead of a positional argument list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineDownloadStarted {
    pub engine_download_id: DownloadId,
    pub context_id: BrowserContextId,
    pub page_id: PageId,
    pub url: String,
    pub suggested_filename: String,
    pub total_bytes: Option<u64>,
    pub media_type: Option<String>,
}

impl DownloadsSnapshot {
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
            engine_id_to_record_id: BTreeMap::new(),
            next_record_index: 1,
        }
    }

    pub fn generate_record_id(&mut self) -> DownloadRecordId {
        let id = DownloadRecordId::new(format!("dl-{}", self.next_record_index));
        self.next_record_index += 1;
        id
    }

    /// Durably creates download intent before issuing a non-idempotent engine operation.
    pub fn create_intent(
        &mut self,
        context_id: BrowserContextId,
        page_id: Option<PageId>,
        url: String,
        suggested_filename: Option<String>,
        started_at_unix_ms: u64,
    ) -> DownloadRecord {
        let record_id = self.generate_record_id();
        let filename = suggested_filename.unwrap_or_else(|| {
            url.split('/')
                .next_back()
                .filter(|s| !s.is_empty())
                .unwrap_or("download")
                .to_string()
        });

        let record = DownloadRecord {
            record_id: record_id.clone(),
            context_id: Some(context_id),
            page_id,
            url,
            suggested_filename: filename,
            media_type: None,
            total_bytes: None,
            received_bytes: 0,
            status: DownloadLifecycleStatus::Pending,
            engine_download_id: None,
            artifact_ref: None,
            error_message: None,
            started_at_unix_ms,
            completed_at_unix_ms: None,
        };

        self.records.insert(record_id.clone(), record.clone());
        record
    }

    /// Binds an engine download ID to an existing durable download record.
    pub fn bind_engine_download(
        &mut self,
        record_id: &DownloadRecordId,
        engine_download_id: DownloadId,
    ) -> bool {
        if let Some(record) = self.records.get_mut(record_id) {
            record.engine_download_id = Some(engine_download_id.clone());
            if record.status == DownloadLifecycleStatus::Pending {
                record.status = DownloadLifecycleStatus::Active;
            }
            self.engine_id_to_record_id
                .insert(engine_download_id, record_id.clone());
            true
        } else {
            false
        }
    }

    /// Idempotently associates an engine download hook event with an existing or new record.
    pub fn handle_engine_download_started(
        &mut self,
        event: EngineDownloadStarted,
        started_at_unix_ms: u64,
    ) -> DownloadRecord {
        let EngineDownloadStarted {
            engine_download_id,
            context_id,
            page_id,
            url,
            suggested_filename,
            total_bytes,
            media_type,
        } = event;
        if let Some(record_id) = self
            .engine_id_to_record_id
            .get(&engine_download_id)
            .cloned()
            && let Some(record) = self.records.get_mut(&record_id)
        {
            record.total_bytes = total_bytes.or(record.total_bytes);
            record.media_type = media_type.or(record.media_type.clone());
            return record.clone();
        }

        // Search for matching pending intent by context_id, page_id, url
        let matching_pending_id = self.records.iter().find_map(|(id, r)| {
            if r.status == DownloadLifecycleStatus::Pending
                && r.context_id.as_ref() == Some(&context_id)
                && r.url == url
            {
                Some(id.clone())
            } else {
                None
            }
        });

        let record_id = if let Some(pending_id) = matching_pending_id {
            pending_id
        } else {
            self.generate_record_id()
        };

        let record = DownloadRecord {
            record_id: record_id.clone(),
            context_id: Some(context_id),
            page_id: Some(page_id),
            url,
            suggested_filename,
            media_type,
            total_bytes,
            received_bytes: 0,
            status: DownloadLifecycleStatus::Active,
            engine_download_id: Some(engine_download_id.clone()),
            artifact_ref: None,
            error_message: None,
            started_at_unix_ms,
            completed_at_unix_ms: None,
        };

        self.records.insert(record_id.clone(), record.clone());
        self.engine_id_to_record_id
            .insert(engine_download_id, record_id);
        record
    }

    /// Updates progress from engine telemetry.
    pub fn update_progress(
        &mut self,
        engine_download_id: &DownloadId,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> Option<DownloadRecord> {
        let record_id = self.engine_id_to_record_id.get(engine_download_id)?.clone();
        let record = self.records.get_mut(&record_id)?;

        record.received_bytes = received_bytes;
        if total_bytes.is_some() {
            record.total_bytes = total_bytes;
        }
        if record.status == DownloadLifecycleStatus::Pending {
            record.status = DownloadLifecycleStatus::Active;
        }
        Some(record.clone())
    }

    /// Marks download as completed and records opaque artifact reference.
    pub fn complete_download(
        &mut self,
        engine_download_id: &DownloadId,
        artifact_ref: ArtifactRef,
        completed_at_unix_ms: u64,
    ) -> Option<DownloadRecord> {
        let record_id = self.engine_id_to_record_id.get(engine_download_id)?.clone();
        let record = self.records.get_mut(&record_id)?;

        record.received_bytes = artifact_ref.size_bytes;
        record.total_bytes = Some(artifact_ref.size_bytes);
        record.status = DownloadLifecycleStatus::Completed;
        record.artifact_ref = Some(artifact_ref);
        record.completed_at_unix_ms = Some(completed_at_unix_ms);
        Some(record.clone())
    }

    /// Marks download as failed.
    pub fn fail_download(
        &mut self,
        engine_download_id: &DownloadId,
        error_message: String,
    ) -> Option<DownloadRecord> {
        let record_id = self.engine_id_to_record_id.get(engine_download_id)?.clone();
        let record = self.records.get_mut(&record_id)?;

        record.status = DownloadLifecycleStatus::Failed;
        record.error_message = Some(error_message);
        Some(record.clone())
    }

    pub fn pause_download(&mut self, record_id: &DownloadRecordId) -> bool {
        if let Some(record) = self.records.get_mut(record_id)
            && (record.status == DownloadLifecycleStatus::Active
                || record.status == DownloadLifecycleStatus::Pending)
        {
            record.status = DownloadLifecycleStatus::Paused;
            return true;
        }
        false
    }

    pub fn resume_download(&mut self, record_id: &DownloadRecordId) -> bool {
        if let Some(record) = self.records.get_mut(record_id)
            && record.status == DownloadLifecycleStatus::Paused
        {
            record.status = DownloadLifecycleStatus::Active;
            return true;
        }
        false
    }

    pub fn cancel_download(&mut self, record_id: &DownloadRecordId) -> bool {
        if let Some(record) = self.records.get_mut(record_id)
            && record.status != DownloadLifecycleStatus::Completed
            && record.status != DownloadLifecycleStatus::Cancelled
            && record.status != DownloadLifecycleStatus::Failed
        {
            record.status = DownloadLifecycleStatus::Cancelled;
            return true;
        }
        false
    }

    /// Reconciles non-terminal downloads on service restart.
    /// Any download whose external completion cannot be proven is transitioned to Reconciling
    /// rather than being automatically re-dispatched.
    pub fn reconcile_on_restart(&mut self, active_engine_downloads: &[DownloadId]) {
        for record in self.records.values_mut() {
            match record.status {
                DownloadLifecycleStatus::Pending | DownloadLifecycleStatus::Active => {
                    if let Some(engine_id) = &record.engine_download_id {
                        if active_engine_downloads.contains(engine_id) {
                            record.status = DownloadLifecycleStatus::Active;
                        } else {
                            record.status = DownloadLifecycleStatus::Reconciling;
                        }
                    } else {
                        record.status = DownloadLifecycleStatus::Reconciling;
                    }
                }
                DownloadLifecycleStatus::Paused => {
                    // Keep Paused status
                }
                DownloadLifecycleStatus::Completed
                | DownloadLifecycleStatus::Cancelled
                | DownloadLifecycleStatus::Failed
                | DownloadLifecycleStatus::Reconciling => {}
            }
        }
    }

    pub fn get_record(&self, record_id: &DownloadRecordId) -> Option<&DownloadRecord> {
        self.records.get(record_id)
    }

    pub fn list_records(
        &self,
        context_id: Option<&BrowserContextId>,
        status: Option<DownloadLifecycleStatus>,
    ) -> Vec<DownloadRecord> {
        self.records
            .values()
            .filter(|r| {
                if let Some(cid) = context_id
                    && r.context_id.as_ref() != Some(cid)
                {
                    return false;
                }
                if let Some(st) = status
                    && r.status != st
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }
}
