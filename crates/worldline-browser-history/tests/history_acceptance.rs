use worldline_browser_contract::identity::{DocumentRevision, NavigationId, PageId};
use worldline_browser_history::{ConsistencyError, HistoryService, HistoryStoreSnapshot};
use worldline_browser_services_contract::{
    ClearHistoryRequest, DeleteHistoryEntryRequest, OP_GET_HISTORY_ENTRY, OP_QUERY_HISTORY,
    QueryHistoryRequest,
};

#[test]
fn history_idempotent_recording_and_deduplication() {
    let service = HistoryService::new();
    let page_id = PageId::new("page-1");
    let nav_id = NavigationId::new("nav-101");
    let rev = DocumentRevision::new(1);
    let url = "https://worldline.test/home".to_string();

    // 1. First event delivery
    let entry1 = service
        .record_navigation(page_id.clone(), nav_id.clone(), rev, url.clone(), 1000)
        .unwrap();
    assert_eq!(entry1.url, url);
    assert_eq!(entry1.visit_count, 1);

    // 2. Duplicate redelivery (at-least-once delivery semantics)
    let entry2 = service
        .record_navigation(page_id.clone(), nav_id.clone(), rev, url.clone(), 1000)
        .unwrap();
    assert_eq!(entry1.entry_id, entry2.entry_id);

    // Total records in history must be exactly 1
    let query_res = service.query_history(QueryHistoryRequest::default());
    assert_eq!(query_res.total_count, 1);
}

#[test]
fn history_conflicting_commit_rejected() {
    let service = HistoryService::new();
    let page_id = PageId::new("page-1");
    let nav_id = NavigationId::new("nav-conflict");
    let rev = DocumentRevision::new(1);
    let url1 = "https://worldline.test/first".to_string();
    let url2 = "https://worldline.test/conflicting".to_string();

    // Initial commit
    service
        .record_navigation(page_id.clone(), nav_id.clone(), rev, url1.clone(), 1000)
        .unwrap();

    // Conflicting commit for same NavigationId
    let conflict_err = service
        .record_navigation(page_id.clone(), nav_id.clone(), rev, url2.clone(), 1000)
        .unwrap_err();

    assert_eq!(
        conflict_err,
        ConsistencyError {
            navigation_id: nav_id.clone(),
            existing_url: url1.clone(),
            conflicting_url: url2,
        }
    );

    // Original entry must be preserved intact
    let query_res = service.query_history(QueryHistoryRequest::default());
    assert_eq!(query_res.entries.len(), 1);
    assert_eq!(query_res.entries[0].url, url1);
}

#[test]
fn history_title_enrichment_via_page_ready() {
    let service = HistoryService::new();
    let page_id = PageId::new("page-1");
    let nav_id = NavigationId::new("nav-ready");
    let rev = DocumentRevision::new(1);
    let url = "https://worldline.test/article".to_string();

    // 1. Navigation committed (no title yet)
    let entry = service
        .record_navigation(page_id.clone(), nav_id, rev, url, 1000)
        .unwrap();
    assert_eq!(entry.title, None);

    // 2. browser.page.ready fact observed -> enrich title
    let enriched = service
        .enrich_title(&page_id, rev, "Awesome Article".to_string())
        .unwrap();
    assert_eq!(enriched.title, Some("Awesome Article".to_string()));

    // Verify query returns enriched title
    let query_res = service.query_history(QueryHistoryRequest {
        query: Some("Awesome".to_string()),
        ..Default::default()
    });
    assert_eq!(query_res.total_count, 1);
    assert_eq!(
        query_res.entries[0].title,
        Some("Awesome Article".to_string())
    );
}

#[test]
fn history_query_delete_clear_and_restart() {
    let service = HistoryService::new();
    let page_id = PageId::new("page-1");

    let e1 = service
        .record_navigation(
            page_id.clone(),
            NavigationId::new("nav-1"),
            DocumentRevision::new(1),
            "https://worldline.test/alpha".to_string(),
            1000,
        )
        .unwrap();

    let e2 = service
        .record_navigation(
            page_id.clone(),
            NavigationId::new("nav-2"),
            DocumentRevision::new(2),
            "https://worldline.test/beta".to_string(),
            2000,
        )
        .unwrap();

    let e3 = service
        .record_navigation(
            page_id.clone(),
            NavigationId::new("nav-3"),
            DocumentRevision::new(3),
            "https://worldline.test/gamma".to_string(),
            3000,
        )
        .unwrap();

    // Query all (sorted descending by timestamp)
    let all = service.query_history(QueryHistoryRequest::default());
    assert_eq!(all.total_count, 3);
    assert_eq!(all.entries[0].entry_id, e3.entry_id);
    assert_eq!(all.entries[1].entry_id, e2.entry_id);
    assert_eq!(all.entries[2].entry_id, e1.entry_id);

    // Query with filter
    let filtered = service.query_history(QueryHistoryRequest {
        query: Some("beta".to_string()),
        ..Default::default()
    });
    assert_eq!(filtered.total_count, 1);
    assert_eq!(filtered.entries[0].entry_id, e2.entry_id);

    // Export and restart (simulate persistence)
    let snapshot = service.export_snapshot();
    let snapshot_json = serde_json::to_string(&snapshot).unwrap();
    let restored: HistoryStoreSnapshot = serde_json::from_str(&snapshot_json).unwrap();
    let restarted_service = HistoryService::from_snapshot(restored);

    // Delete single entry (e2)
    let del_res = restarted_service.delete_history_entry(DeleteHistoryEntryRequest {
        entry_id: e2.entry_id.clone(),
    });
    assert!(del_res.deleted);

    let after_del = restarted_service.query_history(QueryHistoryRequest::default());
    assert_eq!(after_del.total_count, 2);

    // Clear history in time range [500, 1500] (removes e1)
    let clear_res = restarted_service.clear_history(ClearHistoryRequest {
        start_time_unix_ms: Some(500),
        end_time_unix_ms: Some(1500),
    });
    assert_eq!(clear_res.deleted_count, 1);

    let remaining = restarted_service.query_history(QueryHistoryRequest::default());
    assert_eq!(remaining.total_count, 1);
    assert_eq!(remaining.entries[0].entry_id, e3.entry_id);
}

#[test]
fn history_rpc_dispatcher() {
    let service = HistoryService::new();
    service
        .record_navigation(
            PageId::new("page-1"),
            NavigationId::new("nav-rpc"),
            DocumentRevision::new(1),
            "https://worldline.test/rpc".to_string(),
            5000,
        )
        .unwrap();

    let query_payload = serde_json::json!({
        "query": "rpc"
    });
    let query_res = service.dispatch(OP_QUERY_HISTORY, query_payload).unwrap();
    assert_eq!(query_res["total_count"], 1);
    let entry_id = query_res["entries"][0]["entry_id"]
        .as_str()
        .unwrap()
        .to_string();

    let get_payload = serde_json::json!({
        "entry_id": entry_id
    });
    let get_res = service.dispatch(OP_GET_HISTORY_ENTRY, get_payload).unwrap();
    assert_eq!(get_res["entry"]["url"], "https://worldline.test/rpc");
}
