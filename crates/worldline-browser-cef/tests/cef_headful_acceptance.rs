use worldline_browser_cef::{CefBrowserBackend, CefLoopRunner, early_subprocess_dispatch};
use worldline_browser_contract::{
    authority::OP_CREATE_CONTEXT, contracts::CreateContextRequest, error::BrowserError,
};
use worldline_browser_provider::BrowserProviderCore;

#[test]
fn early_subprocess_dispatch_returns_none_for_main_process() {
    let result = early_subprocess_dispatch(0);
    assert!(
        result.is_none(),
        "Main process should not be treated as a CEF subprocess"
    );
}

#[test]
fn cef_loop_runner_requires_bootstrap_sandbox() {
    let error = match CefLoopRunner::spawn() {
        Ok(_) => panic!("direct CEF initialization must not bypass the bootstrap sandbox"),
        Err(error) => error,
    };
    assert!(
        error.contains("sandbox"),
        "unexpected initialization error: {error}"
    );
}

#[test]
fn cef_backend_rejects_direct_initialization_without_bootstrap_sandbox() {
    let backend = CefBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    let result = core.dispatch(
        OP_CREATE_CONTEXT,
        serde_json::to_value(CreateContextRequest {
            profile_id: Some("cef-direct-without-bootstrap-profile".to_string()),
            incognito: false,
            user_agent: None,
        })
        .unwrap(),
    );
    let error = result.expect_err(
        "a direct CEF caller without the bootstrap-owned sandbox context must fail closed",
    );
    assert!(
        matches!(error, BrowserError::EngineCrashed(_)),
        "direct CEF initialization must report an engine failure, not select a reference backend: {error:?}"
    );
}
