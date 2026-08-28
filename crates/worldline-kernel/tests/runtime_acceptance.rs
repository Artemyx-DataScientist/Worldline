use std::sync::{Arc, Mutex};

use worldline_kernel::{
    ActivationContext, CapabilityHandle, CapabilityId, CapabilityService, EffectCleanupError,
    InterfaceVersion, Kernel, LifecyclePhase, NoopRuntime, OwnedEffect, Plugin, PluginDefinition,
    PluginError, PluginRuntime, RuntimeState, TrajectoryEventKind,
};

fn greeting_capability() -> CapabilityId {
    CapabilityId::new("worldline.test", "greeting", InterfaceVersion::new(1, 0))
}

fn push(log: &Arc<Mutex<Vec<String>>>, entry: impl Into<String>) {
    log.lock()
        .expect("test log lock is not poisoned")
        .push(entry.into());
}

struct PrefixService {
    prefix: &'static str,
}

impl CapabilityService for PrefixService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation != "greet" {
            return Err(format!("unsupported operation '{operation}'"));
        }
        let name = String::from_utf8(payload.to_vec()).map_err(|error| error.to_string())?;
        Ok(format!("{}:{name}", self.prefix).into_bytes())
    }
}

struct RecordingRuntime {
    name: &'static str,
    log: Arc<Mutex<Vec<String>>>,
    panic_on_deactivate: bool,
}

impl PluginRuntime for RecordingRuntime {
    fn deactivate(&mut self) -> Result<(), PluginError> {
        push(&self.log, format!("{} deactivated", self.name));
        if self.panic_on_deactivate {
            panic!("{} deactivation panic", self.name);
        }
        Ok(())
    }
}

struct ProviderPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
    name: &'static str,
    log: Arc<Mutex<Vec<String>>>,
    panic_on_deactivate: bool,
}

impl Plugin for ProviderPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        push(&self.log, format!("{} activated", self.name));
        context.publish_capability(self.capability.clone(), Arc::clone(&self.service))?;
        Ok(Box::new(RecordingRuntime {
            name: self.name,
            log: Arc::clone(&self.log),
            panic_on_deactivate: self.panic_on_deactivate,
        }))
    }
}

struct ConsumerPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    handle: Arc<Mutex<Option<CapabilityHandle>>>,
    log: Arc<Mutex<Vec<String>>>,
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
        *self
            .handle
            .lock()
            .expect("consumer handle lock is not poisoned") = Some(handle);
        push(&self.log, "consumer activated");
        Ok(Box::new(RecordingRuntime {
            name: "consumer",
            log: Arc::clone(&self.log),
            panic_on_deactivate: false,
        }))
    }
}

fn provider(
    name: &'static str,
    capability: &CapabilityId,
    prefix: &'static str,
    log: &Arc<Mutex<Vec<String>>>,
) -> ProviderPlugin {
    ProviderPlugin {
        definition: PluginDefinition::new(name).provides(capability.clone()),
        capability: capability.clone(),
        service: Arc::new(PrefixService { prefix }),
        name,
        log: Arc::clone(log),
        panic_on_deactivate: false,
    }
}

fn consumer(
    capability: &CapabilityId,
    handle: &Arc<Mutex<Option<CapabilityHandle>>>,
    log: &Arc<Mutex<Vec<String>>>,
) -> ConsumerPlugin {
    ConsumerPlugin {
        definition: PluginDefinition::new("consumer").requires(capability.clone()),
        capability: capability.clone(),
        handle: Arc::clone(handle),
        log: Arc::clone(log),
    }
}

#[test]
fn pending_activation_and_provider_loss_have_deterministic_stop_order() {
    let capability = greeting_capability();
    let log = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();

    let consumer_id = kernel
        .register(consumer(&capability, &handle, &log))
        .expect("consumer registration must succeed");
    assert_eq!(
        kernel.plugin_state(&consumer_id),
        Some(RuntimeState::Pending)
    );

    let provider_id = kernel
        .register(provider("provider-a", &capability, "A", &log))
        .expect("provider registration must succeed");
    assert_eq!(
        kernel.plugin_state(&consumer_id),
        Some(RuntimeState::Active)
    );
    assert_eq!(
        kernel.plugin_state(&provider_id),
        Some(RuntimeState::Active)
    );

    kernel
        .unregister(&provider_id)
        .expect("provider removal must succeed");
    assert_eq!(
        kernel.plugin_state(&consumer_id),
        Some(RuntimeState::Pending)
    );

    let entries = log.lock().expect("test log lock is not poisoned").clone();
    assert_eq!(
        entries,
        vec![
            "provider-a activated",
            "consumer activated",
            "consumer deactivated",
            "provider-a deactivated",
        ]
    );

    let provider_loss = kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::ProviderLost { provider, .. } if provider.as_str() == "provider-a"
        )
    });
    assert!(provider_loss);
}

#[test]
fn kernel_stop_and_start_reuse_registered_plugin_definitions() {
    let capability = greeting_capability();
    let log = Arc::new(Mutex::new(Vec::new()));
    let handle_slot = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();
    let consumer_id = kernel
        .register(consumer(&capability, &handle_slot, &log))
        .expect("consumer registration must succeed");
    let provider_id = kernel
        .register(provider("provider-a", &capability, "A", &log))
        .expect("provider registration must succeed");

    kernel.stop();
    assert_eq!(
        kernel.plugin_state(&consumer_id),
        Some(RuntimeState::Stopped)
    );
    assert_eq!(
        kernel.plugin_state(&provider_id),
        Some(RuntimeState::Stopped)
    );
    assert!(!kernel.is_capability_available(&capability));
    assert_eq!(
        log.lock().expect("test log lock is not poisoned").clone(),
        vec![
            "provider-a activated",
            "consumer activated",
            "consumer deactivated",
            "provider-a deactivated",
        ]
    );

    kernel.start();
    assert_eq!(
        kernel.plugin_state(&consumer_id),
        Some(RuntimeState::Active)
    );
    assert_eq!(
        kernel.plugin_state(&provider_id),
        Some(RuntimeState::Active)
    );
}

#[test]
fn provider_replacement_keeps_consumer_contract_and_live_handle() {
    let capability = greeting_capability();
    let log = Arc::new(Mutex::new(Vec::new()));
    let handle_slot = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();

    let provider_a = kernel
        .register(provider("provider-a", &capability, "A", &log))
        .expect("provider A registration must succeed");
    let consumer_id = kernel
        .register(consumer(&capability, &handle_slot, &log))
        .expect("consumer registration must succeed");
    let handle = handle_slot
        .lock()
        .expect("consumer handle lock is not poisoned")
        .clone()
        .expect("consumer must have a capability handle");

    assert_eq!(
        String::from_utf8(handle.invoke("greet", b"Worldline").expect("A must answer"))
            .expect("response must be UTF-8"),
        "A:Worldline"
    );

    kernel
        .register(provider("provider-b", &capability, "B", &log))
        .expect("provider B registration must succeed");
    kernel
        .unregister(&provider_a)
        .expect("provider A removal must succeed");

    assert_eq!(
        kernel.plugin_state(&consumer_id),
        Some(RuntimeState::Active)
    );
    assert_eq!(
        String::from_utf8(handle.invoke("greet", b"Worldline").expect("B must answer"))
            .expect("response must be UTF-8"),
        "B:Worldline"
    );
    let entries = log.lock().expect("test log lock is not poisoned").clone();
    assert_eq!(
        entries,
        vec![
            "provider-a activated",
            "consumer activated",
            "provider-b activated",
            "provider-a deactivated",
        ]
    );
    assert!(!entries.iter().any(|entry| entry == "consumer deactivated"));
}

struct EffectPlugin {
    definition: PluginDefinition,
    log: Arc<Mutex<Vec<String>>>,
    fail_first: bool,
}

impl Plugin for EffectPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        let first_log = Arc::clone(&self.log);
        let fail_first = self.fail_first;
        context.own_effect(OwnedEffect::new("first", move || {
            push(&first_log, "first cleanup");
            if fail_first {
                Err(EffectCleanupError::new("first cleanup failed"))
            } else {
                Ok(())
            }
        }));

        let second_log = Arc::clone(&self.log);
        context.own_effect(OwnedEffect::new("second", move || {
            push(&second_log, "second cleanup");
            Ok(())
        }));
        Ok(Box::new(NoopRuntime))
    }
}

#[test]
fn owned_effects_clean_up_in_lifo_and_continue_after_failure() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut kernel = Kernel::new();
    let id = kernel
        .register(EffectPlugin {
            definition: PluginDefinition::new("effects-ok"),
            log: Arc::clone(&log),
            fail_first: false,
        })
        .expect("effect plugin registration must succeed");
    kernel
        .unregister(&id)
        .expect("effect plugin removal must succeed");
    assert_eq!(
        log.lock().expect("test log lock is not poisoned").clone(),
        vec!["second cleanup", "first cleanup"]
    );

    let failed_log = Arc::new(Mutex::new(Vec::new()));
    let mut failed_kernel = Kernel::new();
    let failed_id = failed_kernel
        .register(EffectPlugin {
            definition: PluginDefinition::new("effects-failing"),
            log: Arc::clone(&failed_log),
            fail_first: true,
        })
        .expect("failing effect plugin registration must succeed");
    failed_kernel
        .unregister(&failed_id)
        .expect("failing effect plugin removal must succeed");
    assert_eq!(
        failed_log
            .lock()
            .expect("test log lock is not poisoned")
            .clone(),
        vec!["second cleanup", "first cleanup"]
    );
    assert!(failed_kernel.trajectory().iter().any(|event| {
        matches!(event.kind(), TrajectoryEventKind::EffectCleanupFailed { effect, .. } if effect == "first")
    }));
    assert!(failed_kernel.trajectory().iter().any(|event| {
        matches!(event.kind(), TrajectoryEventKind::EffectCleaned { effect } if effect == "second")
    }));
}

struct PanickingActivationPlugin {
    definition: PluginDefinition,
    cleanup_log: Arc<Mutex<Vec<String>>>,
}

impl Plugin for PanickingActivationPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        let log = Arc::clone(&self.cleanup_log);
        context.own_effect(OwnedEffect::new("panic-effect", move || {
            push(&log, "panic effect cleaned");
            Ok(())
        }));
        panic!("activation boundary panic");
    }
}

struct PanickingRuntimePlugin {
    definition: PluginDefinition,
}

impl Plugin for PanickingRuntimePlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        _context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        Ok(Box::new(PanickingRuntime))
    }
}

struct PanickingRuntime;

impl PluginRuntime for PanickingRuntime {
    fn deactivate(&mut self) -> Result<(), PluginError> {
        panic!("deactivation boundary panic");
    }
}

#[test]
fn lifecycle_panics_are_contained_and_activation_effects_are_reclaimed() {
    let cleanup_log = Arc::new(Mutex::new(Vec::new()));
    let mut activation_kernel = Kernel::new();
    let activation_id = activation_kernel
        .register(PanickingActivationPlugin {
            definition: PluginDefinition::new("panic-activation"),
            cleanup_log: Arc::clone(&cleanup_log),
        })
        .expect("panic plugin registration itself must succeed");
    assert_eq!(
        activation_kernel.plugin_state(&activation_id),
        Some(RuntimeState::Crashed)
    );
    assert_eq!(
        cleanup_log
            .lock()
            .expect("cleanup log lock is not poisoned")
            .clone(),
        vec!["panic effect cleaned"]
    );
    assert!(activation_kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::PluginCrashed {
                phase: LifecyclePhase::Activation,
                ..
            }
        )
    }));

    let mut deactivation_kernel = Kernel::new();
    let deactivation_id = deactivation_kernel
        .register(PanickingRuntimePlugin {
            definition: PluginDefinition::new("panic-deactivation"),
        })
        .expect("deactivation panic plugin registration must succeed");
    deactivation_kernel
        .unregister(&deactivation_id)
        .expect("deactivation panic must remain contained");
    assert!(deactivation_kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::PluginCrashed {
                phase: LifecyclePhase::Deactivation,
                ..
            }
        )
    }));
}

fn deterministic_scenario() -> Vec<worldline_kernel::TrajectoryEvent> {
    let capability = greeting_capability();
    let log = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();
    kernel
        .register(consumer(&capability, &handle, &log))
        .expect("consumer registration must succeed");
    let provider_id = kernel
        .register(provider("provider-a", &capability, "A", &log))
        .expect("provider registration must succeed");
    kernel
        .unregister(&provider_id)
        .expect("provider removal must succeed");
    kernel.trajectory().to_vec()
}

#[test]
fn identical_scenarios_produce_identical_trajectory() {
    assert_eq!(deterministic_scenario(), deterministic_scenario());
}
