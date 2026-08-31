use std::time::Instant;
use worldline_browser_spike::BrowserSpikeFixture;

#[test]
fn engine_spike_empirical_measurements() {
    let boot_start = Instant::now();
    let mut harness = BrowserSpikeFixture::boot().expect("fixture boot");
    let boot_duration = boot_start.elapsed();

    println!(
        "Measurement - Kernel + Provider Boot Duration: {:?}",
        boot_duration
    );
    assert!(boot_duration.as_millis() < 500, "Boot must be fast");

    let ctx_start = Instant::now();
    let ctx = harness
        .create_context(Some("bench-profile".to_string()), false)
        .expect("create context");
    let ctx_duration = ctx_start.elapsed();
    println!(
        "Measurement - Context Creation Duration: {:?}",
        ctx_duration
    );

    let page_start = Instant::now();
    let page = harness
        .create_page(&ctx, Some("http://worldline.local/test-form".to_string()))
        .expect("create page");
    let page_duration = page_start.elapsed();
    println!(
        "Measurement - Page Creation + Navigation Duration: {:?}",
        page_duration
    );

    let query_start = Instant::now();
    let doc = harness.query_document(&page, None).expect("query doc");
    let query_duration = query_start.elapsed();
    println!(
        "Measurement - Document & AX Tree Query Duration: {:?}",
        query_duration
    );
    assert!(query_duration.as_millis() < 50, "Query must be low latency");

    let form_node = &doc.accessibility_tree.root.children[0];
    let input_node = form_node
        .children
        .iter()
        .find(|c| c.role == worldline_browser_contract::query::AccessibilityRole::TextInput)
        .expect("input node");
    let input_ref = input_node.element_ref.clone().expect("element ref");

    let action_start = Instant::now();
    let action_res = harness
        .input_text(&input_ref, "Benchmarking query")
        .expect("input action");
    let action_duration = action_start.elapsed();
    println!(
        "Measurement - Action Dispatch & Execution Duration: {:?}",
        action_duration
    );
    assert!(action_res.success);
}
