use worldline_browser_contract::{
    action::{
        ClickActionRequest, FocusActionRequest, InputActionRequest, ScrollActionRequest,
        SubmitActionRequest,
    },
    authority::*,
    capture::{CaptureFormat, CapturePageRequest, CaptureTarget, ReadCaptureArtifactRequest},
    contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        ElementQueryKind, ExtractTextRequest, ExtractTextResponse, FindElementsRequest,
        FindElementsResponse, ObservePageRequest, PageObservation, QueryAccessibilityRequest,
        QueryDocumentRequest,
    },
    identity::{DocumentRevision, ElementRef, PageId},
    query::{AccessibilityTree, DocumentSnapshot, QueryBounds},
};
use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};

#[test]
fn semantic_query_projection_and_bounds() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    let ctx_val = core
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
        .unwrap();
    let ctx_resp: CreateContextResponse = serde_json::from_value(ctx_val).unwrap();

    let page_val = core
        .dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_resp.context_id,
                initial_url: Some("https://worldline.test/query".to_string()),
            })
            .unwrap(),
        )
        .unwrap();
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();

    // 1. Query full document snapshot
    let doc_val = core
        .dispatch(
            OP_QUERY_DOCUMENT,
            serde_json::to_value(QueryDocumentRequest {
                page_id: page_resp.page_id.clone(),
                bounds: None,
            })
            .unwrap(),
        )
        .unwrap();
    let doc: DocumentSnapshot = serde_json::from_value(doc_val).unwrap();
    assert_eq!(doc.metadata.document_revision, DocumentRevision::new(1));
    assert!(!doc.is_truncated);

    // 2. Query accessibility tree directly
    let ax_val = core
        .dispatch(
            OP_QUERY_ACCESSIBILITY,
            serde_json::to_value(QueryAccessibilityRequest {
                page_id: page_resp.page_id.clone(),
                bounds: None,
            })
            .unwrap(),
        )
        .unwrap();
    let tree: AccessibilityTree = serde_json::from_value(ax_val).unwrap();
    assert_eq!(tree.page_id, page_resp.page_id);
    assert_eq!(tree.total_node_count, 3); // root + button + text input

    // 3. Query with strict bounds causing truncation
    let strict_bounds = QueryBounds {
        max_depth: 1,
        max_nodes: 1,
        max_text_len: 5,
        max_total_text_bytes: 10,
    };
    let bounded_doc_val = core
        .dispatch(
            OP_QUERY_DOCUMENT,
            serde_json::to_value(QueryDocumentRequest {
                page_id: page_resp.page_id.clone(),
                bounds: Some(strict_bounds),
            })
            .unwrap(),
        )
        .unwrap();
    let bounded_doc: DocumentSnapshot = serde_json::from_value(bounded_doc_val).unwrap();
    assert!(bounded_doc.is_truncated);

    // 4. Find elements by query kind
    let find_val = core
        .dispatch(
            OP_FIND_ELEMENTS,
            serde_json::to_value(FindElementsRequest {
                page_id: page_resp.page_id.clone(),
                query: "Button".to_string(),
                kind: ElementQueryKind::AccessibilityRole,
            })
            .unwrap(),
        )
        .unwrap();
    let find_resp: FindElementsResponse = serde_json::from_value(find_val).unwrap();
    assert_eq!(find_resp.elements.len(), 1);
    assert_eq!(find_resp.elements[0].text_content, "Submit");

    // 5. Extract text
    let text_val = core
        .dispatch(
            OP_EXTRACT_TEXT,
            serde_json::to_value(ExtractTextRequest {
                page_id: page_resp.page_id.clone(),
                target_element: None,
            })
            .unwrap(),
        )
        .unwrap();
    let text_resp: ExtractTextResponse = serde_json::from_value(text_val).unwrap();
    assert!(text_resp.text.contains("Submit"));
    assert!(text_resp.text.contains("Search Query"));
}

#[test]
fn by_value_action_dispatch_and_element_ref_validation() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    let ctx_val = core
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
        .unwrap();
    let ctx_resp: CreateContextResponse = serde_json::from_value(ctx_val).unwrap();

    let page_val = core
        .dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_resp.context_id,
                initial_url: Some("https://worldline.test/actions".to_string()),
            })
            .unwrap(),
        )
        .unwrap();
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();

    let page_id = page_resp.page_id;
    let rev1 = DocumentRevision::new(1);

    // 1. Input text action
    let input_req = InputActionRequest {
        element_ref: ElementRef::new(page_id.clone(), rev1, "input-query"),
        text: "hello world".to_string(),
        clear_first: true,
    };
    let input_res = core.dispatch(OP_INPUT, serde_json::to_value(input_req).unwrap());
    assert!(input_res.is_ok());

    // Revision should now be 2
    let obs: PageObservation = serde_json::from_value(
        core.dispatch(
            OP_OBSERVE,
            serde_json::to_value(ObservePageRequest {
                page_id: page_id.clone(),
            })
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(obs.document_revision, DocumentRevision::new(2));

    // 2. Click action using outdated rev1 ElementRef must fail with StaleElementReference
    let stale_click = ClickActionRequest {
        element_ref: ElementRef::new(page_id.clone(), rev1, "btn-submit"),
    };
    let stale_err = core
        .dispatch(OP_CLICK, serde_json::to_value(stale_click).unwrap())
        .expect_err("stale ElementRef must be rejected");
    assert!(stale_err.is_stale_element());

    // 3. Click action with updated rev2 ElementRef must succeed
    let rev2 = DocumentRevision::new(2);
    let valid_click = ClickActionRequest {
        element_ref: ElementRef::new(page_id.clone(), rev2, "btn-submit"),
    };
    let click_res = core.dispatch(OP_CLICK, serde_json::to_value(valid_click).unwrap());
    assert!(click_res.is_ok());

    // 4. Focus and Submit actions
    let rev3 = DocumentRevision::new(3);
    let focus_req = FocusActionRequest {
        element_ref: ElementRef::new(page_id.clone(), rev3, "input-query"),
    };
    assert!(
        core.dispatch(OP_FOCUS, serde_json::to_value(focus_req).unwrap())
            .is_ok()
    );

    let rev4 = DocumentRevision::new(4);
    let submit_req = SubmitActionRequest {
        element_ref: ElementRef::new(page_id.clone(), rev4, "btn-submit"),
    };
    assert!(
        core.dispatch(OP_SUBMIT, serde_json::to_value(submit_req).unwrap())
            .is_ok()
    );

    // 5. Scroll action
    let scroll_req = ScrollActionRequest {
        page_id: page_id.clone(),
        delta_x: 0,
        delta_y: 500,
    };
    assert!(
        core.dispatch(OP_SCROLL, serde_json::to_value(scroll_req).unwrap())
            .is_ok()
    );

    // 6. Action targeting a mismatched page ID is rejected
    let foreign_page = PageId::new("page-foreign");
    let mismatch_click = ClickActionRequest {
        element_ref: ElementRef::new(foreign_page, DocumentRevision::new(5), "btn-submit"),
    };
    let mismatch_err = core
        .dispatch(OP_CLICK, serde_json::to_value(mismatch_click).unwrap())
        .expect_err("foreign page ElementRef must be rejected");
    assert!(matches!(
        mismatch_err,
        worldline_browser_contract::BrowserError::PageNotFound(_)
            | worldline_browser_contract::BrowserError::ResourceMismatch { .. }
    ));
}

#[test]
fn capture_page_artifacts_and_bounded_streaming() {
    let backend = ReferenceBrowserBackend::new();
    let core = BrowserProviderCore::new(backend);

    let ctx_val = core
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
        .unwrap();
    let ctx_resp: CreateContextResponse = serde_json::from_value(ctx_val).unwrap();

    let page_val = core
        .dispatch_contract(
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_resp.context_id,
                initial_url: Some("https://worldline.test/capture".to_string()),
            })
            .unwrap(),
        )
        .unwrap();
    let page_resp: CreatePageResponse = serde_json::from_value(page_val).unwrap();

    // 1. Capture PNG
    let cap_png_req = CapturePageRequest {
        page_id: page_resp.page_id.clone(),
        target: CaptureTarget::PageViewport,
        format: CaptureFormat::Png,
        quality: Some(95),
        max_bytes: Some(1024 * 1024),
    };
    let cap_val = core
        .dispatch(OP_CAPTURE, serde_json::to_value(cap_png_req).unwrap())
        .unwrap();
    let cap_resp: worldline_browser_contract::capture::CapturePageResponse =
        serde_json::from_value(cap_val).unwrap();

    assert_eq!(cap_resp.artifact.mime_type, "image/png");
    assert!(cap_resp.artifact.byte_len > 0);
    assert!(cap_resp.artifact.blob_id.starts_with("sha256:"));

    // 2. Stream artifact in chunks
    let read_chunk1: worldline_browser_contract::capture::ReadCaptureArtifactResponse =
        serde_json::from_value(
            core.dispatch(
                OP_READ_CAPTURE,
                serde_json::to_value(ReadCaptureArtifactRequest {
                    artifact_id: cap_resp.artifact.artifact_id.clone(),
                    offset: 0,
                    max_bytes: 4,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(read_chunk1.data.len(), 4);
    assert!(read_chunk1.is_truncated);

    let read_chunk2: worldline_browser_contract::capture::ReadCaptureArtifactResponse =
        serde_json::from_value(
            core.dispatch(
                OP_READ_CAPTURE,
                serde_json::to_value(ReadCaptureArtifactRequest {
                    artifact_id: cap_resp.artifact.artifact_id.clone(),
                    offset: 4,
                    max_bytes: 1024,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        read_chunk2.data.len(),
        cap_resp.artifact.byte_len.saturating_sub(4)
    );
    assert!(!read_chunk2.is_truncated);

    // 3. Capture JPEG & WEBP
    let cap_jpeg: worldline_browser_contract::capture::CapturePageResponse =
        serde_json::from_value(
            core.dispatch(
                OP_CAPTURE,
                serde_json::to_value(CapturePageRequest {
                    page_id: page_resp.page_id.clone(),
                    target: CaptureTarget::PageViewport,
                    format: CaptureFormat::Jpeg,
                    quality: None,
                    max_bytes: None,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(cap_jpeg.artifact.mime_type, "image/jpeg");

    let cap_webp: worldline_browser_contract::capture::CapturePageResponse =
        serde_json::from_value(
            core.dispatch(
                OP_CAPTURE,
                serde_json::to_value(CapturePageRequest {
                    page_id: page_resp.page_id,
                    target: CaptureTarget::PageViewport,
                    format: CaptureFormat::Webp,
                    quality: None,
                    max_bytes: None,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(cap_webp.artifact.mime_type, "image/webp");
}
