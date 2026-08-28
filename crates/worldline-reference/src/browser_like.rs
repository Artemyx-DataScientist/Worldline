use std::sync::Arc;

use worldline_kernel::{
    ActivationContext, CapabilityHandle, CapabilityId, CapabilityService, InterfaceVersion,
    NoopRuntime, Plugin, PluginDefinition, PluginError, PluginRuntime,
};

use crate::{
    CapabilitySlot, ObservationBus, ObservationDraft, capture_capability,
    increment_activation_count,
};

pub fn navigate_capability() -> CapabilityId {
    CapabilityId::new("reference.browser", "navigate", InterfaceVersion::new(1, 0))
}

/// A browser-shaped probe: it accepts opaque navigation bytes and publishes a
/// separate observation after returning its deterministic capability result.
pub struct BrowserLikeProvider {
    definition: PluginDefinition,
    bus: ObservationBus,
    response_prefix: String,
}

impl BrowserLikeProvider {
    pub fn new(
        plugin_id: impl Into<String>,
        bus: ObservationBus,
        response_prefix: impl Into<String>,
    ) -> Self {
        let capability = navigate_capability();
        Self {
            definition: PluginDefinition::new(plugin_id.into()).provides(capability),
            bus,
            response_prefix: response_prefix.into(),
        }
    }
}

impl Plugin for BrowserLikeProvider {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        increment_activation_count(context)?;
        context.publish_capability(
            navigate_capability(),
            Arc::new(BrowserNavigateService {
                bus: self.bus.clone(),
                response_prefix: self.response_prefix.clone(),
            }),
        )?;
        Ok(Box::new(NoopRuntime))
    }
}

struct BrowserNavigateService {
    bus: ObservationBus,
    response_prefix: String,
}

impl CapabilityService for BrowserNavigateService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation != "navigate" {
            return Err(format!("unsupported browser operation '{operation}'"));
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
            ObservationDraft::new(
                context.provider().clone(),
                "reference.browser.navigation",
                &result,
            )
            .with_causation(context.invocation_id().clone()),
        );
        Ok(result)
    }
}

/// A browser-shaped consumer using only the generic capability dependency API.
pub struct BrowserLikeConsumer {
    definition: PluginDefinition,
    handle: CapabilitySlot,
}

impl BrowserLikeConsumer {
    pub fn new(plugin_id: impl Into<String>) -> (Self, CapabilitySlot) {
        let handle = Arc::new(std::sync::Mutex::new(None));
        (
            Self {
                definition: PluginDefinition::new(plugin_id.into()).requires(navigate_capability()),
                handle: Arc::clone(&handle),
            },
            handle,
        )
    }
}

impl Plugin for BrowserLikeConsumer {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        capture_capability(context, &navigate_capability(), &self.handle)?;
        Ok(Box::new(NoopRuntime))
    }
}

pub fn capability_from_slot(slot: &CapabilitySlot) -> Option<CapabilityHandle> {
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
