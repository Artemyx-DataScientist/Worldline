use std::sync::{Arc, Mutex};

use worldline_kernel::{
    ActivationContext, CapabilityHandle, CapabilityId, CapabilityService, GrantLifetime,
    InterfaceVersion, Kernel, NoopRuntime, Plugin, PluginDefinition, PluginError, PluginRuntime,
    ResourceScope,
};

fn greeting_capability() -> CapabilityId {
    CapabilityId::new("worldline.demo", "greeting", InterfaceVersion::new(1, 0))
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
        Ok(format!("{} {name}", self.prefix).into_bytes())
    }
}

struct ProviderPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    prefix: &'static str,
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
        let activation_count = context
            .state()
            .get("activation-count")
            .map_err(|error| PluginError::new(error.to_string()))?
            .and_then(|value| String::from_utf8(value).ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        let mut state = context
            .state()
            .transaction()
            .map_err(|error| PluginError::new(error.to_string()))?;
        state
            .put("activation-count", activation_count.to_string().as_bytes())
            .map_err(|error| PluginError::new(error.to_string()))?;
        state
            .put("last-prefix", self.prefix.as_bytes())
            .map_err(|error| PluginError::new(error.to_string()))?;
        state
            .commit()
            .map_err(|error| PluginError::new(error.to_string()))?;
        context.publish_capability(self.capability.clone(), Arc::clone(&self.service))?;
        Ok(Box::new(NoopRuntime))
    }
}

struct ConsumerPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    handle: Arc<Mutex<Option<CapabilityHandle>>>,
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
        Ok(Box::new(NoopRuntime))
    }
}

fn main() {
    let capability = greeting_capability();
    let handle = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();

    let consumer_id = kernel
        .register(ConsumerPlugin {
            definition: PluginDefinition::new("demo-consumer").requires(capability.clone()),
            capability: capability.clone(),
            handle: Arc::clone(&handle),
        })
        .expect("consumer registration must succeed");

    let provider_a = kernel
        .register(ProviderPlugin {
            definition: PluginDefinition::new("provider-a").provides(capability.clone()),
            capability: capability.clone(),
            prefix: "A:",
            service: Arc::new(PrefixService { prefix: "A:" }),
        })
        .expect("provider A registration must succeed");
    let provider_installation = kernel
        .installation_id_for_plugin(&provider_a)
        .expect("provider A installation must exist");
    let provider_a_runtime = kernel
        .principal_for_plugin(&provider_a)
        .expect("provider A runtime principal must exist");

    let consumer_handle = handle
        .lock()
        .expect("consumer handle lock is not poisoned")
        .clone()
        .expect("consumer must be active");
    let consumer_principal = kernel
        .principal_for_plugin(&consumer_id)
        .expect("consumer principal must be registered");
    println!(
        "availability (provider is active): {}",
        consumer_handle.is_available()
    );
    println!(
        "before grant: {:?}",
        consumer_handle.invoke("greet", b"Worldline")
    );
    let grant = kernel
        .create_root_grant(
            consumer_principal,
            capability.contract(),
            ["greet"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .expect("demo grant must be created");
    println!(
        "after grant: {}",
        String::from_utf8(
            consumer_handle
                .invoke("greet", b"Worldline")
                .expect("provider A must answer"),
        )
        .expect("provider response must be UTF-8")
    );

    kernel
        .register(ProviderPlugin {
            definition: PluginDefinition::new("provider-b").provides(capability),
            capability: greeting_capability(),
            prefix: "B:",
            service: Arc::new(PrefixService { prefix: "B:" }),
        })
        .expect("provider B registration must succeed");
    kernel
        .unregister(&provider_a)
        .expect("provider A removal must succeed");

    let preserved_count = kernel
        .state_handle(&provider_installation)
        .expect("unregister must preserve installation state")
        .get("activation-count")
        .expect("state read must succeed")
        .expect("provider A must have written activation state");
    let provider_a_restart = kernel
        .register_for_installation(
            ProviderPlugin {
                definition: PluginDefinition::new("provider-a").provides(greeting_capability()),
                capability: greeting_capability(),
                prefix: "A-restarted:",
                service: Arc::new(PrefixService {
                    prefix: "A-restarted:",
                }),
            },
            &provider_installation,
        )
        .expect("provider A must restart on the same installation");
    let provider_b_runtime = kernel
        .principal_for_plugin(&provider_a_restart)
        .expect("restarted provider must have a runtime principal");
    let resumed_count = kernel
        .state_handle(&provider_installation)
        .expect("installation state must remain available")
        .get("activation-count")
        .expect("state read must succeed")
        .expect("activation state must remain present");
    println!(
        "same installation state: {} -> {}; runtime authority identity changed: {} -> {}",
        String::from_utf8_lossy(&preserved_count),
        String::from_utf8_lossy(&resumed_count),
        provider_a_runtime,
        provider_b_runtime,
    );

    println!(
        "after same-installation runtime restart: {}",
        String::from_utf8(
            consumer_handle
                .invoke("greet", b"Worldline")
                .expect("restarted provider must answer"),
        )
        .expect("provider response must be UTF-8")
    );
    kernel
        .revoke_grant(&grant)
        .expect("demo grant revocation must succeed");
    println!(
        "after revoke: {:?}",
        consumer_handle.invoke("greet", b"Worldline")
    );
    println!("trajectory:");
    for event in kernel.trajectory() {
        println!("  {event:?}");
    }
}
