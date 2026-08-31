use worldline_browser_contract::query::AccessibilityRole;
use worldline_browser_spike::BrowserSpikeFixture;

#[test]
fn full_executable_spike_path_local_navigation_and_action() {
    let mut harness = BrowserSpikeFixture::boot().expect("fixture must boot");

    // 1. Create context with explicit storage root
    let context_id = harness
        .create_context(Some("/worldline/profiles/test_user".to_string()), false)
        .expect("must create context");

    // 2. Create page in context
    let page_id = harness
        .create_page(&context_id, None)
        .expect("must create page");

    // 3. Navigate to deterministic local test page
    let nav_resp = harness
        .navigate(&page_id, "http://worldline.local/test-form")
        .expect("must navigate");
    assert!(nav_resp.committed);
    assert_eq!(nav_resp.document_revision.value(), 2);

    // 4. Observe committed page state
    let obs = harness.observe(&page_id).expect("must observe page");
    assert_eq!(obs.title, "Worldline Local Test Form");
    assert_eq!(obs.document_revision.value(), 2);

    // 5. Query structured document & accessibility tree
    let doc = harness
        .query_document(&page_id)
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
fn profile_and_context_storage_isolation() {
    let mut harness = BrowserSpikeFixture::boot().expect("fixture must boot");

    let ctx_a = harness
        .create_context(Some("/worldline/profiles/ctx_a".to_string()), false)
        .expect("create ctx_a");
    let ctx_b = harness
        .create_context(Some("/worldline/profiles/ctx_b".to_string()), false)
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

    let doc_err = harness.query_document(&page).unwrap_err();
    assert!(doc_err.contains("page renderer process has terminated"));

    let nav_err = harness
        .navigate(&page, "http://worldline.local/other")
        .unwrap_err();
    assert!(nav_err.contains("page renderer process has terminated"));
}
