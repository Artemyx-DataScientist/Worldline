use worldline_reference::s2;

#[test]
fn proving_slice_s2_browser_engine_end_to_end() {
    let report = s2::run().expect("Proving slice S2 must complete successfully");

    assert!(!report.context_id.is_empty());
    assert!(!report.page_id.is_empty());
    assert_eq!(report.initial_url, "https://worldline.test/s2-initial");
    assert_eq!(report.initial_revision, 1);
    assert_eq!(report.post_nav_revision, 2);
    assert_eq!(report.post_action_revision, 3);
    assert_eq!(report.found_elements_count, 1);
    assert!(report.stale_action_rejected);
    assert!(report.capture_blob_id.starts_with("sha256:"));
    assert!(report.capture_bytes_read > 0);
    assert!(report.cookies_isolated);
    assert!(report.storage_cleared);
}
