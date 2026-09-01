use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use worldline_browser_contract::identity::DownloadId;
use worldline_browser_services_contract::{
    CancelDownloadRequest, CancelDownloadResponse, DownloadLifecycleStatus,
    GetDownloadRecordRequest, GetDownloadRecordResponse, ListDownloadRecordsRequest,
    ListDownloadRecordsResponse, PauseDownloadRequest, PauseDownloadResponse,
    ResumeDownloadRequest, ResumeDownloadResponse, StartDownloadRequest, StartDownloadResponse,
};

use crate::artifact::ArtifactStore;
use crate::state::DownloadsSnapshot;

/// Downloads service providing durable product-level download lifecycle management,
/// opaque artifact handoff, and crash reconciliation.
pub struct DownloadsService {
    state: Mutex<DownloadsSnapshot>,
    artifact_store: Arc<ArtifactStore>,
    staging_root: PathBuf,
}

impl Default for DownloadsService {
    fn default() -> Self {
        Self::new(Arc::new(ArtifactStore::new()), PathBuf::from("./staging"))
    }
}

impl DownloadsService {
    pub fn new(artifact_store: Arc<ArtifactStore>, staging_root: PathBuf) -> Self {
        Self {
            state: Mutex::new(DownloadsSnapshot::new()),
            artifact_store,
            staging_root,
        }
    }

    pub fn from_snapshot(
        snapshot: DownloadsSnapshot,
        artifact_store: Arc<ArtifactStore>,
        staging_root: PathBuf,
    ) -> Self {
        Self {
            state: Mutex::new(snapshot),
            artifact_store,
            staging_root,
        }
    }

    pub fn export_snapshot(&self) -> DownloadsSnapshot {
        self.state.lock().unwrap().clone()
    }

    pub fn artifact_store(&self) -> Arc<ArtifactStore> {
        self.artifact_store.clone()
    }

    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    // --- Capability RPC Handlers ---

    /// Starts a download by durably establishing intent before issuing engine operations.
    pub fn start_download(&self, req: StartDownloadRequest) -> StartDownloadResponse {
        let mut state = self.state.lock().unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let record = state.create_intent(
            req.context_id,
            req.page_id,
            req.url,
            req.suggested_filename,
            now_ms,
        );

        StartDownloadResponse {
            record_id: record.record_id,
            status: record.status,
        }
    }

    pub fn get_download_record(&self, req: GetDownloadRecordRequest) -> GetDownloadRecordResponse {
        let state = self.state.lock().unwrap();
        let record = state.get_record(&req.record_id).cloned();
        GetDownloadRecordResponse { record }
    }

    pub fn list_download_records(
        &self,
        req: ListDownloadRecordsRequest,
    ) -> ListDownloadRecordsResponse {
        let state = self.state.lock().unwrap();
        let records = state.list_records(req.context_id.as_ref(), req.status);
        ListDownloadRecordsResponse { records }
    }

    pub fn pause_download(&self, req: PauseDownloadRequest) -> PauseDownloadResponse {
        let mut state = self.state.lock().unwrap();
        let success = state.pause_download(&req.record_id);
        let status = state
            .get_record(&req.record_id)
            .map(|r| r.status)
            .unwrap_or(DownloadLifecycleStatus::Failed);

        PauseDownloadResponse { success, status }
    }

    pub fn resume_download(&self, req: ResumeDownloadRequest) -> ResumeDownloadResponse {
        let mut state = self.state.lock().unwrap();
        let success = state.resume_download(&req.record_id);
        let status = state
            .get_record(&req.record_id)
            .map(|r| r.status)
            .unwrap_or(DownloadLifecycleStatus::Failed);

        ResumeDownloadResponse { success, status }
    }

    pub fn cancel_download(&self, req: CancelDownloadRequest) -> CancelDownloadResponse {
        let mut state = self.state.lock().unwrap();
        let success = state.cancel_download(&req.record_id);
        let status = state
            .get_record(&req.record_id)
            .map(|r| r.status)
            .unwrap_or(DownloadLifecycleStatus::Failed);

        CancelDownloadResponse { success, status }
    }

    // --- Engine Notification / Event Handlers ---

    pub fn on_engine_download_started(
        &self,
        engine_download_id: DownloadId,
        context_id: worldline_browser_contract::identity::BrowserContextId,
        page_id: worldline_browser_contract::identity::PageId,
        url: String,
        suggested_filename: String,
        total_bytes: Option<u64>,
        media_type: Option<String>,
    ) {
        let mut state = self.state.lock().unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        state.handle_engine_download_started(
            engine_download_id,
            context_id,
            page_id,
            url,
            suggested_filename,
            total_bytes,
            media_type,
            now_ms,
        );
    }

    pub fn on_engine_download_progress(
        &self,
        engine_download_id: &DownloadId,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        let mut state = self.state.lock().unwrap();
        state.update_progress(engine_download_id, received_bytes, total_bytes);
    }

    pub fn on_engine_download_completed(
        &self,
        engine_download_id: &DownloadId,
        content_bytes: &[u8],
        mime_type: Option<String>,
    ) {
        let artifact_ref = self.artifact_store.store_bytes(content_bytes, mime_type);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut state = self.state.lock().unwrap();
        state.complete_download(engine_download_id, artifact_ref, now_ms);
    }

    pub fn on_engine_download_failed(
        &self,
        engine_download_id: &DownloadId,
        error_message: String,
    ) {
        let mut state = self.state.lock().unwrap();
        state.fail_download(engine_download_id, error_message);
    }

    /// Reconciles non-terminal records against active engine operations on restart.
    pub fn reconcile_on_restart(&self, active_engine_downloads: &[DownloadId]) {
        let mut state = self.state.lock().unwrap();
        state.reconcile_on_restart(active_engine_downloads);
    }

    /// Cleans up temporary staging files strictly inside the authorized staging root.
    pub fn clean_staging_file(&self, file_path: &Path) -> std::io::Result<()> {
        let canonical_root = self
            .staging_root
            .canonicalize()
            .unwrap_or_else(|_| self.staging_root.clone());
        if let Ok(canonical_target) = file_path.canonicalize() {
            if !canonical_target.starts_with(&canonical_root) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Target file is outside authorized staging root",
                ));
            }
            if canonical_target.is_file() {
                std::fs::remove_file(canonical_target)?;
            }
        }
        Ok(())
    }
}
