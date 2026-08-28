fn main() {
    let report = worldline_reference::s0::run().expect("S0 proving slice must pass");

    println!("S0 kernel composition proof");
    println!("installation: {}", report.installation_id);
    println!(
        "RPC result: {} -> {}",
        report.first_result, report.restarted_result
    );
    println!(
        "installation state: {} -> {}",
        report.state_before_restart, report.state_after_restart
    );
    println!(
        "RuntimeId: {} -> {}",
        report.old_runtime_id, report.new_runtime_id
    );
    println!(
        "runtime principals: {} -> {}",
        report.old_runtime, report.new_runtime
    );
    println!("independent observations: {}", report.observations);
    println!(
        "old authority revoked: {}; new runtime inherited authority: {}",
        report.old_runtime_grant_revoked, !report.new_runtime_did_not_inherit_authority
    );

    let s1 = worldline_reference::s1::run().expect("S1 proving slice must pass");
    println!("S1 capability RPC + typed event transport proof");
    println!("RPC result: {} -> {}", s1.first_result, s1.restarted_result);
    println!("event observations: {}", s1.observed_events);
    println!("follow-up RPC: {}", s1.follow_up_result);
    println!(
        "state: {} -> {}; RuntimeId: {} -> {}",
        s1.state_before_restart, s1.state_after_restart, s1.old_runtime_id, s1.new_runtime_id
    );
    println!(
        "metadata-only control event: {}; old authority revoked: {}; new authority required: {}",
        s1.control_observation_was_metadata_only,
        s1.old_runtime_authority_revoked,
        s1.new_runtime_required_explicit_authority
    );
}
