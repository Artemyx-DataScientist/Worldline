use std::sync::Arc;

use worldline_kernel::{
    ActivationContext, CapabilityId, CapabilityService, InterfaceVersion, NoopRuntime, Plugin,
    PluginDefinition, PluginError, PluginRuntime,
};

use crate::{
    CapabilitySlot, ObservationBus, ObservationDraft, capture_capability,
    increment_activation_count,
};

pub fn reason_capability() -> CapabilityId {
    CapabilityId::new("reference.agent", "reason", InterfaceVersion::new(1, 0))
}

/// An agent-shaped ordinary plugin. Its family name has no effect on
/// authority; browser access is an ordinary declared capability dependency.
pub struct AgentLikePlugin {
    definition: PluginDefinition,
    browser_capability: CapabilityId,
    browser_handle: CapabilitySlot,
    bus: ObservationBus,
    response_prefix: String,
}

impl AgentLikePlugin {
    pub fn new(
        plugin_id: impl Into<String>,
        browser_capability: CapabilityId,
        bus: ObservationBus,
        response_prefix: impl Into<String>,
    ) -> (Self, CapabilitySlot) {
        let browser_handle = Arc::new(std::sync::Mutex::new(None));
        (
            Self {
                definition: PluginDefinition::new(plugin_id.into())
                    .requires(browser_capability.clone())
                    .provides(reason_capability()),
                browser_capability,
                browser_handle: Arc::clone(&browser_handle),
                bus,
                response_prefix: response_prefix.into(),
            },
            browser_handle,
        )
    }
}

impl Plugin for AgentLikePlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        increment_activation_count(context)?;
        capture_capability(context, &self.browser_capability, &self.browser_handle)?;
        context.publish_capability(
            reason_capability(),
            Arc::new(AgentReasonService {
                bus: self.bus.clone(),
                response_prefix: self.response_prefix.clone(),
            }),
        )?;
        Ok(Box::new(NoopRuntime))
    }
}

struct AgentReasonService {
    bus: ObservationBus,
    response_prefix: String,
}

impl CapabilityService for AgentReasonService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation != "reason" {
            return Err(format!("unsupported agent operation '{operation}'"));
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
                "reference.agent.reason",
                &result,
            )
            .with_causation(context.invocation_id().clone()),
        );
        Ok(result)
    }
}
