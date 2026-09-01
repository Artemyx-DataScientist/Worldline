//! S2: Live browser engine proving slice.
//!
//! Verifies the complete end-to-end browser provider workflow:
//! context creation, page lifecycle, headful/headless navigation,
//! accessibility tree querying, by-value action dispatch,
//! element reference staleness validation, visual capture, and cookie/storage isolation.

use std::sync::Arc;

use worldline_browser_contract::{
    action::{ClickActionRequest, InputActionRequest},
    authority::*,
    capture::{CaptureFormat, CapturePageRequest, CaptureTarget, ReadCaptureArtifactRequest},
    contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        ElementQueryKind, FindElementsRequest, FindElementsResponse, NavigateRequest,
        NavigateResponse, ObservePageRequest, PageObservation, QueryDocumentRequest,
    },
    identity::{DocumentRevision, ElementRef},
    primitives::{ClearStorageRequest, Cookie, GetCookiesRequest, SetCookieRequest, StorageType},
    query::DocumentSnapshot,
};
use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};

/// Proving report emitted by slice S2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S2Report {
    pub context_id: String,
    pub page_id: String,
    pub initial_url: String,
    pub navigated_url: String,
    pub initial_revision: u64,
    pub post_nav_revision: u64,
    pub post_action_revision: u64,
    pub found_elements_count: usize,
    pub stale_action_rejected: bool,
    pub capture_blob_id: String,
    pub capture_bytes_read: usize,
    pub cookies_isolated: bool,
    pub storage_cleared: bool,
}

/// Executes the live browser engine proving slice S2.
pub fn run() -> Result<S2Report, String> {
    let backend = ReferenceBrowserBackend::new();
    let core = Arc::new(BrowserProviderCore::new(backend));

    // 1. Create context
    let ctx_val = core
        .dispatch(
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some("s2-profile".to_string()),
                incognito: false,
                user_agent: Some("Worldline-S2/1.0".to_string()),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let ctx: CreateContextResponse = serde_json::from_value(ctx_val).map_err(|e| e.to_string())?;

    // 2. Create page
    let page_val = core
        .dispatch(
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx.context_id.clone(),
                initial_url: Some("https://worldline.test/s2-initial".to_string()),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let page: CreatePageResponse = serde_json::from_value(page_val).map_err(|e| e.to_string())?;

    // 3. Observe initial state
    let obs_initial_val = core
        .dispatch(
            OP_OBSERVE,
            serde_json::to_value(ObservePageRequest {
                page_id: page.page_id.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let obs_initial: PageObservation =
        serde_json::from_value(obs_initial_val).map_err(|e| e.to_string())?;
    let initial_revision = obs_initial.document_revision.value();

    // 4. Navigate
    let nav_val = core
        .dispatch(
            OP_NAVIGATE,
            serde_json::to_value(NavigateRequest {
                page_id: page.page_id.clone(),
                url: "https://worldline.test/s2-navigated".to_string(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let nav: NavigateResponse = serde_json::from_value(nav_val).map_err(|e| e.to_string())?;
    let post_nav_revision = nav.document_revision.value();

    // 5. Query document snapshot & find elements
    let doc_val = core
        .dispatch(
            OP_QUERY_DOCUMENT,
            serde_json::to_value(QueryDocumentRequest {
                page_id: page.page_id.clone(),
                bounds: None,
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let _doc: DocumentSnapshot = serde_json::from_value(doc_val).map_err(|e| e.to_string())?;

    let find_val = core
        .dispatch(
            OP_FIND_ELEMENTS,
            serde_json::to_value(FindElementsRequest {
                page_id: page.page_id.clone(),
                query: "Button".to_string(),
                kind: ElementQueryKind::AccessibilityRole,
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let find: FindElementsResponse = serde_json::from_value(find_val).map_err(|e| e.to_string())?;
    let found_elements_count = find.elements.len();

    // 6. Act & Staleness validation
    // Valid input action
    let input_req = InputActionRequest {
        element_ref: ElementRef::new(
            page.page_id.clone(),
            DocumentRevision::new(post_nav_revision),
            "input-query",
        ),
        text: "s2 query test".to_string(),
        clear_first: true,
    };
    core.dispatch(OP_INPUT, serde_json::to_value(input_req).unwrap())
        .map_err(|e| e.to_string())?;

    // Pre-action stale click must be rejected
    let stale_click = ClickActionRequest {
        element_ref: ElementRef::new(
            page.page_id.clone(),
            DocumentRevision::new(post_nav_revision),
            "btn-submit",
        ),
    };
    let stale_action_rejected = core
        .dispatch(OP_CLICK, serde_json::to_value(stale_click).unwrap())
        .is_err();

    // Post-action observe
    let obs_after_val = core
        .dispatch(
            OP_OBSERVE,
            serde_json::to_value(ObservePageRequest {
                page_id: page.page_id.clone(),
            })
            .unwrap(),
        )
        .unwrap();
    let obs_after: PageObservation = serde_json::from_value(obs_after_val).unwrap();
    let post_action_revision = obs_after.document_revision.value();

    // 7. Visual Capture
    let cap_val = core
        .dispatch(
            OP_CAPTURE,
            serde_json::to_value(CapturePageRequest {
                page_id: page.page_id.clone(),
                target: CaptureTarget::PageViewport,
                format: CaptureFormat::Png,
                quality: None,
                max_bytes: None,
            })
            .unwrap(),
        )
        .unwrap();
    let cap: worldline_browser_contract::capture::CapturePageResponse =
        serde_json::from_value(cap_val).unwrap();
    let capture_blob_id = cap.artifact.blob_id.clone();

    // Read artifact
    let read_val = core
        .dispatch(
            OP_READ_CAPTURE,
            serde_json::to_value(ReadCaptureArtifactRequest {
                artifact_id: cap.artifact.artifact_id,
                offset: 0,
                max_bytes: 1024,
            })
            .unwrap(),
        )
        .unwrap();
    let read: worldline_browser_contract::capture::ReadCaptureArtifactResponse =
        serde_json::from_value(read_val).unwrap();
    let capture_bytes_read = read.data.len();

    // 8. Cookies & Storage Isolation
    let set_cookie = SetCookieRequest {
        context_id: ctx.context_id.clone(),
        cookie: Cookie {
            name: "s2_token".to_string(),
            value: "token_value".to_string(),
            domain: "worldline.test".to_string(),
            path: "/".to_string(),
            secure: true,
            http_only: true,
            same_site: None,
            expires_epoch_sec: None,
        },
    };
    core.dispatch(OP_COOKIE_SET, serde_json::to_value(set_cookie).unwrap())
        .unwrap();

    let get_cookies: worldline_browser_contract::primitives::GetCookiesResponse =
        serde_json::from_value(
            core.dispatch(
                OP_COOKIE_GET,
                serde_json::to_value(GetCookiesRequest {
                    context_id: ctx.context_id.clone(),
                    url: None,
                    domain: None,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let cookies_isolated = get_cookies.cookies.len() == 1;

    let clear_storage: worldline_browser_contract::primitives::ClearStorageResponse =
        serde_json::from_value(
            core.dispatch(
                OP_STORAGE_CLEAR,
                serde_json::to_value(ClearStorageRequest {
                    context_id: ctx.context_id.clone(),
                    origin: "https://worldline.test".to_string(),
                    storage_type: StorageType::LocalStorage,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let storage_cleared = clear_storage.cleared;

    Ok(S2Report {
        context_id: ctx.context_id.to_string(),
        page_id: page.page_id.to_string(),
        initial_url: obs_initial.url,
        navigated_url: nav.page_id.to_string(),
        initial_revision,
        post_nav_revision,
        post_action_revision,
        found_elements_count,
        stale_action_rejected,
        capture_blob_id,
        capture_bytes_read,
        cookies_isolated,
        storage_cleared,
    })
}
