use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use crate::{
    CapabilityError, PluginError,
    capability::{CapabilityId, CapabilityPublication, CapabilityService, DependencyKind},
    effect::{LifecycleScope, OwnedEffect},
    invocation::{CapabilityHandle, InvocationBroker},
    runtime::{ActivationReason, LifecycleCancellationToken, LifecycleContext, RuntimeId},
    security::{LifecycleScopeId, PrincipalId},
    state::{
        InstallationId, RuntimeStateHandle, RuntimeStateLease, StateMigration, StateSchemaVersion,
    },
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
    state_schema_version: StateSchemaVersion,
    state_migrations: Vec<StateMigration>,
}

impl PluginDefinition {
    pub fn new(id: impl Into<PluginId>) -> Self {
        Self {
            id: id.into(),
            provides: BTreeSet::new(),
            dependencies: BTreeMap::new(),
            state_schema_version: StateSchemaVersion::default(),
            state_migrations: Vec::new(),
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

    pub fn with_state_schema_version(mut self, version: StateSchemaVersion) -> Self {
        self.state_schema_version = version;
        self
    }

    pub const fn state_schema_version(&self) -> StateSchemaVersion {
        self.state_schema_version
    }

    pub fn with_state_migration(mut self, migration: StateMigration) -> Self {
        self.state_migrations.push(migration);
        self
    }

    pub fn state_migrations(&self) -> &[StateMigration] {
        &self.state_migrations
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
        let mut edges = BTreeSet::new();
        let mut migration_ids = BTreeSet::new();
        for migration in &self.state_migrations {
            if migration.from_schema() == migration.to_schema() {
                return Err(format!(
                    "migration '{}' must change the state schema",
                    migration.migration_id()
                ));
            }
            if !edges.insert((migration.from_schema(), migration.to_schema())) {
                return Err(format!(
                    "duplicate state migration edge {} -> {}",
                    migration.from_schema(),
                    migration.to_schema()
                ));
            }
            if !migration_ids.insert(migration.migration_id().clone()) {
                return Err(format!(
                    "duplicate state migration id '{}'",
                    migration.migration_id()
                ));
            }
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

    /// Lifecycle-aware deactivation hook. Existing runtimes only need to
    /// implement `deactivate`; the default preserves that ABI while allowing
    /// cooperative cancellation in runtime-v1 implementations.
    fn deactivate_with_context(&mut self, _context: &LifecycleContext) -> Result<(), PluginError> {
        self.deactivate()
    }
}

pub struct NoopRuntime;

impl PluginRuntime for NoopRuntime {
    fn deactivate(&mut self) -> Result<(), PluginError> {
        Ok(())
    }
}

pub struct ActivationContext {
    plugin_id: PluginId,
    runtime_id: RuntimeId,
    principal: PrincipalId,
    installation_id: InstallationId,
    scope_id: LifecycleScopeId,
    activation_reason: ActivationReason,
    cancellation: LifecycleCancellationToken,
    deadline: Option<std::time::Duration>,
    dependencies: BTreeMap<CapabilityId, DependencyKind>,
    declared_capabilities: BTreeSet<CapabilityId>,
    broker: Arc<InvocationBroker>,
    state: RuntimeStateHandle,
    effects: Vec<OwnedEffect>,
    publications: Vec<CapabilityPublication>,
}

impl ActivationContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        definition: &PluginDefinition,
        runtime_id: RuntimeId,
        principal: PrincipalId,
        installation_id: InstallationId,
        scope_id: LifecycleScopeId,
        activation_reason: ActivationReason,
        cancellation: LifecycleCancellationToken,
        deadline: Option<std::time::Duration>,
        state: RuntimeStateHandle,
        broker: Arc<InvocationBroker>,
    ) -> Self {
        Self {
            plugin_id: definition.id.clone(),
            runtime_id,
            principal,
            installation_id,
            scope_id,
            activation_reason,
            cancellation,
            deadline,
            dependencies: definition.dependencies.clone(),
            declared_capabilities: definition.provides.clone(),
            broker,
            state,
            effects: Vec::new(),
            publications: Vec::new(),
        }
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub const fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    pub fn installation_id(&self) -> &InstallationId {
        &self.installation_id
    }

    pub fn state(&self) -> &RuntimeStateHandle {
        &self.state
    }

    pub fn lifecycle_scope_id(&self) -> LifecycleScopeId {
        self.scope_id
    }

    pub const fn activation_reason(&self) -> ActivationReason {
        self.activation_reason
    }

    pub fn cancellation(&self) -> LifecycleCancellationToken {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub const fn deadline(&self) -> Option<std::time::Duration> {
        self.deadline
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
            self.principal.clone(),
            Arc::clone(&self.broker),
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

    pub(crate) fn into_parts(
        self,
    ) -> (
        LifecycleScope,
        Vec<CapabilityPublication>,
        std::sync::Arc<RuntimeStateLease>,
    ) {
        (
            LifecycleScope::new(self.scope_id, self.effects),
            self.publications,
            self.state.lease(),
        )
    }
}
