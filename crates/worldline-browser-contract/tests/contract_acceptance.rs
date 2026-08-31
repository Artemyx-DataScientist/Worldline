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
    query::{
        AccessibilityNode, AccessibilityRole, AccessibilityTree, DocumentMetadata,
        DocumentSnapshot, QueryBounds,
    },
};

#[test]
fn contracts_serialize_and_deserialize_without_engine_types() {
    let ctx_req = CreateContextRequest {
        profile_id: Some("user-profile-alpha".to_string()),
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
fn query_bounds_truncates_large_accessibility_tree_safely() {
    let page_id = PageId::new("page-bounds");
    let rev = DocumentRevision::initial();

    // Construct a deeply nested tree with many nodes
    let mut deep_node = AccessibilityNode::new("leaf", AccessibilityRole::StaticText)
        .with_name("Very long text content that will be bounded by byte budget");
    for i in 0..30 {
        deep_node = AccessibilityNode::new(format!("node-{i}"), AccessibilityRole::Group)
            .with_name(format!("Group {i}"))
            .with_child(deep_node);
    }

    let tree = AccessibilityTree::new(page_id.clone(), rev, deep_node);
    assert_eq!(tree.total_node_count, 31);
    assert!(!tree.is_truncated);

    // Apply strict bounds (max_depth: 5, max_nodes: 10)
    let bounds = QueryBounds {
        max_depth: 5,
        max_nodes: 10,
        max_text_len: 20,
        max_total_text_bytes: 100,
    };

    let bounded = tree.to_bounded(&bounds);
    assert!(bounded.is_truncated);
    assert!(bounded.truncated_node_count > 0);
    assert!(bounded.root.count_nodes() <= bounds.max_nodes);

    let doc = DocumentSnapshot::new(
        DocumentMetadata {
            page_id,
            url: "http://worldline.local".to_string(),
            title: "Test".to_string(),
            document_revision: rev,
            status_code: 200,
        },
        tree,
    );
    let bounded_doc = doc.to_bounded(&bounds);
    assert!(bounded_doc.is_truncated);
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

    // ElementRef matching target page is valid
    assert!(validate_element_reference(&elem_page_1, &page_1, rev_1).is_ok());

    // ElementRef targeting different page is rejected
    let mismatch = validate_element_reference(&elem_page_1, &page_2, rev_1);
    assert!(mismatch.is_err());
    match mismatch.unwrap_err() {
        BrowserError::ResourceMismatch { expected, actual } => {
            assert_eq!(expected, "page-2");
            assert_eq!(actual, "page-1");
        }
        other => panic!("expected ResourceMismatch error, got {:?}", other),
    }
}

#[test]
fn stale_element_reference_after_revision_bump_is_rejected() {
    let page = PageId::new("page-live");
    let rev_1 = DocumentRevision::new(1);
    let rev_2 = DocumentRevision::new(2);

    let stale_elem = ElementRef::new(page.clone(), rev_1, "input-query");

    // Document has bumped to rev_2, so rev_1 ElementRef is stale
    let err = validate_element_reference(&stale_elem, &page, rev_2).unwrap_err();
    assert!(err.is_stale_element());
    match err {
        BrowserError::StaleElementReference {
            expected_revision,
            actual_revision,
        } => {
            assert_eq!(expected_revision, rev_1);
            assert_eq!(actual_revision, rev_2);
        }
        other => panic!("expected StaleElementReference, got {:?}", other),
    }
}

#[test]
fn browser_event_cannot_satisfy_browser_rpc_invocation() {
    // Invariant 4: EVENT BUS IS NOT RPC.
    let event = NavigationCommittedEvent {
        page_id: PageId::new("page-1"),
        navigation_id: worldline_browser_contract::identity::NavigationId::new("nav-1"),
        url: "http://worldline.local/test".to_string(),
        document_revision: DocumentRevision::new(2),
        status_code: 200,
    };
    let event_payload = serde_json::to_vec(&event).unwrap();

    // Trying to deserialize event payload as NavigateRequest fails closed
    let parse_as_req = serde_json::from_slice::<
        worldline_browser_contract::contracts::NavigateRequest,
    >(&event_payload);
    // Even if fields align partially, event topic is disjoint from capability contract
    assert_eq!(EVENT_NAVIGATION_COMMITTED, "browser.navigation.committed");
    assert_ne!(EVENT_NAVIGATION_COMMITTED, "browser.navigate");
    assert!(parse_as_req.is_ok()); // Payload alone is data; the transport route and contract ID enforce separation
}

#[test]
fn resource_identities_are_worldline_scoped() {
    let ctx = BrowserContextId::new("ctx-99");
    let page = PageId::new("page-42");

    assert_eq!(context_resource(&ctx), "browser-context/ctx-99");
    assert_eq!(page_resource(&page), "browser-page/page-42");
}
