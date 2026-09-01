use worldline_browser_cef::{CefBrowserBackend, CefLoopRunner, early_subprocess_dispatch};
use worldline_browser_contract::{
    authority::*,
    contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        NavigateRequest, NavigateResponse, ObservePageRequest, PageObservation,
    },
    identity::DocumentRevision,
};
use worldline_browser_provider::BrowserProviderCore;

#[test]
fn early_subprocess_dispatch_returns_none_for_main_process() {
    let result = early_subprocess_dispatch();
    assert!(
        result.is_none(),
        "Main process should not be treated as a CEF subprocess"
    );
}

#[test]
fn cef_loop_runner_spawns_and_dispatches_sync() {
    let runner = CefLoopRunner::spawn().expect("must spawn UI loop runner");
    let result = runner
        .dispatch_sync(|| 42 * 2)
        .expect("sync dispatch must succeed on UI thread");
    assert_eq!(result, 84);
}

#[test]
fn cef_backend_headful_navigation_lifecycle() {
    let backend = CefBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    // Create Context
    let ctx_req = CreateContextRequest {
        profile_id: Some("cef-headful-profile".to_string()),
        incognito: false,
        user_agent: None,
    };
    let ctx_val = core
        .dispatch(OP_CREATE_CONTEXT, serde_json::to_value(ctx_req).unwrap())
        .expect("create context must succeed");
    let ctx_resp: CreateContextResponse = serde_json::from_value(ctx_val).unwrap();

    // Create Page
    let page_req = CreatePageRequest {
        context_id: ctx_resp.context_id.clone(),
        initial_url: Some("https://worldline.test/cef-start".to_string()),
    };
    let page_val = core
        .dispatch(OP_CREATE_PAGE, serde_json::to_value(page_req).unwrap())
        .expect("create page must succeed");
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();

    // Navigate to new URL
    let nav_req = NavigateRequest {
        page_id: page_resp.page_id.clone(),
        url: "https://worldline.test/cef-page2".to_string(),
    };
    let nav_val = core
        .dispatch(OP_NAVIGATE, serde_json::to_value(nav_req).unwrap())
        .expect("navigate must succeed");
    let nav_resp: NavigateResponse = serde_json::from_value(nav_val).unwrap();
    assert!(nav_resp.committed);
    assert_eq!(nav_resp.document_revision, DocumentRevision::new(2));

    // Observe
    let obs_req = ObservePageRequest {
        page_id: page_resp.page_id.clone(),
    };
    let obs_val = core
        .dispatch(OP_OBSERVE, serde_json::to_value(obs_req).unwrap())
        .expect("observe must succeed");
    let obs: PageObservation = serde_json::from_value(obs_val).unwrap();
    assert_eq!(obs.url, "https://worldline.test/cef-page2");
    assert_eq!(obs.document_revision, DocumentRevision::new(2));
}
