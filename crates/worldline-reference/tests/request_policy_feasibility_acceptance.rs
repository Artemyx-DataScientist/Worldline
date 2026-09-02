//! Deterministic reference evidence for the T-004 early feasibility gate.

#[test]
fn request_policy_reference_feasibility_is_bounded_and_fail_open_is_profile_scoped() {
    let report = worldline_reference::request_policy_feasibility::run_reference()
        .expect("reference request-policy feasibility must run");
    assert!(report.accepted, "unexpected feasibility report: {report:?}");
    assert_eq!(report.request_count, report.completed_decisions);
    assert!(report.max_observed_in_flight <= report.queue_bound);
    assert_eq!(report.timeout_decisions, 1);
    assert_eq!(report.fallback_decisions, 1);
    assert_eq!(report.observations, report.request_count + 1);
}
