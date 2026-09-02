//! S3B acceptance test proving downloads, cookies, site-data, artifact handoff, and failure isolation.

#[cfg(windows)]
#[test]
fn s3b_proving_slice_executes() {
    let report = worldline_reference::s3b::run_reference()
        .expect("deterministic S3B proving slice must succeed");
    assert!(
        report.artifact_bytes_verified,
        "Downloaded artifact bytes must match deterministic fixture"
    );
    assert!(
        report.download_survived_restart,
        "Completed download record must persist across service restart"
    );
    assert!(
        report.metadata_only_isolation_ok,
        "Download metadata inspection must be isolated from bytes"
    );
    assert!(
        report.cross_context_cookies_isolated,
        "Cookies between Context A and Context B must be strictly isolated"
    );
    assert!(
        report.cookies_survived_restart,
        "Cookies must remain authoritative in engine profile store across restart"
    );
    assert!(
        report.site_data_clear_isolated,
        "Origin site-data clear must affect only target context"
    );
    assert!(
        report.service_failure_isolation_ok,
        "Direct page navigation, tabs, and history must remain operational after service termination"
    );
}
