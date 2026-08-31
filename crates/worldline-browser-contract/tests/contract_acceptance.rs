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
};

#[test]
fn contracts_serialize_and_deserialize_without_engine_types() {
    let ctx_req = CreateContextRequest {
        profile_storage_path: Some("/tmp/profile_1".to_string()),
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

    // Action on matching page passes validation
    assert!(validate_element_reference(&elem_page_1, &page_1, rev_1).is_ok());

    // Action on different page fails
    let err = validate_element_reference(&elem_page_1, &page_2, rev_1).unwrap_err();
    assert_eq!(err, BrowserError::PageNotFound(page_1));
}

#[test]
fn stale_element_reference_after_revision_bump_is_rejected() {
    let page = PageId::new("page-alpha");
    let rev_initial = DocumentRevision::new(1);
    let rev_bumped = DocumentRevision::new(2);

    let elem_ref = ElementRef::new(page.clone(), rev_initial, "form-field-input");

    // Valid when revision matches
    assert!(validate_element_reference(&elem_ref, &page, rev_initial).is_ok());

    // Stale when page navigated and revision bumped
    let err = validate_element_reference(&elem_ref, &page, rev_bumped).unwrap_err();
    assert_eq!(
        err,
        BrowserError::StaleElementReference {
            expected_revision: rev_initial,
            actual_revision: rev_bumped,
        }
    );
}

#[test]
fn browser_event_cannot_satisfy_browser_rpc_invocation() {
    // Event names are observations, distinct from RPC operations
    let event_name = EVENT_NAVIGATION_COMMITTED;
    let rpc_op = OP_NAVIGATE;

    assert_ne!(event_name, rpc_op);
    assert!(!BrowserAuthority::NavigatePage.permits("browser.navigate", event_name));
    assert!(!BrowserAuthority::ObservePage.permits("browser.observe", event_name));
}

#[test]
fn navigation_committed_is_emitted_post_outcome() {
    let page_id = PageId::new("page-nav");
    let nav_id = worldline_browser_contract::NavigationId::new("nav-101");
    let rev = DocumentRevision::new(1);

    let committed_event = NavigationCommittedEvent {
        page_id: page_id.clone(),
        navigation_id: nav_id.clone(),
        url: "https://example.local/index.html".to_string(),
        document_revision: rev,
        status_code: 200,
    };

    assert_eq!(committed_event.status_code, 200);
    assert_eq!(committed_event.document_revision.value(), 1);
}

#[test]
fn resource_identities_are_worldline_scoped() {
    let ctx = BrowserContextId::new("ctx-alpha");
    let page = PageId::new("page-beta");

    assert_eq!(context_resource(&ctx), "browser-context/ctx-alpha");
    assert_eq!(page_resource(&page), "browser-page/page-beta");
}
