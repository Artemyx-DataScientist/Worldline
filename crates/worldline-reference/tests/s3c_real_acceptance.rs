//! Hosted Windows real-CEF S3C request-policy acceptance.

#[cfg(windows)]
#[test]
fn s3c_real_native_provider_uses_adblock_contract_before_origin() {
    let report = worldline_reference::s3c::run()
        .expect("S3C-real must execute through the hosted native CEF provider");
    println!("S3C run_id=S3C-real-20260902-local-01 report: {report:?}");
    assert!(report.accepted, "unexpected real S3C report: {report:?}");
    assert!(!report.topology.contains("no CEF"));
    assert_eq!(report.blocked_origin_hits, 0);
    assert!(report.allowed_origin_hits >= 1);
}
