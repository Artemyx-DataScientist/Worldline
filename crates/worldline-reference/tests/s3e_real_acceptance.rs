//! Hosted Windows real-CEF S3E browser search proving slice acceptance.

#[cfg(windows)]
#[test]
fn s3e_real_native_provider_search_proving_slice_executes() {
    let report = worldline_reference::s3e::run()
        .expect("S3E-real must execute through the hosted native CEF provider");
    println!("S3E report: {report:?}");
    assert!(report.accepted, "unexpected real S3E report: {report:?}");
    assert_eq!(report.topology, "real-cef");
    assert!(report.provider_a_resolved);
    assert!(report.provider_b_resolved);
    assert!(report.distinct_targets_produced);
    assert!(report.resolve_alone_zero_origin_hits);
    assert!(report.navigation_produced_origin_hit);
    assert!(report.query_decoded_intact);
    assert!(report.search_only_cannot_navigate);
    assert!(report.navigation_only_cannot_search);
    assert!(report.lifecycle_isolation_verified);
    assert!(report.query_privacy_verified);
}
