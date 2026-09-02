use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use worldline_browser_contract::identity::{BrowserContextId, DownloadId, PageId};
use worldline_browser_downloads::{
    AUTH_BLOB_READ, ArtifactStore, BlobReadBroker, DownloadsService, EngineDownloadStarted,
};
use worldline_browser_services_contract::{
    CancelDownloadRequest, DownloadLifecycleStatus, GetDownloadRecordRequest,
    ListDownloadRecordsRequest, PauseDownloadRequest, ResumeDownloadRequest, StartDownloadRequest,
};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "worldline-browser-downloads-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary downloads root must be created");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn download_lifecycle_progression() {
    let artifact_store = Arc::new(ArtifactStore::new());
    let staging_root = PathBuf::from("./target/staging_test");
    let service = DownloadsService::new(artifact_store.clone(), staging_root);

    let context_id = BrowserContextId::new("ctx-prod");
    let page_id = PageId::new("page-1");
    let url = "https://worldline.test/bundle.zip".to_string();

    // 1. Start intent -> Pending
    let start_res = service.start_download(StartDownloadRequest {
        context_id: context_id.clone(),
        page_id: Some(page_id.clone()),
        url: url.clone(),
        suggested_filename: Some("bundle.zip".to_string()),
    });
    assert_eq!(start_res.status, DownloadLifecycleStatus::Pending);
    let record_id = start_res.record_id;

    // 2. Engine download started event -> Active
    let engine_dl_id = DownloadId::new("engine-dl-1");
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: engine_dl_id.clone(),
        context_id: context_id.clone(),
        page_id: page_id.clone(),
        url: url.clone(),
        suggested_filename: "bundle.zip".to_string(),
        total_bytes: Some(4096),
        media_type: Some("application/zip".to_string()),
    });

    let rec = service
        .get_download_record(GetDownloadRecordRequest {
            record_id: record_id.clone(),
        })
        .record
        .expect("Record must exist");
    assert_eq!(rec.status, DownloadLifecycleStatus::Active);
    assert_eq!(rec.engine_download_id, Some(engine_dl_id.clone()));

    // 3. Engine progress
    service.on_engine_download_progress(&engine_dl_id, 2048, Some(4096));
    let rec = service
        .get_download_record(GetDownloadRecordRequest {
            record_id: record_id.clone(),
        })
        .record
        .unwrap();
    assert_eq!(rec.received_bytes, 2048);
    assert_eq!(rec.total_bytes, Some(4096));

    // 4. Engine completion with payload bytes
    let payload = vec![0x50, 0x4b, 0x03, 0x04, 0x00, 0x00];
    service.on_engine_download_completed(
        &engine_dl_id,
        &payload,
        Some("application/zip".to_string()),
    );

    let rec = service
        .get_download_record(GetDownloadRecordRequest {
            record_id: record_id.clone(),
        })
        .record
        .unwrap();
    assert_eq!(rec.status, DownloadLifecycleStatus::Completed);
    let artifact_ref = rec.artifact_ref.expect("ArtifactRef must be materialized");
    assert_eq!(artifact_ref.size_bytes, payload.len() as u64);
    assert_eq!(artifact_ref.mime_type, Some("application/zip".to_string()));

    // Verify artifact bytes in store
    let blob_broker = BlobReadBroker::new();
    let blob_grant = blob_broker
        .issue(
            "download-reader",
            "blob.read",
            artifact_ref.artifact_id.as_str(),
        )
        .unwrap();
    let stored_bytes = artifact_store
        .read_bytes_with_authority(&artifact_ref.artifact_id, &blob_grant)
        .unwrap();
    assert_eq!(stored_bytes, payload);
}

#[test]
fn download_pause_resume_cancel() {
    let service = DownloadsService::default();
    let start_res = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: None,
        url: "https://worldline.test/video.mp4".to_string(),
        suggested_filename: None,
    });
    let record_id = start_res.record_id;

    // Pause
    let pause_res = service.pause_download(PauseDownloadRequest {
        record_id: record_id.clone(),
    });
    assert!(pause_res.success);
    assert_eq!(pause_res.status, DownloadLifecycleStatus::Paused);

    // Resume
    let resume_res = service.resume_download(ResumeDownloadRequest {
        record_id: record_id.clone(),
    });
    assert!(resume_res.success);
    assert_eq!(resume_res.status, DownloadLifecycleStatus::Active);

    // Cancel
    let cancel_res = service.cancel_download(CancelDownloadRequest {
        record_id: record_id.clone(),
    });
    assert!(cancel_res.success);
    assert_eq!(cancel_res.status, DownloadLifecycleStatus::Cancelled);

    // Cannot pause a cancelled download
    let pause_again = service.pause_download(PauseDownloadRequest {
        record_id: record_id.clone(),
    });
    assert!(!pause_again.success);
}

#[test]
fn durable_intent_and_restart_reconciliation() {
    let artifact_store = Arc::new(ArtifactStore::new());
    let staging_root = PathBuf::from("./target/staging_test");
    let service = DownloadsService::new(artifact_store.clone(), staging_root.clone());

    // Record 1: Started and completed
    let r1 = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: None,
        url: "https://worldline.test/file1.txt".to_string(),
        suggested_filename: Some("file1.txt".to_string()),
    });
    let e1 = DownloadId::new("eng-1");
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: e1.clone(),
        context_id: BrowserContextId::new("ctx-1"),
        page_id: PageId::new("p1"),
        url: "https://worldline.test/file1.txt".to_string(),
        suggested_filename: "file1.txt".to_string(),
        total_bytes: Some(10),
        media_type: None,
    });
    service.on_engine_download_completed(&e1, b"hello file", None);

    // Record 2: Active in engine
    let r2 = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: None,
        url: "https://worldline.test/file2.txt".to_string(),
        suggested_filename: Some("file2.txt".to_string()),
    });
    let e2 = DownloadId::new("eng-2");
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: e2.clone(),
        context_id: BrowserContextId::new("ctx-1"),
        page_id: PageId::new("p1"),
        url: "https://worldline.test/file2.txt".to_string(),
        suggested_filename: "file2.txt".to_string(),
        total_bytes: Some(100),
        media_type: None,
    });

    // Record 3: Intent created but engine outcome lost / unknown
    let r3 = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: None,
        url: "https://worldline.test/file3.txt".to_string(),
        suggested_filename: Some("file3.txt".to_string()),
    });

    // Simulate service restart from snapshot
    let snapshot = service.export_snapshot();
    let restarted_service =
        DownloadsService::from_snapshot(snapshot, artifact_store.clone(), staging_root);

    // Reconcile against active engine operations: only eng-2 is currently active in engine
    let active_engine_ops = vec![e2.clone()];
    restarted_service.reconcile_on_restart(&active_engine_ops);

    // Check states
    let rec1 = restarted_service
        .get_download_record(GetDownloadRecordRequest {
            record_id: r1.record_id,
        })
        .record
        .unwrap();
    assert_eq!(rec1.status, DownloadLifecycleStatus::Completed);

    let rec2 = restarted_service
        .get_download_record(GetDownloadRecordRequest {
            record_id: r2.record_id,
        })
        .record
        .unwrap();
    assert_eq!(rec2.status, DownloadLifecycleStatus::Active);

    let rec3 = restarted_service
        .get_download_record(GetDownloadRecordRequest {
            record_id: r3.record_id,
        })
        .record
        .unwrap();
    // Outcome was uncertain -> Reconciling (NO auto retry!)
    assert_eq!(rec3.status, DownloadLifecycleStatus::Reconciling);
}

#[test]
fn redelivered_engine_events_are_idempotent() {
    let service = DownloadsService::default();
    let engine_id = DownloadId::new("eng-repeat");

    // First delivery
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: engine_id.clone(),
        context_id: BrowserContextId::new("ctx-1"),
        page_id: PageId::new("p1"),
        url: "https://worldline.test/data.bin".to_string(),
        suggested_filename: "data.bin".to_string(),
        total_bytes: Some(500),
        media_type: None,
    });

    // Redelivery
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: engine_id.clone(),
        context_id: BrowserContextId::new("ctx-1"),
        page_id: PageId::new("p1"),
        url: "https://worldline.test/data.bin".to_string(),
        suggested_filename: "data.bin".to_string(),
        total_bytes: Some(500),
        media_type: None,
    });

    let list_res = service.list_download_records(ListDownloadRecordsRequest::default());
    assert_eq!(
        list_res.records.len(),
        1,
        "Repeated engine events must not create duplicate DownloadRecordId entries"
    );
}

#[test]
fn artifact_authority_isolation() {
    let artifact_store = Arc::new(ArtifactStore::new());
    let service = DownloadsService::new(artifact_store.clone(), PathBuf::from("./staging"));

    let start_res = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: None,
        url: "https://worldline.test/secret_doc.pdf".to_string(),
        suggested_filename: None,
    });
    let engine_id = DownloadId::new("eng-sec");
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: engine_id.clone(),
        context_id: BrowserContextId::new("ctx-1"),
        page_id: PageId::new("p1"),
        url: "https://worldline.test/secret_doc.pdf".to_string(),
        suggested_filename: "secret_doc.pdf".to_string(),
        total_bytes: Some(12),
        media_type: Some("application/pdf".to_string()),
    });
    service.on_engine_download_completed(
        &engine_id,
        b"%PDF-1.7-TEST",
        Some("application/pdf".to_string()),
    );

    let rec = service
        .get_download_record(GetDownloadRecordRequest {
            record_id: start_res.record_id,
        })
        .record
        .unwrap();

    let art_ref = rec.artifact_ref.unwrap();
    // Possession of DownloadRecord or metadata alone does not yield bytes.
    assert_eq!(art_ref.size_bytes, 13);
    assert_eq!(art_ref.mime_type, Some("application/pdf".to_string()));

    let blob_broker = BlobReadBroker::new();
    assert!(
        blob_broker
            .issue(
                "metadata-reader",
                "browser.downloads.read",
                art_ref.artifact_id.as_str()
            )
            .is_err()
    );
    let blob_grant = blob_broker
        .issue("blob-reader", "blob.read", art_ref.artifact_id.as_str())
        .unwrap();
    let bytes = artifact_store
        .read_bytes_with_authority(&art_ref.artifact_id, &blob_grant)
        .unwrap();
    assert_eq!(bytes, b"%PDF-1.7-TEST");
}

#[test]
fn persistent_service_metadata_and_blob_survive_restart() {
    let root = TempRoot::new("persistent-restart");
    let artifact_store = Arc::new(
        ArtifactStore::open(root.path().join("blobs")).expect("generic blob store must open"),
    );
    let state_path = root.path().join("service-state").join("records.json");
    let staging_root = root.path().join("staging");
    let service = DownloadsService::open_persistent(
        Arc::clone(&artifact_store),
        staging_root.clone(),
        state_path.clone(),
    )
    .expect("persistent downloads service must open");
    let start = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-persistent"),
        page_id: Some(PageId::new("page-persistent")),
        url: "https://worldline.test/persistent.bin".to_string(),
        suggested_filename: Some("persistent.bin".to_string()),
    });
    let engine_id = DownloadId::new("engine-persistent");
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: engine_id.clone(),
        context_id: BrowserContextId::new("ctx-persistent"),
        page_id: PageId::new("page-persistent"),
        url: "https://worldline.test/persistent.bin".to_string(),
        suggested_filename: "persistent.bin".to_string(),
        total_bytes: Some(9),
        media_type: Some("application/octet-stream".to_string()),
    });
    service.on_engine_download_completed(
        &engine_id,
        b"durable\n",
        Some("application/octet-stream".to_string()),
    );
    service
        .check_persistence()
        .expect("service metadata must be durably flushed");
    drop(service);
    drop(artifact_store);

    let reopened_store =
        Arc::new(ArtifactStore::open(root.path().join("blobs")).expect("blob store must reopen"));
    let restarted =
        DownloadsService::open_persistent(Arc::clone(&reopened_store), staging_root, state_path)
            .expect("downloads service must reopen from durable metadata");
    let record = restarted
        .get_download_record(GetDownloadRecordRequest {
            record_id: start.record_id,
        })
        .record
        .expect("download record must survive restart");
    let artifact = record
        .artifact_ref
        .clone()
        .expect("artifact reference must survive restart");
    assert_eq!(record.status, DownloadLifecycleStatus::Completed);
    assert_eq!(
        artifact.sha256_hash.as_deref(),
        artifact.artifact_id.strip_prefix("sha256-v1-")
    );
    assert!(artifact.sha256_hash.as_deref().is_some_and(
        |digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    ));
    let broker = BlobReadBroker::new();
    let grant = broker
        .issue(
            "persistent-reader",
            AUTH_BLOB_READ,
            artifact.artifact_id.clone(),
        )
        .expect("generic blob broker must issue an exact read grant");
    assert_eq!(
        reopened_store
            .read_bytes_with_authority(&artifact.artifact_id, &grant)
            .expect("reopened blob must be readable with generic authority"),
        b"durable\n"
    );
}

#[test]
fn download_failure_isolation() {
    let service = DownloadsService::default();
    let start_res = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: None,
        url: "https://worldline.test/broken".to_string(),
        suggested_filename: None,
    });
    let engine_id = DownloadId::new("eng-fail");
    service.on_engine_download_started(EngineDownloadStarted {
        engine_download_id: engine_id.clone(),
        context_id: BrowserContextId::new("ctx-1"),
        page_id: PageId::new("p1"),
        url: "https://worldline.test/broken".to_string(),
        suggested_filename: "broken".to_string(),
        total_bytes: None,
        media_type: None,
    });

    service.on_engine_download_failed(&engine_id, "HTTP 404 Not Found".to_string());

    let rec = service
        .get_download_record(GetDownloadRecordRequest {
            record_id: start_res.record_id,
        })
        .record
        .unwrap();

    assert_eq!(rec.status, DownloadLifecycleStatus::Failed);
    assert_eq!(rec.error_message, Some("HTTP 404 Not Found".to_string()));
}
