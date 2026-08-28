use std::{
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use crate::{
    KernelError, PluginError,
    capability::{CapabilityPublication, CapabilityRegistry},
    effect::LifecycleScope,
    error::panic_message,
    invocation::{CapabilityHandle, InvocationBroker},
    plugin::{ActivationContext, Plugin, PluginDefinition, PluginId, PluginRuntime},
    security::{
        CapabilityGrant, GrantId, GrantRequest, GrantStatus, LifecycleScopeId, Principal,
        PrincipalId, PrincipalKind, SecurityError,
    },
    state::{
        InMemoryStateBackend, InstallationId, InstallationRecord, RuntimeStateLease, StateBackend,
        StateError, StateHandle, StateSchemaVersion, StateStore,
    },
    trajectory::{LifecyclePhase, Trajectory, TrajectoryEvent, TrajectoryEventKind},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeState {
    Registered,
    Pending,
    Active,
    Stopped,
    Failed,
    Crashed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconcileReport {
    pub activated: Vec<PluginId>,
    pub pending: Vec<PluginId>,
    pub failed: Vec<PluginId>,
    pub crashed: Vec<PluginId>,
}

struct RuntimeInstance {
    runtime: Box<dyn PluginRuntime>,
    scope: LifecycleScope,
    publications: Vec<CapabilityPublication>,
    runtime_principal: PrincipalId,
    state_lease: Arc<RuntimeStateLease>,
}

struct PluginRecord {
    plugin: Arc<dyn Plugin>,
    definition: PluginDefinition,
    installation_id: InstallationId,
    state: RuntimeState,
    runtime: Option<RuntimeInstance>,
    last_missing: Option<Vec<crate::CapabilityId>>,
}

pub struct Kernel {
    registry: Arc<CapabilityRegistry>,
    security: Arc<crate::security::SecurityStore>,
    broker: Arc<InvocationBroker>,
    state: Arc<StateStore>,
    plugins: std::collections::BTreeMap<PluginId, PluginRecord>,
    default_installations: std::collections::BTreeMap<PluginId, InstallationId>,
    memory_backend: Option<Arc<InMemoryStateBackend>>,
    trajectory: Trajectory,
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Kernel {
    pub fn new() -> Self {
        let backend = Arc::new(InMemoryStateBackend::new());
        Self::from_state_backend(backend.clone(), Some(backend))
            .expect("the in-memory state backend must initialize")
    }

    /// Creates a kernel over a caller-owned backend. Sharing an in-memory
    /// backend between kernel instances models restart without sharing runtime
    /// principals or grants.
    pub fn with_state_backend(backend: Arc<dyn StateBackend>) -> Result<Self, StateError> {
        Self::from_state_backend(backend, None)
    }

    fn from_state_backend(
        backend: Arc<dyn StateBackend>,
        memory_backend: Option<Arc<InMemoryStateBackend>>,
    ) -> Result<Self, StateError> {
        let trajectory = Trajectory::default();
        let registry = Arc::new(CapabilityRegistry::default());
        let security = Arc::new(crate::security::SecurityStore::new());
        let broker = Arc::new(InvocationBroker::new(
            Arc::clone(&registry),
            Arc::clone(&security),
            trajectory.clone(),
        ));
        trajectory.push_security(TrajectoryEventKind::PrincipalRegistered {
            principal: security.system_principal(),
            kind: PrincipalKind::System,
        });
        let state = Arc::new(StateStore::new(backend, trajectory.clone())?);
        let mut default_installations = std::collections::BTreeMap::new();
        for record in state.all_records() {
            let principal = installation_principal(&record);
            let inserted = security
                .ensure_principal(principal.clone())
                .expect("persisted installation principal must be registerable");
            if inserted {
                trajectory.push_security(TrajectoryEventKind::PrincipalRegistered {
                    principal: principal.id().clone(),
                    kind: principal.kind(),
                });
            }
            default_installations
                .entry(record.plugin_id().clone())
                .or_insert_with(|| record.installation_id().clone());
        }
        Ok(Self {
            registry,
            security,
            broker,
            state,
            plugins: std::collections::BTreeMap::new(),
            default_installations,
            memory_backend,
            trajectory,
        })
    }

    pub fn register<P>(&mut self, plugin: P) -> Result<PluginId, KernelError>
    where
        P: Plugin + 'static,
    {
        self.register_arc(Arc::new(plugin))
    }

    pub fn register_arc(&mut self, plugin: Arc<dyn Plugin>) -> Result<PluginId, KernelError> {
        let definition = self.read_definition(&plugin)?;
        let id = definition.id().clone();
        let installation =
            self.ensure_default_installation(&id, definition.state_schema_version())?;
        self.register_arc_with_definition(plugin, definition, installation)
    }

    pub fn register_for_installation<P>(
        &mut self,
        plugin: P,
        installation: impl Into<InstallationId>,
    ) -> Result<PluginId, KernelError>
    where
        P: Plugin + 'static,
    {
        self.register_arc_for_installation(Arc::new(plugin), installation)
    }

    pub fn register_arc_for_installation(
        &mut self,
        plugin: Arc<dyn Plugin>,
        installation: impl Into<InstallationId>,
    ) -> Result<PluginId, KernelError> {
        let definition = self.read_definition(&plugin)?;
        let installation = installation.into();
        let record =
            self.state
                .record(&installation)
                .ok_or_else(|| StateError::UnknownInstallation {
                    installation: installation.clone(),
                })?;
        if record.plugin_id() != definition.id() {
            return Err(StateError::StateAccessDenied { installation }.into());
        }
        self.register_arc_with_definition(plugin, definition, installation)
    }

    fn register_arc_with_definition(
        &mut self,
        plugin: Arc<dyn Plugin>,
        definition: PluginDefinition,
        installation: InstallationId,
    ) -> Result<PluginId, KernelError> {
        if self.plugins.contains_key(definition.id()) {
            return Err(KernelError::DuplicatePlugin {
                id: definition.id().clone(),
            });
        }
        self.state.prepare_retry(&installation)?;
        self.state.prepare_for_schema(
            &installation,
            definition.state_schema_version(),
            definition.state_migrations(),
        )?;
        let id = definition.id().clone();
        self.plugins.insert(
            id.clone(),
            PluginRecord {
                plugin,
                definition,
                installation_id: installation,
                state: RuntimeState::Registered,
                runtime: None,
                last_missing: None,
            },
        );
        self.trajectory
            .push(id.clone(), TrajectoryEventKind::Registered);
        self.reconcile();
        Ok(id)
    }

    fn read_definition(&self, plugin: &Arc<dyn Plugin>) -> Result<PluginDefinition, KernelError> {
        let definition =
            catch_unwind(AssertUnwindSafe(|| plugin.definition().clone())).map_err(|payload| {
                KernelError::PluginDefinitionPanicked {
                    message: panic_message(payload.as_ref()),
                }
            })?;
        if let Err(reason) = definition.validate() {
            return Err(KernelError::InvalidDefinition {
                id: definition.id().clone(),
                reason,
            });
        }
        Ok(definition)
    }

    /// Creates a persistent installation. The first installation for a
    /// PluginId becomes the default target of `register`; additional
    /// installations remain independently addressable.
    pub fn create_installation(
        &mut self,
        plugin: impl Into<PluginId>,
        schema: StateSchemaVersion,
    ) -> Result<InstallationId, KernelError> {
        let plugin = plugin.into();
        let installation = self.state.create_installation(plugin.clone(), schema)?;
        self.register_installation_principal(&installation)?;
        self.default_installations
            .entry(plugin)
            .or_insert_with(|| installation.clone());
        Ok(installation)
    }

    pub fn installation(&self, installation: &InstallationId) -> Option<InstallationRecord> {
        self.state.record(installation)
    }

    pub fn installations_for_plugin(&self, plugin: &PluginId) -> Vec<InstallationRecord> {
        self.state.records_for_plugin(plugin)
    }

    pub fn installation_id_for_plugin(&self, plugin: &PluginId) -> Option<InstallationId> {
        if let Some(record) = self.plugins.get(plugin) {
            return Some(record.installation_id.clone());
        }
        let installations = self.state.records_for_plugin(plugin);
        (installations.len() == 1).then(|| installations[0].installation_id().clone())
    }

    pub fn principal_for_installation(&self, installation: &InstallationId) -> Option<PrincipalId> {
        self.state
            .record(installation)
            .map(|record| PrincipalId::plugin_installation(record.installation_id().as_str()))
            .filter(|principal| self.security.principal_exists(principal))
    }

    /// Returns a state handle for trusted kernel-side code. Plugin runtimes
    /// receive their bound handle through ActivationContext instead.
    pub fn state_handle(&self, installation: &InstallationId) -> Result<StateHandle, StateError> {
        self.state.handle(installation)
    }

    /// Verifies a plugin's installation binding before returning its state
    /// handle. This is the diagnostic counterpart of the immutable runtime
    /// binding carried by ActivationContext.
    pub fn state_handle_for_plugin(
        &self,
        plugin: &PluginId,
        installation: &InstallationId,
    ) -> Result<StateHandle, StateError> {
        let record = self
            .plugins
            .get(plugin)
            .ok_or_else(|| StateError::StateAccessDenied {
                installation: installation.clone(),
            })?;
        if &record.installation_id != installation {
            return Err(StateError::RuntimeInstallationMismatch {
                expected: record.installation_id.clone(),
                actual: installation.clone(),
            });
        }
        self.state.handle(installation)
    }

    /// Causes one subsequent in-memory transaction commit to fail. This hook
    /// exists for acceptance tests; other backend implementations decide their
    /// own fault-injection mechanism.
    pub fn fail_next_state_commit(&self) {
        if let Some(backend) = &self.memory_backend {
            backend.fail_next_commit();
        }
    }

    /// Causes one installation metadata transition to fail. This hook exists
    /// for recovery-path acceptance tests.
    pub fn fail_next_state_record_update(&self) {
        if let Some(backend) = &self.memory_backend {
            backend.fail_next_record_update();
        }
    }

    /// Causes an installation metadata transition to fail after the requested
    /// number of successful transitions.
    pub fn fail_state_record_update_after(&self, successful_updates: u64) {
        if let Some(backend) = &self.memory_backend {
            backend.fail_record_update_after(successful_updates);
        }
    }

    /// Causes one subsequent in-memory uninstall/delete to fail.
    pub fn fail_next_state_delete(&self) {
        if let Some(backend) = &self.memory_backend {
            backend.fail_next_delete();
        }
    }

    pub fn uninstall(&mut self, installation: &InstallationId) -> Result<(), KernelError> {
        let plugin_id = self
            .state
            .record(installation)
            .map(|record| record.plugin_id().clone());
        let plugin = self
            .plugins
            .iter()
            .find(|(_, record)| &record.installation_id == installation)
            .map(|(id, _)| id.clone());
        if let Some(plugin) = plugin {
            self.unregister(&plugin)?;
        }
        self.state.uninstall(installation)?;
        self.retire_installation_principal(installation)?;
        if let Some(plugin_id) = plugin_id
            && self
                .default_installations
                .get(&plugin_id)
                .is_some_and(|default| default == installation)
        {
            self.default_installations.remove(&plugin_id);
        }
        Ok(())
    }

    fn retire_installation_principal(
        &self,
        installation: &InstallationId,
    ) -> Result<(), KernelError> {
        let principal = PrincipalId::plugin_installation(installation.as_str());
        let (kind, direct_grants, descendant_grants) =
            self.security.retire_principal(&principal)?;
        for grant in direct_grants {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantRevoked { grant });
        }
        for grant in descendant_grants {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantAutoRevoked { grant });
        }
        self.trajectory
            .push_security(TrajectoryEventKind::PrincipalRetired { principal, kind });
        Ok(())
    }

    fn ensure_default_installation(
        &mut self,
        plugin: &PluginId,
        schema: StateSchemaVersion,
    ) -> Result<InstallationId, KernelError> {
        let installations = self.state.records_for_plugin(plugin);
        match installations.as_slice() {
            [] => self.create_installation(plugin.clone(), schema),
            [installation] => {
                let installation = installation.installation_id().clone();
                self.default_installations
                    .insert(plugin.clone(), installation.clone());
                Ok(installation)
            }
            _ => Err(StateError::AmbiguousInstallation {
                plugin: plugin.clone(),
                installations: installations
                    .into_iter()
                    .map(|record| record.installation_id().clone())
                    .collect(),
            }
            .into()),
        }
    }

    fn register_installation_principal(
        &self,
        installation: &InstallationId,
    ) -> Result<(), KernelError> {
        let principal = Principal::new(
            PrincipalId::plugin_installation(installation.as_str()),
            PrincipalKind::PluginInstallation,
        );
        if self.security.ensure_principal(principal.clone())? {
            self.trajectory
                .push_security(TrajectoryEventKind::PrincipalRegistered {
                    principal: principal.id().clone(),
                    kind: principal.kind(),
                });
        }
        Ok(())
    }

    pub fn reconcile(&mut self) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        loop {
            self.refresh_dependency_resolution();
            let Some(id) = self.next_eligible_plugin() else {
                break;
            };
            match self.activate_plugin(&id) {
                ActivationOutcome::Activated => report.activated.push(id),
                ActivationOutcome::Failed => report.failed.push(id),
                ActivationOutcome::Crashed => report.crashed.push(id),
            }
        }
        self.refresh_dependency_resolution();
        report.pending = self
            .plugins
            .iter()
            .filter(|(_, record)| record.state == RuntimeState::Pending)
            .map(|(id, _)| id.clone())
            .collect();
        report
    }

    pub fn start(&mut self) -> ReconcileReport {
        for record in self.plugins.values_mut() {
            if record.state == RuntimeState::Stopped {
                record.state = RuntimeState::Registered;
                record.last_missing = None;
            }
        }
        self.reconcile()
    }

    pub fn stop(&mut self) {
        while let Some(root) = self
            .plugins
            .iter()
            .find(|(_, record)| record.state == RuntimeState::Active)
            .map(|(id, _)| id.clone())
        {
            let order = self.deactivation_order(&root);
            if order.is_empty() {
                break;
            }
            self.log_provider_losses(&order);
            for plugin_id in &order {
                self.deactivate_one_with_state(plugin_id, RuntimeState::Stopped);
            }
        }
    }

    pub fn unregister(&mut self, id: &PluginId) -> Result<(), KernelError> {
        if !self.plugins.contains_key(id) {
            return Err(KernelError::UnknownPlugin { id: id.clone() });
        }

        let order = self.deactivation_order(id);
        if !order.is_empty() {
            self.log_provider_losses(&order);
            for plugin_id in &order {
                self.deactivate_one_with_state(plugin_id, RuntimeState::Pending);
            }
        }
        self.plugins.remove(id);
        self.trajectory
            .push(id.clone(), TrajectoryEventKind::Unregistered);
        self.reconcile();
        Ok(())
    }

    pub fn plugin_state(&self, id: &PluginId) -> Option<RuntimeState> {
        self.plugins.get(id).map(|record| record.state)
    }

    pub fn registered_plugin_ids(&self) -> Vec<PluginId> {
        self.plugins.keys().cloned().collect()
    }

    pub fn is_capability_available(&self, capability: &crate::CapabilityId) -> bool {
        self.registry.has_provider(capability)
    }

    pub fn trajectory(&self) -> Vec<TrajectoryEvent> {
        self.trajectory.events()
    }

    pub fn register_principal(&self, principal: Principal) -> Result<(), SecurityError> {
        self.security.register_principal(principal.clone())?;
        self.trajectory
            .push_security(TrajectoryEventKind::PrincipalRegistered {
                principal: principal.id().clone(),
                kind: principal.kind(),
            });
        Ok(())
    }

    pub fn register_principal_id(
        &self,
        id: impl Into<PrincipalId>,
        kind: PrincipalKind,
    ) -> Result<PrincipalId, SecurityError> {
        let principal = Principal::new(id, kind);
        let id = principal.id().clone();
        self.register_principal(principal)?;
        Ok(id)
    }

    pub fn principal(&self, id: &PrincipalId) -> Option<Principal> {
        self.security.principal(id)
    }

    pub fn system_principal(&self) -> PrincipalId {
        self.security.system_principal()
    }

    pub fn principal_for_plugin(&self, plugin: &PluginId) -> Option<PrincipalId> {
        self.security
            .principal(
                &self
                    .plugins
                    .get(plugin)?
                    .runtime
                    .as_ref()?
                    .runtime_principal,
            )
            .map(|principal| principal.id().clone())
    }

    pub fn create_grant(&self, request: GrantRequest) -> Result<GrantId, SecurityError> {
        let grant = self.security.issue(request)?;
        let id = grant.id().clone();
        let event = if let Some(parent) = grant.parent_grant() {
            TrajectoryEventKind::GrantDelegated {
                grant: id.clone(),
                parent: parent.clone(),
                issuer: grant.issuer().clone(),
                subject: grant.subject().clone(),
                capability: grant.capability_contract().clone(),
                allowed_operations: grant.allowed_operations().clone(),
                resource_scope: grant.resource_scope().clone(),
                delegable: grant.delegable(),
                lifetime: grant.lifetime(),
            }
        } else {
            TrajectoryEventKind::GrantCreated {
                grant: id.clone(),
                issuer: grant.issuer().clone(),
                subject: grant.subject().clone(),
                capability: grant.capability_contract().clone(),
                allowed_operations: grant.allowed_operations().clone(),
                resource_scope: grant.resource_scope().clone(),
                delegable: grant.delegable(),
                lifetime: grant.lifetime(),
            }
        };
        self.trajectory.push_security(event);
        Ok(id)
    }

    pub fn create_root_grant(
        &self,
        subject: impl Into<PrincipalId>,
        capability: impl Into<crate::CapabilityContract>,
        operations: impl IntoIterator<Item = impl Into<crate::OperationId>>,
        resource_scope: crate::ResourceScope,
        delegable: bool,
        lifetime: crate::GrantLifetime,
    ) -> Result<GrantId, SecurityError> {
        let request = GrantRequest::new(self.system_principal(), subject, capability)
            .allow_operations(operations)
            .with_resource_scope(resource_scope)
            .with_delegable(delegable)
            .with_lifetime(lifetime);
        self.create_grant(request)
    }

    pub fn delegate_grant(
        &self,
        parent: impl Into<GrantId>,
        subject: impl Into<PrincipalId>,
        operations: impl IntoIterator<Item = impl Into<crate::OperationId>>,
        resource_scope: crate::ResourceScope,
        lifetime: crate::GrantLifetime,
    ) -> Result<GrantId, SecurityError> {
        let parent = parent.into();
        let parent_grant =
            self.security
                .grant(&parent)
                .ok_or_else(|| SecurityError::UnknownGrant {
                    grant: parent.clone(),
                })?;
        let request = GrantRequest::new(
            parent_grant.subject().clone(),
            subject,
            parent_grant.capability_contract().clone(),
        )
        .allow_operations(operations)
        .with_resource_scope(resource_scope)
        .with_parent_grant(parent)
        .with_delegable(parent_grant.delegable())
        .with_lifetime(lifetime);
        self.create_grant(request)
    }

    pub fn grant(&self, id: &GrantId) -> Option<CapabilityGrant> {
        self.security.grant(id)
    }

    pub fn revoke_grant(&self, id: &GrantId) -> Result<(), SecurityError> {
        let revoked = self.security.revoke(id)?;
        for (index, grant) in revoked.iter().enumerate() {
            self.trajectory.push_security(if index == 0 {
                TrajectoryEventKind::GrantRevoked {
                    grant: grant.clone(),
                }
            } else {
                TrajectoryEventKind::GrantAutoRevoked {
                    grant: grant.clone(),
                }
            });
        }
        Ok(())
    }

    pub fn is_grant_active(&self, id: &GrantId) -> bool {
        self.security
            .grant(id)
            .is_some_and(|grant| grant.status() == GrantStatus::Active)
    }

    pub fn lifecycle_scope_for(&self, plugin: &PluginId) -> Option<LifecycleScopeId> {
        self.plugins
            .get(plugin)
            .and_then(|record| record.runtime.as_ref())
            .map(|instance| instance.scope.id())
    }

    pub fn revoke_lifecycle_scope(&self, scope: LifecycleScopeId) {
        let revoked = self.security.revoke_lifecycle_scope(scope);
        for grant in revoked {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantAutoRevoked { grant });
        }
    }

    pub fn capability_for(
        &self,
        caller: impl Into<PrincipalId>,
        capability: impl Into<crate::CapabilityId>,
    ) -> Result<CapabilityHandle, crate::CapabilityError> {
        let caller = caller.into();
        if !self.security.principal_exists(&caller) {
            return Err(crate::CapabilityError::PrincipalUnavailable { principal: caller });
        }
        Ok(CapabilityHandle::new(
            capability.into(),
            caller,
            Arc::clone(&self.broker),
        ))
    }

    /// Submits an invocation to the broker.
    ///
    /// Authorization is admission-time. Once the broker admits a request,
    /// revoking its grant does not cancel the provider call already in flight;
    /// the revocation affects subsequent admissions.
    pub fn invoke(
        &self,
        request: crate::InvocationRequest,
    ) -> Result<Vec<u8>, crate::CapabilityError> {
        self.broker.invoke(request)
    }

    fn refresh_dependency_resolution(&mut self) {
        let ids: Vec<PluginId> = self.plugins.keys().cloned().collect();
        for id in ids {
            let Some((state, definition)) = self
                .plugins
                .get(&id)
                .map(|record| (record.state, record.definition.clone()))
            else {
                continue;
            };
            if !matches!(state, RuntimeState::Registered | RuntimeState::Pending) {
                continue;
            }

            let missing: Vec<crate::CapabilityId> = definition
                .dependencies()
                .iter()
                .filter(|(capability, kind)| {
                    **kind == crate::DependencyKind::Required
                        && !self.registry.has_provider(capability)
                })
                .map(|(capability, _)| capability.clone())
                .collect();
            let changed = self
                .plugins
                .get(&id)
                .and_then(|record| record.last_missing.as_ref())
                != Some(&missing);
            if changed {
                if let Some(record) = self.plugins.get_mut(&id) {
                    record.last_missing = Some(missing.clone());
                    record.state = if missing.is_empty() {
                        RuntimeState::Registered
                    } else {
                        RuntimeState::Pending
                    };
                }
                self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::DependencyResolution { missing },
                );
            } else if let Some(record) = self.plugins.get_mut(&id) {
                record.state = if missing.is_empty() {
                    RuntimeState::Registered
                } else {
                    RuntimeState::Pending
                };
            }
        }
    }

    fn next_eligible_plugin(&self) -> Option<PluginId> {
        self.plugins
            .iter()
            .find_map(|(id, record)| (record.state == RuntimeState::Registered).then(|| id.clone()))
    }

    fn activate_plugin(&mut self, id: &PluginId) -> ActivationOutcome {
        let (plugin, definition, installation) = {
            let record = self
                .plugins
                .get(id)
                .expect("eligible plugin must remain registered");
            (
                Arc::clone(&record.plugin),
                record.definition.clone(),
                record.installation_id.clone(),
            )
        };
        self.trajectory
            .push(id.clone(), TrajectoryEventKind::ActivationStarted);
        let runtime_generation = match self.state.allocate_runtime_generation(&installation) {
            Ok(generation) => generation,
            Err(error) => {
                self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::PluginFailure {
                        phase: LifecyclePhase::Activation,
                        message: error.to_string(),
                    },
                );
                if let Some(record) = self.plugins.get_mut(id) {
                    record.state = RuntimeState::Failed;
                }
                return ActivationOutcome::Failed;
            }
        };
        let scope_id = self.security.allocate_scope();
        let runtime_principal = self
            .security
            .allocate_runtime_principal(id.as_str(), runtime_generation);
        let state_lease = RuntimeStateLease::new();
        self.trajectory
            .push_security(TrajectoryEventKind::PrincipalRegistered {
                principal: runtime_principal.id().clone(),
                kind: runtime_principal.kind(),
            });
        self.trajectory
            .push_security(TrajectoryEventKind::RuntimeBoundToInstallation {
                installation: installation.clone(),
                runtime: runtime_principal.id().clone(),
            });
        let state = match self
            .state
            .runtime_handle(&installation, Arc::clone(&state_lease))
        {
            Ok(state) => state,
            Err(error) => {
                state_lease.revoke();
                self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::PluginFailure {
                        phase: LifecyclePhase::Activation,
                        message: error.to_string(),
                    },
                );
                self.cleanup_scope(id, LifecycleScope::new(scope_id, Vec::new()));
                self.revoke_runtime_authority(runtime_principal.id());
                if let Some(record) = self.plugins.get_mut(id) {
                    record.state = RuntimeState::Failed;
                }
                return ActivationOutcome::Failed;
            }
        };
        let mut context = ActivationContext::new(
            &definition,
            runtime_principal.id().clone(),
            installation,
            scope_id,
            state,
            Arc::clone(&self.broker),
        );
        let activation = catch_unwind(AssertUnwindSafe(|| plugin.activate(&mut context)));
        let (scope, publications, state_lease) = context.into_parts();

        match activation {
            Ok(Ok(runtime)) => {
                if let Some(missing) =
                    definition
                        .provided_capabilities()
                        .iter()
                        .find(|capability| {
                            !publications
                                .iter()
                                .any(|publication| publication.id == **capability)
                        })
                {
                    let error = PluginError::new(format!(
                        "plugin '{}' declared capability '{}' but did not publish it",
                        id, missing
                    ));
                    state_lease.revoke();
                    self.record_activation_failure(
                        id,
                        LifecyclePhase::Activation,
                        error,
                        scope,
                        runtime,
                        runtime_principal.id(),
                    );
                    ActivationOutcome::Failed
                } else {
                    for publication in &publications {
                        self.registry.publish(
                            id.clone(),
                            runtime_principal.id().clone(),
                            publication.id.clone(),
                            Arc::clone(&publication.service),
                        );
                    }
                    if let Some(record) = self.plugins.get_mut(id) {
                        record.runtime = Some(RuntimeInstance {
                            runtime,
                            scope,
                            publications,
                            runtime_principal: runtime_principal.id().clone(),
                            state_lease,
                        });
                        record.state = RuntimeState::Active;
                    }
                    self.trajectory
                        .push(id.clone(), TrajectoryEventKind::Activated);
                    ActivationOutcome::Activated
                }
            }
            Ok(Err(error)) => {
                state_lease.revoke();
                self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::PluginFailure {
                        phase: LifecyclePhase::Activation,
                        message: error.to_string(),
                    },
                );
                self.cleanup_scope(id, scope);
                self.revoke_runtime_authority(runtime_principal.id());
                if let Some(record) = self.plugins.get_mut(id) {
                    record.state = RuntimeState::Failed;
                }
                ActivationOutcome::Failed
            }
            Err(payload) => {
                state_lease.revoke();
                let message = panic_message(payload.as_ref());
                self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::PluginCrashed {
                        phase: LifecyclePhase::Activation,
                        message,
                    },
                );
                self.cleanup_scope(id, scope);
                self.revoke_runtime_authority(runtime_principal.id());
                if let Some(record) = self.plugins.get_mut(id) {
                    record.state = RuntimeState::Crashed;
                }
                ActivationOutcome::Crashed
            }
        }
    }

    fn record_activation_failure(
        &mut self,
        id: &PluginId,
        phase: LifecyclePhase,
        error: PluginError,
        scope: LifecycleScope,
        runtime: Box<dyn PluginRuntime>,
        runtime_principal: &PrincipalId,
    ) {
        self.trajectory.push(
            id.clone(),
            TrajectoryEventKind::PluginFailure {
                phase,
                message: error.to_string(),
            },
        );
        let drop_result = catch_unwind(AssertUnwindSafe(|| drop(runtime)));
        if let Err(payload) = drop_result {
            self.trajectory.push(
                id.clone(),
                TrajectoryEventKind::PluginCrashed {
                    phase: LifecyclePhase::RuntimeDrop,
                    message: panic_message(payload.as_ref()),
                },
            );
        }
        self.cleanup_scope(id, scope);
        self.revoke_runtime_authority(runtime_principal);
        if let Some(record) = self.plugins.get_mut(id) {
            record.state = RuntimeState::Failed;
        }
    }

    fn cleanup_scope(&mut self, id: &PluginId, scope: LifecycleScope) {
        self.revoke_lifecycle_scope(scope.id());
        for effect in scope.into_effects().into_iter().rev() {
            let label = effect.label().to_owned();
            self.trajectory.push(
                id.clone(),
                TrajectoryEventKind::EffectCleanupStarted {
                    effect: label.clone(),
                },
            );
            match catch_unwind(AssertUnwindSafe(|| effect.cleanup())) {
                Ok(Ok(())) => self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::EffectCleaned { effect: label },
                ),
                Ok(Err(error)) => self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::EffectCleanupFailed {
                        effect: label,
                        error: error.to_string(),
                    },
                ),
                Err(payload) => self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::EffectCleanupFailed {
                        effect: label,
                        error: format!("cleanup panicked: {}", panic_message(payload.as_ref())),
                    },
                ),
            }
        }
    }

    fn deactivate_one_with_state(&mut self, id: &PluginId, inactive_state: RuntimeState) {
        let Some(instance) = self
            .plugins
            .get_mut(id)
            .and_then(|record| record.runtime.take())
        else {
            return;
        };
        instance.state_lease.revoke();
        self.trajectory
            .push(id.clone(), TrajectoryEventKind::DeactivationStarted);
        let mut runtime = instance.runtime;
        let deactivation = catch_unwind(AssertUnwindSafe(|| runtime.deactivate()));
        match deactivation {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.trajectory.push(
                id.clone(),
                TrajectoryEventKind::PluginFailure {
                    phase: LifecyclePhase::Deactivation,
                    message: error.to_string(),
                },
            ),
            Err(payload) => self.trajectory.push(
                id.clone(),
                TrajectoryEventKind::PluginCrashed {
                    phase: LifecyclePhase::Deactivation,
                    message: panic_message(payload.as_ref()),
                },
            ),
        }
        let drop_result = catch_unwind(AssertUnwindSafe(|| drop(runtime)));
        if let Err(payload) = drop_result {
            self.trajectory.push(
                id.clone(),
                TrajectoryEventKind::PluginCrashed {
                    phase: LifecyclePhase::RuntimeDrop,
                    message: panic_message(payload.as_ref()),
                },
            );
        }
        self.cleanup_scope(id, instance.scope);
        self.revoke_runtime_authority(&instance.runtime_principal);
        for publication in &instance.publications {
            self.registry.unpublish(id, &publication.id);
        }
        if let Some(record) = self.plugins.get_mut(id) {
            record.state = inactive_state;
        }
        self.trajectory
            .push(id.clone(), TrajectoryEventKind::Deactivated);
    }

    fn revoke_runtime_authority(&self, principal: &PrincipalId) {
        let (direct_grants, descendant_grants) = self.security.revoke_subject(principal);
        for grant in direct_grants {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantRevoked { grant });
        }
        for grant in descendant_grants {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantAutoRevoked { grant });
        }
    }

    fn deactivation_order(&self, root: &PluginId) -> Vec<PluginId> {
        let is_active = self
            .plugins
            .get(root)
            .is_some_and(|record| record.state == RuntimeState::Active);
        if !is_active {
            return Vec::new();
        }

        let mut visited = BTreeSet::new();
        let mut order = Vec::new();
        visited.insert(root.clone());
        self.collect_dependents(root, &mut visited, &mut order);
        order.push(root.clone());
        order
    }

    fn collect_dependents(
        &self,
        provider: &PluginId,
        visited: &mut BTreeSet<PluginId>,
        order: &mut Vec<PluginId>,
    ) {
        for dependent in self.direct_dependents(provider) {
            if visited.insert(dependent.clone()) {
                self.collect_dependents(&dependent, visited, order);
                order.push(dependent);
            }
        }
    }

    fn direct_dependents(&self, provider: &PluginId) -> Vec<PluginId> {
        let mut dependents = Vec::new();
        let mut excluded = BTreeSet::new();
        excluded.insert(provider.clone());
        for (id, record) in &self.plugins {
            if record.state != RuntimeState::Active || id == provider {
                continue;
            }
            let depends_on_provider =
                record
                    .definition
                    .dependencies()
                    .iter()
                    .any(|(capability, kind)| {
                        *kind == crate::DependencyKind::Required
                            && self.registry.provider_for(capability).as_ref() == Some(provider)
                            && self
                                .registry
                                .provider_for_except(capability, &excluded)
                                .is_none()
                    });
            if depends_on_provider {
                dependents.push(id.clone());
            }
        }
        dependents
    }

    fn log_provider_losses(&mut self, removal_order: &[PluginId]) {
        let removal_set: BTreeSet<PluginId> = removal_order.iter().cloned().collect();
        for consumer in removal_order {
            let Some(record) = self.plugins.get(consumer) else {
                continue;
            };
            for (capability, kind) in record.definition.dependencies() {
                if *kind != crate::DependencyKind::Required {
                    continue;
                }
                let Some(provider) = self.registry.provider_for(capability) else {
                    continue;
                };
                if removal_set.contains(&provider)
                    && self
                        .registry
                        .provider_for_except(capability, &removal_set)
                        .is_none()
                {
                    self.trajectory.push(
                        consumer.clone(),
                        TrajectoryEventKind::ProviderLost {
                            capability: capability.clone(),
                            provider,
                        },
                    );
                }
            }
        }
    }
}

enum ActivationOutcome {
    Activated,
    Failed,
    Crashed,
}

fn installation_principal(record: &InstallationRecord) -> Principal {
    Principal::new(
        PrincipalId::plugin_installation(record.installation_id().as_str()),
        PrincipalKind::PluginInstallation,
    )
}
