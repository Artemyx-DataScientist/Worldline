use std::sync::Arc;

use worldline_kernel::{
    ActivationContext, CapabilityId, CapabilityService, InterfaceVersion, NoopRuntime, Plugin,
    PluginDefinition, PluginError, PluginRuntime,
};

use crate::{
    CapabilitySlot, ObservationBus, ObservationDraft, capture_capability,
    increment_activation_count,
};

pub fn surface_capability() -> CapabilityId {
    CapabilityId::new("reference.ui", "surface", InterfaceVersion::new(1, 0))
}

pub fn command_capability() -> CapabilityId {
    CapabilityId::new("reference.ui", "command", InterfaceVersion::new(1, 0))
}

/// A UI-shaped provider that publishes opaque surface/command capabilities.
/// The kernel has no surface, panel, or renderer entity to understand.
pub struct UiLikeProvider {
    definition: PluginDefinition,
    agent_capability: CapabilityId,
    agent_handle: CapabilitySlot,
    bus: ObservationBus,
    response_prefix: String,
}

impl UiLikeProvider {
    pub fn new(
        plugin_id: impl Into<String>,
        agent_capability: CapabilityId,
        bus: ObservationBus,
        response_prefix: impl Into<String>,
    ) -> (Self, CapabilitySlot) {
        let agent_handle = Arc::new(std::sync::Mutex::new(None));
        (
            Self {
                definition: PluginDefinition::new(plugin_id.into())
                    .requires(agent_capability.clone())
                    .provides(surface_capability())
                    .provides(command_capability()),
                agent_capability,
                agent_handle: Arc::clone(&agent_handle),
                bus,
                response_prefix: response_prefix.into(),
            },
            agent_handle,
        )
    }
}

impl Plugin for UiLikeProvider {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        increment_activation_count(context)?;
        capture_capability(context, &self.agent_capability, &self.agent_handle)?;
        context.publish_capability(
            surface_capability(),
            Arc::new(UiService {
                bus: self.bus.clone(),
                response_prefix: self.response_prefix.clone(),
                operation: "render",
                topic: "reference.ui.surface",
            }),
        )?;
        context.publish_capability(
            command_capability(),
            Arc::new(UiService {
                bus: self.bus.clone(),
                response_prefix: self.response_prefix.clone(),
                operation: "command",
                topic: "reference.ui.command",
            }),
        )?;
        Ok(Box::new(NoopRuntime))
    }
}

struct UiService {
    bus: ObservationBus,
    response_prefix: String,
    operation: &'static str,
    topic: &'static str,
}

impl CapabilityService for UiService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation != self.operation {
            return Err(format!("unsupported UI operation '{operation}'"));
        }
        Ok(format!(
            "{}:{}",
            self.response_prefix,
            String::from_utf8(payload.to_vec()).map_err(|error| error.to_string())?
        )
        .into_bytes())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let result = self.invoke(context.operation().as_str(), payload)?;
        self.bus.publish(
            ObservationDraft::new(context.provider().clone(), self.topic, &result)
                .with_causation(context.invocation_id().clone()),
        );
        Ok(result)
    }
}

/// A UI-shaped consumer that only knows the generic capability contract.
pub struct UiLikeConsumer {
    definition: PluginDefinition,
    handle: CapabilitySlot,
}

impl UiLikeConsumer {
    pub fn new(plugin_id: impl Into<String>) -> (Self, CapabilitySlot) {
        let handle = Arc::new(std::sync::Mutex::new(None));
        (
            Self {
                definition: PluginDefinition::new(plugin_id.into()).requires(surface_capability()),
                handle: Arc::clone(&handle),
            },
            handle,
        )
    }
}

impl Plugin for UiLikeConsumer {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        capture_capability(context, &surface_capability(), &self.handle)?;
        Ok(Box::new(NoopRuntime))
    }
}
