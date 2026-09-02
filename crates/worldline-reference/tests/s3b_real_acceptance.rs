//! Hosted Windows acceptance for the production S3B native CEF path.

#[cfg(windows)]
#[test]
fn s3b_real_native_provider_proving_slice_executes() {
    let report = worldline_reference::s3b::run()
        .expect("S3B-real must execute through the hosted native CEF provider");
    assert!(report.artifact_bytes_verified);
    assert!(report.download_survived_restart);
    assert!(report.metadata_only_isolation_ok);
    assert!(report.cross_context_cookies_isolated);
    assert!(report.cookies_survived_restart);
    assert!(report.site_data_clear_isolated);
    assert!(report.service_failure_isolation_ok);
}
