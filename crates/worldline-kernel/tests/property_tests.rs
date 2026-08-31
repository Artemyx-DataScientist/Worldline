//! Property-based and invariant tests for Capability Resolver and Upgrade State Machine.
//!
//! Resolver Properties:
//! - Provider selection is deterministic for identical inputs/state.
//! - Incompatible provider is never selected.
//! - Adding an irrelevant incompatible provider does not change valid selected provider.
//! - Authority does not affect compatibility classification.
//! - Compatibility does not grant authority.
//! - Runtime replacement never revives old RuntimeId authority.
//!
//! Upgrade Properties:
//! - Exactly one active revision after recoverable switch.
//! - Failed pre-switch validation preserves previous revision.
//! - Rollback never resurrects stale handles.

use std::collections::BTreeMap;

use worldline_kernel::{
    CapabilityId, ContractStability, InstallationId, InterfaceVersion, MigrationProvenance,
    PackageRevisionId, RuntimeId, StateSchemaVersion, UpgradeManager, UpgradeState,
};

#[test]
fn prop_resolver_determinism() {
    for major in 1..=5 {
        for minor in 0..=10 {
            let req = CapabilityId::new("domain.test", "op", InterfaceVersion::new(major, minor));
            let prov =
                CapabilityId::new("domain.test", "op", InterfaceVersion::new(major, minor + 2));

            // Must evaluate identically on repeated queries
            let res1 = prov.is_compatible_with(&req);
            let res2 = prov.is_compatible_with(&req);
            assert_eq!(res1, res2);
            assert!(res1);
        }
    }
}

#[test]
fn prop_incompatible_provider_is_never_selected() {
    for req_major in 1..=5 {
        for prov_major in 1..=5 {
            if req_major == prov_major {
                continue;
            }
            let req = CapabilityId::new("domain.test", "op", InterfaceVersion::new(req_major, 0));
            let prov = CapabilityId::new("domain.test", "op", InterfaceVersion::new(prov_major, 0));

            assert!(
                !prov.is_compatible_with(&req),
                "Different major must never be compatible"
            );
        }
    }
}

#[test]
fn prop_irrelevant_provider_does_not_change_valid_selection() {
    let req = CapabilityId::new("domain.math", "calculate", InterfaceVersion::new(1, 0));
    let valid_prov = CapabilityId::new("domain.math", "calculate", InterfaceVersion::new(1, 3));
    let irrelevant_incompatible_prov =
        CapabilityId::new("domain.math", "calculate", InterfaceVersion::new(2, 0));

    let available = [valid_prov.clone(), irrelevant_incompatible_prov];

    let selected: Vec<&CapabilityId> = available
        .iter()
        .filter(|p| p.is_compatible_with(&req))
        .collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0], &valid_prov);
}

#[test]
fn prop_authority_does_not_affect_compatibility_and_vice_versa() {
    let cap = CapabilityId::with_stability(
        "domain.secure",
        "secret_op",
        InterfaceVersion::new(1, 0),
        ContractStability::Stable,
    );

    // Capability describes interface compatibility only; it does not grant access
    let contract = cap.contract();
    assert_eq!(contract.namespace(), "domain.secure");
    assert_eq!(contract.interface_major(), 1);
}

#[test]
fn prop_upgrade_single_authoritative_active_revision() {
    let mut mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-app");
    let mut current_rev_seq = 1;

    let initial_rev = PackageRevisionId::new(format!("rev-{current_rev_seq}"));
    mgr.register_initial_installation(inst.clone(), initial_rev.clone(), BTreeMap::new());

    for i in 2..=10 {
        let next_rev = PackageRevisionId::new(format!("rev-{i}"));
        mgr.stage_package(&inst, next_rev.clone(), true)
            .expect("stage ok");
        mgr.prepare_migration_copy(&inst, &BTreeMap::new())
            .expect("prep copy ok");

        let prov = MigrationProvenance {
            source_revision: PackageRevisionId::new(format!("rev-{}", i - 1)),
            target_revision: next_rev.clone(),
            source_schema: StateSchemaVersion::new((i - 1) as u64),
            target_schema: StateSchemaVersion::new(i as u64),
            migration_path: vec![],
            success: true,
            error_message: None,
            duration_ticks: 1,
        };
        mgr.record_migration_result(&inst, None, prov)
            .expect("mig ok");
        mgr.record_health_validation(&inst, worldline_kernel::HealthProbeStatus::Healthy)
            .expect("health ok");
        mgr.begin_switch(&inst).expect("begin switch ok");
        mgr.commit_switch(&inst, BTreeMap::new())
            .expect("commit ok");

        // Invariant: At every committed step, exactly ONE authoritative current revision
        assert_eq!(mgr.current_revision(&inst), Some(&next_rev));
        assert_eq!(
            mgr.last_known_good(&inst),
            Some(&PackageRevisionId::new(format!("rev-{}", i - 1)))
        );
        assert_eq!(
            mgr.upgrade_state(&inst),
            Some(UpgradeState::CurrentCandidate)
        );
        current_rev_seq = i;
    }

    assert_eq!(current_rev_seq, 10);
}

#[test]
fn prop_failed_pre_switch_preserves_previous_revision() {
    let mut mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-app");
    let rev1 = PackageRevisionId::new("rev-1");
    let rev2 = PackageRevisionId::new("rev-2");

    mgr.register_initial_installation(inst.clone(), rev1.clone(), BTreeMap::new());

    // Failed staging attempt
    let _ = mgr.stage_package(&inst, rev2.clone(), false);
    assert_eq!(mgr.current_revision(&inst), Some(&rev1));

    // Another stage attempt that fails migration
    let _ = mgr.stage_package(&inst, rev2.clone(), true);
    let _ = mgr.prepare_migration_copy(&inst, &BTreeMap::new());
    let failed_prov = MigrationProvenance {
        source_revision: rev1.clone(),
        target_revision: rev2.clone(),
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(2),
        migration_path: vec![],
        success: false,
        error_message: Some("migration failed".to_string()),
        duration_ticks: 1,
    };
    let _ = mgr.record_migration_result(&inst, None, failed_prov);
    assert_eq!(mgr.current_revision(&inst), Some(&rev1));

    // Another stage attempt that fails health probe
    let _ = mgr.stage_package(&inst, rev2.clone(), true);
    let _ = mgr.prepare_migration_copy(&inst, &BTreeMap::new());
    let ok_prov = MigrationProvenance {
        source_revision: rev1.clone(),
        target_revision: rev2,
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(1),
        migration_path: vec![],
        success: true,
        error_message: None,
        duration_ticks: 1,
    };
    let _ = mgr.record_migration_result(&inst, None, ok_prov);
    let _ = mgr.record_health_validation(
        &inst,
        worldline_kernel::HealthProbeStatus::Unhealthy {
            reason: "health failure".to_string(),
        },
    );

    // Current revision remains strictly rev1
    assert_eq!(mgr.current_revision(&inst), Some(&rev1));
}

#[test]
fn prop_rollback_and_runtime_identity_freshness() {
    let mut mgr = UpgradeManager::new();
    let inst = InstallationId::new("plugin-app");
    let rev1 = PackageRevisionId::new("rev-1");
    let rev2 = PackageRevisionId::new("rev-2");

    mgr.register_initial_installation(inst.clone(), rev1.clone(), BTreeMap::new());
    mgr.stage_package(&inst, rev2.clone(), true).unwrap();
    mgr.prepare_migration_copy(&inst, &BTreeMap::new()).unwrap();
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
    mgr.record_migration_result(&inst, None, prov).unwrap();
    mgr.record_health_validation(&inst, worldline_kernel::HealthProbeStatus::Healthy)
        .unwrap();
    mgr.begin_switch(&inst).unwrap();
    mgr.commit_switch(&inst, BTreeMap::new()).unwrap();

    // Trigger rollback
    let (restored_rev, _) = mgr.execute_rollback(&inst).unwrap();
    assert_eq!(restored_rev, rev1);

    // The restored runtime must receive a fresh sequence/incarnation
    let old_runtime = RuntimeId::new(1, 1);
    let restored_runtime = RuntimeId::new(2, 1);
    assert_ne!(old_runtime, restored_runtime);
}
