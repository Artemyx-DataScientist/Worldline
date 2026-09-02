//! Deterministic S3C request-policy acceptance.

#[test]
fn s3c_reference_blocks_before_origin_and_preserves_profile_failure_semantics() {
    let report =
        worldline_reference::s3c::run_reference().expect("deterministic S3C fixture must run");
    assert!(report.accepted, "unexpected S3C report: {report:?}");
    assert_eq!(report.blocked_origin_hits, 0);
    assert_eq!(report.allowed_origin_hits, 1);
    assert!(report.page_usable);
    assert!(report.exact_scope_isolated);
    assert!(report.replacement_isolated);
    assert!(report.lifecycle_cleanup);
    assert!(report.fail_open_timeout);
    assert!(report.fail_open_unavailable);
    assert!(report.safe_observations);
}
