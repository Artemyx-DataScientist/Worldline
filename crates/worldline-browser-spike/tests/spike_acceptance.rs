use worldline_browser_contract::{
    contracts::{ElementQueryKind, PermissionDecision, PermissionType},
    events::{NavigationCommittedEvent, PageCreatedEvent},
    identity::{DocumentRevision, ElementRef},
    query::{AccessibilityRole, QueryBounds},
};
use worldline_browser_spike::BrowserSpikeFixture;

#[test]
fn full_executable_spike_path_local_navigation_and_action() {
    let mut harness = BrowserSpikeFixture::boot().expect("fixture must boot");

    // Subscribe to browser events via M0.4 kernel event transport
    let sub_created = harness
        .subscribe("page.created")
        .expect("must subscribe to page.created");
    let sub_nav = harness
        .subscribe("navigation.committed")
        .expect("must subscribe to navigation.committed");

    // 1. Create context with logical profile ID
    let context_id = harness
        .create_context(Some("profile-primary".to_string()), false)
        .expect("must create context");

    // 2. Create page in context
    let page_id = harness
        .create_page(&context_id, None)
        .expect("must create page");

    // Verify PageCreatedEvent was published via kernel event transport and received by subscriber
    let env_created = sub_created
        .try_recv()
        .expect("event recv must succeed")
        .expect("must receive PageCreatedEvent envelope");
    assert_eq!(env_created.contract().name(), "page.created");
    assert_eq!(env_created.producer(), harness.provider_principal());

    let page_created_ev: PageCreatedEvent = serde_json::from_slice(env_created.payload()).unwrap();
    assert_eq!(page_created_ev.page_id, page_id);
    assert_eq!(page_created_ev.context_id, context_id);

    // 3. Navigate to deterministic local test page
    let nav_resp = harness
        .navigate(&page_id, "http://worldline.local/test-form")
        .expect("must navigate");
    assert!(nav_resp.committed);
    assert_eq!(nav_resp.document_revision.value(), 2);

    // Verify NavigationCommittedEvent was published via kernel event transport and received by subscriber
    let env_nav = sub_nav
        .try_recv()
        .expect("event recv must succeed")
        .expect("must receive NavigationCommittedEvent envelope");
    assert_eq!(env_nav.contract().name(), "navigation.committed");
    assert_eq!(env_nav.producer(), harness.provider_principal());

    let nav_ev: NavigationCommittedEvent = serde_json::from_slice(env_nav.payload()).unwrap();
    assert_eq!(nav_ev.page_id, page_id);
    assert_eq!(nav_ev.url, "http://worldline.local/test-form");
    assert_eq!(nav_ev.document_revision.value(), 2);

    // 4. Observe committed page state
    let obs = harness.observe(&page_id).expect("must observe page");
    assert_eq!(obs.title, "Worldline Local Test Form");
    assert_eq!(obs.document_revision.value(), 2);

    // 5. Query structured document & accessibility tree with bounds
    let bounds = QueryBounds {
        max_depth: 8,
        max_nodes: 50,
        max_text_len: 200,
        max_total_text_bytes: 4096,
    };
    let doc = harness
        .query_document(&page_id, Some(bounds))
        .expect("must query document snapshot");
    assert_eq!(doc.metadata.title, "Worldline Local Test Form");
    assert_eq!(doc.accessibility_tree.root.role, AccessibilityRole::Root);

    // Find the input element and submit button in the accessibility tree
    let form_node = &doc.accessibility_tree.root.children[0];
    let input_node = form_node
        .children
        .iter()
        .find(|c| c.role == AccessibilityRole::TextInput)
        .expect("must find input node");
    let input_ref = input_node
        .element_ref
        .clone()
        .expect("must have element ref");

    let button_node = form_node
        .children
        .iter()
        .find(|c| c.role == AccessibilityRole::Button)
        .expect("must find button node");
    let button_ref = button_node
        .element_ref
        .clone()
        .expect("must have element ref");

    // 6. Execute authorized interaction: Input text into search field
    let input_res = harness
        .input_text(&input_ref, "Rust microkernel")
        .expect("input action must succeed");
    assert!(input_res.success);

    // 7. Execute authorized interaction: Click submit button
    let click_res = harness
        .click_element(&button_ref)
        .expect("click action must succeed");
    assert!(click_res.success);
    assert_eq!(click_res.document_revision.value(), 3);

    // 8. Observe resulting page state after interaction
    let obs_post = harness
        .observe(&page_id)
        .expect("must observe page after action");
    assert_eq!(obs_post.title, "Results for Rust microkernel");
    assert_eq!(obs_post.document_revision.value(), 3);
}

#[test]
fn confused_deputy_resource_mismatch_is_rejected() {
    let mut harness = BrowserSpikeFixture::boot().expect("fixture must boot");

    let ctx_1 = harness
        .create_context(None, false)
        .expect("create context 1");
    let ctx_2 = harness
        .create_context(None, false)
        .expect("create context 2");

    let page_1 = harness.create_page(&ctx_1, None).expect("create page 1");
    let page_2 = harness
        .create_page(&ctx_2, Some("http://worldline.local/test-form".to_string()))
        .expect("create page 2");

    let elem_page_2 = ElementRef::new(page_2.clone(), DocumentRevision::new(2), "submit-btn");

    // 1. Caller attempts to invoke an action on page_2 using resource scope for page_1 -> REJECTED
    let err_page_mismatch = harness
        .invoke_confused_deputy_act(&page_1, &elem_page_2)
        .unwrap_err();
    assert!(
        err_page_mismatch.contains("ResourceMismatch")
            || err_page_mismatch.contains("resource scope mismatch"),
        "Must reject confused-deputy page resource mismatch: {err_page_mismatch}"
    );

    // 2. Caller attempts to invoke an action on page_2 (owned by ctx_2) using resource scope for ctx_1 -> REJECTED (no prefix loophole)
    let err_ctx_mismatch = harness
        .invoke_cross_context_page_act(&ctx_1, &elem_page_2)
        .unwrap_err();
    assert!(
        err_ctx_mismatch.contains("ResourceMismatch")
            || err_ctx_mismatch.contains("resource scope mismatch"),
        "Must reject cross-context unauthorized resource scope: {err_ctx_mismatch}"
    );

    // 3. Caller invokes action on page_2 using valid owning context resource scope for ctx_2 -> ACCEPTED
    let ok_ctx_match = harness
        .invoke_cross_context_page_act(&ctx_2, &elem_page_2)
        .expect("Owning context resource scope must authorize page action");
    assert!(ok_ctx_match.success);
}

#[test]
fn all_v1_capability_operations_are_concretely_supported() {
    let mut harness = BrowserSpikeFixture::boot().expect("fixture must boot");

    let ctx = harness.create_context(None, false).expect("create context");
    let page = harness
        .create_page(&ctx, Some("http://worldline.local/test-form".to_string()))
        .expect("create page");

    // 1. Reload
    let reload_resp = harness.reload(&page).expect("reload");
    assert!(reload_resp.reloaded);

    // 2. Extract text
    let text_resp = harness.extract_text(&page, None).expect("extract text");
    assert!(text_resp.text.contains("Search Query"));

    // 3. Find elements
    let find_resp = harness
        .find_elements(&page, "query", ElementQueryKind::CssSelector)
        .expect("find elements");
    assert!(!find_resp.elements.is_empty());

    // 4. Download
    let dl_resp = harness
        .start_download(&page, "http://worldline.local/file.bin")
        .expect("start download");
    assert_eq!(dl_resp.page_id, page);

    // 5. Permission
    let perm_resp = harness
        .set_permission(
            &ctx,
            "http://worldline.local",
            PermissionType::Notifications,
            PermissionDecision::Granted,
        )
        .expect("set perm");
    assert_eq!(perm_resp.decision, PermissionDecision::Granted);
}

#[test]
fn profile_and_context_storage_isolation() {
    let mut harness = BrowserSpikeFixture::boot().expect("fixture must boot");

    let ctx_a = harness
        .create_context(Some("profile-a".to_string()), false)
        .expect("create ctx_a");
    let ctx_b = harness
        .create_context(Some("profile-b".to_string()), false)
        .expect("create ctx_b");

    // Set cookie in Context A
    harness
        .supervisor()
        .set_cookie(&ctx_a, "session_token", "secret_token_a")
        .expect("set cookie A");

    // Verify Context A has the cookie
    let cookie_a = harness
        .supervisor()
        .get_cookie(&ctx_a, "session_token")
        .expect("get cookie A");
    assert_eq!(cookie_a.as_deref(), Some("secret_token_a"));

    // Verify Context B does NOT have the cookie from Context A
    let cookie_b = harness
        .supervisor()
        .get_cookie(&ctx_b, "session_token")
        .expect("get cookie B");
    assert_eq!(cookie_b, None);
}

#[test]
fn engine_crash_containment_and_host_survival() {
    let mut harness = BrowserSpikeFixture::boot().expect("fixture must boot");

    let ctx = harness
        .create_context(None, true)
        .expect("create incognito context");
    let page = harness
        .create_page(&ctx, Some("http://worldline.local/test-form".to_string()))
        .expect("create page");

    // Page is healthy
    let obs = harness.observe(&page).expect("observe healthy page");
    assert_eq!(obs.title, "Worldline Local Test Form");

    // Deliberately crash / terminate the child process for this page
    harness
        .supervisor()
        .crash_page_process(&page)
        .expect("crash simulation");

    // Host remains ALIVE! Kernel state is intact.
    // Subsequent calls to crashed page fail explicitly with EngineCrashed error:
    let err = harness.observe(&page).unwrap_err();
    assert!(err.contains("page renderer process has terminated"));

    let doc_err = harness.query_document(&page, None).unwrap_err();
    assert!(doc_err.contains("page renderer process has terminated"));

    let nav_err = harness
        .navigate(&page, "http://worldline.local/other")
        .unwrap_err();
    assert!(nav_err.contains("page renderer process has terminated"));
}
