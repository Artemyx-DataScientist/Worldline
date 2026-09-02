use worldline_browser_contract::{
    action::InputActionRequest,
    authority::*,
    contracts::{
        CloseContextRequest, ClosePageRequest, CreateContextRequest, CreateContextResponse,
        CreatePageRequest, CreatePageResponse, ListContextsResponse, NavigateRequest,
        NavigateResponse, ObservePageRequest, ReloadRequest, ReloadResponse,
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
fn provider_max_contexts_budget_enforcement() {
    let backend = ReferenceBrowserBackend::new();
    let limits = ProviderBudgetLimits {
        max_contexts: 2,
        max_pages_per_context: 2,
        max_action_text_len: 1024,
    };
    let core = BrowserProviderCore::with_limits(backend, limits);

    let create_ctx = || CreateContextRequest {
        profile_id: None,
        incognito: false,
        user_agent: None,
    };

    // First two contexts succeed
    let ctx1_val = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(create_ctx()).unwrap(),
        )
        .expect("context 1 must succeed");
    let ctx1: CreateContextResponse = serde_json::from_value(ctx1_val).unwrap();

    let ctx2_val = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(create_ctx()).unwrap(),
        )
        .expect("context 2 must succeed");
    let ctx2: CreateContextResponse = serde_json::from_value(ctx2_val).unwrap();

    // Third context creation fails with explicit limit error before excess engine creation
    let ctx3_err = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(create_ctx()).unwrap(),
        )
        .expect_err("context 3 must be rejected on saturation");
    assert!(matches!(ctx3_err, BrowserError::InvalidRequest(_)));
    assert!(ctx3_err.to_string().contains("context limit"));

    // list_contexts reports exactly 2 created contexts
    let list_val = core
        .dispatch_contract("browser.context", OP_LIST_CONTEXTS, serde_json::json!({}))
        .expect("list_contexts must succeed");
    let list: ListContextsResponse = serde_json::from_value(list_val).unwrap();
    assert_eq!(list.contexts.len(), 2);
    assert!(list.contexts.contains(&ctx1.context_id));
    assert!(list.contexts.contains(&ctx2.context_id));

    // Closing context 1 releases budget occupancy
    core.dispatch_contract(
        "browser.context",
        OP_CLOSE_CONTEXT,
        serde_json::to_value(CloseContextRequest {
            context_id: ctx1.context_id.clone(),
        })
        .unwrap(),
    )
    .expect("close context 1 must succeed");

    // Replacement context can now be created
    let ctx3_val = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(create_ctx()).unwrap(),
        )
        .expect("context 3 must succeed after slot released");
    let ctx3: CreateContextResponse = serde_json::from_value(ctx3_val).unwrap();

    let list_after_val = core
        .dispatch_contract("browser.context", OP_LIST_CONTEXTS, serde_json::json!({}))
        .expect("list_contexts must succeed");
    let list_after: ListContextsResponse = serde_json::from_value(list_after_val).unwrap();
    assert_eq!(list_after.contexts.len(), 2);
    assert!(list_after.contexts.contains(&ctx2.context_id));
    assert!(list_after.contexts.contains(&ctx3.context_id));
}

#[test]
fn provider_max_pages_per_context_budget_enforcement() {
    let backend = ReferenceBrowserBackend::new();
    let limits = ProviderBudgetLimits {
        max_contexts: 4,
        max_pages_per_context: 2,
        max_action_text_len: 1024,
    };
    let core = BrowserProviderCore::with_limits(backend, limits);

    let create_ctx = || CreateContextRequest {
        profile_id: None,
        incognito: false,
        user_agent: None,
    };

    let ctx_a: CreateContextResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(create_ctx()).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let ctx_b: CreateContextResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(create_ctx()).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    // Context A: first two pages succeed
    let page_a1: CreatePageResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_a.context_id.clone(),
                initial_url: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let _page_a2: CreatePageResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_a.context_id.clone(),
                initial_url: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    // Context A: third page fails explicitly
    let page_a3_err = core
        .dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_a.context_id.clone(),
                initial_url: None,
            })
            .unwrap(),
        )
        .expect_err("page 3 in context A must fail on saturation");
    assert!(matches!(page_a3_err, BrowserError::InvalidRequest(_)));
    assert!(page_a3_err.to_string().contains("page limit"));

    // Context B retains its own independent page budget: 2 pages succeed
    let _page_b1: CreatePageResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_b.context_id.clone(),
                initial_url: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    let _page_b2: CreatePageResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_b.context_id.clone(),
                initial_url: None,
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    // Closing page A1 releases budget occupancy in context A
    core.dispatch_contract(
        "browser.page",
        OP_CLOSE_PAGE,
        serde_json::to_value(ClosePageRequest {
            page_id: page_a1.page_id,
        })
        .unwrap(),
    )
    .expect("close page A1 must succeed");

    // Context A can now create a new replacement page
    let _page_a4: CreatePageResponse = serde_json::from_value(
        core.dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_a.context_id.clone(),
                initial_url: None,
            })
            .unwrap(),
        )
        .expect("page in context A must succeed after slot released"),
    )
    .unwrap();
}

#[test]
fn provider_budget_limits_zero_value_deny_all() {
    let backend = ReferenceBrowserBackend::new();
    let limits = ProviderBudgetLimits {
        max_contexts: 0,
        max_pages_per_context: 0,
        max_action_text_len: 1024,
    };
    let core = BrowserProviderCore::with_limits(backend, limits);

    let err = core
        .dispatch_contract(
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: None,
                incognito: false,
                user_agent: None,
            })
            .unwrap(),
        )
        .expect_err("zero limit must deny-all context creation");
    assert!(matches!(err, BrowserError::InvalidRequest(_)));
    assert!(err.to_string().contains("context limit of 0 exceeded"));
}

#[test]
fn provider_budget_limits_action_text_len_enforcement() {
    let backend = ReferenceBrowserBackend::new();
    let limits = ProviderBudgetLimits {
        max_contexts: 2,
        max_pages_per_context: 2,
        max_action_text_len: 16,
    };
    let core = BrowserProviderCore::with_limits(backend, limits);

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
        core.dispatch_contract(
            "browser.act",
            OP_INPUT,
            serde_json::to_value(input_short).unwrap()
        )
        .is_ok()
    );

    // 2. Text input exceeding 16 byte limit is rejected with InvalidRequest
    let input_huge = InputActionRequest {
        element_ref: ElementRef::new(page.page_id, DocumentRevision::new(2), "input-query"),
        text: "this text is way longer than sixteen bytes".to_string(),
        clear_first: false,
    };
    let huge_err = core
        .dispatch_contract(
            "browser.act",
            OP_INPUT,
            serde_json::to_value(input_huge).unwrap()
        )
        .expect_err("huge input text must be rejected");
    assert!(matches!(huge_err, BrowserError::InvalidRequest(_)));
}

#[test]
fn provider_ambiguous_bare_operations_rejected() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    // Bare "create" must fail closed as ambiguous without inspecting payload
    let err_create = core
        .dispatch("create", serde_json::json!({"context_id": "ctx-1"}))
        .expect_err("bare create must fail closed");
    assert!(matches!(err_create, BrowserError::InvalidRequest(_)));
    assert!(err_create.to_string().contains("ambiguous bare operation"));

    // Bare "close" must fail closed as ambiguous
    let err_close = core
        .dispatch("close", serde_json::json!({"page_id": "page-1"}))
        .expect_err("bare close must fail closed");
    assert!(matches!(err_close, BrowserError::InvalidRequest(_)));
    assert!(err_close.to_string().contains("ambiguous bare operation"));

    // Bare "list" must fail closed as ambiguous
    let err_list = core
        .dispatch("list", serde_json::json!({"context_id": "ctx-1"}))
        .expect_err("bare list must fail closed");
    assert!(matches!(err_list, BrowserError::InvalidRequest(_)));
    assert!(err_list.to_string().contains("ambiguous bare operation"));
}

