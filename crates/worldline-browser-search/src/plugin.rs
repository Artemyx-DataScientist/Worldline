//! Plugin implementation for ordinary replaceable search provider.

use std::sync::Arc;

use worldline_browser_services_contract::CONTRACT_BROWSER_SEARCH;
use worldline_kernel::{
    ActivationContext, CapabilityId, InterfaceVersion, NoopRuntime, Plugin, PluginDefinition,
    PluginError, PluginRuntime,
};

use crate::config::SearchProviderConfig;
use crate::service::SearchProviderService;

/// Returns the standard `browser.search/0.1` capability identifier.
pub fn search_capability() -> CapabilityId {
    CapabilityId::new(
        CONTRACT_BROWSER_SEARCH,
        "resolve",
        InterfaceVersion::new(0, 1),
    )
}

/// An ordinary replaceable search provider plugin with installation-owned configuration.
pub struct SearchProviderPlugin {
    definition: PluginDefinition,
    config: SearchProviderConfig,
}

impl SearchProviderPlugin {
    pub fn new(plugin_id: impl Into<String>, config: SearchProviderConfig) -> Self {
        let definition = PluginDefinition::new(plugin_id.into()).provides(search_capability());
        Self { definition, config }
    }

    pub fn config(&self) -> &SearchProviderConfig {
        &self.config
    }
}

impl Plugin for SearchProviderPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        let service = SearchProviderService::new(self.config.clone())
            .map_err(|err| PluginError::new(format!("invalid search provider config: {err}")))?;

        context.publish_capability(search_capability(), Arc::new(service))?;
        Ok(Box::new(NoopRuntime))
    }
}
