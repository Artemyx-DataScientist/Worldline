//! Hosted Windows real-CEF S3D diagnostics acceptance.

#[cfg(windows)]
#[test]
fn s3d_real_native_provider_diagnostics_proving_slice_executes() {
    let report = worldline_reference::s3d::run()
        .expect("S3D-real must execute through the hosted native CEF provider");
    println!("S3D run_id=S3D-real-20260903-local-01 report: {report:?}");
    assert!(report.accepted, "unexpected real S3D report: {report:?}");
    assert_eq!(report.topology, "cef_hosted");
    assert!(report.console_log_captured);
    assert!(report.console_warn_captured);
    assert!(report.console_error_captured);
    assert!(report.network_ok_captured);
    assert!(report.network_404_captured);
    assert!(report.runtime_snapshot_valid);
    assert!(report.context_isolation_enforced);
    assert!(report.overflow_drops_counted);
    assert!(report.lifecycle_cleanup_verified);
}
