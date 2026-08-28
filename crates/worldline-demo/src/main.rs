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
            service: Arc::new(PrefixService { prefix: "A:" }),
        })
        .expect("provider A registration must succeed");

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
            service: Arc::new(PrefixService { prefix: "B:" }),
        })
        .expect("provider B registration must succeed");
    kernel
        .unregister(&provider_a)
        .expect("provider A removal must succeed");

    println!(
        "after compatible provider replacement: {}",
        String::from_utf8(
            consumer_handle
                .invoke("greet", b"Worldline")
                .expect("provider B must answer"),
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
