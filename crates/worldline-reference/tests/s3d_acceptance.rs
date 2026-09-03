//! Deterministic S3D diagnostics acceptance test.

#[test]
fn s3d_reference_diagnostics_proving_slice_executes() {
    let report =
        worldline_reference::s3d::run_reference().expect("deterministic S3D fixture must run");
    assert!(
        report.accepted,
        "unexpected S3D reference report: {report:?}"
    );
    assert_eq!(report.topology, "reference");
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
