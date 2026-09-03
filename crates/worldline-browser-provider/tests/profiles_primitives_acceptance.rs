use worldline_browser_contract::{
    authority::*,
    contracts::{
        CloseContextRequest, ControlDownloadRequest, CreateContextRequest, CreateContextResponse,
        CreatePageRequest, CreatePageResponse, DownloadAction, DownloadState,
        DownloadStatusResponse, ListContextsResponse, PermissionDecision, PermissionResponse,
        PermissionType, QueryPermissionRequest, SetPermissionRequest, StartDownloadRequest,
    },
    primitives::{
        ClearStorageRequest, ClearStorageResponse, Cookie, DeleteCookiesRequest,
        DeleteCookiesResponse, GetCookiesRequest, GetCookiesResponse, SetCookieRequest,
        SetCookieResponse, StorageType,
    },
};
use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};

#[test]
fn context_profile_isolation_and_lifecycle() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    // 1. Create persistent profile context
    let ctx1_val = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some("user-profile-alpha".to_string()),
                incognito: false,
                user_agent: Some("CustomAgent/1.0".to_string()),
            })
            .unwrap(),
        )
        .unwrap();
    let ctx1: CreateContextResponse = serde_json::from_value(ctx1_val).unwrap();
    assert_eq!(ctx1.profile_id, Some("user-profile-alpha".to_string()));
    assert!(!ctx1.incognito);

    // 2. Create incognito ephemeral context
    let ctx2_val = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: None,
                incognito: true,
                user_agent: None,
            })
            .unwrap(),
        )
        .unwrap();
    let ctx2: CreateContextResponse = serde_json::from_value(ctx2_val).unwrap();
    assert!(ctx2.incognito);

    // 3. List contexts contains both
    let list_val = core
        .dispatch_contract("browser.context", OP_LIST_CONTEXTS, serde_json::json!({}))
        .unwrap();
    let list_resp: ListContextsResponse = serde_json::from_value(list_val).unwrap();
    assert_eq!(list_resp.contexts.len(), 2);
    assert!(list_resp.contexts.contains(&ctx1.context_id));
    assert!(list_resp.contexts.contains(&ctx2.context_id));

    // 4. Close persistent context
    core.dispatch_contract(
        "browser.context",
        OP_CLOSE_CONTEXT,
        serde_json::to_value(CloseContextRequest {
            context_id: ctx1.context_id.clone(),
        })
        .unwrap(),
    )
    .unwrap();

    let list_val2 = core
        .dispatch_contract("browser.context", OP_LIST_CONTEXTS, serde_json::json!({}))
        .unwrap();
    let list_resp2: ListContextsResponse = serde_json::from_value(list_val2).unwrap();
    assert_eq!(list_resp2.contexts.len(), 1);
    assert!(!list_resp2.contexts.contains(&ctx1.context_id));
    assert!(list_resp2.contexts.contains(&ctx2.context_id));
}

#[test]
fn cookie_store_and_storage_partitioning() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    let ctx1: CreateContextResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some("p1".to_string()),
                incognito: false,
                user_agent: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let ctx2: CreateContextResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: None,
                incognito: true,
                user_agent: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    // 1. Set cookie in ctx1
    let set_cookie_ctx1 = SetCookieRequest {
        context_id: ctx1.context_id.clone(),
        cookie: Cookie {
            name: "session_id".to_string(),
            value: "xyz123".to_string(),
            domain: "alpha.test".to_string(),
            path: "/".to_string(),
            secure: true,
            http_only: true,
            same_site: None,
            expires_epoch_sec: None,
        },
    };
    let set_res1: SetCookieResponse = serde_json::from_value(
        core.dispatch(
            OP_COOKIE_SET,
            serde_json::to_value(set_cookie_ctx1).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(set_res1.success);

    // 2. Cookie is visible in ctx1 but NOT in ctx2
    let get_c1: GetCookiesResponse = serde_json::from_value(
        core.dispatch(
            OP_COOKIE_GET,
            serde_json::to_value(GetCookiesRequest {
                context_id: ctx1.context_id.clone(),
                url: None,
                domain: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(get_c1.cookies.len(), 1);
    assert_eq!(get_c1.cookies[0].name, "session_id");

    let get_c2: GetCookiesResponse = serde_json::from_value(
        core.dispatch(
            OP_COOKIE_GET,
            serde_json::to_value(GetCookiesRequest {
                context_id: ctx2.context_id.clone(),
                url: None,
                domain: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(get_c2.cookies.len(), 0);

    // 3. Delete cookies by name
    let del_resp: DeleteCookiesResponse = serde_json::from_value(
        core.dispatch(
            OP_COOKIE_DELETE,
            serde_json::to_value(DeleteCookiesRequest {
                context_id: ctx1.context_id.clone(),
                url: None,
                name: Some("session_id".to_string()),
                domain: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(del_resp.deleted_count, 1);

    // 4. Clear storage
    let clear_resp: ClearStorageResponse = serde_json::from_value(
        core.dispatch(
            OP_STORAGE_CLEAR,
            serde_json::to_value(ClearStorageRequest {
                context_id: ctx1.context_id.clone(),
                origin: "https://alpha.test".to_string(),
                storage_type: StorageType::LocalStorage,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(clear_resp.cleared);
}

#[test]
fn permissions_and_download_lifecycle() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    let ctx: CreateContextResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: None,
                incognito: false,
                user_agent: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let page: CreatePageResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx.context_id.clone(),
                initial_url: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    // 1. Query initial permission -> Prompt
    let perm_query: PermissionResponse = serde_json::from_value(
        core.dispatch(
            OP_PERMISSION_QUERY,
            serde_json::to_value(QueryPermissionRequest {
                context_id: ctx.context_id.clone(),
                origin: "https://secure.test".to_string(),
                permission_type: PermissionType::Geolocation,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(perm_query.decision, PermissionDecision::Prompt);

    // 2. Set permission -> Granted
    let perm_set: PermissionResponse = serde_json::from_value(
        core.dispatch(
            OP_PERMISSION_SET,
            serde_json::to_value(SetPermissionRequest {
                context_id: ctx.context_id.clone(),
                origin: "https://secure.test".to_string(),
                permission_type: PermissionType::Geolocation,
                decision: PermissionDecision::Granted,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(perm_set.decision, PermissionDecision::Granted);

    // 3. Start download
    let dl_resp: DownloadStatusResponse = serde_json::from_value(
        core.dispatch(
            OP_DOWNLOAD_START,
            serde_json::to_value(StartDownloadRequest {
                page_id: page.page_id.clone(),
                url: "https://secure.test/file.zip".to_string(),
                destination_path: Some("archive.zip".to_string()),
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(dl_resp.state, DownloadState::InProgress);
    assert_eq!(dl_resp.destination_path, "archive.zip");

    // 4. Control download -> Pause
    let dl_pause: DownloadStatusResponse = serde_json::from_value(
        core.dispatch(
            OP_DOWNLOAD_CONTROL,
            serde_json::to_value(ControlDownloadRequest {
                download_id: dl_resp.download_id.clone(),
                action: DownloadAction::Pause,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(dl_pause.state, DownloadState::Paused);

    // 5. Control download -> Cancel
    let dl_cancel: DownloadStatusResponse = serde_json::from_value(
        core.dispatch(
            OP_DOWNLOAD_CONTROL,
            serde_json::to_value(ControlDownloadRequest {
                download_id: dl_resp.download_id,
                action: DownloadAction::Cancel,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(dl_cancel.state, DownloadState::Cancelled);
}
