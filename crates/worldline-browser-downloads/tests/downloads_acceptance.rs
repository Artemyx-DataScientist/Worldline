use std::path::PathBuf;
use std::sync::Arc;

use worldline_browser_contract::identity::{BrowserContextId, DownloadId, PageId};
use worldline_browser_downloads::{ArtifactStore, DownloadsService};
use worldline_browser_services_contract::{
    CancelDownloadRequest, DownloadLifecycleStatus, GetDownloadRecordRequest,
    ListDownloadRecordsRequest, PauseDownloadRequest, ResumeDownloadRequest, StartDownloadRequest,
};

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
    service.on_engine_download_started(
        engine_dl_id.clone(),
        context_id.clone(),
        page_id.clone(),
        url.clone(),
        "bundle.zip".to_string(),
        Some(4096),
        Some("application/zip".to_string()),
    );

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
    let stored_bytes = artifact_store
        .read_bytes(&artifact_ref.artifact_id)
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
    service.on_engine_download_started(
        e1.clone(),
        BrowserContextId::new("ctx-1"),
        PageId::new("p1"),
        "https://worldline.test/file1.txt".to_string(),
        "file1.txt".to_string(),
        Some(10),
        None,
    );
    service.on_engine_download_completed(&e1, b"hello file", None);

    // Record 2: Active in engine
    let r2 = service.start_download(StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: None,
        url: "https://worldline.test/file2.txt".to_string(),
        suggested_filename: Some("file2.txt".to_string()),
    });
    let e2 = DownloadId::new("eng-2");
    service.on_engine_download_started(
        e2.clone(),
        BrowserContextId::new("ctx-1"),
        PageId::new("p1"),
        "https://worldline.test/file2.txt".to_string(),
        "file2.txt".to_string(),
        Some(100),
        None,
    );

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
    service.on_engine_download_started(
        engine_id.clone(),
        BrowserContextId::new("ctx-1"),
        PageId::new("p1"),
        "https://worldline.test/data.bin".to_string(),
        "data.bin".to_string(),
        Some(500),
        None,
    );

    // Redelivery
    service.on_engine_download_started(
        engine_id.clone(),
        BrowserContextId::new("ctx-1"),
        PageId::new("p1"),
        "https://worldline.test/data.bin".to_string(),
        "data.bin".to_string(),
        Some(500),
        None,
    );

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
    service.on_engine_download_started(
        engine_id.clone(),
        BrowserContextId::new("ctx-1"),
        PageId::new("p1"),
        "https://worldline.test/secret_doc.pdf".to_string(),
        "secret_doc.pdf".to_string(),
        Some(12),
        Some("application/pdf".to_string()),
    );
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
    // Possession of DownloadRecord or metadata alone does not yield bytes;
    // bytes must be fetched explicitly through ArtifactStore
    assert_eq!(art_ref.size_bytes, 13);
    assert_eq!(art_ref.mime_type, Some("application/pdf".to_string()));

    let bytes = artifact_store.read_bytes(&art_ref.artifact_id).unwrap();
    assert_eq!(bytes, b"%PDF-1.7-TEST");
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
    service.on_engine_download_started(
        engine_id.clone(),
        BrowserContextId::new("ctx-1"),
        PageId::new("p1"),
        "https://worldline.test/broken".to_string(),
        "broken".to_string(),
        None,
        None,
    );

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
