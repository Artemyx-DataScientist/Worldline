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
use worldline_plugin_protocol::MessageKind;

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

    let (connection, ack) = NativeProviderConnection::connect(
        provider_process_spec(),
        &identity,
        Arc::new(DummySink),
        16,
    )
    .expect("must connect to browser provider process");

    assert_eq!(ack.package_id, "worldline.browser.pkg");
    assert_eq!(ack.plugin_definition_id, "worldline.browser.provider");

    // 1. Create context
    let ctx_req = CreateContextRequest {
        profile_id: Some("out-of-process-profile".to_string()),
        incognito: false,
        user_agent: None,
    };
    let ctx_val = call_op(
        &connection,
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
    let page_val = call_op(
        &connection,
        OP_CREATE_PAGE,
        serde_json::to_value(page_req).unwrap(),
    )
    .expect("create page must succeed");
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();

    // 3. Observe page
    let obs_req = ObservePageRequest {
        page_id: page_resp.page_id.clone(),
    };
    let obs_val = call_op(
        &connection,
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
    let doc_val = call_op(
        &connection,
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
    let act_val = call_op(
        &connection,
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
    let cap_val = call_op(
        &connection,
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
    let read_val = call_op(
        &connection,
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
    call_op(
        &connection,
        OP_COOKIE_SET,
        serde_json::to_value(set_cookie).unwrap(),
    )
    .expect("set cookie must succeed");

    let get_cookies = GetCookiesRequest {
        context_id: ctx_resp.context_id.clone(),
        url: None,
        domain: None,
    };
    let cookies_val = call_op(
        &connection,
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
    call_op(
        &connection,
        OP_STORAGE_CLEAR,
        serde_json::to_value(clear_storage).unwrap(),
    )
    .expect("clear storage must succeed");

    // 10. Clean shutdown
    connection
        .close(Duration::from_millis(500))
        .expect("orderly shutdown must complete");
}
