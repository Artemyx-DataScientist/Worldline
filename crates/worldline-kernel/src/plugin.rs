use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use crate::{
    CapabilityError, PluginError,
    capability::{
        CapabilityHandle, CapabilityId, CapabilityPublication, CapabilityRegistry,
        CapabilityService, DependencyKind,
    },
    effect::{LifecycleScope, OwnedEffect},
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PluginId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PluginId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginDefinition {
    id: PluginId,
    provides: BTreeSet<CapabilityId>,
    dependencies: BTreeMap<CapabilityId, DependencyKind>,
}

impl PluginDefinition {
    pub fn new(id: impl Into<PluginId>) -> Self {
        Self {
            id: id.into(),
            provides: BTreeSet::new(),
            dependencies: BTreeMap::new(),
        }
    }

    pub fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn provides(mut self, capability: CapabilityId) -> Self {
        self.provides.insert(capability);
        self
    }

    pub fn requires(mut self, capability: CapabilityId) -> Self {
        self.dependencies
            .insert(capability, DependencyKind::Required);
        self
    }

    pub fn optionally_requires(mut self, capability: CapabilityId) -> Self {
        self.dependencies
            .entry(capability)
            .or_insert(DependencyKind::Optional);
        self
    }

    pub fn provided_capabilities(&self) -> &BTreeSet<CapabilityId> {
        &self.provides
    }

    pub fn dependencies(&self) -> &BTreeMap<CapabilityId, DependencyKind> {
        &self.dependencies
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.id.as_str().trim().is_empty() {
            return Err("plugin id must not be empty".to_owned());
        }
        if self
            .provides
            .iter()
            .any(|capability| !capability.is_well_formed())
        {
            return Err("provided capability identity must have namespace and name".to_owned());
        }
        if self
            .dependencies
            .keys()
            .any(|capability| !capability.is_well_formed())
        {
            return Err("dependency capability identity must have namespace and name".to_owned());
        }
        Ok(())
    }
}

pub trait Plugin: Send + Sync {
    fn definition(&self) -> &PluginDefinition;

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError>;
}

pub trait PluginRuntime: Send {
    fn deactivate(&mut self) -> Result<(), PluginError>;
}

pub struct NoopRuntime;

impl PluginRuntime for NoopRuntime {
    fn deactivate(&mut self) -> Result<(), PluginError> {
        Ok(())
    }
}

pub struct ActivationContext {
    plugin_id: PluginId,
    dependencies: BTreeMap<CapabilityId, DependencyKind>,
    declared_capabilities: BTreeSet<CapabilityId>,
    registry: Arc<CapabilityRegistry>,
    effects: Vec<OwnedEffect>,
    publications: Vec<CapabilityPublication>,
}

impl ActivationContext {
    pub(crate) fn new(definition: &PluginDefinition, registry: Arc<CapabilityRegistry>) -> Self {
        Self {
            plugin_id: definition.id.clone(),
            dependencies: definition.dependencies.clone(),
            declared_capabilities: definition.provides.clone(),
            registry,
            effects: Vec::new(),
            publications: Vec::new(),
        }
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub fn capability(
        &self,
        capability: &CapabilityId,
    ) -> Result<CapabilityHandle, CapabilityError> {
        if !self.dependencies.contains_key(capability) {
            return Err(CapabilityError::UndeclaredDependency {
                capability: capability.clone(),
                consumer: self.plugin_id.clone(),
            });
        }
        Ok(CapabilityHandle::new(
            capability.clone(),
            Arc::clone(&self.registry),
        ))
    }

    pub fn own_effect(&mut self, effect: OwnedEffect) {
        self.effects.push(effect);
    }

    pub fn publish_capability(
        &mut self,
        capability: CapabilityId,
        service: Arc<dyn CapabilityService>,
    ) -> Result<(), PluginError> {
        if !self.declared_capabilities.contains(&capability) {
            return Err(PluginError::new(format!(
                "plugin '{}' attempted to publish undeclared capability '{}'",
                self.plugin_id, capability
            )));
        }
        if self
            .publications
            .iter()
            .any(|publication| publication.id == capability)
        {
            return Err(PluginError::new(format!(
                "plugin '{}' published capability '{}' more than once",
                self.plugin_id, capability
            )));
        }
        self.publications.push(CapabilityPublication {
            id: capability,
            service,
        });
        Ok(())
    }

    pub(crate) fn into_parts(self) -> (LifecycleScope, Vec<CapabilityPublication>) {
        (LifecycleScope::new(self.effects), self.publications)
    }
}
