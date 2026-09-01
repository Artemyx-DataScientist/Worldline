use worldline_browser_contract::{
    action::InputActionRequest,
    authority::*,
    contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        NavigateRequest, NavigateResponse, ObservePageRequest, ReloadRequest, ReloadResponse,
    },
    error::BrowserError,
    identity::{DocumentRevision, ElementRef},
};
use worldline_browser_provider::{
    BrowserBackend, BrowserProviderCore, ProviderBudgetLimits, ReferenceBrowserBackend,
};

#[test]
fn renderer_crash_detection_and_reload_recovery() {
    let mut backend = ReferenceBrowserBackend::new();

    let ctx_resp: CreateContextResponse = serde_json::from_value(
        backend
            .create_context(&CreateContextRequest {
                profile_id: None,
                incognito: false,
                user_agent: None,
            })
            .map(|r| serde_json::to_value(r).unwrap())
            .unwrap(),
    )
    .unwrap();

    let page_resp: CreatePageResponse = serde_json::from_value(
        backend
            .create_page(&CreatePageRequest {
                context_id: ctx_resp.context_id,
                initial_url: Some("https://worldline.test/crash-test".to_string()),
            })
            .map(|r| serde_json::to_value(r).unwrap())
            .unwrap(),
    )
    .unwrap();

    let page_id = page_resp.page_id;

    // 1. Observe healthy page
    let obs = backend
        .observe(&ObservePageRequest {
            page_id: page_id.clone(),
        })
        .expect("must observe before crash");
    assert_eq!(obs.document_revision, DocumentRevision::new(1));

    // 2. Simulate renderer crash
    backend
        .simulate_renderer_crash(&page_id)
        .expect("simulate crash must succeed");

    // 3. Observe should fail while page is crashed
    let crash_err = backend
        .observe(&ObservePageRequest {
            page_id: page_id.clone(),
        })
        .expect_err("observe on crashed page must fail");
    assert!(matches!(crash_err, BrowserError::NavigationFailed(_)));

    // 4. Reload page to recover from crash
    let reload_resp: ReloadResponse = backend
        .reload(&ReloadRequest {
            page_id: page_id.clone(),
            ignore_cache: true,
        })
        .expect("reload on crashed page must succeed and recover");
    assert!(reload_resp.reloaded);
    assert_eq!(reload_resp.document_revision, DocumentRevision::new(2));

    // 5. Post-recovery observe succeeds
    let post_obs = backend
        .observe(&ObservePageRequest {
            page_id: page_id.clone(),
        })
        .expect("observe after recovery must succeed");
    assert_eq!(post_obs.document_revision, DocumentRevision::new(2));

    // 6. Navigation also recovers crashed page
    backend
        .simulate_renderer_crash(&page_id)
        .expect("simulate crash 2");
    let nav_resp: NavigateResponse = backend
        .navigate(&NavigateRequest {
            page_id: page_id.clone(),
            url: "https://worldline.test/recovered".to_string(),
        })
        .expect("navigate must recover crashed page");
    assert!(nav_resp.committed);
    assert_eq!(nav_resp.document_revision, DocumentRevision::new(3));
}

#[test]
fn provider_budget_limits_enforcement() {
    let backend = ReferenceBrowserBackend::new();
    let limits = ProviderBudgetLimits {
        max_contexts: 2,
        max_pages_per_context: 2,
        max_action_text_len: 16,
    };
    let core = BrowserProviderCore::with_limits(backend, limits);

    let ctx: CreateContextResponse = serde_json::from_value(
        core.dispatch(
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
        core.dispatch(
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx.context_id,
                initial_url: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    // 1. Text input within limit succeeds
    let input_short = InputActionRequest {
        element_ref: ElementRef::new(
            page.page_id.clone(),
            DocumentRevision::new(1),
            "input-query",
        ),
        text: "short text".to_string(),
        clear_first: false,
    };
    assert!(
        core.dispatch(OP_INPUT, serde_json::to_value(input_short).unwrap())
            .is_ok()
    );

    // 2. Text input exceeding 16 byte limit is rejected with InvalidRequest
    let input_huge = InputActionRequest {
        element_ref: ElementRef::new(page.page_id, DocumentRevision::new(2), "input-query"),
        text: "this text is way longer than sixteen bytes".to_string(),
        clear_first: false,
    };
    let huge_err = core
        .dispatch(OP_INPUT, serde_json::to_value(input_huge).unwrap())
        .expect_err("huge input text must be rejected");
    assert!(matches!(huge_err, BrowserError::InvalidRequest(_)));
}
