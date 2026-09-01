use std::fs;

use worldline_browser_contract::{
    action::InteractionKind,
    error::BrowserError,
    identity::{ElementRef, PageId},
    query::{AccessibilityNode, AccessibilityRole, QueryBounds},
};
use worldline_browser_spike::ChromiumEngineSupervisor;

fn find_node_by_role(
    node: &AccessibilityNode,
    role: AccessibilityRole,
) -> Option<&AccessibilityNode> {
    if node.role == role {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_node_by_role(child, role))
}

#[test]
fn real_out_of_process_chromium_spike_navigation_and_dom_interaction() {
    // 1. Spawn real out-of-process Chromium/Edge headless process
    let mut supervisor = match ChromiumEngineSupervisor::spawn() {
        Ok(s) => s,
        Err(e) => {
            if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
                panic!(
                    "FAIL-CLOSED: Chromium/Edge browser binary MUST be available and runnable in CI environment: {e}"
                );
            }
            println!(
                "Skipping real Chromium test (no browser found in local non-CI environment): {e}"
            );
            return;
        }
    };

    println!(
        "Real Chromium Spike: Running on '{}' (Boot latency: {:?})",
        supervisor.browser_name(),
        supervisor.startup_duration()
    );
    assert!(supervisor.is_host_alive(), "Browser process must be alive");

    // 2. Measure initial memory footprint
    if let Some(ram_bytes) = supervisor.measure_memory_bytes() {
        println!(
            "Real Chromium Spike: Initial Working Set Memory: {} KB",
            ram_bytes / 1024
        );
        assert!(ram_bytes > 0, "Memory must be measurable");
    }

    // 3. Create a real page
    let page_id = PageId::new("page-real-1");
    supervisor
        .create_page(page_id.clone())
        .expect("must create page via CDP");

    // 4. Create a deterministic local HTML test fixture file
    let fixture_path =
        std::env::temp_dir().join(format!("worldline_fixture_{}.html", std::process::id()));
    let html_content = r#"<!DOCTYPE html>
<html>
<head>
    <title>Worldline Real Chromium Fixture</title>
</head>
<body>
    <h1>Worldline Browser Spike</h1>
    <form id="spike-form">
        <label for="search-input">Search Query</label>
        <input type="text" id="search-input" name="q" value="initial query" />
        <button type="button" id="submit-btn" onclick="document.title = 'Submitted: ' + document.getElementById('search-input').value">Submit Query</button>
    </form>
</body>
</html>"#;
    fs::write(&fixture_path, html_content).expect("must write fixture file");
    let file_url = format!(
        "file:///{}",
        fixture_path.display().to_string().replace('\\', "/")
    );

    // 5. Navigate to local HTML fixture
    let (_nav_id, rev) = supervisor
        .navigate(&page_id, &file_url)
        .expect("must navigate real Chromium page");
    assert_eq!(rev.value(), 2);

    // 6. Query real Blink accessibility tree and apply query bounds
    let bounds = QueryBounds {
        max_depth: 10,
        max_nodes: 50,
        max_text_len: 100,
        max_total_text_bytes: 2048,
    };
    let doc_snapshot = supervisor
        .query_document(&page_id, Some(&bounds))
        .expect("must query real AX tree");
    assert_eq!(
        doc_snapshot.metadata.title,
        "Worldline Real Chromium Fixture"
    );

    // 7. Discover specific interactive elements from the tree
    let input_node = find_node_by_role(
        &doc_snapshot.accessibility_tree.root,
        AccessibilityRole::TextInput,
    )
    .expect("must find input node in tree");
    let input_ref = input_node
        .element_ref
        .as_ref()
        .expect("input must have ElementRef");
    assert!(input_ref.node_key().starts_with("backend-dom-node-"));

    let btn_node = find_node_by_role(
        &doc_snapshot.accessibility_tree.root,
        AccessibilityRole::Button,
    )
    .expect("must find button node in tree");
    let btn_ref = btn_node
        .element_ref
        .as_ref()
        .expect("button must have ElementRef");
    assert!(btn_ref.node_key().starts_with("backend-dom-node-"));

    // 8. Dispatch real text input action targeted specifically to search-input
    let hostile_text = r#""; document.title = "INJECTED"; //"#;
    let input_res = supervisor
        .execute_action(input_ref, InteractionKind::Input, Some(hostile_text))
        .expect("input action must succeed");
    assert!(input_res.success);

    let before_click = supervisor
        .query_document(&page_id, None)
        .expect("must query after hostile input");
    assert_eq!(
        before_click.metadata.title, "Worldline Real Chromium Fixture",
        "input data must not execute as JavaScript"
    );

    // 9. Dispatch real click action targeted specifically to submit-btn
    let click_res = supervisor
        .execute_action(btn_ref, InteractionKind::Click, None)
        .expect("click action must succeed");
    assert!(click_res.success);

    // Verify that the title updated in the real Chromium DOM as a result of the click handler
    let post_query = supervisor
        .query_document(&page_id, None)
        .expect("must query post-action");
    assert_eq!(
        post_query.metadata.title,
        format!("Submitted: {hostile_text}"),
        "hostile input must remain literal DOM data"
    );

    // 10. Verify that targeting a nonexistent ElementRef fails with ElementNotFound
    let invalid_ref = ElementRef::new(
        page_id.clone(),
        post_query.metadata.document_revision,
        "nonexistent-element-99",
    );
    let err = supervisor
        .execute_action(&invalid_ref, InteractionKind::Click, None)
        .unwrap_err();
    assert!(
        matches!(err, BrowserError::ElementNotFound(_)),
        "Must fail with ElementNotFound on nonexistent element, got: {err:?}"
    );

    // 11. Deliberately crash the renderer process via Page.crash
    supervisor.crash_renderer(&page_id).expect("crash call");

    // Host supervisor process remains ALIVE!
    assert!(
        supervisor.is_host_alive(),
        "Host browser supervisor must survive renderer crash"
    );

    // Subsequent calls return typed EngineCrashed error
    let crash_err = supervisor.navigate(&page_id, "about:blank").unwrap_err();
    assert!(crash_err.is_crashed(), "Must report EngineCrashed error");

    // Clean up fixture file
    let _ = fs::remove_file(&fixture_path);
}
