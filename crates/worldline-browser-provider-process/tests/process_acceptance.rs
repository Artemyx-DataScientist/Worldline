use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
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
use worldline_native_host::{
    ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError, NativeProviderConnection,
};
use worldline_plugin_protocol::{MessageKind, REQUEST_POLICY_INTERFACE};

struct DummySink;

impl HostRequestSink for DummySink {
    fn on_child_request(
        &self,
        _kind: MessageKind,
        _correlation_id: u64,
        _payload: Value,
    ) -> Result<Option<Value>, NativeHostError> {
        Ok(None)
    }
}

fn provider_process_spec() -> NativeChildSpec {
    NativeChildSpec {
        program: PathBuf::from(env!("CARGO_BIN_EXE_worldline-browser-provider-process")),
        args: vec![
            "--package-id".to_string(),
            "worldline.browser.pkg".to_string(),
            "--definition-id".to_string(),
            "worldline.browser.provider".to_string(),
        ],
        max_frame_bytes: 4 * 1024 * 1024,
        stderr_max_bytes: 64 * 1024,
        enable_process_tree_containment: true,
    }
}

fn call_contract_op(
    connection: &NativeProviderConnection,
    contract: &str,
    operation: &str,
    payload: Value,
) -> Result<Value, String> {
    let call_payload = serde_json::json!({
        "contract": contract,
        "operation": operation,
        "payload": payload
    });
    let val = connection
        .call(call_payload)
        .map_err(|e| format!("IPC error: {e}"))?;
    if let Some(err) = val.get("error").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    val.get("result")
        .cloned()
        .ok_or_else(|| "Missing result field in response".to_string())
}

fn call_op(
    connection: &NativeProviderConnection,
    operation: &str,
    payload: Value,
) -> Result<Value, String> {
    let call_payload = serde_json::json!({
        "operation": operation,
        "payload": payload
    });
    let val = connection
        .call(call_payload)
        .map_err(|e| format!("IPC error: {e}"))?;
    if let Some(err) = val.get("error").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    val.get("result")
        .cloned()
        .ok_or_else(|| "Missing result field in response".to_string())
}

#[test]
fn out_of_process_browser_provider_ipc_full_lifecycle() {
    let identity = ExpectedIdentity {
        package_id: "worldline.browser.pkg".to_string(),
        plugin_definition_id: "worldline.browser.provider".to_string(),
    };

    let (connection, ack) = NativeProviderConnection::connect_with_required_interface(
        provider_process_spec(),
        &identity,
        Arc::new(DummySink),
        16,
        REQUEST_POLICY_INTERFACE,
    )
    .expect("must connect to browser provider process");

    assert_eq!(ack.package_id, "worldline.browser.pkg");
    assert_eq!(ack.plugin_definition_id, "worldline.browser.provider");
    assert!(ack.supports_interface(REQUEST_POLICY_INTERFACE));

    // 1. Create context
    let ctx_req = CreateContextRequest {
        profile_id: Some("out-of-process-profile".to_string()),
        incognito: false,
        user_agent: None,
    };
    let ctx_val = call_contract_op(
        &connection,
        "browser.context",
        OP_CREATE_CONTEXT,
        serde_json::to_value(ctx_req).unwrap(),
    )
    .expect("create context must succeed");
    let ctx_resp: CreateContextResponse = serde_json::from_value(ctx_val).unwrap();

    // 2. Create page
    let page_req = CreatePageRequest {
        context_id: ctx_resp.context_id.clone(),
        initial_url: Some("https://worldline.test/oop".to_string()),
    };
    let page_val = call_contract_op(
        &connection,
        "browser.page",
        OP_CREATE_PAGE,
        serde_json::to_value(page_req).unwrap(),
    )
    .expect("create page must succeed");
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();

    // 3. Observe page
    let obs_req = ObservePageRequest {
        page_id: page_resp.page_id.clone(),
    };
    let obs_val = call_contract_op(
        &connection,
        "browser.observe",
        OP_OBSERVE,
        serde_json::to_value(obs_req).unwrap(),
    )
    .expect("observe must succeed");
    let obs: PageObservation = serde_json::from_value(obs_val).unwrap();
    assert_eq!(obs.url, "https://worldline.test/oop");
    assert_eq!(obs.document_revision, DocumentRevision::new(1));

    // 4. Query accessibility
    let query_req = QueryDocumentRequest {
        page_id: page_resp.page_id.clone(),
        bounds: None,
    };
    let doc_val = call_contract_op(
        &connection,
        "browser.query",
        OP_QUERY_DOCUMENT,
        serde_json::to_value(query_req).unwrap(),
    )
    .expect("query document must succeed");
    let doc: DocumentSnapshot = serde_json::from_value(doc_val).unwrap();
    assert_eq!(doc.metadata.document_revision, DocumentRevision::new(1));

    // 5. Act (Click)
    let click_req = ClickActionRequest {
        element_ref: ElementRef::new(
            page_resp.page_id.clone(),
            DocumentRevision::new(1),
            "btn-submit",
        ),
    };
    let act_val = call_contract_op(
        &connection,
        "browser.act",
        OP_CLICK,
        serde_json::to_value(click_req).unwrap(),
    )
    .expect("click must succeed");
    assert!(act_val.get("document_revision").is_some() || act_val.get("success").is_some());

    // 6. Capture page
    let cap_req = CapturePageRequest {
        page_id: page_resp.page_id.clone(),
        target: CaptureTarget::PageViewport,
        format: CaptureFormat::Png,
        quality: None,
        max_bytes: None,
    };
    let cap_val = call_contract_op(
        &connection,
        "browser.capture",
        OP_CAPTURE,
        serde_json::to_value(cap_req).unwrap(),
    )
    .expect("capture must succeed");
    let cap_resp: worldline_browser_contract::capture::CapturePageResponse =
        serde_json::from_value(cap_val).unwrap();
    assert!(cap_resp.artifact.byte_len > 0);

    // 7. Read capture artifact
    let read_req = ReadCaptureArtifactRequest {
        artifact_id: cap_resp.artifact.artifact_id,
        offset: 0,
        max_bytes: 512,
    };
    let read_val = call_contract_op(
        &connection,
        "browser.capture",
        OP_READ_CAPTURE,
        serde_json::to_value(read_req).unwrap(),
    )
    .expect("read capture must succeed");
    let read_resp: worldline_browser_contract::capture::ReadCaptureArtifactResponse =
        serde_json::from_value(read_val).unwrap();
    assert_eq!(read_resp.data.len(), cap_resp.artifact.byte_len);

    // 8. Cookies
    let set_cookie = SetCookieRequest {
        context_id: ctx_resp.context_id.clone(),
        cookie: Cookie {
            name: "session".to_string(),
            value: "token-999".to_string(),
            domain: "worldline.test".to_string(),
            path: "/".to_string(),
            secure: true,
            http_only: true,
            same_site: None,
            expires_epoch_sec: None,
        },
    };
    call_contract_op(
        &connection,
        "browser.engine.cookies",
        OP_COOKIE_SET,
        serde_json::to_value(set_cookie).unwrap(),
    )
    .expect("set cookie must succeed");

    let get_cookies = GetCookiesRequest {
        context_id: ctx_resp.context_id.clone(),
        url: None,
        domain: None,
    };
    let cookies_val = call_contract_op(
        &connection,
        "browser.engine.cookies",
        OP_COOKIE_GET,
        serde_json::to_value(get_cookies).unwrap(),
    )
    .expect("get cookies must succeed");
    let cookies_resp: worldline_browser_contract::primitives::GetCookiesResponse =
        serde_json::from_value(cookies_val).unwrap();
    assert_eq!(cookies_resp.cookies.len(), 1);

    // 9. Storage
    let clear_storage = ClearStorageRequest {
        context_id: ctx_resp.context_id.clone(),
        origin: "https://worldline.test".to_string(),
        storage_type: StorageType::LocalStorage,
    };
    call_contract_op(
        &connection,
        "browser.engine.storage",
        OP_STORAGE_CLEAR,
        serde_json::to_value(clear_storage).unwrap(),
    )
    .expect("clear storage must succeed");

    // 10. Clean shutdown
    connection
        .close(Duration::from_millis(500))
        .expect("orderly shutdown must complete");
}

#[test]
fn out_of_process_browser_provider_bare_ambiguous_operations_rejected() {
    let identity = ExpectedIdentity {
        package_id: "worldline.browser.pkg".to_string(),
        plugin_definition_id: "worldline.browser.provider".to_string(),
    };

    let (connection, _ack) = NativeProviderConnection::connect_with_required_interface(
        provider_process_spec(),
        &identity,
        Arc::new(DummySink),
        16,
        REQUEST_POLICY_INTERFACE,
    )
    .expect("must connect to browser provider process");

    // Bare "create" must fail closed
    let err_create = call_op(&connection, "create", serde_json::json!({"context_id": "ctx-1"}))
        .expect_err("bare create must fail closed");
    assert!(err_create.contains("ambiguous bare operation"));

    // Bare "close" must fail closed
    let err_close = call_op(&connection, "close", serde_json::json!({"page_id": "page-1"}))
        .expect_err("bare close must fail closed");
    assert!(err_close.contains("ambiguous bare operation"));

    // Bare "list" must fail closed
    let err_list = call_op(&connection, "list", serde_json::json!({"context_id": "ctx-1"}))
        .expect_err("bare list must fail closed");
    assert!(err_list.contains("ambiguous bare operation"));

    connection
        .close(Duration::from_millis(500))
        .expect("shutdown must succeed");
}

#[test]
fn out_of_process_browser_provider_contract_mismatch_and_unknown_fail_closed() {
    let identity = ExpectedIdentity {
        package_id: "worldline.browser.pkg".to_string(),
        plugin_definition_id: "worldline.browser.provider".to_string(),
    };

    let (connection, _ack) = NativeProviderConnection::connect_with_required_interface(
        provider_process_spec(),
        &identity,
        Arc::new(DummySink),
        16,
        REQUEST_POLICY_INTERFACE,
    )
    .expect("must connect to browser provider process");

    // Mismatched contract/operation: browser.context with create_page
    let err_mismatch = call_contract_op(
        &connection,
        "browser.context",
        "create_page",
        serde_json::json!({"context_id": "c1"}),
    )
    .expect_err("mismatched contract/op must fail closed");
    assert!(err_mismatch.contains("unknown operation: browser.context/create_page"));

    // Unknown contract: unknown.contract with create
    let err_unknown = call_contract_op(
        &connection,
        "unknown.contract",
        "create",
        serde_json::json!({}),
    )
    .expect_err("unknown contract must fail closed");
    assert!(err_unknown.contains("unknown operation: browser.unknown.contract/create"));

    connection
        .close(Duration::from_millis(500))
        .expect("shutdown must succeed");
}

#[test]
fn out_of_process_browser_provider_payload_shape_does_not_drive_routing() {
    let identity = ExpectedIdentity {
        package_id: "worldline.browser.pkg".to_string(),
        plugin_definition_id: "worldline.browser.provider".to_string(),
    };

    let (connection, _ack) = NativeProviderConnection::connect_with_required_interface(
        provider_process_spec(),
        &identity,
        Arc::new(DummySink),
        16,
        REQUEST_POLICY_INTERFACE,
    )
    .expect("must connect to browser provider process");

    // 1. Context creation with a rogue "page_id" injected:
    // Because contract is explicitly "browser.context", it routes to context creation and ignores page_id
    let malicious_ctx_payload = serde_json::json!({
        "profile_id": "injected-profile",
        "incognito": false,
        "page_id": "malicious-injected-page-id",
        "context_id": "malicious-injected-context-id"
    });
    let ctx_val = call_contract_op(
        &connection,
        "browser.context",
        OP_CREATE_CONTEXT,
        malicious_ctx_payload,
    )
    .expect("context creation with rogue fields must succeed via explicit contract routing");
    let ctx_resp: CreateContextResponse = serde_json::from_value(ctx_val).unwrap();
    assert_eq!(ctx_resp.profile_id, Some("injected-profile".to_string()));

    // 2. Page creation with rogue context fields:
    // Routed strictly to "browser.page", succeeds as page creation
    let page_payload = serde_json::json!({
        "context_id": ctx_resp.context_id,
        "initial_url": "about:blank",
        "profile_id": "rogue-profile-id"
    });
    let page_val = call_contract_op(
        &connection,
        "browser.page",
        OP_CREATE_PAGE,
        page_payload,
    )
    .expect("page creation with rogue context fields must succeed via explicit page routing");
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();
    assert!(!page_resp.page_id.as_str().is_empty());

    // 3. Sending page creation payload to browser.context fails closed and never creates a page
    let invalid_ctx_req = serde_json::json!({
        "initial_url": "https://worldline.test/leak"
    });
    let ctx_err = call_contract_op(
        &connection,
        "browser.context",
        OP_CREATE_PAGE, // "create"
        invalid_ctx_req,
    )
    .expect_err("page payload to context contract must fail closed");
    assert!(ctx_err.contains("create_context payload invalid") || ctx_err.contains("unknown operation"));

    connection
        .close(Duration::from_millis(500))
        .expect("shutdown must succeed");
}
