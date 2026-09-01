use worldline_reference::s3a;

#[test]
fn proving_slice_s3a_acceptance() {
    let report = s3a::run().expect("S3A proving slice execution must succeed");

    assert!(!report.page_id.is_empty());
    assert!(!report.tab_id.is_empty());
    assert!(!report.history_entry_id.is_empty());
    assert_eq!(report.initial_url, "https://worldline.test/s3a-initial");
    assert_eq!(report.navigated_url, "https://worldline.test/s3a-navigated");
    assert!(
        report.history_survived_restart,
        "History must survive restart from snapshot"
    );
    assert!(
        report.page_survived_tab_removal,
        "PageId must survive tab detachment/closure"
    );
    assert!(
        report.post_removal_navigation_ok,
        "PageId must remain fully operational after tab removal"
    );
}
