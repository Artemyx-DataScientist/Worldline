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
        "runtime identity: {} -> {}",
        report.old_runtime, report.new_runtime
    );
    println!("independent observations: {}", report.observations);
    println!(
        "old authority revoked: {}; new runtime inherited authority: {}",
        report.old_runtime_grant_revoked, !report.new_runtime_did_not_inherit_authority
    );
}
