//! Hosted Windows real-CEF evidence for the T-004 early feasibility gate.

#[test]
fn request_policy_real_cef_feasibility_is_explicitly_separate_from_reference() {
    let report = worldline_reference::request_policy_feasibility::run_real()
        .expect("T-004 real CEF feasibility must execute on the hosted runtime");
    println!("T-004 run_id=T004-real-20260902-local-01 report: {report:?}");
    assert!(
        report.accepted,
        "unexpected real feasibility report: {report:?}"
    );
    assert!(!report.topology.contains("reference broker only"));
}
