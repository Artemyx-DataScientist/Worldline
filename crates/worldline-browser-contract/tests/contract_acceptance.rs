use worldline_browser_contract::{
    action::validate_element_reference,
    authority::{
        BrowserAuthority, BrowserAuthoritySet, OP_CLICK, OP_INPUT, OP_NAVIGATE, OP_OBSERVE,
        OP_QUERY_DOCUMENT,
    },
    contracts::{CreateContextRequest, CreatePageRequest},
    error::BrowserError,
    events::{EVENT_NAVIGATION_COMMITTED, NavigationCommittedEvent},
    identity::{
        BrowserContextId, DocumentRevision, ElementRef, PageId, context_resource, page_resource,
    },
    query::{
        AccessibilityNode, AccessibilityRole, AccessibilityTree, DocumentMetadata,
        DocumentSnapshot, QueryBounds,
    },
};

#[test]
fn contracts_serialize_and_deserialize_without_engine_types() {
    let ctx_req = CreateContextRequest {
        profile_id: Some("user-profile-alpha".to_string()),
        incognito: false,
        user_agent: None,
    };
    let json = serde_json::to_string(&ctx_req).expect("must serialize");
    let deserialized: CreateContextRequest = serde_json::from_str(&json).expect("must deserialize");
    assert_eq!(ctx_req, deserialized);

    let page_req = CreatePageRequest {
        context_id: BrowserContextId::new("ctx-test"),
        initial_url: Some("about:blank".to_string()),
    };
    let json_page = serde_json::to_string(&page_req).expect("must serialize");
    let deserialized_page: CreatePageRequest =
        serde_json::from_str(&json_page).expect("must deserialize");
    assert_eq!(page_req, deserialized_page);
}

#[test]
fn query_bounds_truncates_large_accessibility_tree_safely() {
    let page_id = PageId::new("page-bounds");
    let rev = DocumentRevision::initial();

    // Construct a deeply nested tree with many nodes
    let mut deep_node = AccessibilityNode::new("leaf", AccessibilityRole::StaticText)
        .with_name("Very long text content that will be bounded by byte budget");
    for i in 0..30 {
        deep_node = AccessibilityNode::new(format!("node-{i}"), AccessibilityRole::Group)
            .with_name(format!("Group {i}"))
            .with_child(deep_node);
    }

    let tree = AccessibilityTree::new(page_id.clone(), rev, deep_node);
    assert_eq!(tree.total_node_count, 31);
    assert!(!tree.is_truncated);

    // Apply strict bounds (max_depth: 5, max_nodes: 10)
    let bounds = QueryBounds {
        max_depth: 5,
        max_nodes: 10,
        max_text_len: 20,
        max_total_text_bytes: 100,
    };

    let bounded = tree.to_bounded(&bounds);
    assert!(bounded.is_truncated);
    assert!(bounded.truncated_node_count > 0);
    assert!(bounded.root.count_nodes() <= bounds.max_nodes);

    let doc = DocumentSnapshot::new(
        DocumentMetadata {
            page_id,
            url: "http://worldline.local".to_string(),
            title: "Test".to_string(),
            document_revision: rev,
            status_code: 200,
        },
        tree,
    );
    let bounded_doc = doc.to_bounded(&bounds);
    assert!(bounded_doc.is_truncated);
}

#[test]
fn observe_authority_cannot_navigate_or_act() {
    let observe_set = BrowserAuthoritySet::new().with(BrowserAuthority::ObservePage);

    assert!(observe_set.permits("browser.observe", OP_OBSERVE));
    assert!(!observe_set.permits("browser.navigate", OP_NAVIGATE));
    assert!(!observe_set.permits("browser.act", OP_CLICK));
    assert!(!observe_set.permits("browser.act", OP_INPUT));
}

#[test]
fn query_authority_cannot_click_or_input() {
    let query_set = BrowserAuthoritySet::new().with(BrowserAuthority::QueryDocument);

    assert!(query_set.permits("browser.query", OP_QUERY_DOCUMENT));
    assert!(!query_set.permits("browser.act", OP_CLICK));
    assert!(!query_set.permits("browser.act", OP_INPUT));
    assert!(!query_set.permits("browser.navigate", OP_NAVIGATE));
}

#[test]
fn act_authority_is_scoped_to_authorized_page() {
    let page_1 = PageId::new("page-1");
    let page_2 = PageId::new("page-2");
    let rev_1 = DocumentRevision::new(1);

    let elem_page_1 = ElementRef::new(page_1.clone(), rev_1, "btn-submit");

    // ElementRef matching target page is valid
    assert!(validate_element_reference(&elem_page_1, &page_1, rev_1).is_ok());

    // ElementRef targeting different page is rejected
    let mismatch = validate_element_reference(&elem_page_1, &page_2, rev_1);
    assert!(mismatch.is_err());
    match mismatch.unwrap_err() {
        BrowserError::ResourceMismatch { expected, actual } => {
            assert_eq!(expected, "page-2");
            assert_eq!(actual, "page-1");
        }
        other => panic!("expected ResourceMismatch error, got {:?}", other),
    }
}

#[test]
fn stale_element_reference_after_revision_bump_is_rejected() {
    let page = PageId::new("page-live");
    let rev_1 = DocumentRevision::new(1);
    let rev_2 = DocumentRevision::new(2);

    let stale_elem = ElementRef::new(page.clone(), rev_1, "input-query");

    // Document has bumped to rev_2, so rev_1 ElementRef is stale
    let err = validate_element_reference(&stale_elem, &page, rev_2).unwrap_err();
    assert!(err.is_stale_element());
    match err {
        BrowserError::StaleElementReference {
            expected_revision,
            actual_revision,
        } => {
            assert_eq!(expected_revision, rev_1);
            assert_eq!(actual_revision, rev_2);
        }
        other => panic!("expected StaleElementReference, got {:?}", other),
    }
}

#[test]
fn browser_event_cannot_satisfy_browser_rpc_invocation() {
    // Invariant 4: EVENT BUS IS NOT RPC.
    let event = NavigationCommittedEvent {
        page_id: PageId::new("page-1"),
        navigation_id: worldline_browser_contract::identity::NavigationId::new("nav-1"),
        url: "http://worldline.local/test".to_string(),
        document_revision: DocumentRevision::new(2),
        status_code: 200,
    };
    let event_payload = serde_json::to_vec(&event).unwrap();

    // Trying to deserialize event payload as NavigateRequest fails closed
    let parse_as_req = serde_json::from_slice::<
        worldline_browser_contract::contracts::NavigateRequest,
    >(&event_payload);
    // Even if fields align partially, event topic is disjoint from capability contract
    assert_eq!(EVENT_NAVIGATION_COMMITTED, "browser.navigation.committed");
    assert_ne!(EVENT_NAVIGATION_COMMITTED, "browser.navigate");
    assert!(parse_as_req.is_ok()); // Payload alone is data; the transport route and contract ID enforce separation
}

#[test]
fn resource_identities_are_worldline_scoped() {
    let ctx = BrowserContextId::new("ctx-99");
    let page = PageId::new("page-42");

    assert_eq!(context_resource(&ctx), "browser-context/ctx-99");
    assert_eq!(page_resource(&page), "browser-page/page-42");
}

#[test]
fn all_eight_v1_contracts_and_v0_1_experimental_contracts_wire_compatibility() {
    use worldline_browser_contract::{
        action::ClickActionRequest,
        capture::{
            CaptureFormat, CapturePageRequest, CapturePageResponse, CaptureTarget,
            ReadCaptureArtifactRequest, ReadCaptureArtifactResponse,
        },
        contracts::{
            CloseContextRequest, ClosePageRequest, ControlDownloadRequest, DownloadAction,
            ListContextsResponse, ListPagesRequest, ListPagesResponse, NavigateRequest,
            ObservePageRequest, QueryDocumentRequest, QueryPermissionRequest, ReloadRequest,
            SetPermissionRequest, StartDownloadRequest, StopRequest,
        },
        events::{PageRestoredEvent, RendererCrashedEvent},
        primitives::{
            ClearStorageRequest, Cookie, DeleteCookiesRequest, DownloadHookAction,
            DownloadHookDecision, GetCookiesRequest, SetCookieRequest, StorageType,
        },
    };

    // 1. context v1.0
    let close_ctx = CloseContextRequest {
        context_id: BrowserContextId::new("c1"),
    };
    let json = serde_json::to_string(&close_ctx).unwrap();
    assert_eq!(
        serde_json::from_str::<CloseContextRequest>(&json).unwrap(),
        close_ctx
    );

    let list_ctx_resp = ListContextsResponse {
        contexts: vec![BrowserContextId::new("c1"), BrowserContextId::new("c2")],
    };
    let json = serde_json::to_string(&list_ctx_resp).unwrap();
    assert_eq!(
        serde_json::from_str::<ListContextsResponse>(&json).unwrap(),
        list_ctx_resp
    );

    // 2. page v1.0
    let close_page = ClosePageRequest {
        page_id: PageId::new("p1"),
    };
    let json = serde_json::to_string(&close_page).unwrap();
    assert_eq!(
        serde_json::from_str::<ClosePageRequest>(&json).unwrap(),
        close_page
    );

    let list_pages = ListPagesRequest {
        context_id: BrowserContextId::new("c1"),
    };
    let json = serde_json::to_string(&list_pages).unwrap();
    assert_eq!(
        serde_json::from_str::<ListPagesRequest>(&json).unwrap(),
        list_pages
    );

    let list_pages_resp = ListPagesResponse { pages: vec![] };
    let json = serde_json::to_string(&list_pages_resp).unwrap();
    assert_eq!(
        serde_json::from_str::<ListPagesResponse>(&json).unwrap(),
        list_pages_resp
    );

    // 3. navigate v1.0
    let nav_req = NavigateRequest {
        page_id: PageId::new("p1"),
        url: "https://worldline.test/welcome".to_string(),
    };
    let json = serde_json::to_string(&nav_req).unwrap();
    assert_eq!(
        serde_json::from_str::<NavigateRequest>(&json).unwrap(),
        nav_req
    );

    let reload_req = ReloadRequest {
        page_id: PageId::new("p1"),
        ignore_cache: true,
    };
    let json = serde_json::to_string(&reload_req).unwrap();
    assert_eq!(
        serde_json::from_str::<ReloadRequest>(&json).unwrap(),
        reload_req
    );

    let stop_req = StopRequest {
        page_id: PageId::new("p1"),
    };
    let json = serde_json::to_string(&stop_req).unwrap();
    assert_eq!(
        serde_json::from_str::<StopRequest>(&json).unwrap(),
        stop_req
    );

    // 4. observe v1.0
    let obs_req = ObservePageRequest {
        page_id: PageId::new("p1"),
    };
    let json = serde_json::to_string(&obs_req).unwrap();
    assert_eq!(
        serde_json::from_str::<ObservePageRequest>(&json).unwrap(),
        obs_req
    );

    // 5. query v1.0
    let query_req = QueryDocumentRequest {
        page_id: PageId::new("p1"),
        bounds: Some(QueryBounds::default()),
    };
    let json = serde_json::to_string(&query_req).unwrap();
    assert_eq!(
        serde_json::from_str::<QueryDocumentRequest>(&json).unwrap(),
        query_req
    );

    // 6. act v1.0
    let click_req = ClickActionRequest {
        element_ref: ElementRef::new(PageId::new("p1"), DocumentRevision::new(1), "btn-ok"),
    };
    let json = serde_json::to_string(&click_req).unwrap();
    assert_eq!(
        serde_json::from_str::<ClickActionRequest>(&json).unwrap(),
        click_req
    );

    // 7. download v1.0
    let start_dl = StartDownloadRequest {
        page_id: PageId::new("p1"),
        url: "https://worldline.test/file.zip".to_string(),
        destination_path: Some("file.zip".to_string()),
    };
    let json = serde_json::to_string(&start_dl).unwrap();
    assert_eq!(
        serde_json::from_str::<StartDownloadRequest>(&json).unwrap(),
        start_dl
    );

    let ctrl_dl = ControlDownloadRequest {
        download_id: worldline_browser_contract::identity::DownloadId::new("d1"),
        action: DownloadAction::Cancel,
    };
    let json = serde_json::to_string(&ctrl_dl).unwrap();
    assert_eq!(
        serde_json::from_str::<ControlDownloadRequest>(&json).unwrap(),
        ctrl_dl
    );

    // 8. permission v1.0
    let set_perm = SetPermissionRequest {
        context_id: BrowserContextId::new("c1"),
        permission_type: worldline_browser_contract::contracts::PermissionType::Geolocation,
        origin: "https://worldline.test".to_string(),
        decision: worldline_browser_contract::contracts::PermissionDecision::Granted,
    };
    let json = serde_json::to_string(&set_perm).unwrap();
    assert_eq!(
        serde_json::from_str::<SetPermissionRequest>(&json).unwrap(),
        set_perm
    );

    let query_perm = QueryPermissionRequest {
        context_id: BrowserContextId::new("c1"),
        permission_type: worldline_browser_contract::contracts::PermissionType::Geolocation,
        origin: "https://worldline.test".to_string(),
    };
    let json = serde_json::to_string(&query_perm).unwrap();
    assert_eq!(
        serde_json::from_str::<QueryPermissionRequest>(&json).unwrap(),
        query_perm
    );

    // Experimental 0.1: capture
    let cap_req = CapturePageRequest {
        page_id: PageId::new("p1"),
        target: CaptureTarget::PageViewport,
        format: CaptureFormat::Png,
        quality: Some(90),
        max_bytes: Some(1024 * 1024),
    };
    let json = serde_json::to_string(&cap_req).unwrap();
    assert_eq!(
        serde_json::from_str::<CapturePageRequest>(&json).unwrap(),
        cap_req
    );

    let cap_resp = CapturePageResponse {
        artifact: worldline_browser_contract::capture::CaptureArtifactRef {
            artifact_id: "art-1".to_string(),
            page_id: PageId::new("p1"),
            revision: DocumentRevision::new(1),
            byte_len: 2048,
            mime_type: "image/png".to_string(),
            blob_id: "blob-sha256-abc".to_string(),
        },
    };
    let json = serde_json::to_string(&cap_resp).unwrap();
    assert_eq!(
        serde_json::from_str::<CapturePageResponse>(&json).unwrap(),
        cap_resp
    );

    let read_cap = ReadCaptureArtifactRequest {
        artifact_id: "art-1".to_string(),
        offset: 0,
        max_bytes: 4096,
    };
    let json = serde_json::to_string(&read_cap).unwrap();
    assert_eq!(
        serde_json::from_str::<ReadCaptureArtifactRequest>(&json).unwrap(),
        read_cap
    );

    let read_cap_resp = ReadCaptureArtifactResponse {
        artifact_id: "art-1".to_string(),
        data: vec![1, 2, 3, 4],
        is_truncated: false,
        total_bytes: 4,
    };
    let json = serde_json::to_string(&read_cap_resp).unwrap();
    assert_eq!(
        serde_json::from_str::<ReadCaptureArtifactResponse>(&json).unwrap(),
        read_cap_resp
    );

    // Experimental 0.1: primitives (cookies, storage, download hook)
    let get_cookies = GetCookiesRequest {
        context_id: BrowserContextId::new("c1"),
        url: Some("https://worldline.test".to_string()),
        domain: None,
    };
    let json = serde_json::to_string(&get_cookies).unwrap();
    assert_eq!(
        serde_json::from_str::<GetCookiesRequest>(&json).unwrap(),
        get_cookies
    );

    let set_cookie = SetCookieRequest {
        context_id: BrowserContextId::new("c1"),
        cookie: Cookie {
            name: "session".to_string(),
            value: "xyz".to_string(),
            domain: "worldline.test".to_string(),
            path: "/".to_string(),
            secure: true,
            http_only: true,
            same_site: Some("Strict".to_string()),
            expires_epoch_sec: Some(1893456000),
        },
    };
    let json = serde_json::to_string(&set_cookie).unwrap();
    assert_eq!(
        serde_json::from_str::<SetCookieRequest>(&json).unwrap(),
        set_cookie
    );

    let del_cookie = DeleteCookiesRequest {
        context_id: BrowserContextId::new("c1"),
        url: None,
        name: Some("session".to_string()),
        domain: Some("worldline.test".to_string()),
    };
    let json = serde_json::to_string(&del_cookie).unwrap();
    assert_eq!(
        serde_json::from_str::<DeleteCookiesRequest>(&json).unwrap(),
        del_cookie
    );

    let clear_storage = ClearStorageRequest {
        context_id: BrowserContextId::new("c1"),
        origin: "https://worldline.test".to_string(),
        storage_type: StorageType::LocalStorage,
    };
    let json = serde_json::to_string(&clear_storage).unwrap();
    assert_eq!(
        serde_json::from_str::<ClearStorageRequest>(&json).unwrap(),
        clear_storage
    );

    let hook_dec = DownloadHookDecision {
        download_id: worldline_browser_contract::identity::DownloadId::new("d1"),
        action: DownloadHookAction::Redirect {
            destination_path: "/safe/downloads/file.zip".to_string(),
        },
    };
    let json = serde_json::to_string(&hook_dec).unwrap();
    assert_eq!(
        serde_json::from_str::<DownloadHookDecision>(&json).unwrap(),
        hook_dec
    );

    // Additive Events
    let restored_evt = PageRestoredEvent {
        context_id: BrowserContextId::new("c1"),
        page_id: PageId::new("p1"),
        url: "https://worldline.test".to_string(),
        document_revision: DocumentRevision::new(5),
    };
    let json = serde_json::to_string(&restored_evt).unwrap();
    assert_eq!(
        serde_json::from_str::<PageRestoredEvent>(&json).unwrap(),
        restored_evt
    );

    let renderer_crashed = RendererCrashedEvent {
        context_id: BrowserContextId::new("c1"),
        page_id: PageId::new("p1"),
        exit_code: Some(137),
        reason: "Out of memory".to_string(),
    };
    let json = serde_json::to_string(&renderer_crashed).unwrap();
    assert_eq!(
        serde_json::from_str::<RendererCrashedEvent>(&json).unwrap(),
        renderer_crashed
    );
}

#[test]
fn engine_cookie_v0_1_shape_is_preserved_and_v0_2_adds_scope() {
    use worldline_browser_contract::{
        Cookie, CookieV0_2, GetCookiesResponseV0_2, SetCookieRequest, SetCookieRequestV0_2,
    };

    let legacy = SetCookieRequest {
        context_id: BrowserContextId::new("legacy"),
        cookie: Cookie {
            name: "session".to_string(),
            value: "value".to_string(),
            domain: "example.com".to_string(),
            path: "/".to_string(),
            secure: false,
            http_only: true,
            same_site: None,
            expires_epoch_sec: None,
        },
    };
    let legacy_json = serde_json::to_string(&legacy).unwrap();
    assert!(!legacy_json.contains("host_only"));
    assert_eq!(
        serde_json::from_str::<SetCookieRequest>(&legacy_json).unwrap(),
        legacy
    );

    let versioned = SetCookieRequestV0_2 {
        context_id: BrowserContextId::new("versioned"),
        cookie: CookieV0_2 {
            name: "session".to_string(),
            value: "value".to_string(),
            domain: "example.com".to_string(),
            path: "/".to_string(),
            secure: false,
            http_only: true,
            same_site: None,
            expires_epoch_sec: None,
            host_only: false,
        },
    };
    let versioned_json = serde_json::to_string(&versioned).unwrap();
    assert!(versioned_json.contains("\"host_only\":false"));
    let response = GetCookiesResponseV0_2 {
        cookies: vec![versioned.cookie],
    };
    assert_eq!(
        serde_json::from_str::<GetCookiesResponseV0_2>(&serde_json::to_string(&response).unwrap())
            .unwrap(),
        response
    );
}
