use std::{
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use worldline_kernel::{
    ActivationContext, ActivationMode, CapabilityError, CapabilityId, CapabilityService,
    InterfaceVersion, Kernel, LifecycleContext, NoopRuntime, Plugin, PluginDefinition, PluginError,
    PluginRuntime, ResourceId, ResourceScope, RestartPolicy, RuntimeCriticality,
    RuntimeFailureClass, RuntimeLaunchPolicy, RuntimeState, StartupBudget, StateSchemaVersion,
    TrajectoryEventKind,
};

fn capability(name: &str, version: InterfaceVersion) -> CapabilityId {
    CapabilityId::new("worldline.runtime-v1", name, version)
}

fn wait_until(kernel: &mut Kernel, mut predicate: impl FnMut(&Kernel) -> bool) {
    for _ in 0..250 {
        let _ = kernel.poll_lifecycle();
        if predicate(kernel) {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("lifecycle condition did not become true before the test deadline");
}

struct EchoService {
    value: &'static str,
}

impl CapabilityService for EchoService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation != "echo" {
            return Err(format!("unsupported operation '{operation}'"));
        }
        Ok(format!("{}:{}", self.value, String::from_utf8_lossy(payload)).into_bytes())
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

fn provider(plugin: &str, provided: CapabilityId, value: &'static str) -> ProviderPlugin {
    ProviderPlugin {
        definition: PluginDefinition::new(plugin).provides(provided.clone()),
        capability: provided,
        service: Arc::new(EchoService { value }),
    }
}

struct CounterPlugin {
    definition: PluginDefinition,
}

impl Plugin for CounterPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        let current = context
            .state()
            .get("activation-count")
            .map_err(|error| PluginError::new(error.to_string()))?
            .and_then(|value| String::from_utf8(value).ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_default();
        let mut transaction = context
            .state()
            .transaction()
            .map_err(|error| PluginError::new(error.to_string()))?;
        transaction
            .put("activation-count", (current + 1).to_string().as_bytes())
            .map_err(|error| PluginError::new(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| PluginError::new(error.to_string()))?;
        Ok(Box::new(NoopRuntime))
    }
}

struct LeaseCapturePlugin {
    definition: PluginDefinition,
    slot: Arc<Mutex<Option<worldline_kernel::RuntimeStateHandle>>>,
}

impl Plugin for LeaseCapturePlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        *self.slot.lock().expect("lease slot lock is not poisoned") = Some(context.state().clone());
        Ok(Box::new(NoopRuntime))
    }
}

struct Gate {
    entered: Barrier,
    release: AtomicBool,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Barrier::new(2),
            release: AtomicBool::new(false),
        })
    }

    fn wait(&self) {
        self.entered.wait();
        while !self.release.load(Ordering::Acquire) {
            thread::yield_now();
        }
    }

    fn release(&self) {
        self.release.store(true, Ordering::Release);
    }
}

struct BlockingPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
    gate: Arc<Gate>,
}

impl Plugin for BlockingPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        self.gate.wait();
        context.publish_capability(self.capability.clone(), Arc::clone(&self.service))?;
        Ok(Box::new(NoopRuntime))
    }
}

struct TeardownRuntime {
    gate: Arc<Gate>,
}

impl PluginRuntime for TeardownRuntime {
    fn deactivate(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    fn deactivate_with_context(&mut self, _context: &LifecycleContext) -> Result<(), PluginError> {
        self.gate.wait();
        Ok(())
    }
}

struct TeardownPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
    gate: Arc<Gate>,
}

impl Plugin for TeardownPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        context.publish_capability(self.capability.clone(), Arc::clone(&self.service))?;
        Ok(Box::new(TeardownRuntime {
            gate: Arc::clone(&self.gate),
        }))
    }
}

struct PartialFailurePlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
}

impl Plugin for PartialFailurePlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        context.publish_capability(self.capability.clone(), Arc::clone(&self.service))?;
        Err(PluginError::new("intentional activation failure"))
    }
}

struct PanicPlugin {
    definition: PluginDefinition,
}

impl Plugin for PanicPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        _context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        panic!("intentional activation panic")
    }
}

struct FailingPlugin {
    definition: PluginDefinition,
}

impl Plugin for FailingPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        _context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        Err(PluginError::new("failure for restart policy"))
    }
}

struct SelfService {
    target: CapabilityId,
}

impl CapabilityService for SelfService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(b"direct".to_vec())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        context
            .invoke_self(
                self.target.clone(),
                "echo",
                ResourceId::root(self.target.namespace()),
                payload,
            )
            .map_err(|error| error.to_string())
    }
}

fn install(kernel: &mut Kernel, plugin: &str) -> worldline_kernel::InstallationId {
    kernel
        .create_installation(plugin, StateSchemaVersion::default())
        .expect("test installation must be created")
}

fn lazy_policy() -> RuntimeLaunchPolicy {
    RuntimeLaunchPolicy::lazy(RuntimeCriticality::Required)
}

fn service(value: &'static str) -> Arc<dyn CapabilityService> {
    Arc::new(EchoService { value })
}

#[test]
fn runtime_id_is_explicit_and_runtime_principal_contains_it() {
    let cap = capability("identity", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let plugin = kernel
        .register(provider("identity-provider", cap, "identity"))
        .expect("provider must register");
    let installation = kernel
        .installation_id_for_plugin(&plugin)
        .expect("default installation must exist");
    let runtime = kernel
        .runtime_id_for_installation(&installation)
        .expect("runtime must be active");
    let metadata = kernel
        .runtime_metadata(&runtime)
        .expect("runtime metadata must be retained");

    assert_eq!(metadata.state(), RuntimeState::Active);
    assert_eq!(metadata.installation_id(), &installation);
    assert_eq!(metadata.runtime_id(), runtime);
    assert!(metadata.principal().as_str().contains(&runtime.to_string()));
    assert!(runtime.value() > 0);
}

#[test]
fn restart_allocates_new_runtime_and_lifecycle_scope() {
    let cap = capability("restart", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let plugin = kernel
        .register(provider("restart-provider", cap, "first"))
        .expect("provider must register");
    let installation = kernel
        .installation_id_for_plugin(&plugin)
        .expect("installation must exist");
    let old_runtime = kernel
        .runtime_id_for_installation(&installation)
        .expect("old runtime must exist");
    let old_scope = kernel
        .runtime_metadata(&old_runtime)
        .expect("old metadata must exist")
        .lifecycle_scope_id();

    kernel
        .unregister(&plugin)
        .expect("unregister must stop the old runtime");
    kernel
        .register_for_installation(
            provider(
                "restart-provider",
                capability("restart", InterfaceVersion::new(1, 0)),
                "second",
            ),
            &installation,
        )
        .expect("same installation must be registerable again");
    let new_runtime = kernel
        .runtime_id_for_installation(&installation)
        .expect("new runtime must exist");
    let new_scope = kernel
        .runtime_metadata(&new_runtime)
        .expect("new metadata must exist")
        .lifecycle_scope_id();

    assert_ne!(old_runtime, new_runtime);
    assert_ne!(old_scope, new_scope);
    assert_eq!(
        kernel
            .runtime_metadata(&old_runtime)
            .expect("old runtime metadata is retained")
            .state(),
        RuntimeState::Stopped
    );
}

#[test]
fn same_definition_supports_two_active_installations() {
    let cap = capability("multi", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let first = install(&mut kernel, "multi-provider");
    let second = install(&mut kernel, "multi-provider");
    kernel
        .register_for_installation(provider("multi-provider", cap.clone(), "first"), &first)
        .expect("first installation must activate");
    kernel
        .register_for_installation(provider("multi-provider", cap.clone(), "second"), &second)
        .expect("second installation must activate");

    let first_runtime = kernel
        .runtime_id_for_installation(&first)
        .expect("first runtime must exist");
    let second_runtime = kernel
        .runtime_id_for_installation(&second)
        .expect("second runtime must exist");
    let discovered = kernel.discover_capabilities_for(&cap);

    assert_ne!(first_runtime, second_runtime);
    assert_eq!(discovered.len(), 2);
    assert!(discovered.iter().all(|entry| entry.runtime_id().is_some()));
    assert_eq!(
        kernel
            .select_provider(&cap)
            .expect("one provider must be selected")
            .1
            .compatible_candidate_count(),
        2
    );
}

#[test]
fn removing_one_installation_keeps_the_other_publication() {
    let cap = capability("replacement", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let first = install(&mut kernel, "replacement-provider");
    let second = install(&mut kernel, "replacement-provider");
    kernel
        .register_for_installation(
            provider("replacement-provider", cap.clone(), "first"),
            &first,
        )
        .expect("first provider must activate");
    kernel
        .register_for_installation(
            provider("replacement-provider", cap.clone(), "second"),
            &second,
        )
        .expect("second provider must activate");
    let second_runtime = kernel
        .runtime_id_for_installation(&second)
        .expect("second runtime must exist");

    kernel
        .unregister_installation(&first)
        .expect("first installation must unregister");
    let selected = kernel
        .select_provider(&cap)
        .expect("the remaining provider must stay selectable")
        .0;

    assert_eq!(selected.runtime_id(), second_runtime);
    assert!(kernel.is_capability_available(&cap));
    assert_eq!(
        kernel
            .plugin_state_for_installation(&second)
            .expect("second installation remains registered"),
        RuntimeState::Active
    );
}

#[test]
fn restart_preserves_installation_state_but_not_runtime_authority() {
    let protected = capability("state-authority", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let plugin = kernel
        .register(CounterPlugin {
            definition: PluginDefinition::new("counter-plugin"),
        })
        .expect("counter must register");
    let installation = kernel
        .installation_id_for_plugin(&plugin)
        .expect("installation must exist");
    let old_runtime = kernel
        .runtime_id_for_installation(&installation)
        .expect("old runtime must exist");
    let old_principal = kernel
        .principal_for_runtime(&old_runtime)
        .expect("old principal must exist");
    let grant = kernel
        .create_root_grant(
            old_principal.clone(),
            protected.contract(),
            ["echo"],
            ResourceScope::Any,
            false,
            worldline_kernel::GrantLifetime::Persistent,
        )
        .expect("old runtime grant must be created");
    assert_eq!(
        kernel
            .state_handle(&installation)
            .expect("state must be available")
            .get("activation-count")
            .expect("state read must succeed")
            .as_deref(),
        Some(b"1".as_slice())
    );

    kernel
        .unregister(&plugin)
        .expect("unregister must preserve installation state");
    assert!(!kernel.is_grant_active(&grant));
    kernel
        .register_for_installation(
            CounterPlugin {
                definition: PluginDefinition::new("counter-plugin"),
            },
            &installation,
        )
        .expect("the same installation must restart");
    let new_runtime = kernel
        .runtime_id_for_installation(&installation)
        .expect("new runtime must exist");
    let new_principal = kernel
        .principal_for_runtime(&new_runtime)
        .expect("new principal must exist");
    assert_ne!(old_runtime, new_runtime);
    assert_ne!(old_principal, new_principal);
    assert_eq!(
        kernel
            .state_handle(&installation)
            .expect("state must remain available")
            .get("activation-count")
            .expect("state read must succeed")
            .as_deref(),
        Some(b"2".as_slice())
    );
    assert!(matches!(
        kernel
            .capability_for(new_principal, protected)
            .expect("new runtime principal is registered")
            .invoke("echo", b"authority"),
        Err(CapabilityError::Denied { .. })
    ));
}

#[test]
fn terminated_runtime_lease_rejects_later_state_access() {
    let slot = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();
    let plugin = kernel
        .register(LeaseCapturePlugin {
            definition: PluginDefinition::new("lease-capture"),
            slot: Arc::clone(&slot),
        })
        .expect("lease capture must register");
    let handle = slot
        .lock()
        .expect("lease slot lock is not poisoned")
        .clone()
        .expect("plugin must capture its runtime handle");

    kernel
        .unregister(&plugin)
        .expect("unregister must revoke the runtime lease");
    assert!(handle.get("after-stop").is_err());
}

#[test]
fn pending_activation_does_not_block_unrelated_activation() {
    let slow_cap = capability("slow", InterfaceVersion::new(1, 0));
    let fast_cap = capability("fast", InterfaceVersion::new(1, 0));
    let gate = Gate::new();
    let mut kernel = Kernel::new();
    let slow_installation = install(&mut kernel, "slow-provider");
    kernel
        .register_for_installation_with_policy(
            BlockingPlugin {
                definition: PluginDefinition::new("slow-provider").provides(slow_cap.clone()),
                capability: slow_cap,
                service: service("slow"),
                gate: Arc::clone(&gate),
            },
            &slow_installation,
            lazy_policy(),
        )
        .expect("slow plugin must register lazily");
    let operation = kernel
        .begin_activation_for_installation(&slow_installation)
        .expect("slow activation must start");
    gate.entered.wait();

    let fast = kernel
        .register(provider("fast-provider", fast_cap.clone(), "fast"))
        .expect("unrelated provider must activate");
    let fast_installation = kernel
        .installation_id_for_plugin(&fast)
        .expect("fast installation must exist");
    assert_eq!(
        kernel.plugin_state_for_installation(&fast_installation),
        Some(RuntimeState::Active)
    );
    assert!(kernel.is_capability_available(&fast_cap));

    gate.release();
    wait_until(&mut kernel, |kernel| {
        kernel.plugin_state_for_installation(&slow_installation) == Some(RuntimeState::Active)
    });
    assert_eq!(
        operation.runtime_id(),
        kernel
            .runtime_id_for_installation(&slow_installation)
            .expect("slow runtime must be active")
    );
}

#[test]
fn cancelled_activation_rejects_late_success_and_is_idempotent() {
    let cap = capability("cancel", InterfaceVersion::new(1, 0));
    let gate = Gate::new();
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "cancel-provider");
    kernel
        .register_for_installation_with_policy(
            BlockingPlugin {
                definition: PluginDefinition::new("cancel-provider").provides(cap.clone()),
                capability: cap.clone(),
                service: service("cancel"),
                gate: Arc::clone(&gate),
            },
            &installation,
            lazy_policy(),
        )
        .expect("cancel plugin must register lazily");
    let operation = kernel
        .begin_activation_for_installation(&installation)
        .expect("activation must start");
    gate.entered.wait();

    assert!(
        kernel
            .cancel_lifecycle(&operation)
            .expect("cancel must succeed")
    );
    assert!(
        !kernel
            .cancel_lifecycle(&operation)
            .expect("repeat cancel must succeed")
    );
    assert_eq!(
        kernel.plugin_state_for_installation(&installation),
        Some(RuntimeState::Cancelled)
    );
    gate.release();

    let mut stale = false;
    for _ in 0..250 {
        let report = kernel.poll_lifecycle();
        stale |= !report.stale_completions.is_empty();
        if stale {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(stale, "late completion must be observable as rejected");
    assert!(!kernel.is_capability_available(&cap));
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::LifecycleCompletionRejected {
                classification: RuntimeFailureClass::StaleCompletion,
                ..
            }
        )
    }));
}

#[test]
fn activation_deadline_marks_runtime_hung_without_publishing() {
    let cap = capability("hung", InterfaceVersion::new(1, 0));
    let gate = Gate::new();
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "hung-provider");
    let policy = lazy_policy().with_activation_deadline(Duration::from_millis(1));
    kernel
        .register_for_installation_with_policy(
            BlockingPlugin {
                definition: PluginDefinition::new("hung-provider").provides(cap.clone()),
                capability: cap.clone(),
                service: service("hung"),
                gate: Arc::clone(&gate),
            },
            &installation,
            policy,
        )
        .expect("hung plugin must register lazily");
    let operation = kernel
        .begin_activation_for_installation(&installation)
        .expect("activation must start");
    gate.entered.wait();
    thread::sleep(Duration::from_millis(8));

    let report = kernel.poll_lifecycle();
    assert!(report.hung_runtime_ids.contains(&operation.runtime_id()));
    assert_eq!(
        kernel
            .runtime_metadata(&operation.runtime_id())
            .expect("hung metadata must be retained")
            .state(),
        RuntimeState::Hung
    );
    assert!(!kernel.is_capability_available(&cap));

    gate.release();
    wait_until(&mut kernel, |kernel| {
        kernel
            .trajectory()
            .iter()
            .any(|event| matches!(event.kind(), TrajectoryEventKind::LifecycleCompletionRejected { runtime_id, .. } if *runtime_id == operation.runtime_id()))
    });
    assert!(!kernel.is_capability_available(&cap));
}

#[test]
fn async_deactivation_unpublishes_before_teardown_and_allows_unrelated_work() {
    let cap = capability("teardown", InterfaceVersion::new(1, 0));
    let independent = capability("teardown-independent", InterfaceVersion::new(1, 0));
    let gate = Gate::new();
    let mut kernel = Kernel::new();
    let plugin = TeardownPlugin {
        definition: PluginDefinition::new("teardown-provider").provides(cap.clone()),
        capability: cap.clone(),
        service: service("teardown"),
        gate: Arc::clone(&gate),
    };
    kernel
        .register(plugin)
        .expect("teardown provider must activate");
    let installation = kernel
        .installations_for_plugin(&"teardown-provider".into())
        .first()
        .expect("installation must exist")
        .installation_id()
        .clone();
    let operation = kernel
        .begin_deactivation_for_installation(&installation)
        .expect("deactivation must start");
    gate.entered.wait();

    assert_eq!(
        kernel.plugin_state_for_installation(&installation),
        Some(RuntimeState::Deactivating)
    );
    assert!(!kernel.is_capability_available(&cap));
    kernel
        .register(provider(
            "teardown-independent",
            independent.clone(),
            "independent",
        ))
        .expect("unrelated provider must activate while teardown is pending");
    assert!(kernel.is_capability_available(&independent));

    gate.release();
    wait_until(&mut kernel, |kernel| {
        kernel.plugin_state_for_installation(&installation) == Some(RuntimeState::Stopped)
    });
    assert_eq!(
        operation.runtime_id(),
        kernel
            .runtime_metadata(&operation.runtime_id())
            .expect("metadata retained")
            .runtime_id()
    );
}

#[test]
fn activation_error_does_not_publish_partial_capabilities() {
    let cap = capability("partial-error", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let plugin = kernel
        .register(PartialFailurePlugin {
            definition: PluginDefinition::new("partial-error").provides(cap.clone()),
            capability: cap.clone(),
            service: service("partial"),
        })
        .expect("registration itself must remain successful");

    assert_eq!(kernel.plugin_state(&plugin), Some(RuntimeState::Failed));
    assert!(!kernel.is_capability_available(&cap));
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::RuntimeFailed {
                classification: RuntimeFailureClass::PluginError,
                ..
            }
        )
    }));
}

#[test]
fn activation_panic_is_contained_as_crashed() {
    let cap = capability("panic", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let plugin = kernel
        .register(PanicPlugin {
            definition: PluginDefinition::new("panic-runtime").provides(cap.clone()),
        })
        .expect("panic registration must be contained");

    assert_eq!(kernel.plugin_state(&plugin), Some(RuntimeState::Crashed));
    assert!(!kernel.is_capability_available(&cap));
    assert!(
        kernel
            .trajectory()
            .iter()
            .any(|event| { matches!(event.kind(), TrajectoryEventKind::RuntimeCrashed { .. }) })
    );
}

#[test]
fn restart_policy_reaches_quarantine_and_recovery_uses_new_runtime() {
    let mut kernel = Kernel::new();
    let policy = RuntimeLaunchPolicy::required_eager()
        .with_restart_policy(RestartPolicy::on_failure(2).with_quarantine_after(2));
    let plugin = kernel
        .register_with_policy(
            FailingPlugin {
                definition: PluginDefinition::new("quarantine-plugin"),
            },
            policy,
        )
        .expect("failing plugin registration must return its definition id");
    let installation = kernel
        .installation_id_for_plugin(&plugin)
        .expect("installation must exist");
    let quarantined_runtime = kernel
        .runtime_id_for_installation(&installation)
        .expect("quarantined runtime metadata must be retained");

    assert_eq!(
        kernel.plugin_state_for_installation(&installation),
        Some(RuntimeState::Quarantined)
    );
    let _ = kernel.reconcile();
    assert_eq!(
        kernel.runtime_id_for_installation(&installation),
        Some(quarantined_runtime)
    );

    let _ = kernel
        .recover_installation(&installation)
        .expect("explicit recovery must be accepted");
    let recovered_runtime = kernel
        .runtime_id_for_installation(&installation)
        .expect("recovery must create a runtime attempt");
    assert_ne!(quarantined_runtime, recovered_runtime);
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::RuntimeRestartAttempted { .. }
        )
    }));
}

#[test]
fn optional_failure_degrades_without_rolling_back_independent_runtime() {
    let independent_cap = capability("independent", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let independent = kernel
        .register(provider(
            "independent-runtime",
            independent_cap,
            "independent",
        ))
        .expect("independent provider must activate");
    let independent_installation = kernel
        .installation_id_for_plugin(&independent)
        .expect("independent installation must exist");
    kernel
        .register_with_policy(
            FailingPlugin {
                definition: PluginDefinition::new("optional-failure"),
            },
            RuntimeLaunchPolicy::optional_eager(),
        )
        .expect("optional failure must not reject host registration");

    let report = kernel.reconcile();
    assert!(report.degraded);
    assert!(!report.healthy);
    assert!(
        report
            .failed
            .iter()
            .any(|id| id.as_str() == "optional-failure")
    );
    assert_eq!(
        kernel.plugin_state_for_installation(&independent_installation),
        Some(RuntimeState::Active)
    );
}

#[test]
fn lazy_installation_is_discoverable_without_becoming_authority() {
    let cap = capability("lazy-discovery", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "lazy-discovery-provider");
    kernel
        .register_for_installation_with_policy(
            provider("lazy-discovery-provider", cap.clone(), "lazy"),
            &installation,
            lazy_policy(),
        )
        .expect("lazy provider must register");
    let caller = kernel
        .register_principal_id("lazy-caller", worldline_kernel::PrincipalKind::User)
        .expect("caller principal must register");
    let descriptor = kernel
        .discover_capabilities_for(&cap)
        .into_iter()
        .next()
        .expect("declared lazy capability must be discoverable");

    assert_eq!(descriptor.runtime_id(), None);
    assert_eq!(descriptor.activation_mode(), ActivationMode::Lazy);
    assert_eq!(
        kernel.plugin_state_for_installation(&installation),
        Some(RuntimeState::Registered)
    );
    assert!(matches!(
        kernel
            .capability_for(caller, cap)
            .expect("caller handle can be constructed")
            .invoke("echo", b"before-demand"),
        Err(CapabilityError::Denied { .. })
    ));
}

#[test]
fn lazy_demand_prefers_highest_compatible_minor_without_granting_caller() {
    let required = capability("lazy-versioned", InterfaceVersion::new(1, 0));
    let higher = capability("lazy-versioned", InterfaceVersion::new(1, 2));
    let mut kernel = Kernel::new();
    let low_installation = install(&mut kernel, "lazy-low");
    let high_installation = install(&mut kernel, "lazy-high");
    kernel
        .register_for_installation_with_policy(
            provider("lazy-low", required.clone(), "low"),
            &low_installation,
            lazy_policy(),
        )
        .expect("low lazy provider must register");
    kernel
        .register_for_installation_with_policy(
            provider("lazy-high", higher.clone(), "high"),
            &high_installation,
            lazy_policy(),
        )
        .expect("high lazy provider must register");

    let _ = kernel.demand_capability(required.clone());
    let selected = kernel
        .select_provider(&required)
        .expect("demand must activate one compatible lazy provider");
    let caller = kernel
        .register_principal_id(
            "lazy-versioned-caller",
            worldline_kernel::PrincipalKind::User,
        )
        .expect("caller principal must register");

    assert_eq!(selected.1.compatible_candidate_count(), 1);
    assert_eq!(selected.0.capability().interface_version().minor(), 2);
    assert_eq!(
        kernel.plugin_state_for_installation(&high_installation),
        Some(RuntimeState::Active)
    );
    assert_eq!(
        kernel.plugin_state_for_installation(&low_installation),
        Some(RuntimeState::Registered)
    );
    assert!(matches!(
        kernel
            .capability_for(caller, required)
            .expect("caller handle can be constructed")
            .invoke("echo", b"no-authority"),
        Err(CapabilityError::Denied { .. })
    ));
}

#[test]
fn startup_budget_is_observable_and_does_not_classify_deferred_work_as_failure() {
    let first_cap = capability("budget-first", InterfaceVersion::new(1, 0));
    let second_cap = capability("budget-second", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let first = kernel
        .register(provider("budget-first", first_cap, "first"))
        .expect("first provider must activate");
    let second = kernel
        .register(provider("budget-second", second_cap, "second"))
        .expect("second provider must activate");
    let first_installation = kernel
        .installation_id_for_plugin(&first)
        .expect("first installation must exist");
    let second_installation = kernel
        .installation_id_for_plugin(&second)
        .expect("second installation must exist");
    kernel.stop();

    let report =
        kernel.start_with_budget(StartupBudget::unlimited().with_max_simultaneous_activations(1));
    assert!(report.startup_budget_exhausted);
    assert!(report.failed.is_empty());
    assert!(report.crashed.is_empty());
    assert_eq!(
        kernel.plugin_state_for_installation(&first_installation),
        Some(RuntimeState::Active)
    );
    assert_eq!(
        kernel.plugin_state_for_installation(&second_installation),
        Some(RuntimeState::Registered)
    );

    let final_report = kernel.reconcile();
    assert!(!final_report.startup_budget_exhausted);
    assert_eq!(
        kernel.plugin_state_for_installation(&second_installation),
        Some(RuntimeState::Active)
    );
}

#[test]
fn provider_selection_is_deterministic_and_reports_version_negotiation() {
    let required = capability("selection", InterfaceVersion::new(1, 0));
    let higher = capability("selection", InterfaceVersion::new(1, 3));
    let mut kernel = Kernel::new();
    let low = install(&mut kernel, "selection-low");
    let high = install(&mut kernel, "selection-high");
    kernel
        .register_for_installation(provider("selection-low", required.clone(), "low"), &low)
        .expect("low provider must activate");
    kernel
        .register_for_installation(provider("selection-high", higher.clone(), "high"), &high)
        .expect("high provider must activate");

    let first = kernel
        .select_provider(&required)
        .expect("provider must exist");
    let second = kernel
        .select_provider(&required)
        .expect("provider must exist");
    assert_eq!(first, second);
    assert_eq!(first.1.compatible_candidate_count(), 2);
    assert_eq!(first.1.negotiated_capability(), Some(&higher));
    assert!(first.1.policy().contains("highest-compatible-minor"));
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(event.kind(), TrajectoryEventKind::CapabilityVersionNegotiated { selected: Some(selected), .. } if selected == &higher)
    }));
}

#[test]
fn incompatible_major_version_is_not_selected_or_reported_available() {
    let required = capability("major", InterfaceVersion::new(1, 0));
    let provided = capability("major", InterfaceVersion::new(2, 0));
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "major-provider");
    kernel
        .register_for_installation(provider("major-provider", provided, "major"), &installation)
        .expect("provider must activate");

    assert!(kernel.select_provider(&required).is_none());
    assert!(!kernel.is_capability_available(&required));
    assert!(kernel.discover_capabilities_for(&required).is_empty());
}

#[test]
fn provider_self_authority_stays_bound_to_replaced_runtime() {
    let target = capability("self-target", InterfaceVersion::new(1, 0));
    let entry = capability("self-entry", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    kernel
        .register(provider("self-target-provider", target.clone(), "target"))
        .expect("target provider must activate");
    let entry_installation = install(&mut kernel, "self-entry-provider");
    kernel
        .register_for_installation(
            ProviderPlugin {
                definition: PluginDefinition::new("self-entry-provider").provides(entry.clone()),
                capability: entry.clone(),
                service: Arc::new(SelfService {
                    target: target.clone(),
                }),
            },
            &entry_installation,
        )
        .expect("entry provider must activate");
    let entry_runtime = kernel
        .runtime_id_for_installation(&entry_installation)
        .expect("entry runtime must exist");
    let entry_principal = kernel
        .principal_for_runtime(&entry_runtime)
        .expect("entry principal must exist");
    kernel
        .create_root_grant(
            entry_principal,
            target.contract(),
            ["echo"],
            ResourceScope::Any,
            false,
            worldline_kernel::GrantLifetime::Persistent,
        )
        .expect("provider self grant must be created");
    let caller = kernel
        .register_principal_id("self-caller", worldline_kernel::PrincipalKind::User)
        .expect("caller principal must register");
    kernel
        .create_root_grant(
            caller.clone(),
            entry.contract(),
            ["echo"],
            ResourceScope::Any,
            false,
            worldline_kernel::GrantLifetime::Persistent,
        )
        .expect("caller entry grant must be created");
    let handle = kernel
        .capability_for(caller.clone(), entry.clone())
        .expect("caller entry handle must be constructible");
    assert_eq!(
        handle
            .invoke("echo", b"before-replacement")
            .expect("self call must succeed"),
        b"target:before-replacement"
    );

    kernel
        .unregister_installation(&entry_installation)
        .expect("entry runtime must be replaceable");
    kernel
        .register_for_installation(
            ProviderPlugin {
                definition: PluginDefinition::new("self-entry-provider").provides(entry.clone()),
                capability: entry,
                service: Arc::new(SelfService { target }),
            },
            &entry_installation,
        )
        .expect("replacement entry runtime must activate");
    assert!(matches!(
        handle.invoke("echo", b"after-replacement"),
        Err(CapabilityError::InvocationFailed { .. })
    ));
}
