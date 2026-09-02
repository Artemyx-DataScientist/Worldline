use worldline_browser_contract::{
    action::ClickActionRequest,
    authority::*,
    capture::{CaptureFormat, CapturePageRequest, CaptureTarget, ReadCaptureArtifactRequest},
    contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        ObservePageRequest, PageObservation, QueryDocumentRequest,
    },
    identity::{DocumentRevision, ElementRef},
    primitives::{ClearStorageRequest, Cookie, GetCookiesRequest, SetCookieRequest, StorageType},
    query::DocumentSnapshot,
};
use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};

#[test]
fn provider_core_full_page_lifecycle_and_actions() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    // 1. Create context
    let ctx_req = CreateContextRequest {
        profile_id: Some("test-profile".to_string()),
        incognito: false,
        user_agent: None,
    };
    let ctx_val = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(ctx_req).unwrap(),
        )
        .expect("must create context");
    let ctx_resp: CreateContextResponse = serde_json::from_value(ctx_val).unwrap();

    // 2. Create page
    let page_req = CreatePageRequest {
        context_id: ctx_resp.context_id.clone(),
        initial_url: Some("https://worldline.test/start".to_string()),
    };
    let page_val = core
        .dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(page_req).unwrap(),
        )
        .expect("must create page");
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();

    // 3. Observe initial page
    let obs_val = core
        .dispatch(
            OP_OBSERVE,
            serde_json::to_value(ObservePageRequest {
                page_id: page_resp.page_id.clone(),
            })
            .unwrap(),
        )
        .expect("must observe");
    let obs: PageObservation = serde_json::from_value(obs_val).unwrap();
    assert_eq!(obs.url, "https://worldline.test/start");
    assert_eq!(obs.document_revision, DocumentRevision::new(1));

    // 4. Query accessibility tree
    let doc_val = core
        .dispatch(
            OP_QUERY_DOCUMENT,
            serde_json::to_value(QueryDocumentRequest {
                page_id: page_resp.page_id.clone(),
                bounds: None,
            })
            .unwrap(),
        )
        .expect("must query document");
    let doc: DocumentSnapshot = serde_json::from_value(doc_val).unwrap();
    assert_eq!(doc.metadata.document_revision, DocumentRevision::new(1));

    // 5. Perform click action with valid element reference
    let click_req = ClickActionRequest {
        element_ref: ElementRef::new(
            page_resp.page_id.clone(),
            DocumentRevision::new(1),
            "btn-submit",
        ),
    };
    let act_val = core
        .dispatch(OP_CLICK, serde_json::to_value(click_req).unwrap())
        .expect("click must succeed");
    assert!(act_val.get("status").is_some() || act_val.get("document_revision").is_some());

    // 6. Observe revision bump
    let obs_after_val = core
        .dispatch(
            OP_OBSERVE,
            serde_json::to_value(ObservePageRequest {
                page_id: page_resp.page_id.clone(),
            })
            .unwrap(),
        )
        .expect("must observe");
    let obs_after: PageObservation = serde_json::from_value(obs_after_val).unwrap();
    assert_eq!(obs_after.document_revision, DocumentRevision::new(2));

    // 7. Stale element reference is rejected
    let stale_click = ClickActionRequest {
        element_ref: ElementRef::new(
            page_resp.page_id.clone(),
            DocumentRevision::new(1),
            "btn-submit",
        ),
    };
    let err = core
        .dispatch(OP_CLICK, serde_json::to_value(stale_click).unwrap())
        .expect_err("stale click must fail");
    assert!(err.is_stale_element());
}

#[test]
fn provider_core_capture_and_cookies_and_storage() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    let ctx_req = CreateContextRequest {
        profile_id: None,
        incognito: false,
        user_agent: None,
    };
    let ctx_resp: CreateContextResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(ctx_req).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let page_req = CreatePageRequest {
        context_id: ctx_resp.context_id.clone(),
        initial_url: None,
    };
    let page_resp: CreatePageResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(page_req).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    // Capture
    let cap_req = CapturePageRequest {
        page_id: page_resp.page_id.clone(),
        target: CaptureTarget::PageViewport,
        format: CaptureFormat::Png,
        quality: None,
        max_bytes: None,
    };
    let cap_val = core
        .dispatch(OP_CAPTURE, serde_json::to_value(cap_req).unwrap())
        .expect("capture must succeed");
    let cap_resp: worldline_browser_contract::capture::CapturePageResponse =
        serde_json::from_value(cap_val).unwrap();
    assert!(cap_resp.artifact.byte_len > 0);
    assert!(cap_resp.artifact.blob_id.starts_with("sha256:"));

    // Read capture
    let read_req = ReadCaptureArtifactRequest {
        artifact_id: cap_resp.artifact.artifact_id,
        offset: 0,
        max_bytes: 1024,
    };
    let read_val = core
        .dispatch(OP_READ_CAPTURE, serde_json::to_value(read_req).unwrap())
        .expect("read capture must succeed");
    let read_resp: worldline_browser_contract::capture::ReadCaptureArtifactResponse =
        serde_json::from_value(read_val).unwrap();
    assert_eq!(read_resp.data.len(), cap_resp.artifact.byte_len);

    // Cookies
    let set_cookie = SetCookieRequest {
        context_id: ctx_resp.context_id.clone(),
        cookie: Cookie {
            name: "auth_token".to_string(),
            value: "secret123".to_string(),
            domain: "worldline.test".to_string(),
            path: "/".to_string(),
            secure: true,
            http_only: true,
            same_site: None,
            expires_epoch_sec: None,
        },
    };
    core.dispatch(OP_COOKIE_SET, serde_json::to_value(set_cookie).unwrap())
        .expect("set cookie must succeed");

    let get_cookies = GetCookiesRequest {
        context_id: ctx_resp.context_id.clone(),
        url: None,
        domain: None,
    };
    let cookies_val = core
        .dispatch(OP_COOKIE_GET, serde_json::to_value(get_cookies).unwrap())
        .expect("get cookies must succeed");
    let cookies_resp: worldline_browser_contract::primitives::GetCookiesResponse =
        serde_json::from_value(cookies_val).unwrap();
    assert_eq!(cookies_resp.cookies.len(), 1);
    assert_eq!(cookies_resp.cookies[0].name, "auth_token");

    // Clear storage
    let clear_storage = ClearStorageRequest {
        context_id: ctx_resp.context_id.clone(),
        origin: "https://worldline.test".to_string(),
        storage_type: StorageType::LocalStorage,
    };
    core.dispatch(
        OP_STORAGE_CLEAR,
        serde_json::to_value(clear_storage).unwrap(),
    )
    .expect("clear storage must succeed");
}
