use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use worldline_browser_contract::identity::DownloadId;
use worldline_browser_services_contract::{
    CancelDownloadRequest, CancelDownloadResponse, DownloadLifecycleStatus,
    GetDownloadRecordRequest, GetDownloadRecordResponse, ListDownloadRecordsRequest,
    ListDownloadRecordsResponse, PauseDownloadRequest, PauseDownloadResponse,
    ResumeDownloadRequest, ResumeDownloadResponse, StartDownloadRequest, StartDownloadResponse,
};

use crate::artifact::ArtifactStore;
use crate::state::{DownloadsSnapshot, EngineDownloadStarted};

/// Downloads service providing durable product-level download lifecycle management,
/// opaque artifact handoff, and crash reconciliation.
pub struct DownloadsService {
    state: Mutex<DownloadsSnapshot>,
    artifact_store: Arc<ArtifactStore>,
    staging_root: PathBuf,
    state_path: Option<PathBuf>,
    persistence_error: Mutex<Option<String>>,
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
            state_path: None,
            persistence_error: Mutex::new(None),
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
            state_path: None,
            persistence_error: Mutex::new(None),
        }
    }

    /// Opens the downloads service with a durable JSON snapshot for its
    /// product metadata. The blob bytes remain in the generic host blob store;
    /// this file contains only the service-owned records and opaque refs.
    pub fn open_persistent(
        artifact_store: Arc<ArtifactStore>,
        staging_root: PathBuf,
        state_path: PathBuf,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(&staging_root)
            .map_err(|error| format!("create downloads staging root: {error}"))?;
        if let Some(parent) = state_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create downloads state root: {error}"))?;
        }
        let snapshot = if state_path.is_file() {
            let bytes = std::fs::read(&state_path).map_err(|error| {
                format!(
                    "read downloads snapshot '{}': {error}",
                    state_path.display()
                )
            })?;
            serde_json::from_slice(&bytes).map_err(|error| {
                format!(
                    "decode downloads snapshot '{}': {error}",
                    state_path.display()
                )
            })?
        } else {
            DownloadsSnapshot::new()
        };
        let service = Self {
            state: Mutex::new(snapshot),
            artifact_store,
            staging_root,
            state_path: Some(state_path),
            persistence_error: Mutex::new(None),
        };
        service.flush_persistence()?;
        Ok(service)
    }

    /// Opens a service snapshot next to its staging root. Hosted callers may
    /// use [`Self::open_persistent`] when the host selects a separate state
    /// directory.
    pub fn open(artifact_store: Arc<ArtifactStore>, staging_root: PathBuf) -> Result<Self, String> {
        let state_path = staging_root.join("downloads.snapshot.json");
        Self::open_persistent(artifact_store, staging_root, state_path)
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

    /// Returns an explicit persistence failure instead of silently presenting
    /// an in-memory mutation as durable.
    pub fn check_persistence(&self) -> Result<(), String> {
        self.persistence_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .map_or(Ok(()), Err)
    }

    fn persist_snapshot(&self, snapshot: &DownloadsSnapshot) -> Result<(), String> {
        let Some(state_path) = &self.state_path else {
            return Ok(());
        };
        let parent = state_path
            .parent()
            .ok_or_else(|| format!("downloads snapshot has no parent: {}", state_path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create downloads snapshot parent: {error}"))?;
        let bytes = serde_json::to_vec_pretty(snapshot)
            .map_err(|error| format!("encode downloads snapshot: {error}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock before Unix epoch: {error}"))?
            .as_nanos();
        let temporary_path =
            state_path.with_extension(format!("json.{}.{}.tmp", std::process::id(), nonce));
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary_path)
                .map_err(|error| {
                    format!(
                        "create temporary downloads snapshot '{}': {error}",
                        temporary_path.display()
                    )
                })?;
            file.write_all(&bytes).map_err(|error| {
                format!(
                    "write temporary downloads snapshot '{}': {error}",
                    temporary_path.display()
                )
            })?;
            file.sync_all().map_err(|error| {
                format!(
                    "sync temporary downloads snapshot '{}': {error}",
                    temporary_path.display()
                )
            })?;
            drop(file);
            if state_path.exists() {
                std::fs::remove_file(state_path).map_err(|error| {
                    format!(
                        "replace downloads snapshot '{}': {error}",
                        state_path.display()
                    )
                })?;
            }
            std::fs::rename(&temporary_path, state_path).map_err(|error| {
                format!(
                    "publish downloads snapshot '{}': {error}",
                    state_path.display()
                )
            })?;
            Ok::<(), String>(())
        })();
        if write_result.is_err() {
            let _ = std::fs::remove_file(&temporary_path);
        }
        write_result
    }

    fn persist_locked(&self, snapshot: &DownloadsSnapshot) {
        if let Err(error) = self.persist_snapshot(snapshot) {
            *self
                .persistence_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
        }
    }

    pub fn flush_persistence(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        self.persist_snapshot(&state)?;
        *self
            .persistence_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        Ok(())
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
        self.persist_locked(&state);

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
        self.persist_locked(&state);

        PauseDownloadResponse { success, status }
    }

    pub fn resume_download(&self, req: ResumeDownloadRequest) -> ResumeDownloadResponse {
        let mut state = self.state.lock().unwrap();
        let success = state.resume_download(&req.record_id);
        let status = state
            .get_record(&req.record_id)
            .map(|r| r.status)
            .unwrap_or(DownloadLifecycleStatus::Failed);
        self.persist_locked(&state);

        ResumeDownloadResponse { success, status }
    }

    pub fn cancel_download(&self, req: CancelDownloadRequest) -> CancelDownloadResponse {
        let mut state = self.state.lock().unwrap();
        let success = state.cancel_download(&req.record_id);
        let status = state
            .get_record(&req.record_id)
            .map(|r| r.status)
            .unwrap_or(DownloadLifecycleStatus::Failed);
        self.persist_locked(&state);

        CancelDownloadResponse { success, status }
    }

    // --- Engine Notification / Event Handlers ---

    pub fn on_engine_download_started(&self, event: EngineDownloadStarted) {
        let mut state = self.state.lock().unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        state.handle_engine_download_started(event, now_ms);
        self.persist_locked(&state);
    }

    pub fn on_engine_download_progress(
        &self,
        engine_download_id: &DownloadId,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        let mut state = self.state.lock().unwrap();
        state.update_progress(engine_download_id, received_bytes, total_bytes);
        self.persist_locked(&state);
    }

    pub fn on_engine_download_completed(
        &self,
        engine_download_id: &DownloadId,
        content_bytes: &[u8],
        mime_type: Option<String>,
    ) {
        let artifact_ref = match self.artifact_store.store_bytes(content_bytes, mime_type) {
            Ok(artifact_ref) => artifact_ref,
            Err(error) => {
                let mut state = self.state.lock().unwrap();
                state.fail_download(engine_download_id, error);
                self.persist_locked(&state);
                return;
            }
        };
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut state = self.state.lock().unwrap();
        state.complete_download(engine_download_id, artifact_ref, now_ms);
        self.persist_locked(&state);
    }

    pub fn on_engine_download_failed(
        &self,
        engine_download_id: &DownloadId,
        error_message: String,
    ) {
        let mut state = self.state.lock().unwrap();
        state.fail_download(engine_download_id, error_message);
        self.persist_locked(&state);
    }

    /// Reconciles non-terminal records against active engine operations on restart.
    pub fn reconcile_on_restart(&self, active_engine_downloads: &[DownloadId]) {
        let mut state = self.state.lock().unwrap();
        state.reconcile_on_restart(active_engine_downloads);
        self.persist_locked(&state);
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
