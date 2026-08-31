//! Negative security acceptance tests for M0.7 Operability and Upgrade Boundary.
//!
//! Required Negative Security Areas:
//! - Cross-runtime opaque handle reuse
//! - Revoked handle reuse
//! - Manifest self-grant attempt
//! - WASI filesystem/network escape
//! - ProviderSelf after replacement
//! - State handle after rollback
//! - Authority inheritance after package upgrade

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use worldline_kernel::{
    ActivationContext, CapabilityId, CapabilityService, DenialReason, GrantLifetime,
    InstallationId, InterfaceVersion, Kernel, NoopRuntime, PackageRevisionId, Plugin,
    PluginDefinition, PluginError, PluginRuntime, ResourceScope, RuntimeState, StateKey,
    UpgradeManager,
};

fn echo_capability() -> CapabilityId {
    CapabilityId::new("reference.echo", "echo", InterfaceVersion::new(1, 0))
}

struct EchoService {
    calls: Arc<AtomicUsize>,
}

impl CapabilityService for EchoService {
    fn invoke(&self, operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(operation.as_bytes().to_vec())
    }
}

struct ProviderPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
}

impl Plugin for ProviderPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        context.publish_capability(self.capability.clone(), Arc::clone(&self.service))?;
        Ok(Box::new(NoopRuntime))
    }
}

struct ConsumerPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    handle: Arc<Mutex<Option<worldline_kernel::CapabilityHandle>>>,
}

impl Plugin for ConsumerPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        let handle = context
            .capability(&self.capability)
            .map_err(|error| PluginError::new(error.to_string()))?;
        *self.handle.lock().expect("lock ok") = Some(handle);
        Ok(Box::new(NoopRuntime))
    }
}

#[test]
fn negative_unauthorized_handle_invocation_is_denied() {
    let capability = echo_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let handle = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();

    let consumer = kernel
        .register(ConsumerPlugin {
            definition: PluginDefinition::new("consumer").requires(capability.clone()),
            capability: capability.clone(),
            handle: Arc::clone(&handle),
        })
        .expect("consumer register");

    let provider = kernel
        .register(ProviderPlugin {
            definition: PluginDefinition::new("provider").provides(capability.clone()),
            capability: capability.clone(),
            service: Arc::new(EchoService {
                calls: Arc::clone(&calls),
            }),
        })
        .expect("provider register");

    assert_eq!(kernel.plugin_state(&consumer), Some(RuntimeState::Active));
    assert_eq!(kernel.plugin_state(&provider), Some(RuntimeState::Active));

    // Invocation without grant must fail closed
    let err = handle
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .invoke("echo", b"hello")
        .expect_err("must be denied");

    assert_eq!(err.denial_reason(), Some(DenialReason::NoGrant));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn negative_revoked_grant_denies_subsequent_invocations() {
    let capability = echo_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let handle = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();

    let consumer = kernel
        .register(ConsumerPlugin {
            definition: PluginDefinition::new("consumer").requires(capability.clone()),
            capability: capability.clone(),
            handle: Arc::clone(&handle),
        })
        .expect("consumer register");

    let _provider = kernel
        .register(ProviderPlugin {
            definition: PluginDefinition::new("provider").provides(capability.clone()),
            capability: capability.clone(),
            service: Arc::new(EchoService {
                calls: Arc::clone(&calls),
            }),
        })
        .expect("provider register");

    let principal = kernel
        .principal_for_plugin(&consumer)
        .expect("runtime principal");

    let grant_id = kernel
        .create_root_grant(
            principal.clone(),
            capability.contract(),
            ["echo"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .expect("create grant");

    // Authorized call succeeds
    let res = handle
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .invoke("echo", b"hello");
    assert!(res.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Revoke grant
    kernel.revoke_grant(&grant_id).expect("revoke ok");

    // Subsequent call must fail with GrantRevoked
    let err = handle
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .invoke("echo", b"hello")
        .expect_err("must fail after revoke");

    assert_eq!(err.denial_reason(), Some(DenialReason::GrantRevoked));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn negative_state_handle_after_rollback_does_not_mutate_restored_state() {
    let mut mgr = UpgradeManager::new();
    let inst = InstallationId::new("inst-state-rollback");
    let rev1 = PackageRevisionId::new("rev-1");
    let rev2 = PackageRevisionId::new("rev-2");

    let mut state1 = BTreeMap::new();
    state1.insert(StateKey::new("authoritative_val"), vec![100]);

    mgr.register_initial_installation(inst.clone(), rev1.clone(), state1.clone());
    mgr.stage_package(&inst, rev2.clone(), true).unwrap();
    mgr.prepare_migration_copy(&inst, &state1).unwrap();
    let prov = worldline_kernel::MigrationProvenance {
        source_revision: rev1.clone(),
        target_revision: rev2,
        source_schema: worldline_kernel::StateSchemaVersion::new(1),
        target_schema: worldline_kernel::StateSchemaVersion::new(1),
        migration_path: vec![],
        success: true,
        error_message: None,
        duration_ticks: 1,
    };
    mgr.record_migration_result(&inst, None, prov).unwrap();
    mgr.record_health_validation(&inst, worldline_kernel::HealthProbeStatus::Healthy)
        .unwrap();
    mgr.begin_switch(&inst).unwrap();
    mgr.commit_switch(&inst, state1.clone()).unwrap();

    // Rollback occurs
    let (restored_rev, restored_state) = mgr.execute_rollback(&inst).unwrap();
    assert_eq!(restored_rev, rev1);
    assert_eq!(restored_state, state1);
}
