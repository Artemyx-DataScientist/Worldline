use worldline_browser_contract::{
    authority::{OP_CLICK, OP_NAVIGATE, OP_OBSERVE, OP_QUERY_DOCUMENT},
    identity::{BrowserContextId, DocumentRevision, DownloadId, NavigationId, PageId},
};
use worldline_browser_services_contract::{
    AUTH_COOKIES_ADMIN, AUTH_COOKIES_METADATA_READ, AUTH_COOKIES_MUTATE, AUTH_COOKIES_VALUE_READ,
    AUTH_DOWNLOADS_CONTROL, AUTH_DOWNLOADS_READ, AUTH_HISTORY_DELETE, AUTH_HISTORY_READ,
    AUTH_SITE_DATA_CLEAR, AUTH_TABS_MUTATE, AUTH_TABS_READ, ArtifactRef, CONTRACT_BROWSER_COOKIES,
    CONTRACT_BROWSER_COOKIES_VERSION, CONTRACT_BROWSER_DOWNLOADS,
    CONTRACT_BROWSER_DOWNLOADS_VERSION, CONTRACT_BROWSER_HISTORY, CONTRACT_BROWSER_HISTORY_VERSION,
    CONTRACT_BROWSER_SITE_DATA, CONTRACT_BROWSER_SITE_DATA_VERSION, CONTRACT_BROWSER_TABS,
    CONTRACT_BROWSER_TABS_VERSION, CancelDownloadRequest, CancelDownloadResponse,
    ClearHistoryRequest, ClearSiteDataRequest, ClearSiteDataResponse, CloseTabRequest,
    CloseTabResponse, CookieMetadata, CookieValue, CreateTabRequest, DeleteCookieServiceRequest,
    DeleteCookieServiceResponse, DownloadLifecycleStatus, DownloadRecord, DownloadRecordId,
    GetCookieMetadataRequest, GetCookieMetadataResponse, GetCookieValueRequest,
    GetCookieValueResponse, GetDownloadRecordRequest, GetDownloadRecordResponse, HistoryEntry,
    HistoryEntryId, ListDownloadRecordsRequest, ListDownloadRecordsResponse, PauseDownloadRequest,
    PauseDownloadResponse, QueryHistoryRequest, ResumeDownloadRequest, ResumeDownloadResponse,
    SetCookieServiceRequest, SetCookieServiceResponse, StartDownloadRequest, StartDownloadResponse,
    StorageType, TabGroupId, TabId, TabState,
};

#[test]
fn tabs_contract_wire_fixtures_stability() {
    assert_eq!(CONTRACT_BROWSER_TABS, "browser.tabs");
    assert_eq!(CONTRACT_BROWSER_TABS_VERSION, "0.1");

    let create_tab_json = r#"{
        "page_id": "page-123",
        "group_id": "grp-1",
        "pinned": true,
        "select": true
    }"#;
    let create_req: CreateTabRequest = serde_json::from_str(create_tab_json).unwrap();
    assert_eq!(create_req.page_id.as_str(), "page-123");
    assert_eq!(
        create_req.group_id.as_ref().map(|g| g.as_str()),
        Some("grp-1")
    );
    assert_eq!(create_req.pinned, Some(true));
    assert_eq!(create_req.select, Some(true));

    let tab_state = TabState {
        id: TabId::new("tab-1"),
        page_id: PageId::new("page-123"),
        group_id: Some(TabGroupId::new("grp-1")),
        pinned: true,
        selected: true,
        order_index: 0,
        title: Some("Example Tab".to_string()),
        url: Some("https://worldline.test/page".to_string()),
    };
    let tab_json = serde_json::to_string(&tab_state).unwrap();
    let tab_res: TabState = serde_json::from_str(&tab_json).unwrap();
    assert_eq!(tab_state, tab_res);

    let close_req = CloseTabRequest {
        tab_id: TabId::new("tab-1"),
    };
    let close_json = serde_json::to_string(&close_req).unwrap();
    let close_parsed: CloseTabRequest = serde_json::from_str(&close_json).unwrap();
    assert_eq!(close_req, close_parsed);

    let close_res = CloseTabResponse {
        closed_tab_id: TabId::new("tab-1"),
        detached_page_id: PageId::new("page-123"),
        new_selected_tab_id: None,
    };
    let close_res_json = serde_json::to_string(&close_res).unwrap();
    let close_res_parsed: CloseTabResponse = serde_json::from_str(&close_res_json).unwrap();
    assert_eq!(close_res, close_res_parsed);
}

#[test]
fn history_contract_wire_fixtures_stability() {
    assert_eq!(CONTRACT_BROWSER_HISTORY, "browser.history");
    assert_eq!(CONTRACT_BROWSER_HISTORY_VERSION, "0.1");

    let entry = HistoryEntry {
        entry_id: HistoryEntryId::new("hist-1"),
        page_id: PageId::new("page-123"),
        navigation_id: NavigationId::new("nav-55"),
        document_revision: DocumentRevision::new(2),
        url: "https://worldline.test/docs".to_string(),
        title: Some("Worldline Docs".to_string()),
        committed_at_unix_ms: 1725180000000,
        visit_count: 1,
    };

    let entry_json = serde_json::to_string(&entry).unwrap();
    let parsed_entry: HistoryEntry = serde_json::from_str(&entry_json).unwrap();
    assert_eq!(entry, parsed_entry);

    let query_req = QueryHistoryRequest {
        query: Some("Docs".to_string()),
        max_results: Some(10),
        start_time_unix_ms: Some(1725000000000),
        end_time_unix_ms: None,
    };
    let query_json = serde_json::to_string(&query_req).unwrap();
    let parsed_query: QueryHistoryRequest = serde_json::from_str(&query_json).unwrap();
    assert_eq!(query_req, parsed_query);

    let clear_req = ClearHistoryRequest {
        start_time_unix_ms: None,
        end_time_unix_ms: None,
    };
    let clear_json = serde_json::to_string(&clear_req).unwrap();
    let parsed_clear: ClearHistoryRequest = serde_json::from_str(&clear_json).unwrap();
    assert_eq!(clear_req, parsed_clear);
}

#[test]
fn downloads_contract_wire_fixtures_stability() {
    assert_eq!(CONTRACT_BROWSER_DOWNLOADS, "browser.downloads");
    assert_eq!(CONTRACT_BROWSER_DOWNLOADS_VERSION, "0.1");

    let start_req = StartDownloadRequest {
        context_id: BrowserContextId::new("ctx-1"),
        page_id: Some(PageId::new("page-1")),
        url: "https://worldline.test/archive.tar.gz".to_string(),
        suggested_filename: Some("archive.tar.gz".to_string()),
    };
    let start_json = serde_json::to_string(&start_req).unwrap();
    let parsed_start_req: StartDownloadRequest = serde_json::from_str(&start_json).unwrap();
    assert_eq!(start_req, parsed_start_req);

    let start_res = StartDownloadResponse {
        record_id: DownloadRecordId::new("dl-rec-1"),
        status: DownloadLifecycleStatus::Pending,
    };
    let start_res_json = serde_json::to_string(&start_res).unwrap();
    let parsed_start_res: StartDownloadResponse = serde_json::from_str(&start_res_json).unwrap();
    assert_eq!(start_res, parsed_start_res);

    let artifact = ArtifactRef::new(
        "blob-abc-123",
        1024,
        Some("application/gzip".to_string()),
        Some("hash-sha256-hex".to_string()),
    );
    let record = DownloadRecord {
        record_id: DownloadRecordId::new("dl-rec-1"),
        context_id: Some(BrowserContextId::new("ctx-1")),
        page_id: Some(PageId::new("page-1")),
        url: "https://worldline.test/archive.tar.gz".to_string(),
        suggested_filename: "archive.tar.gz".to_string(),
        media_type: Some("application/gzip".to_string()),
        total_bytes: Some(1024),
        received_bytes: 1024,
        status: DownloadLifecycleStatus::Completed,
        engine_download_id: Some(DownloadId::new("dl-42")),
        artifact_ref: Some(artifact),
        error_message: None,
        started_at_unix_ms: 1725180000000,
        completed_at_unix_ms: Some(1725180001000),
    };
    let record_json = serde_json::to_string(&record).unwrap();
    let parsed_record: DownloadRecord = serde_json::from_str(&record_json).unwrap();
    assert_eq!(record, parsed_record);

    let get_req = GetDownloadRecordRequest {
        record_id: DownloadRecordId::new("dl-rec-1"),
    };
    let get_json = serde_json::to_string(&get_req).unwrap();
    let parsed_get: GetDownloadRecordRequest = serde_json::from_str(&get_json).unwrap();
    assert_eq!(get_req, parsed_get);

    let get_res = GetDownloadRecordResponse {
        record: Some(record.clone()),
    };
    let get_res_json = serde_json::to_string(&get_res).unwrap();
    let parsed_get_res: GetDownloadRecordResponse = serde_json::from_str(&get_res_json).unwrap();
    assert_eq!(get_res, parsed_get_res);

    let list_req = ListDownloadRecordsRequest {
        context_id: Some(BrowserContextId::new("ctx-1")),
        status: Some(DownloadLifecycleStatus::Completed),
    };
    let list_json = serde_json::to_string(&list_req).unwrap();
    let parsed_list: ListDownloadRecordsRequest = serde_json::from_str(&list_json).unwrap();
    assert_eq!(list_req, parsed_list);

    let list_res = ListDownloadRecordsResponse {
        records: vec![record],
    };
    let list_res_json = serde_json::to_string(&list_res).unwrap();
    let parsed_list_res: ListDownloadRecordsResponse =
        serde_json::from_str(&list_res_json).unwrap();
    assert_eq!(list_res, parsed_list_res);

    let pause_req = PauseDownloadRequest {
        record_id: DownloadRecordId::new("dl-rec-1"),
    };
    let pause_json = serde_json::to_string(&pause_req).unwrap();
    let parsed_pause: PauseDownloadRequest = serde_json::from_str(&pause_json).unwrap();
    assert_eq!(pause_req, parsed_pause);

    let pause_res = PauseDownloadResponse {
        success: true,
        status: DownloadLifecycleStatus::Paused,
    };
    let pause_res_json = serde_json::to_string(&pause_res).unwrap();
    let parsed_pause_res: PauseDownloadResponse = serde_json::from_str(&pause_res_json).unwrap();
    assert_eq!(pause_res, parsed_pause_res);

    let resume_req = ResumeDownloadRequest {
        record_id: DownloadRecordId::new("dl-rec-1"),
    };
    let resume_json = serde_json::to_string(&resume_req).unwrap();
    let parsed_resume: ResumeDownloadRequest = serde_json::from_str(&resume_json).unwrap();
    assert_eq!(resume_req, parsed_resume);

    let resume_res = ResumeDownloadResponse {
        success: true,
        status: DownloadLifecycleStatus::Active,
    };
    let resume_res_json = serde_json::to_string(&resume_res).unwrap();
    let parsed_resume_res: ResumeDownloadResponse = serde_json::from_str(&resume_res_json).unwrap();
    assert_eq!(resume_res, parsed_resume_res);

    let cancel_req = CancelDownloadRequest {
        record_id: DownloadRecordId::new("dl-rec-1"),
    };
    let cancel_json = serde_json::to_string(&cancel_req).unwrap();
    let parsed_cancel: CancelDownloadRequest = serde_json::from_str(&cancel_json).unwrap();
    assert_eq!(cancel_req, parsed_cancel);

    let cancel_res = CancelDownloadResponse {
        success: true,
        status: DownloadLifecycleStatus::Cancelled,
    };
    let cancel_res_json = serde_json::to_string(&cancel_res).unwrap();
    let parsed_cancel_res: CancelDownloadResponse = serde_json::from_str(&cancel_res_json).unwrap();
    assert_eq!(cancel_res, parsed_cancel_res);
}

#[test]
fn cookies_and_site_data_contract_wire_fixtures_stability() {
    assert_eq!(CONTRACT_BROWSER_COOKIES, "browser.cookies");
    assert_eq!(CONTRACT_BROWSER_COOKIES_VERSION, "0.1");
    assert_eq!(CONTRACT_BROWSER_SITE_DATA, "browser.site-data");
    assert_eq!(CONTRACT_BROWSER_SITE_DATA_VERSION, "0.1");

    let meta = CookieMetadata {
        name: "session_id".to_string(),
        domain: "127.0.0.1".to_string(),
        path: "/".to_string(),
        secure: true,
        http_only: true,
        same_site: Some("Strict".to_string()),
        expires_epoch_sec: Some(1800000000),
    };
    let meta_json = serde_json::to_string(&meta).unwrap();
    let parsed_meta: CookieMetadata = serde_json::from_str(&meta_json).unwrap();
    assert_eq!(meta, parsed_meta);

    let val = CookieValue::new("session_id", "127.0.0.1", "/", "super_secret_auth_token");
    let val_json = serde_json::to_string(&val).unwrap();
    let parsed_val: CookieValue = serde_json::from_str(&val_json).unwrap();
    assert_eq!(val, parsed_val);
    assert_eq!(parsed_val.expose_value(), "super_secret_auth_token");

    let get_meta_req = GetCookieMetadataRequest {
        context_id: BrowserContextId::new("ctx-1"),
        url: Some("http://127.0.0.1:8080/".to_string()),
        domain: Some("127.0.0.1".to_string()),
    };
    let get_meta_json = serde_json::to_string(&get_meta_req).unwrap();
    let parsed_get_meta: GetCookieMetadataRequest = serde_json::from_str(&get_meta_json).unwrap();
    assert_eq!(get_meta_req, parsed_get_meta);

    let get_meta_res = GetCookieMetadataResponse {
        cookies: vec![meta],
    };
    let get_meta_res_json = serde_json::to_string(&get_meta_res).unwrap();
    let parsed_get_meta_res: GetCookieMetadataResponse =
        serde_json::from_str(&get_meta_res_json).unwrap();
    assert_eq!(get_meta_res, parsed_get_meta_res);

    let get_val_req = GetCookieValueRequest {
        context_id: BrowserContextId::new("ctx-1"),
        domain: "127.0.0.1".to_string(),
        name: "session_id".to_string(),
        path: Some("/".to_string()),
        url: None,
    };
    let get_val_json = serde_json::to_string(&get_val_req).unwrap();
    let parsed_get_val: GetCookieValueRequest = serde_json::from_str(&get_val_json).unwrap();
    assert_eq!(get_val_req, parsed_get_val);

    let get_val_res = GetCookieValueResponse { cookie: Some(val) };
    let get_val_res_json = serde_json::to_string(&get_val_res).unwrap();
    let parsed_get_val_res: GetCookieValueResponse =
        serde_json::from_str(&get_val_res_json).unwrap();
    assert_eq!(get_val_res, parsed_get_val_res);

    let set_req = SetCookieServiceRequest {
        context_id: BrowserContextId::new("ctx-1"),
        name: "theme".to_string(),
        value: "dark".to_string(),
        domain: "127.0.0.1".to_string(),
        path: Some("/".to_string()),
        secure: Some(false),
        http_only: Some(false),
        same_site: Some("Lax".to_string()),
        expires_epoch_sec: None,
    };
    let set_json = serde_json::to_string(&set_req).unwrap();
    let parsed_set: SetCookieServiceRequest = serde_json::from_str(&set_json).unwrap();
    assert_eq!(set_req, parsed_set);

    let set_res = SetCookieServiceResponse { success: true };
    let set_res_json = serde_json::to_string(&set_res).unwrap();
    let parsed_set_res: SetCookieServiceResponse = serde_json::from_str(&set_res_json).unwrap();
    assert_eq!(set_res, parsed_set_res);

    let del_req = DeleteCookieServiceRequest {
        context_id: BrowserContextId::new("ctx-1"),
        domain: "127.0.0.1".to_string(),
        name: "theme".to_string(),
        path: Some("/".to_string()),
        url: None,
    };
    let del_json = serde_json::to_string(&del_req).unwrap();
    let parsed_del: DeleteCookieServiceRequest = serde_json::from_str(&del_json).unwrap();
    assert_eq!(del_req, parsed_del);

    let del_res = DeleteCookieServiceResponse { deleted_count: 1 };
    let del_res_json = serde_json::to_string(&del_res).unwrap();
    let parsed_del_res: DeleteCookieServiceResponse = serde_json::from_str(&del_res_json).unwrap();
    assert_eq!(del_res, parsed_del_res);

    let clear_site_req = ClearSiteDataRequest {
        context_id: BrowserContextId::new("ctx-1"),
        origin: "http://127.0.0.1:8080".to_string(),
        storage_type: StorageType::LocalStorage,
    };
    let clear_site_json = serde_json::to_string(&clear_site_req).unwrap();
    let parsed_clear_site: ClearSiteDataRequest = serde_json::from_str(&clear_site_json).unwrap();
    assert_eq!(clear_site_req, parsed_clear_site);

    let clear_site_res = ClearSiteDataResponse { cleared: true };
    let clear_site_res_json = serde_json::to_string(&clear_site_res).unwrap();
    let parsed_clear_site_res: ClearSiteDataResponse =
        serde_json::from_str(&clear_site_res_json).unwrap();
    assert_eq!(clear_site_res, parsed_clear_site_res);
}

#[test]
fn cookie_value_secret_redaction_diagnostic() {
    let secret = "super-secret-production-token-999";
    let cookie_val = CookieValue::new("auth", "worldline.test", "/", secret);

    let debug_output = format!("{:?}", cookie_val);
    assert!(
        debug_output.contains("[REDACTED]"),
        "Debug output must redact cookie value"
    );
    assert!(
        !debug_output.contains(secret),
        "Debug output must NOT leak raw secret cookie string"
    );

    let set_req = SetCookieServiceRequest {
        context_id: BrowserContextId::new("ctx-1"),
        name: "auth".to_string(),
        value: secret.to_string(),
        domain: "worldline.test".to_string(),
        path: Some("/".to_string()),
        secure: Some(true),
        http_only: Some(true),
        same_site: Some("Strict".to_string()),
        expires_epoch_sec: None,
    };
    let set_debug = format!("{:?}", set_req);
    assert!(
        set_debug.contains("[REDACTED]"),
        "Set request Debug output must redact cookie value"
    );
    assert!(
        !set_debug.contains(secret),
        "Set request Debug output must NOT leak raw secret cookie string"
    );

    // Serialization across wire must preserve the plaintext for authorized caller
    let serialized = serde_json::to_string(&cookie_val).unwrap();
    assert!(
        serialized.contains(secret),
        "Wire serialization must carry the actual value"
    );
}

#[test]
fn authority_separation_invariants() {
    let tabs_authorities = [AUTH_TABS_READ, AUTH_TABS_MUTATE];
    let history_authorities = [AUTH_HISTORY_READ, AUTH_HISTORY_DELETE];
    let downloads_authorities = [AUTH_DOWNLOADS_READ, AUTH_DOWNLOADS_CONTROL];
    let cookies_authorities = [
        AUTH_COOKIES_METADATA_READ,
        AUTH_COOKIES_VALUE_READ,
        AUTH_COOKIES_MUTATE,
        AUTH_COOKIES_ADMIN,
    ];
    let site_data_authorities = [AUTH_SITE_DATA_CLEAR];

    let engine_mutation_authorities = [OP_NAVIGATE, OP_CLICK];
    let engine_observation_authorities = [OP_OBSERVE, OP_QUERY_DOCUMENT];

    // All service authorities must be distinct
    let mut all_service_auths = Vec::new();
    all_service_auths.extend_from_slice(&tabs_authorities);
    all_service_auths.extend_from_slice(&history_authorities);
    all_service_auths.extend_from_slice(&downloads_authorities);
    all_service_auths.extend_from_slice(&cookies_authorities);
    all_service_auths.extend_from_slice(&site_data_authorities);

    for (i, auth_a) in all_service_auths.iter().enumerate() {
        for (j, auth_b) in all_service_auths.iter().enumerate() {
            if i != j {
                assert_ne!(
                    *auth_a, *auth_b,
                    "Distinct service authorities must have distinct identifier strings"
                );
            }
        }

        // None of the service authorities grant engine mutation or observation
        for eng_mut in &engine_mutation_authorities {
            assert_ne!(
                *auth_a, *eng_mut,
                "Service authority must not grant engine mutation"
            );
        }
        for eng_obs in &engine_observation_authorities {
            assert_ne!(
                *auth_a, *eng_obs,
                "Service authority must not grant engine observation"
            );
        }
    }
}
