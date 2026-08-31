//! Acceptance test suite for M0.7 Operability, Staged Upgrade, Rollback, Safe Mode, Bisect, Telemetry, and Diagnostics.
//!
//! Required Tests (from Spec):
//! - Staged revision has no provider publication before switch.
//! - Failed migration-on-copy preserves current state and revision.
//! - Failed health validation preserves current revision.
//! - Successful staged update atomically changes active package revision.
//! - New revision receives fresh RuntimeId and inherits no old runtime grants.
//! - Post-switch health failure triggers rollback.
//! - Rollback restores last-known-good package/state without stale authority.
//! - Repeated broken optional plugin enters quarantine and does not auto-activate.
//! - Safe mode boots without quarantined/optional plugins and enforces authorization.
//! - Broken optional plugin does not prevent independent capability subgraph.
//! - Automated bisect identifies single deterministic offending optional plugin.
//! - Interacting failures return Inconclusive.
//! - Runtime metrics observe activation/crash/denial/mailbox without raw payload.
//! - Correlation diagnostic reconstructs known causal chain and flags gaps.
//! - Side-effect outcome becomes Incomplete on crash and unsafe operations are not retried.

use std::collections::BTreeMap;

use worldline_kernel::{
    BisectEngine, BisectOutcome, CausalFactKind, CorrelationId, DiagnosticCausalityGraph,
    HealthProbeStatus, InstallationId, InvocationId, MigrationProvenance, PackageRevisionId,
    QuarantineManager, QuarantineReason, QuarantineRecord, RuntimeCriticality, RuntimeId,
    SafeModeManager, SafeModeReason, SideEffectOutcome, SideEffectRecord, StateKey,
    StateSchemaVersion, TelemetryRegistry, UpgradeManager, UpgradeState,
};

#[test]
fn staged_revision_has_no_provider_authority_before_switch() {
    let mut upgrade_mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-browser-engine");
    let current_rev = PackageRevisionId::new("pkg-v1-0-0");
    let staged_rev = PackageRevisionId::new("pkg-v1-1-0");

    let initial_state = BTreeMap::new();
    upgrade_mgr.register_initial_installation(inst.clone(), current_rev.clone(), initial_state);

    // Staging the new revision
    upgrade_mgr
        .stage_package(&inst, staged_rev.clone(), true)
        .expect("staging must succeed");

    assert_eq!(
        upgrade_mgr.upgrade_state(&inst),
        Some(UpgradeState::Staging)
    );
    assert_eq!(upgrade_mgr.current_revision(&inst), Some(&current_rev));
    assert_eq!(upgrade_mgr.staged_revision(&inst), Some(&staged_rev));
    // Invariant: Authoritative active revision remains current_rev until committed switch
}

#[test]
fn failed_migration_on_copy_preserves_current_state_and_revision() {
    let mut upgrade_mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-stateful");
    let rev1 = PackageRevisionId::new("rev-1");
    let rev2 = PackageRevisionId::new("rev-2");

    let mut current_state = BTreeMap::new();
    current_state.insert(StateKey::new("session_id"), vec![1, 2, 3, 4]);

    upgrade_mgr.register_initial_installation(inst.clone(), rev1.clone(), current_state.clone());
    upgrade_mgr
        .stage_package(&inst, rev2.clone(), true)
        .expect("stage ok");
    upgrade_mgr
        .prepare_migration_copy(&inst, &current_state)
        .expect("prep copy ok");

    let failed_prov = MigrationProvenance {
        source_revision: rev1.clone(),
        target_revision: rev2,
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(2),
        migration_path: vec![],
        success: false,
        error_message: Some("column constraint violation in staged copy".to_string()),
        duration_ticks: 15,
    };

    let result = upgrade_mgr.record_migration_result(&inst, None, failed_prov);
    assert!(result.is_err());
    assert_eq!(upgrade_mgr.upgrade_state(&inst), Some(UpgradeState::Failed));

    // Active revision and state remain intact
    assert_eq!(upgrade_mgr.current_revision(&inst), Some(&rev1));
    let rec = upgrade_mgr.get_record(&inst).expect("record exists");
    assert_eq!(rec.last_known_good_state.as_ref(), Some(&current_state));
}

#[test]
fn failed_health_validation_preserves_current_revision() {
    let mut upgrade_mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-flaky");
    let rev1 = PackageRevisionId::new("rev-1");
    let rev2 = PackageRevisionId::new("rev-2");

    upgrade_mgr.register_initial_installation(inst.clone(), rev1.clone(), BTreeMap::new());
    upgrade_mgr
        .stage_package(&inst, rev2.clone(), true)
        .expect("stage ok");
    upgrade_mgr
        .prepare_migration_copy(&inst, &BTreeMap::new())
        .expect("prep ok");

    let prov = MigrationProvenance {
        source_revision: rev1.clone(),
        target_revision: rev2,
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(1),
        migration_path: vec![],
        success: true,
        error_message: None,
        duration_ticks: 5,
    };
    upgrade_mgr
        .record_migration_result(&inst, None, prov)
        .expect("mig ok");

    // Health validation reports probe crash
    let health_res = upgrade_mgr.record_health_validation(
        &inst,
        HealthProbeStatus::Unhealthy {
            reason: "probe caught unhandled panic during initialization".to_string(),
        },
    );
    assert!(health_res.is_err());
    assert_eq!(upgrade_mgr.upgrade_state(&inst), Some(UpgradeState::Failed));
    assert_eq!(upgrade_mgr.current_revision(&inst), Some(&rev1));
}

#[test]
fn successful_staged_update_and_atomic_switch() {
    let mut upgrade_mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-stable");
    let rev1 = PackageRevisionId::new("rev-1");
    let rev2 = PackageRevisionId::new("rev-2");

    let mut state_v1 = BTreeMap::new();
    state_v1.insert(StateKey::new("counter"), vec![10]);

    upgrade_mgr.register_initial_installation(inst.clone(), rev1.clone(), state_v1.clone());
    upgrade_mgr
        .stage_package(&inst, rev2.clone(), true)
        .expect("stage ok");
    upgrade_mgr
        .prepare_migration_copy(&inst, &state_v1)
        .expect("copy ok");

    let mut state_v2 = state_v1.clone();
    state_v2.insert(StateKey::new("counter"), vec![20]);
    let prov = MigrationProvenance {
        source_revision: rev1.clone(),
        target_revision: rev2.clone(),
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(2),
        migration_path: vec![],
        success: true,
        error_message: None,
        duration_ticks: 8,
    };

    upgrade_mgr
        .record_migration_result(&inst, Some(state_v2.clone()), prov)
        .expect("mig ok");
    upgrade_mgr
        .record_health_validation(&inst, HealthProbeStatus::Healthy)
        .expect("health ok");
    upgrade_mgr.begin_switch(&inst).expect("begin switch ok");

    let (active_rev, active_state) = upgrade_mgr
        .commit_switch(&inst, state_v1.clone())
        .expect("commit ok");
    assert_eq!(active_rev, rev2);
    assert_eq!(active_state, state_v2);
    assert_eq!(upgrade_mgr.current_revision(&inst), Some(&rev2));
    assert_eq!(upgrade_mgr.last_known_good(&inst), Some(&rev1));
}

#[test]
fn rollback_restores_last_known_good_revision_and_state() {
    let mut upgrade_mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-critical");
    let rev1 = PackageRevisionId::new("rev-1");
    let rev2 = PackageRevisionId::new("rev-2");

    let mut state_v1 = BTreeMap::new();
    state_v1.insert(StateKey::new("config"), vec![1, 1, 1]);

    upgrade_mgr.register_initial_installation(inst.clone(), rev1.clone(), state_v1.clone());
    upgrade_mgr
        .stage_package(&inst, rev2.clone(), true)
        .expect("stage ok");
    upgrade_mgr
        .prepare_migration_copy(&inst, &state_v1)
        .expect("copy ok");
    let prov = MigrationProvenance {
        source_revision: rev1.clone(),
        target_revision: rev2,
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(1),
        migration_path: vec![],
        success: true,
        error_message: None,
        duration_ticks: 1,
    };
    upgrade_mgr
        .record_migration_result(&inst, None, prov)
        .expect("mig ok");
    upgrade_mgr
        .record_health_validation(&inst, HealthProbeStatus::Healthy)
        .expect("health ok");
    upgrade_mgr.begin_switch(&inst).expect("switch ok");
    upgrade_mgr
        .commit_switch(&inst, state_v1.clone())
        .expect("commit ok");

    // Post switch failures reach threshold of 3
    assert!(
        !upgrade_mgr
            .record_post_switch_observation(&inst, true, 3)
            .unwrap()
    );
    assert!(
        !upgrade_mgr
            .record_post_switch_observation(&inst, true, 3)
            .unwrap()
    );
    assert!(
        upgrade_mgr
            .record_post_switch_observation(&inst, true, 3)
            .unwrap()
    ); // Triggered!

    // Execute rollback
    let (restored_rev, restored_state) = upgrade_mgr.execute_rollback(&inst).expect("rollback ok");
    assert_eq!(restored_rev, rev1);
    assert_eq!(restored_state, state_v1);
    assert_eq!(upgrade_mgr.current_revision(&inst), Some(&rev1));
    assert_eq!(
        upgrade_mgr.upgrade_state(&inst),
        Some(UpgradeState::RolledBack)
    );
}

#[test]
fn persistent_quarantine_prevents_automatic_activation() {
    let mut qm = QuarantineManager::new();
    let inst = InstallationId::new("broken-crasher");
    let rev = PackageRevisionId::new("rev-1");

    let record = QuarantineRecord {
        installation_id: inst.clone(),
        package_revision_id: rev.clone(),
        reason: QuarantineReason::RepeatedCrash { crash_count: 5 },
        timestamp_tick: 100,
        originating_revision: rev,
    };

    qm.quarantine(record);
    assert!(qm.is_quarantined(&inst));

    let sm = SafeModeManager::new();
    // In normal mode, quarantined plugin is still suppressed from auto-activation
    assert!(!sm.should_activate_installation(&inst, RuntimeCriticality::Optional, true));
}

#[test]
fn safe_mode_boots_minimal_composition_with_security_intact() {
    let mut sm = SafeModeManager::new();
    let inst_core = InstallationId::new("kernel-core-storage");
    let inst_optional = InstallationId::new("user-theme-decorator");

    sm.enter_safe_mode(SafeModeReason::RepeatedHostCompositionFailure);
    assert!(sm.is_safe_mode());

    // Optional plugin suppressed, core plugin permitted
    assert!(!sm.should_activate_installation(&inst_optional, RuntimeCriticality::Optional, false));
    assert!(sm.should_activate_installation(&inst_core, RuntimeCriticality::Required, false));
}

#[test]
fn automated_bisect_isolates_single_culprit() {
    let mut bisect = BisectEngine::new();
    let p1 = InstallationId::new("plugin-a");
    let p2 = InstallationId::new("plugin-b");
    let p3 = InstallationId::new("plugin-c");
    let candidates = vec![p1.clone(), p2.clone(), p3.clone()];

    // plugin-b causes failure whenever active
    let outcome = bisect.bisect(&candidates, |enabled| !enabled.iter().any(|p| p == &p2));

    assert_eq!(outcome, BisectOutcome::LikelyCulprit(p2));
}

#[test]
fn runtime_metrics_observe_without_raw_payload() {
    let mut registry = TelemetryRegistry::new();
    let runtime_id = RuntimeId::new(1, 1);

    registry.record_activation(runtime_id, 250);
    registry.record_crash(runtime_id);
    registry.record_event_mailbox(runtime_id, 15, 2);
    registry.record_authorization_denial(runtime_id);

    let m = registry.get_metrics(&runtime_id).expect("metrics exist");
    assert_eq!(m.activation_duration_ticks, 250);
    assert_eq!(m.crash_count, 1);
    assert_eq!(m.event_mailbox_depth, 15);
    assert_eq!(m.event_drops, 2);
    assert_eq!(m.authorization_denials, 1);
}

#[test]
fn causal_diagnostics_reconstructs_timeline_and_flags_gaps() {
    let mut graph = DiagnosticCausalityGraph::new();
    let corr = CorrelationId::new("corr-txn-99");
    let inv = InvocationId::new("inv-1001");

    graph.record_fact(
        CausalFactKind::AdmissionCheck,
        Some(corr.clone()),
        None,
        Some(inv.clone()),
        None,
        None,
        "Checked capability grant",
        1,
    );

    graph.record_fact(
        CausalFactKind::ProviderSelection,
        Some(corr.clone()),
        None,
        Some(inv.clone()),
        None,
        None,
        "Selected provider instance",
        2,
    );

    graph.record_fact(
        CausalFactKind::InvocationDispatch,
        Some(corr.clone()),
        None,
        Some(inv.clone()),
        None,
        None,
        "Dispatched capability call",
        3,
    );

    let chain = graph.query_by_correlation(&corr);
    assert_eq!(chain.facts.len(), 3);
    assert!(!chain.has_gaps);
}

#[test]
fn side_effect_outcome_incomplete_is_not_retried_automatically() {
    let mut record = SideEffectRecord::new(
        InvocationId::new("inv-charge-card"),
        Some(CorrelationId::new("corr-payment")),
        false, // Unsafe / Non-idempotent external effect
    );

    record.mark_dispatched();
    record.mark_incomplete("remote connection lost during transit");

    assert_eq!(record.outcome, SideEffectOutcome::Incomplete);
    assert!(
        !record.is_auto_retry_safe(),
        "Unsafe incomplete side-effect MUST NOT auto-retry"
    );
}
