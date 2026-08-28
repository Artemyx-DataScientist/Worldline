use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

use crate::{
    KernelError, PluginError,
    capability::{
        CapabilityDiscoveryDescriptor, CapabilityPublication, CapabilityRegistry,
        ProviderDescriptor, ProviderSelectionDiagnostic,
    },
    effect::LifecycleScope,
    error::panic_message,
    invocation::{CapabilityHandle, InvocationBroker},
    plugin::{ActivationContext, Plugin, PluginDefinition, PluginId, PluginRuntime},
    runtime::{
        ActivationMode, ActivationReason, LifecycleCancellationToken, LifecycleContext,
        LifecycleOperation, LifecycleOperationId, RestartMode, RuntimeFailureClass, RuntimeId,
        RuntimeLaunchPolicy, RuntimeLifecycleState as RuntimeState, StartupBudget,
    },
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

/// Live control-plane metadata for one activation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeMetadata {
    runtime_id: RuntimeId,
    plugin_id: PluginId,
    installation_id: InstallationId,
    principal: PrincipalId,
    scope_id: LifecycleScopeId,
    state: RuntimeState,
    activation_reason: ActivationReason,
    activation_attempt: u32,
    policy: RuntimeLaunchPolicy,
    start_tick: u64,
    activation_deadline: Option<Duration>,
    deactivation_deadline: Option<Duration>,
    last_failure: Option<RuntimeFailureClass>,
}

impl RuntimeMetadata {
    pub const fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub fn installation_id(&self) -> &InstallationId {
        &self.installation_id
    }

    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    pub const fn lifecycle_scope_id(&self) -> LifecycleScopeId {
        self.scope_id
    }

    pub const fn state(&self) -> RuntimeState {
        self.state
    }

    pub const fn activation_reason(&self) -> ActivationReason {
        self.activation_reason
    }

    pub const fn activation_attempt(&self) -> u32 {
        self.activation_attempt
    }

    pub const fn policy(&self) -> RuntimeLaunchPolicy {
        self.policy
    }

    pub const fn start_tick(&self) -> u64 {
        self.start_tick
    }

    pub const fn activation_deadline(&self) -> Option<Duration> {
        self.activation_deadline
    }

    pub const fn deactivation_deadline(&self) -> Option<Duration> {
        self.deactivation_deadline
    }

    pub const fn last_failure(&self) -> Option<RuntimeFailureClass> {
        self.last_failure
    }
}

/// Structured result of one reconciliation/boot pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconcileReport {
    /// Compatibility summary keyed by logical plugin definition.
    pub activated: Vec<PluginId>,
    pub pending: Vec<PluginId>,
    pub failed: Vec<PluginId>,
    pub crashed: Vec<PluginId>,
    /// Runtime/install-specific evidence for M0.3 callers.
    pub activated_installations: Vec<InstallationId>,
    pub active: Vec<RuntimeId>,
    pub waiting: Vec<InstallationId>,
    pub failed_runtime_ids: Vec<RuntimeId>,
    pub crashed_runtime_ids: Vec<RuntimeId>,
    pub hung_runtime_ids: Vec<RuntimeId>,
    pub quarantined_installations: Vec<InstallationId>,
    pub degraded: bool,
    pub healthy: bool,
    pub degraded_reasons: Vec<String>,
    pub startup_budget_exhausted: bool,
    pub stale_completions: Vec<RuntimeId>,
}

struct RuntimeInstance {
    runtime: Box<dyn PluginRuntime>,
    scope: LifecycleScope,
    publications: Vec<CapabilityPublication>,
    runtime_principal: PrincipalId,
    state_lease: Arc<RuntimeStateLease>,
}

struct RuntimeRecord {
    metadata: RuntimeMetadata,
    instance: Option<RuntimeInstance>,
    state_lease: Option<Arc<RuntimeStateLease>>,
    operation_id: Option<LifecycleOperationId>,
}

struct PluginRecord {
    plugin: Arc<dyn Plugin>,
    definition: PluginDefinition,
    installation_id: InstallationId,
    state: RuntimeState,
    current_runtime: Option<RuntimeId>,
    last_missing: Option<Vec<crate::CapabilityId>>,
    policy: RuntimeLaunchPolicy,
    activation_attempt: u32,
    failure_count: u32,
    next_restart_at: Option<Instant>,
    next_activation_reason: ActivationReason,
}

struct ActivationSetup {
    runtime_id: RuntimeId,
    operation_id: LifecycleOperationId,
    plugin_id: PluginId,
    installation_id: InstallationId,
    plugin: Arc<dyn Plugin>,
    context: ActivationContext,
    cancellation: LifecycleCancellationToken,
    deadline: Option<Duration>,
    scope_id: LifecycleScopeId,
}

struct PendingActivation {
    runtime_id: RuntimeId,
    operation_id: LifecycleOperationId,
    plugin_id: PluginId,
    installation_id: InstallationId,
    state_lease: Arc<RuntimeStateLease>,
    cancellation: LifecycleCancellationToken,
    deadline_at: Option<Instant>,
    receiver: Receiver<ActivationCompletion>,
}

struct ActivationCompletion {
    runtime_id: RuntimeId,
    operation_id: LifecycleOperationId,
    result: ActivationResult,
    scope: LifecycleScope,
    publications: Vec<CapabilityPublication>,
    state_lease: Arc<RuntimeStateLease>,
}

enum ActivationResult {
    Success(Box<dyn PluginRuntime>),
    Failed(PluginError),
    Crashed(String),
}

struct PendingDeactivation {
    runtime_id: RuntimeId,
    operation_id: LifecycleOperationId,
    plugin_id: PluginId,
    installation_id: InstallationId,
    principal: PrincipalId,
    scope_id: LifecycleScopeId,
    state_lease: Arc<RuntimeStateLease>,
    cancellation: LifecycleCancellationToken,
    deadline_at: Option<Instant>,
    receiver: Receiver<DeactivationCompletion>,
}

struct DeactivationCompletion {
    runtime_id: RuntimeId,
    operation_id: LifecycleOperationId,
    result: DeactivationResult,
    runtime: Box<dyn PluginRuntime>,
    scope: LifecycleScope,
}

enum DeactivationResult {
    Success,
    Failed(PluginError),
    Crashed(String),
}

pub struct Kernel {
    registry: Arc<CapabilityRegistry>,
    security: Arc<crate::security::SecurityStore>,
    broker: Arc<InvocationBroker>,
    state: Arc<StateStore>,
    /// Registration cardinality is installation-scoped. A definition may
    /// therefore have many records without overwriting another runtime.
    plugins: BTreeMap<InstallationId, PluginRecord>,
    default_installations: BTreeMap<PluginId, InstallationId>,
    runtimes: BTreeMap<RuntimeId, RuntimeRecord>,
    pending_activations: BTreeMap<RuntimeId, PendingActivation>,
    pending_deactivations: BTreeMap<RuntimeId, PendingDeactivation>,
    demanded_capabilities: BTreeSet<crate::CapabilityId>,
    memory_backend: Option<Arc<InMemoryStateBackend>>,
    trajectory: Trajectory,
    next_runtime_sequence: u64,
    next_operation_sequence: u64,
    next_start_tick: u64,
    stale_completions: Vec<RuntimeId>,
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        self.stop();
        let operation_ids: Vec<LifecycleOperationId> = self
            .pending_activations
            .values()
            .map(|operation| operation.operation_id)
            .chain(
                self.pending_deactivations
                    .values()
                    .map(|operation| operation.operation_id),
            )
            .collect();
        for operation in operation_ids {
            self.apply_cancellation(operation);
        }
    }
}

impl Kernel {
    pub fn new() -> Self {
        let backend = Arc::new(InMemoryStateBackend::new());
        Self::from_state_backend(backend.clone(), Some(backend))
            .expect("the in-memory state backend must initialize")
    }

    /// Creates a kernel over a caller-owned backend. Sharing an in-memory
    /// backend models a host restart while keeping runtime/security state
    /// ephemeral to each kernel instance.
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
        let mut default_installations = BTreeMap::new();
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
            plugins: BTreeMap::new(),
            default_installations,
            runtimes: BTreeMap::new(),
            pending_activations: BTreeMap::new(),
            pending_deactivations: BTreeMap::new(),
            demanded_capabilities: BTreeSet::new(),
            memory_backend,
            trajectory,
            next_runtime_sequence: 0,
            next_operation_sequence: 0,
            next_start_tick: 0,
            stale_completions: Vec::new(),
        })
    }

    pub fn register<P>(&mut self, plugin: P) -> Result<PluginId, KernelError>
    where
        P: Plugin + 'static,
    {
        self.register_arc(Arc::new(plugin))
    }

    pub fn register_with_policy<P>(
        &mut self,
        plugin: P,
        policy: RuntimeLaunchPolicy,
    ) -> Result<PluginId, KernelError>
    where
        P: Plugin + 'static,
    {
        self.register_arc_with_policy(Arc::new(plugin), policy)
    }

    pub fn register_arc(&mut self, plugin: Arc<dyn Plugin>) -> Result<PluginId, KernelError> {
        self.register_arc_with_policy(plugin, RuntimeLaunchPolicy::default())
    }

    pub fn register_arc_with_policy(
        &mut self,
        plugin: Arc<dyn Plugin>,
        policy: RuntimeLaunchPolicy,
    ) -> Result<PluginId, KernelError> {
        let definition = self.read_definition(&plugin)?;
        let id = definition.id().clone();
        let installation =
            self.ensure_default_installation(&id, definition.state_schema_version())?;
        self.register_arc_for_installation_with_definition(plugin, definition, installation, policy)
    }

    pub fn register_for_installation<P>(
        &mut self,
        plugin: P,
        installation: impl Into<InstallationId>,
    ) -> Result<PluginId, KernelError>
    where
        P: Plugin + 'static,
    {
        self.register_for_installation_with_policy(
            plugin,
            installation,
            RuntimeLaunchPolicy::default(),
        )
    }

    pub fn register_for_installation_with_policy<P>(
        &mut self,
        plugin: P,
        installation: impl Into<InstallationId>,
        policy: RuntimeLaunchPolicy,
    ) -> Result<PluginId, KernelError>
    where
        P: Plugin + 'static,
    {
        self.register_arc_for_installation_with_policy(Arc::new(plugin), installation, policy)
    }

    pub fn register_arc_for_installation(
        &mut self,
        plugin: Arc<dyn Plugin>,
        installation: impl Into<InstallationId>,
    ) -> Result<PluginId, KernelError> {
        self.register_arc_for_installation_with_policy(
            plugin,
            installation,
            RuntimeLaunchPolicy::default(),
        )
    }

    pub fn register_arc_for_installation_with_policy(
        &mut self,
        plugin: Arc<dyn Plugin>,
        installation: impl Into<InstallationId>,
        policy: RuntimeLaunchPolicy,
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
        self.register_arc_for_installation_with_definition(plugin, definition, installation, policy)
    }

    fn register_arc_for_installation_with_definition(
        &mut self,
        plugin: Arc<dyn Plugin>,
        definition: PluginDefinition,
        installation: InstallationId,
        policy: RuntimeLaunchPolicy,
    ) -> Result<PluginId, KernelError> {
        if self.plugins.contains_key(&installation) {
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
            installation.clone(),
            PluginRecord {
                plugin,
                definition,
                installation_id: installation,
                state: RuntimeState::Registered,
                current_runtime: None,
                last_missing: None,
                policy,
                activation_attempt: 0,
                failure_count: 0,
                next_restart_at: None,
                next_activation_reason: ActivationReason::Boot,
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

    /// Creates persistent installation metadata. The first installation for a
    /// definition remains the compatibility target of `register`.
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
        self.default_installations
            .get(plugin)
            .filter(|installation| self.plugins.contains_key(*installation))
            .cloned()
            .or_else(|| {
                self.plugins
                    .values()
                    .find(|record| record.definition.id() == plugin)
                    .map(|record| record.installation_id.clone())
            })
            .or_else(|| {
                let installations = self.state.records_for_plugin(plugin);
                (installations.len() == 1).then(|| installations[0].installation_id().clone())
            })
    }

    pub fn principal_for_installation(&self, installation: &InstallationId) -> Option<PrincipalId> {
        self.state
            .record(installation)
            .map(|record| PrincipalId::plugin_installation(record.installation_id().as_str()))
            .filter(|principal| self.security.principal_exists(principal))
    }

    /// Returns a state handle for trusted kernel-side code. Plugin runtimes
    /// receive their lease-bound handle through `ActivationContext`.
    pub fn state_handle(&self, installation: &InstallationId) -> Result<StateHandle, StateError> {
        self.state.handle(installation)
    }

    pub fn state_handle_for_plugin(
        &self,
        plugin: &PluginId,
        installation: &InstallationId,
    ) -> Result<StateHandle, StateError> {
        let record = if let Some(record) = self
            .plugins
            .get(installation)
            .filter(|record| record.definition.id() == plugin)
        {
            record
        } else if let Some(expected) = self
            .plugins
            .values()
            .find(|record| record.definition.id() == plugin)
            .map(|record| record.installation_id.clone())
        {
            return Err(StateError::RuntimeInstallationMismatch {
                expected,
                actual: installation.clone(),
            });
        } else {
            return Err(StateError::StateAccessDenied {
                installation: installation.clone(),
            });
        };
        if &record.installation_id != installation {
            return Err(StateError::RuntimeInstallationMismatch {
                expected: record.installation_id.clone(),
                actual: installation.clone(),
            });
        }
        self.state.handle(installation)
    }

    pub fn fail_next_state_commit(&self) {
        if let Some(backend) = &self.memory_backend {
            backend.fail_next_commit();
        }
    }

    pub fn fail_next_state_record_update(&self) {
        if let Some(backend) = &self.memory_backend {
            backend.fail_next_record_update();
        }
    }

    pub fn fail_state_record_update_after(&self, successful_updates: u64) {
        if let Some(backend) = &self.memory_backend {
            backend.fail_record_update_after(successful_updates);
        }
    }

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
        if self.plugins.contains_key(installation) {
            self.unregister_installation(installation)?;
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
            if let Some(replacement) = self
                .state
                .records_for_plugin(&plugin_id)
                .first()
                .map(|record| record.installation_id().clone())
            {
                self.default_installations.insert(plugin_id, replacement);
            }
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
        self.log_grant_revocations(direct_grants, descendant_grants);
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
        self.reconcile_with_budget(StartupBudget::default())
    }

    pub fn reconcile_with_budget(&mut self, budget: StartupBudget) -> ReconcileReport {
        self.poll_lifecycle_internal();
        let started = Instant::now();
        let mut report = ReconcileReport::default();
        let mut activation_count = 0usize;

        loop {
            self.prepare_restarts();
            self.refresh_dependency_resolution();
            self.deactivate_unavailable_consumers();
            self.poll_lifecycle_internal();
            let Some(installation) = self.next_eligible_installation() else {
                break;
            };
            if activation_count >= budget.max_simultaneous_activations()
                || budget
                    .overall_boot_deadline()
                    .is_some_and(|deadline| started.elapsed() >= deadline)
            {
                report.startup_budget_exhausted = true;
                break;
            }
            activation_count += 1;
            let (reason, deadline) = self
                .plugins
                .get(&installation)
                .map(|record| {
                    (
                        record.next_activation_reason,
                        min_duration(
                            budget.per_runtime_activation_deadline(),
                            record.policy.activation_deadline(),
                        ),
                    )
                })
                .unwrap_or((
                    ActivationReason::Boot,
                    budget.per_runtime_activation_deadline(),
                ));
            let outcome = self.activate_plugin(&installation, reason, deadline);
            match outcome {
                ActivationOutcome::Activated {
                    plugin_id,
                    installation,
                    runtime_id,
                } => {
                    report.activated.push(plugin_id);
                    report.activated_installations.push(installation);
                    if let Some(runtime_id) = runtime_id {
                        report.active.push(runtime_id);
                    }
                }
                ActivationOutcome::Failed {
                    plugin_id,
                    runtime_id,
                    installation,
                } => {
                    report.failed.push(plugin_id);
                    if let Some(runtime_id) = runtime_id {
                        report.failed_runtime_ids.push(runtime_id);
                    }
                    report
                        .activated_installations
                        .retain(|candidate| candidate != &installation);
                }
                ActivationOutcome::Crashed {
                    plugin_id,
                    runtime_id,
                    installation,
                } => {
                    report.crashed.push(plugin_id);
                    if let Some(runtime_id) = runtime_id {
                        report.crashed_runtime_ids.push(runtime_id);
                    }
                    report
                        .activated_installations
                        .retain(|candidate| candidate != &installation);
                }
                ActivationOutcome::Hung {
                    plugin_id,
                    runtime_id,
                    installation,
                } => {
                    report.failed.push(plugin_id);
                    if let Some(runtime_id) = runtime_id {
                        report.hung_runtime_ids.push(runtime_id);
                    }
                    report
                        .activated_installations
                        .retain(|candidate| candidate != &installation);
                }
            }
        }

        self.poll_lifecycle_internal();
        self.refresh_dependency_resolution();
        self.deactivate_unavailable_consumers();
        self.poll_lifecycle_internal();
        self.extend_report_from_state(&mut report);
        if report.startup_budget_exhausted {
            report
                .degraded_reasons
                .push("startup budget exhausted".to_owned());
        }
        report.degraded = !report.degraded_reasons.is_empty();
        report.healthy = !report.degraded;
        if report.degraded {
            self.trajectory
                .push_security(TrajectoryEventKind::BootDegraded {
                    reasons: report.degraded_reasons.clone(),
                });
        }
        report.stale_completions = std::mem::take(&mut self.stale_completions);
        report
    }

    pub fn start(&mut self) -> ReconcileReport {
        self.start_with_budget(StartupBudget::default())
    }

    /// Restarts stopped installations under an explicit startup budget.
    /// Budget exhaustion leaves deferred installations registered for a later
    /// reconcile pass instead of classifying them as plugin failures.
    pub fn start_with_budget(&mut self, budget: StartupBudget) -> ReconcileReport {
        for record in self.plugins.values_mut() {
            if record.state == RuntimeState::Stopped {
                record.state = RuntimeState::Registered;
                record.last_missing = None;
                record.next_activation_reason = ActivationReason::Boot;
            }
        }
        self.reconcile_with_budget(budget)
    }

    pub fn stop(&mut self) {
        self.poll_lifecycle_internal();
        let pending: Vec<LifecycleOperationId> = self
            .pending_activations
            .values()
            .map(|operation| operation.operation_id)
            .collect();
        for operation in pending {
            self.apply_cancellation(operation);
        }
        let roots: Vec<RuntimeId> = self
            .runtimes
            .values()
            .filter(|record| record.metadata.state == RuntimeState::Active)
            .map(|record| record.metadata.runtime_id)
            .collect();
        for root in roots {
            if self
                .runtimes
                .get(&root)
                .is_some_and(|record| record.metadata.state == RuntimeState::Active)
            {
                let order = self.deactivation_order(root);
                self.log_provider_losses(&order);
                for runtime_id in order {
                    self.deactivate_runtime_sync(runtime_id, RuntimeState::Stopped);
                }
            }
        }
    }

    /// Removes one installation registration while leaving other installations
    /// of the same definition untouched.
    pub fn unregister_installation(
        &mut self,
        installation: &InstallationId,
    ) -> Result<(), KernelError> {
        let plugin_id = self
            .plugins
            .get(installation)
            .map(|record| record.definition.id().clone())
            .ok_or_else(|| KernelError::UnknownPlugin {
                id: self
                    .state
                    .record(installation)
                    .map(|record| record.plugin_id().clone())
                    .unwrap_or_else(|| PluginId::new(installation.as_str())),
            })?;
        self.poll_lifecycle_internal();
        if let Some(runtime_id) = self
            .plugins
            .get(installation)
            .and_then(|record| record.current_runtime)
        {
            if self
                .runtimes
                .get(&runtime_id)
                .is_some_and(|runtime| runtime.metadata.state == RuntimeState::Active)
            {
                let order = self.deactivation_order(runtime_id);
                self.log_provider_losses(&order);
                for runtime_id in order {
                    self.deactivate_runtime_sync(runtime_id, RuntimeState::Pending);
                }
            } else {
                self.cancel_pending_for_installation(installation);
            }
            self.revoke_runtime_if_needed(runtime_id);
        }
        self.cancel_pending_for_installation(installation);
        self.plugins.remove(installation);
        if self
            .default_installations
            .get(&plugin_id)
            .is_some_and(|default| default == installation)
        {
            self.default_installations.remove(&plugin_id);
            if let Some(replacement) = self
                .plugins
                .values()
                .find(|record| record.definition.id() == &plugin_id)
                .map(|record| record.installation_id.clone())
                .or_else(|| {
                    self.state
                        .records_for_plugin(&plugin_id)
                        .into_iter()
                        .find(|record| record.installation_id() != installation)
                        .map(|record| record.installation_id().clone())
                })
            {
                self.default_installations
                    .insert(plugin_id.clone(), replacement);
            }
        }
        self.trajectory
            .push(plugin_id.clone(), TrajectoryEventKind::Unregistered);
        self.reconcile();
        Ok(())
    }

    pub fn unregister(&mut self, id: &PluginId) -> Result<(), KernelError> {
        let installation = self
            .installation_for_plugin(id)
            .ok_or_else(|| KernelError::UnknownPlugin { id: id.clone() })?;
        self.unregister_installation(&installation)
    }

    pub fn plugin_state(&self, id: &PluginId) -> Option<RuntimeState> {
        self.installation_for_plugin(id)
            .and_then(|installation| self.plugins.get(&installation))
            .map(|record| record.state)
    }

    pub fn plugin_state_for_installation(
        &self,
        installation: &InstallationId,
    ) -> Option<RuntimeState> {
        self.plugins.get(installation).map(|record| record.state)
    }

    pub fn registered_plugin_ids(&self) -> Vec<PluginId> {
        self.plugins
            .values()
            .map(|record| record.definition.id().clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
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
        let installation = self.installation_for_plugin(plugin)?;
        let runtime_id = self.plugins.get(&installation)?.current_runtime?;
        self.principal_for_runtime(&runtime_id)
    }

    pub fn principal_for_runtime(&self, runtime_id: &RuntimeId) -> Option<PrincipalId> {
        self.runtimes
            .get(runtime_id)
            .map(|record| record.metadata.principal.clone())
            .filter(|principal| self.security.principal_exists(principal))
    }

    pub fn runtime_metadata(&self, runtime_id: &RuntimeId) -> Option<RuntimeMetadata> {
        self.runtimes
            .get(runtime_id)
            .map(|record| record.metadata.clone())
    }

    pub fn runtime_id_for_installation(&self, installation: &InstallationId) -> Option<RuntimeId> {
        self.plugins.get(installation)?.current_runtime
    }

    pub fn runtime_id_for_plugin(&self, plugin: &PluginId) -> Option<RuntimeId> {
        let installation = self.installation_for_plugin(plugin)?;
        self.runtime_id_for_installation(&installation)
    }

    pub fn live_runtime_id_for_installation(
        &self,
        installation: &InstallationId,
    ) -> Option<RuntimeId> {
        let runtime_id = self.runtime_id_for_installation(installation)?;
        self.runtimes
            .get(&runtime_id)
            .filter(|runtime| {
                matches!(
                    runtime.metadata.state,
                    RuntimeState::Active | RuntimeState::Activating | RuntimeState::Deactivating
                )
            })
            .map(|_| runtime_id)
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
        if let Some(first) = revoked.first() {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantRevoked {
                    grant: first.clone(),
                });
            for grant in revoked.iter().skip(1) {
                self.trajectory
                    .push_security(TrajectoryEventKind::GrantAutoRevoked {
                        grant: grant.clone(),
                    });
            }
        }
        Ok(())
    }

    pub fn is_grant_active(&self, id: &GrantId) -> bool {
        self.security
            .grant(id)
            .is_some_and(|grant| grant.status() == GrantStatus::Active)
    }

    pub fn lifecycle_scope_for(&self, plugin: &PluginId) -> Option<LifecycleScopeId> {
        self.runtime_id_for_plugin(plugin)
            .and_then(|runtime_id| self.runtimes.get(&runtime_id))
            .map(|record| record.metadata.scope_id)
    }

    pub fn lifecycle_scope_for_installation(
        &self,
        installation: &InstallationId,
    ) -> Option<LifecycleScopeId> {
        self.live_runtime_id_for_installation(installation)
            .and_then(|runtime_id| self.runtimes.get(&runtime_id))
            .map(|record| record.metadata.scope_id)
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

    /// Submits an invocation to the broker. Authorization is admission-time.
    pub fn invoke(
        &self,
        request: crate::InvocationRequest,
    ) -> Result<Vec<u8>, crate::CapabilityError> {
        self.broker.invoke(request)
    }

    /// Selects a compatible active provider and records the policy rationale.
    /// Selection never issues or widens caller authority.
    pub fn select_provider(
        &self,
        required: &crate::CapabilityId,
    ) -> Option<(ProviderDescriptor, ProviderSelectionDiagnostic)> {
        let (selected, diagnostic) = self.registry.selection(required, &BTreeSet::new());
        self.trajectory
            .push_security(TrajectoryEventKind::ProviderSelectionMade {
                requested: diagnostic.requested().clone(),
                compatible_candidate_count: diagnostic.compatible_candidate_count(),
                selected_runtime_id: diagnostic.selected_runtime_id(),
                selected_installation: diagnostic.selected_installation_id().cloned(),
                policy: diagnostic.policy().to_owned(),
                reason: diagnostic.reason().to_owned(),
            });
        self.trajectory
            .push_security(TrajectoryEventKind::CapabilityVersionNegotiated {
                requested: required.clone(),
                selected: diagnostic.negotiated_capability().cloned(),
            });
        selected.map(|provider| (provider.descriptor, diagnostic))
    }

    /// Returns declared providers, including eligible inactive lazy/eager
    /// installations, without creating a capability handle or authority.
    pub fn discover_capabilities(&self) -> Vec<CapabilityDiscoveryDescriptor> {
        let mut descriptors = Vec::new();
        for record in self.plugins.values() {
            if !matches!(
                record.state,
                RuntimeState::Active
                    | RuntimeState::Registered
                    | RuntimeState::Pending
                    | RuntimeState::WaitingDependencies
            ) {
                continue;
            }
            let runtime_id = record.current_runtime.filter(|runtime_id| {
                self.runtimes
                    .get(runtime_id)
                    .is_some_and(|runtime| runtime.metadata.state == RuntimeState::Active)
            });
            for capability in record.definition.provided_capabilities() {
                descriptors.push(CapabilityDiscoveryDescriptor::new(
                    capability.clone(),
                    record.definition.id().clone(),
                    record.installation_id.clone(),
                    runtime_id,
                    record.state,
                    record.policy.activation_mode(),
                ));
            }
        }
        descriptors.sort_by(|left, right| {
            left.capability()
                .cmp(right.capability())
                .then_with(|| left.installation_id().cmp(right.installation_id()))
                .then_with(|| left.runtime_id().cmp(&right.runtime_id()))
        });
        descriptors
    }

    pub fn discover_capabilities_for(
        &self,
        capability: &crate::CapabilityId,
    ) -> Vec<CapabilityDiscoveryDescriptor> {
        self.discover_capabilities()
            .into_iter()
            .filter(|descriptor| descriptor.capability().is_compatible_with(capability))
            .collect()
    }

    /// Requests a capability from the host composition.  This is a launch
    /// demand only: it can make a compatible lazy installation eligible, but
    /// it never creates caller authority or a capability handle.
    pub fn demand_capability(
        &mut self,
        capability: impl Into<crate::CapabilityId>,
    ) -> ReconcileReport {
        self.demanded_capabilities.insert(capability.into());
        self.reconcile()
    }

    /// Starts activation on a worker thread. Plugin code executes without a
    /// registry lock; completion must be committed through `poll_lifecycle`.
    pub fn begin_activation(
        &mut self,
        plugin: &PluginId,
    ) -> Result<LifecycleOperation, KernelError> {
        let installation = self
            .installation_for_plugin(plugin)
            .ok_or_else(|| KernelError::UnknownPlugin { id: plugin.clone() })?;
        self.begin_activation_for_installation(&installation)
    }

    pub fn begin_activation_for_installation(
        &mut self,
        installation: &InstallationId,
    ) -> Result<LifecycleOperation, KernelError> {
        self.poll_lifecycle_internal();
        self.refresh_dependency_resolution();
        let state = self
            .plugins
            .get(installation)
            .map(|record| record.state)
            .ok_or_else(|| KernelError::UnknownPlugin {
                id: self
                    .state
                    .record(installation)
                    .map(|record| record.plugin_id().clone())
                    .unwrap_or_else(|| PluginId::new(installation.as_str())),
            })?;
        if state == RuntimeState::Quarantined {
            return Err(KernelError::RuntimeQuarantined {
                installation: installation.clone(),
            });
        }
        if state != RuntimeState::Registered {
            return if self
                .plugins
                .get(installation)
                .and_then(|record| record.last_missing.as_ref())
                .is_some_and(|missing| !missing.is_empty())
            {
                Err(KernelError::NoCompatibleProvider {
                    capability: self
                        .plugins
                        .get(installation)
                        .and_then(|record| record.last_missing.as_ref())
                        .and_then(|missing| missing.first())
                        .cloned()
                        .expect("missing dependency must contain one capability"),
                })
            } else {
                Err(KernelError::RuntimeAlreadyActiveForInstallation {
                    installation: installation.clone(),
                })
            };
        }
        let policy = self
            .plugins
            .get(installation)
            .map(|record| record.policy)
            .expect("activation record was checked above");
        let setup = self.create_activation_setup(
            installation,
            ActivationReason::Explicit,
            policy.activation_deadline(),
        )?;
        let ActivationSetup {
            runtime_id,
            operation_id,
            plugin_id,
            installation_id,
            plugin,
            context,
            cancellation,
            deadline,
            scope_id,
        } = setup;
        let (sender, receiver) = mpsc::channel();
        let worker_runtime_id = runtime_id;
        let worker_operation_id = operation_id;
        if let Err(error) = std::thread::Builder::new()
            .name(format!("worldline-activate-{runtime_id}"))
            .spawn(move || {
                let mut context = context;
                let result = catch_unwind(AssertUnwindSafe(|| plugin.activate(&mut context)));
                let (scope, publications, state_lease) = context.into_parts();
                let result = match result {
                    Ok(Ok(runtime)) => ActivationResult::Success(runtime),
                    Ok(Err(error)) => ActivationResult::Failed(error),
                    Err(payload) => ActivationResult::Crashed(panic_message(payload.as_ref())),
                };
                let _ = sender.send(ActivationCompletion {
                    runtime_id: worker_runtime_id,
                    operation_id: worker_operation_id,
                    result,
                    scope,
                    publications,
                    state_lease,
                });
            })
        {
            let lease = self
                .runtimes
                .get(&runtime_id)
                .and_then(|runtime| runtime.state_lease.clone())
                .expect("runtime state lease must exist");
            self.finalize_activation_failure(
                runtime_id,
                plugin_id,
                installation_id,
                RuntimeState::Failed,
                RuntimeFailureClass::PluginError,
                error.to_string(),
                LifecycleScope::new(scope_id, Vec::new()),
                lease,
                None,
            );
            return Err(KernelError::RuntimeActivationFailed {
                runtime: runtime_id,
                message: error.to_string(),
            });
        }
        let deadline_at = deadline.map(|duration| Instant::now() + duration);
        let state_lease = self
            .runtimes
            .get(&runtime_id)
            .and_then(|runtime| runtime.state_lease.clone())
            .expect("runtime state lease must exist");
        self.pending_activations.insert(
            runtime_id,
            PendingActivation {
                runtime_id,
                operation_id,
                plugin_id,
                installation_id,
                state_lease,
                cancellation: cancellation.clone(),
                deadline_at,
                receiver,
            },
        );
        if let Some(runtime) = self.runtimes.get_mut(&runtime_id) {
            runtime.operation_id = Some(operation_id);
        }
        Ok(LifecycleOperation::new(
            operation_id,
            runtime_id,
            cancellation,
            deadline,
        ))
    }

    /// Starts deactivation asynchronously. Publications and future provider
    /// selection are removed before plugin teardown begins.
    pub fn begin_deactivation_for_installation(
        &mut self,
        installation: &InstallationId,
    ) -> Result<LifecycleOperation, KernelError> {
        self.poll_lifecycle_internal();
        let runtime_id = self
            .live_runtime_id_for_installation(installation)
            .ok_or_else(|| KernelError::UnknownRuntime {
                runtime: self
                    .runtime_id_for_installation(installation)
                    .unwrap_or_else(|| RuntimeId::new(0, 0)),
            })?;
        let (plugin_id, principal, policy, instance) = {
            let runtime =
                self.runtimes
                    .get_mut(&runtime_id)
                    .ok_or(KernelError::UnknownRuntime {
                        runtime: runtime_id,
                    })?;
            if runtime.metadata.state != RuntimeState::Active {
                return Err(KernelError::InvalidRuntimeTransition {
                    runtime: runtime_id,
                    from: runtime.metadata.state,
                    to: RuntimeState::Deactivating,
                });
            }
            let instance = runtime
                .instance
                .take()
                .expect("active runtime must own an instance");
            (
                runtime.metadata.plugin_id.clone(),
                runtime.metadata.principal.clone(),
                runtime.metadata.policy,
                instance,
            )
        };
        for publication in &instance.publications {
            self.registry.unpublish(&runtime_id, &publication.id);
        }
        instance.state_lease.revoke();
        self.transition_runtime(runtime_id, RuntimeState::Deactivating)?;
        if let Some(record) = self.plugins.get_mut(installation) {
            record.state = RuntimeState::Deactivating;
        }
        self.trajectory
            .push(plugin_id.clone(), TrajectoryEventKind::DeactivationStarted);
        self.trajectory.push(
            plugin_id.clone(),
            TrajectoryEventKind::RuntimeDeactivationStarted {
                runtime_id,
                installation: installation.clone(),
            },
        );
        let operation_id = self.next_operation_id();
        let cancellation = LifecycleCancellationToken::default();
        let deadline = policy.deactivation_deadline();
        let lifecycle_context = LifecycleContext::new(runtime_id, cancellation.clone(), deadline);
        let (sender, receiver) = mpsc::channel();
        let scope_id = instance.scope.id();
        let state_lease = Arc::clone(&instance.state_lease);
        let error_lease = Arc::clone(&state_lease);
        if let Err(error) = std::thread::Builder::new()
            .name(format!("worldline-deactivate-{runtime_id}"))
            .spawn(move || {
                let RuntimeInstance {
                    mut runtime,
                    scope,
                    publications: _,
                    runtime_principal: _,
                    state_lease: _,
                } = instance;
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.deactivate_with_context(&lifecycle_context)
                }));
                let result = match result {
                    Ok(Ok(())) => DeactivationResult::Success,
                    Ok(Err(error)) => DeactivationResult::Failed(error),
                    Err(payload) => DeactivationResult::Crashed(panic_message(payload.as_ref())),
                };
                let _ = sender.send(DeactivationCompletion {
                    runtime_id,
                    operation_id,
                    result,
                    runtime,
                    scope,
                });
            })
        {
            // The runtime was already unpublished and its lease revoked. The
            // failed spawn drops the owned callback closure; keep the kernel
            // state terminal even though no worker ran.
            self.cleanup_scope(&plugin_id, LifecycleScope::new(scope_id, Vec::new()));
            error_lease.revoke();
            self.revoke_runtime_authority(&principal);
            self.set_runtime_terminal(
                runtime_id,
                RuntimeState::Failed,
                Some(RuntimeFailureClass::PluginError),
            );
            if let Some(record) = self.plugins.get_mut(installation) {
                record.state = RuntimeState::Failed;
            }
            return Err(KernelError::RuntimeDeactivationFailed {
                runtime: runtime_id,
                message: error.to_string(),
            });
        }
        let deadline_at = deadline.map(|duration| Instant::now() + duration);
        self.pending_deactivations.insert(
            runtime_id,
            PendingDeactivation {
                runtime_id,
                operation_id,
                plugin_id,
                installation_id: installation.clone(),
                principal,
                scope_id,
                state_lease,
                cancellation: cancellation.clone(),
                deadline_at,
                receiver,
            },
        );
        if let Some(runtime) = self.runtimes.get_mut(&runtime_id) {
            runtime.operation_id = Some(operation_id);
        }
        Ok(LifecycleOperation::new(
            operation_id,
            runtime_id,
            cancellation,
            deadline,
        ))
    }

    /// Applies cancellation to a lifecycle operation. Repeating the call is
    /// safe and does not create duplicate lifecycle transitions.
    pub fn cancel_lifecycle(
        &mut self,
        operation: &LifecycleOperation,
    ) -> Result<bool, KernelError> {
        let first = operation.cancel();
        self.apply_cancellation(operation.id());
        Ok(first)
    }

    /// Polls split-phase lifecycle completions and returns current boot health.
    pub fn poll_lifecycle(&mut self) -> ReconcileReport {
        self.poll_lifecycle_internal();
        let mut report = ReconcileReport::default();
        self.refresh_dependency_resolution();
        self.extend_report_from_state(&mut report);
        report.stale_completions = std::mem::take(&mut self.stale_completions);
        report.degraded = !report.degraded_reasons.is_empty();
        report.healthy = !report.degraded;
        report
    }

    /// Marks lifecycle work logically hung. In-process native code may remain
    /// physically alive; its authority/publications are still isolated.
    pub fn mark_runtime_hung(&mut self, runtime_id: RuntimeId) -> Result<(), KernelError> {
        if !self.runtimes.contains_key(&runtime_id) {
            return Err(KernelError::UnknownRuntime {
                runtime: runtime_id,
            });
        }
        self.mark_runtime_hung_internal(runtime_id);
        Ok(())
    }

    pub fn recover_installation(
        &mut self,
        installation: &InstallationId,
    ) -> Result<ReconcileReport, KernelError> {
        let record =
            self.plugins
                .get_mut(installation)
                .ok_or_else(|| KernelError::UnknownPlugin {
                    id: self
                        .state
                        .record(installation)
                        .map(|record| record.plugin_id().clone())
                        .unwrap_or_else(|| PluginId::new(installation.as_str())),
                })?;
        if record.state != RuntimeState::Quarantined
            && !matches!(
                record.state,
                RuntimeState::Failed | RuntimeState::Crashed | RuntimeState::Hung
            )
        {
            return Err(KernelError::RuntimeQuarantined {
                installation: installation.clone(),
            });
        }
        record.state = RuntimeState::Registered;
        record.failure_count = 0;
        record.next_restart_at = None;
        record.next_activation_reason = ActivationReason::Recovery;
        Ok(self.reconcile())
    }

    fn refresh_dependency_resolution(&mut self) {
        let installations: Vec<InstallationId> = self.plugins.keys().cloned().collect();
        for installation in installations {
            let Some((state, definition)) = self
                .plugins
                .get(&installation)
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
            for capability in &missing {
                if self.has_eligible_lazy_provider(capability) {
                    self.demanded_capabilities.insert(capability.clone());
                }
            }
            let changed = self
                .plugins
                .get(&installation)
                .and_then(|record| record.last_missing.as_ref())
                != Some(&missing);
            if changed {
                if let Some(record) = self.plugins.get_mut(&installation) {
                    record.last_missing = Some(missing.clone());
                    record.state = if missing.is_empty() {
                        RuntimeState::Registered
                    } else {
                        RuntimeState::Pending
                    };
                }
                self.trajectory.push(
                    definition.id().clone(),
                    TrajectoryEventKind::DependencyResolution {
                        missing: missing.clone(),
                    },
                );
                if !missing.is_empty() {
                    self.trajectory.push(
                        definition.id().clone(),
                        TrajectoryEventKind::RuntimeWaitingDependencies {
                            installation: installation.clone(),
                            plugin: definition.id().clone(),
                            missing,
                        },
                    );
                }
            } else if let Some(record) = self.plugins.get_mut(&installation) {
                record.state = if missing.is_empty() {
                    RuntimeState::Registered
                } else {
                    RuntimeState::Pending
                };
            }
        }
    }

    fn next_eligible_installation(&self) -> Option<InstallationId> {
        if let Some((installation, _)) = self.plugins.iter().find(|(_, record)| {
            record.state == RuntimeState::Registered
                && record.policy.activation_mode() == ActivationMode::Eager
        }) {
            return Some(installation.clone());
        }

        let mut lazy_candidates: Vec<(InstallationId, u16)> = self
            .plugins
            .iter()
            .filter_map(|(installation, record)| {
                (record.state == RuntimeState::Registered
                    && record.policy.activation_mode() == ActivationMode::Lazy)
                    .then(|| {
                        self.best_demanded_provider_minor(record)
                            .map(|minor| (installation.clone(), minor))
                    })
                    .flatten()
            })
            .collect();
        lazy_candidates.sort_by(
            |(left_installation, left_minor), (right_installation, right_minor)| {
                right_minor
                    .cmp(left_minor)
                    .then_with(|| left_installation.cmp(right_installation))
            },
        );
        lazy_candidates
            .into_iter()
            .next()
            .map(|(installation, _)| installation)
    }

    fn best_demanded_provider_minor(&self, record: &PluginRecord) -> Option<u16> {
        record
            .definition
            .provided_capabilities()
            .iter()
            .filter(|capability| {
                self.demanded_capabilities.iter().any(|required| {
                    capability.is_compatible_with(required) && !self.registry.has_provider(required)
                })
            })
            .map(|capability| capability.interface_version().minor())
            .max()
    }

    fn has_eligible_lazy_provider(&self, required: &crate::CapabilityId) -> bool {
        self.plugins.values().any(|record| {
            record.state == RuntimeState::Registered
                && record.policy.activation_mode() == ActivationMode::Lazy
                && record
                    .definition
                    .provided_capabilities()
                    .iter()
                    .any(|provided| provided.is_compatible_with(required))
        })
    }

    fn activate_plugin(
        &mut self,
        installation: &InstallationId,
        reason: ActivationReason,
        deadline: Option<Duration>,
    ) -> ActivationOutcome {
        let (plugin_id, last_runtime) = self
            .plugins
            .get(installation)
            .map(|record| (record.definition.id().clone(), record.current_runtime))
            .unwrap_or_else(|| (PluginId::new(installation.as_str()), None));
        let setup = match self.create_activation_setup(installation, reason, deadline) {
            Ok(setup) => setup,
            Err(error) => {
                if let Some(record) = self.plugins.get_mut(installation) {
                    record.state = RuntimeState::Failed;
                }
                self.trajectory.push(
                    plugin_id.clone(),
                    TrajectoryEventKind::PluginFailure {
                        phase: LifecyclePhase::Activation,
                        message: error.to_string(),
                    },
                );
                return ActivationOutcome::Failed {
                    plugin_id,
                    runtime_id: last_runtime,
                    installation: installation.clone(),
                };
            }
        };
        let ActivationSetup {
            runtime_id,
            operation_id,
            plugin_id,
            installation_id,
            plugin,
            mut context,
            cancellation: _,
            deadline,
            scope_id: _,
        } = setup;
        let started = Instant::now();
        let activation = catch_unwind(AssertUnwindSafe(|| plugin.activate(&mut context)));
        let (scope, publications, state_lease) = context.into_parts();
        let result = match activation {
            Ok(Ok(runtime)) => ActivationResult::Success(runtime),
            Ok(Err(error)) => ActivationResult::Failed(error),
            Err(payload) => ActivationResult::Crashed(panic_message(payload.as_ref())),
        };
        let timed_out = deadline.is_some_and(|deadline| started.elapsed() > deadline);
        self.finish_activation_result(
            runtime_id,
            operation_id,
            plugin_id,
            installation_id,
            result,
            scope,
            publications,
            state_lease,
            timed_out,
        )
    }

    fn create_activation_setup(
        &mut self,
        installation: &InstallationId,
        reason: ActivationReason,
        deadline: Option<Duration>,
    ) -> Result<ActivationSetup, KernelError> {
        let current_runtime = self
            .plugins
            .get(installation)
            .ok_or_else(|| KernelError::UnknownPlugin {
                id: PluginId::new(installation.as_str()),
            })?
            .current_runtime;
        if let Some(runtime_id) = current_runtime
            && self.runtimes.get(&runtime_id).is_some_and(|runtime| {
                matches!(
                    runtime.metadata.state,
                    RuntimeState::Active | RuntimeState::Activating | RuntimeState::Deactivating
                )
            })
        {
            return Err(KernelError::RuntimeAlreadyActiveForInstallation {
                installation: installation.clone(),
            });
        }
        let (plugin, definition, policy, activation_attempt) = {
            let record = self
                .plugins
                .get_mut(installation)
                .expect("installation was checked above");
            record.activation_attempt = record.activation_attempt.saturating_add(1);
            (
                Arc::clone(&record.plugin),
                record.definition.clone(),
                record.policy,
                record.activation_attempt,
            )
        };
        let runtime_generation = self.state.allocate_runtime_generation(installation)?;
        self.next_runtime_sequence = self.next_runtime_sequence.saturating_add(1);
        let runtime_id = RuntimeId::new(runtime_generation, self.next_runtime_sequence);
        let operation_id = self.next_operation_id();
        let scope_id = self.security.allocate_scope();
        let runtime_principal = self.security.allocate_runtime_principal(
            definition.id().as_str(),
            runtime_id,
            runtime_generation,
        );
        let state_lease = RuntimeStateLease::new();
        let effective_deadline = min_duration(deadline, policy.activation_deadline());
        self.next_start_tick = self.next_start_tick.saturating_add(1);
        self.runtimes.insert(
            runtime_id,
            RuntimeRecord {
                metadata: RuntimeMetadata {
                    runtime_id,
                    plugin_id: definition.id().clone(),
                    installation_id: installation.clone(),
                    principal: runtime_principal.id().clone(),
                    scope_id,
                    state: RuntimeState::Created,
                    activation_reason: reason,
                    activation_attempt,
                    policy,
                    start_tick: self.next_start_tick,
                    activation_deadline: effective_deadline,
                    deactivation_deadline: policy.deactivation_deadline(),
                    last_failure: None,
                },
                instance: None,
                state_lease: Some(Arc::clone(&state_lease)),
                operation_id: Some(operation_id),
            },
        );
        if let Some(record) = self.plugins.get_mut(installation) {
            record.current_runtime = Some(runtime_id);
            record.state = RuntimeState::Activating;
            record.last_missing = Some(Vec::new());
            if reason == ActivationReason::Restart {
                record.next_restart_at = None;
            }
        }
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
        self.trajectory.push(
            definition.id().clone(),
            TrajectoryEventKind::RuntimeCreated {
                runtime_id,
                installation: installation.clone(),
                plugin: definition.id().clone(),
                principal: runtime_principal.id().clone(),
                scope: scope_id,
                activation_reason: reason,
                activation_attempt,
            },
        );
        if reason == ActivationReason::Restart {
            self.trajectory.push(
                definition.id().clone(),
                TrajectoryEventKind::RuntimeRestartAttempted {
                    runtime_id,
                    installation: installation.clone(),
                    attempt: activation_attempt,
                },
            );
        }
        self.transition_runtime(runtime_id, RuntimeState::Activating)?;
        self.trajectory.push(
            definition.id().clone(),
            TrajectoryEventKind::ActivationStarted,
        );
        self.trajectory.push(
            definition.id().clone(),
            TrajectoryEventKind::RuntimeActivationStarted {
                runtime_id,
                installation: installation.clone(),
            },
        );
        let state = match self
            .state
            .runtime_handle(installation, Arc::clone(&state_lease))
        {
            Ok(state) => state,
            Err(error) => {
                state_lease.revoke();
                self.revoke_lifecycle_scope(scope_id);
                self.revoke_runtime_authority(runtime_principal.id());
                self.set_runtime_terminal(
                    runtime_id,
                    RuntimeState::Failed,
                    Some(RuntimeFailureClass::PluginError),
                );
                return Err(error.into());
            }
        };
        let cancellation = LifecycleCancellationToken::default();
        let context = ActivationContext::new(
            &definition,
            runtime_id,
            runtime_principal.id().clone(),
            installation.clone(),
            scope_id,
            reason,
            cancellation.clone(),
            effective_deadline,
            state,
            Arc::clone(&self.broker),
        );
        Ok(ActivationSetup {
            runtime_id,
            operation_id,
            plugin_id: definition.id().clone(),
            installation_id: installation.clone(),
            plugin,
            context,
            cancellation,
            deadline: effective_deadline,
            scope_id,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_activation_result(
        &mut self,
        runtime_id: RuntimeId,
        operation_id: LifecycleOperationId,
        plugin_id: PluginId,
        installation_id: InstallationId,
        result: ActivationResult,
        scope: LifecycleScope,
        publications: Vec<CapabilityPublication>,
        state_lease: Arc<RuntimeStateLease>,
        timed_out: bool,
    ) -> ActivationOutcome {
        let valid = self.runtime_is_current(runtime_id, &installation_id, operation_id)
            && self
                .runtimes
                .get(&runtime_id)
                .is_some_and(|runtime| runtime.metadata.state == RuntimeState::Activating);
        if !valid {
            let runtime = match result {
                ActivationResult::Success(runtime) => Some(runtime),
                ActivationResult::Failed(_) | ActivationResult::Crashed(_) => None,
            };
            self.cleanup_activation_resources(runtime_id, &plugin_id, runtime, scope, state_lease);
            self.record_stale_completion(runtime_id, operation_id);
            return ActivationOutcome::Failed {
                plugin_id,
                runtime_id: Some(runtime_id),
                installation: installation_id,
            };
        }
        if timed_out {
            let runtime = match result {
                ActivationResult::Success(runtime) => Some(runtime),
                ActivationResult::Failed(_) | ActivationResult::Crashed(_) => None,
            };
            self.finalize_activation_failure(
                runtime_id,
                plugin_id.clone(),
                installation_id.clone(),
                RuntimeState::Hung,
                RuntimeFailureClass::DeadlineExceeded,
                "activation deadline exceeded".to_owned(),
                scope,
                state_lease,
                runtime,
            );
            return ActivationOutcome::Hung {
                plugin_id,
                runtime_id: Some(runtime_id),
                installation: installation_id,
            };
        }
        match result {
            ActivationResult::Success(runtime) => {
                let missing = self.plugins.get(&installation_id).and_then(|record| {
                    record
                        .definition
                        .provided_capabilities()
                        .iter()
                        .find(|capability| {
                            !publications
                                .iter()
                                .any(|publication| publication.id == **capability)
                        })
                        .cloned()
                });
                if let Some(missing) = missing {
                    let error = PluginError::new(format!(
                        "plugin '{}' declared capability '{}' but did not publish it",
                        plugin_id, missing
                    ));
                    self.finalize_activation_failure(
                        runtime_id,
                        plugin_id.clone(),
                        installation_id.clone(),
                        RuntimeState::Failed,
                        RuntimeFailureClass::PluginError,
                        error.to_string(),
                        scope,
                        state_lease,
                        Some(runtime),
                    );
                    return ActivationOutcome::Failed {
                        plugin_id,
                        runtime_id: Some(runtime_id),
                        installation: installation_id,
                    };
                }
                let principal = self
                    .runtimes
                    .get(&runtime_id)
                    .expect("current runtime metadata must exist")
                    .metadata
                    .principal
                    .clone();
                for publication in &publications {
                    self.registry.publish(
                        plugin_id.clone(),
                        installation_id.clone(),
                        runtime_id,
                        principal.clone(),
                        publication.id.clone(),
                        Arc::clone(&publication.service),
                    );
                }
                if let Some(runtime_record) = self.runtimes.get_mut(&runtime_id) {
                    runtime_record.instance = Some(RuntimeInstance {
                        runtime,
                        scope,
                        publications,
                        runtime_principal: principal,
                        state_lease: Arc::clone(&state_lease),
                    });
                    runtime_record.state_lease = Some(state_lease);
                    runtime_record.operation_id = None;
                    runtime_record.metadata.state = RuntimeState::Active;
                }
                if let Some(record) = self.plugins.get_mut(&installation_id) {
                    record.state = RuntimeState::Active;
                    record.last_missing = Some(Vec::new());
                    record.failure_count = 0;
                    record.next_restart_at = None;
                }
                self.trajectory
                    .push(plugin_id.clone(), TrajectoryEventKind::Activated);
                self.trajectory.push(
                    plugin_id.clone(),
                    TrajectoryEventKind::RuntimeActivated {
                        runtime_id,
                        installation: installation_id.clone(),
                    },
                );
                ActivationOutcome::Activated {
                    plugin_id,
                    runtime_id: Some(runtime_id),
                    installation: installation_id,
                }
            }
            ActivationResult::Failed(error) => {
                self.finalize_activation_failure(
                    runtime_id,
                    plugin_id.clone(),
                    installation_id.clone(),
                    RuntimeState::Failed,
                    RuntimeFailureClass::PluginError,
                    error.to_string(),
                    scope,
                    state_lease,
                    None,
                );
                ActivationOutcome::Failed {
                    plugin_id,
                    runtime_id: Some(runtime_id),
                    installation: installation_id,
                }
            }
            ActivationResult::Crashed(message) => {
                self.finalize_activation_failure(
                    runtime_id,
                    plugin_id.clone(),
                    installation_id.clone(),
                    RuntimeState::Crashed,
                    RuntimeFailureClass::Panic,
                    message,
                    scope,
                    state_lease,
                    None,
                );
                ActivationOutcome::Crashed {
                    plugin_id,
                    runtime_id: Some(runtime_id),
                    installation: installation_id,
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finalize_activation_failure(
        &mut self,
        runtime_id: RuntimeId,
        plugin_id: PluginId,
        installation_id: InstallationId,
        state: RuntimeState,
        classification: RuntimeFailureClass,
        message: String,
        scope: LifecycleScope,
        state_lease: Arc<RuntimeStateLease>,
        runtime: Option<Box<dyn PluginRuntime>>,
    ) {
        state_lease.revoke();
        if let Some(runtime) = runtime {
            self.drop_runtime(&plugin_id, runtime, LifecyclePhase::Activation);
        }
        self.cleanup_scope(&plugin_id, scope);
        if let Some(principal) = self
            .runtimes
            .get(&runtime_id)
            .map(|record| record.metadata.principal.clone())
        {
            self.revoke_runtime_authority(&principal);
        }
        self.set_runtime_terminal(runtime_id, state, Some(classification));
        if matches!(classification, RuntimeFailureClass::Panic) {
            self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::PluginCrashed {
                    phase: LifecyclePhase::Activation,
                    message: message.clone(),
                },
            );
            self.trajectory.push(
                plugin_id,
                TrajectoryEventKind::RuntimeCrashed {
                    runtime_id,
                    installation: installation_id.clone(),
                    message,
                },
            );
        } else if classification == RuntimeFailureClass::Cancelled {
            self.trajectory.push(
                plugin_id,
                TrajectoryEventKind::RuntimeCancelled {
                    runtime_id,
                    installation: installation_id.clone(),
                },
            );
        } else if matches!(
            classification,
            RuntimeFailureClass::Hung | RuntimeFailureClass::DeadlineExceeded
        ) {
            self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::PluginFailure {
                    phase: LifecyclePhase::Activation,
                    message: message.clone(),
                },
            );
            self.trajectory.push(
                plugin_id,
                TrajectoryEventKind::RuntimeHung {
                    runtime_id,
                    installation: installation_id.clone(),
                },
            );
        } else {
            self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::PluginFailure {
                    phase: LifecyclePhase::Activation,
                    message: message.clone(),
                },
            );
            self.trajectory.push(
                plugin_id,
                TrajectoryEventKind::RuntimeFailed {
                    runtime_id,
                    installation: installation_id.clone(),
                    classification,
                    message,
                },
            );
        }
        self.note_failure(&installation_id, runtime_id);
    }

    fn cleanup_activation_resources(
        &mut self,
        runtime_id: RuntimeId,
        plugin_id: &PluginId,
        runtime: Option<Box<dyn PluginRuntime>>,
        scope: LifecycleScope,
        state_lease: Arc<RuntimeStateLease>,
    ) {
        state_lease.revoke();
        if let Some(runtime) = runtime {
            self.drop_runtime(plugin_id, runtime, LifecyclePhase::Activation);
        }
        self.cleanup_scope(plugin_id, scope);
        if let Some(principal) = self
            .runtimes
            .get(&runtime_id)
            .map(|record| record.metadata.principal.clone())
        {
            self.revoke_runtime_authority(&principal);
        }
    }

    fn drop_runtime(
        &mut self,
        plugin_id: &PluginId,
        runtime: Box<dyn PluginRuntime>,
        phase: LifecyclePhase,
    ) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(runtime))) {
            self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::PluginCrashed {
                    phase: LifecyclePhase::RuntimeDrop,
                    message: format!(
                        "{phase:?} runtime drop panicked: {}",
                        panic_message(payload.as_ref())
                    ),
                },
            );
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

    fn deactivate_runtime_sync(&mut self, runtime_id: RuntimeId, inactive_state: RuntimeState) {
        let Some((plugin_id, installation_id, policy, instance)) =
            self.runtimes.get_mut(&runtime_id).and_then(|runtime| {
                (runtime.metadata.state == RuntimeState::Active).then(|| {
                    (
                        runtime.metadata.plugin_id.clone(),
                        runtime.metadata.installation_id.clone(),
                        runtime.metadata.policy,
                        runtime
                            .instance
                            .take()
                            .expect("active runtime must own instance"),
                    )
                })
            })
        else {
            return;
        };
        for publication in &instance.publications {
            self.registry.unpublish(&runtime_id, &publication.id);
        }
        instance.state_lease.revoke();
        let _ = self.transition_runtime(runtime_id, RuntimeState::Deactivating);
        if let Some(record) = self.plugins.get_mut(&installation_id) {
            record.state = RuntimeState::Deactivating;
        }
        self.trajectory
            .push(plugin_id.clone(), TrajectoryEventKind::DeactivationStarted);
        self.trajectory.push(
            plugin_id.clone(),
            TrajectoryEventKind::RuntimeDeactivationStarted {
                runtime_id,
                installation: installation_id.clone(),
            },
        );
        let lifecycle_context = LifecycleContext::new(
            runtime_id,
            LifecycleCancellationToken::default(),
            policy.deactivation_deadline(),
        );
        let started = Instant::now();
        let RuntimeInstance {
            mut runtime,
            scope,
            publications: _,
            runtime_principal,
            state_lease: _,
        } = instance;
        let deactivation = catch_unwind(AssertUnwindSafe(|| {
            runtime.deactivate_with_context(&lifecycle_context)
        }));
        let timed_out = policy
            .deactivation_deadline()
            .is_some_and(|deadline| started.elapsed() > deadline);
        let result = match deactivation {
            Ok(Ok(())) if !timed_out => DeactivationResult::Success,
            Ok(Ok(())) => {
                DeactivationResult::Failed(PluginError::new("deactivation deadline exceeded"))
            }
            Ok(Err(error)) => DeactivationResult::Failed(error),
            Err(payload) => DeactivationResult::Crashed(panic_message(payload.as_ref())),
        };
        match &result {
            DeactivationResult::Success => {}
            DeactivationResult::Failed(error) => self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::PluginFailure {
                    phase: LifecyclePhase::Deactivation,
                    message: error.to_string(),
                },
            ),
            DeactivationResult::Crashed(message) => self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::PluginCrashed {
                    phase: LifecyclePhase::Deactivation,
                    message: message.clone(),
                },
            ),
        }
        self.drop_runtime(&plugin_id, runtime, LifecyclePhase::Deactivation);
        self.cleanup_scope(&plugin_id, scope);
        self.revoke_runtime_authority(&runtime_principal);
        let terminal = match result {
            DeactivationResult::Success => RuntimeState::Stopped,
            DeactivationResult::Failed(_) if timed_out => RuntimeState::Hung,
            DeactivationResult::Failed(_) => RuntimeState::Failed,
            DeactivationResult::Crashed(_) => RuntimeState::Crashed,
        };
        self.set_runtime_terminal(
            runtime_id,
            terminal,
            match terminal {
                RuntimeState::Stopped => None,
                RuntimeState::Hung => Some(RuntimeFailureClass::DeadlineExceeded),
                RuntimeState::Failed => Some(RuntimeFailureClass::PluginError),
                RuntimeState::Crashed => Some(RuntimeFailureClass::Panic),
                _ => None,
            },
        );
        if let Some(record) = self.plugins.get_mut(&installation_id) {
            record.state = if terminal == RuntimeState::Stopped {
                inactive_state
            } else {
                terminal
            };
        }
        if terminal == RuntimeState::Stopped {
            self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::RuntimeStopped {
                    runtime_id,
                    installation: installation_id.clone(),
                },
            );
        } else if terminal == RuntimeState::Hung {
            self.trajectory.push(
                plugin_id.clone(),
                TrajectoryEventKind::RuntimeHung {
                    runtime_id,
                    installation: installation_id.clone(),
                },
            );
        }
        self.trajectory
            .push(plugin_id, TrajectoryEventKind::Deactivated);
    }

    fn deactivation_order(&self, root: RuntimeId) -> Vec<RuntimeId> {
        if !self
            .runtimes
            .get(&root)
            .is_some_and(|record| record.metadata.state == RuntimeState::Active)
        {
            return Vec::new();
        }
        let mut visited = BTreeSet::new();
        let mut order = Vec::new();
        visited.insert(root);
        self.collect_dependents(root, &mut visited, &mut order);
        order.push(root);
        order
    }

    fn collect_dependents(
        &self,
        provider: RuntimeId,
        visited: &mut BTreeSet<RuntimeId>,
        order: &mut Vec<RuntimeId>,
    ) {
        for dependent in self.direct_dependents(provider) {
            if visited.insert(dependent) {
                self.collect_dependents(dependent, visited, order);
                order.push(dependent);
            }
        }
    }

    fn direct_dependents(&self, provider: RuntimeId) -> Vec<RuntimeId> {
        let mut dependents = Vec::new();
        for runtime in self.runtimes.values() {
            if runtime.metadata.state != RuntimeState::Active
                || runtime.metadata.runtime_id == provider
            {
                continue;
            }
            let Some(record) = self.plugins.get(&runtime.metadata.installation_id) else {
                continue;
            };
            let depends_on_provider =
                record
                    .definition
                    .dependencies()
                    .iter()
                    .any(|(capability, kind)| {
                        *kind == crate::DependencyKind::Required
                            && self.registry.provider_runtime_for(capability) == Some(provider)
                            && self
                                .registry
                                .provider_runtime_for_except(
                                    capability,
                                    &BTreeSet::from([provider]),
                                )
                                .is_none()
                    });
            if depends_on_provider {
                dependents.push(runtime.metadata.runtime_id);
            }
        }
        dependents.sort();
        dependents
    }

    fn log_provider_losses(&mut self, removal_order: &[RuntimeId]) {
        let removal_set: BTreeSet<RuntimeId> = removal_order.iter().copied().collect();
        for runtime_id in removal_order {
            let Some(runtime) = self.runtimes.get(runtime_id) else {
                continue;
            };
            let Some(record) = self.plugins.get(&runtime.metadata.installation_id) else {
                continue;
            };
            for (capability, kind) in record.definition.dependencies() {
                if *kind != crate::DependencyKind::Required {
                    continue;
                }
                let Some(provider_runtime) = self.registry.provider_runtime_for(capability) else {
                    continue;
                };
                if removal_set.contains(&provider_runtime)
                    && self
                        .registry
                        .provider_runtime_for_except(capability, &removal_set)
                        .is_none()
                {
                    let provider = self
                        .runtimes
                        .get(&provider_runtime)
                        .map(|provider| provider.metadata.plugin_id.clone())
                        .unwrap_or_else(|| PluginId::new("unknown-provider"));
                    self.trajectory.push(
                        record.definition.id().clone(),
                        TrajectoryEventKind::ProviderLost {
                            capability: capability.clone(),
                            provider: provider.clone(),
                        },
                    );
                    self.trajectory.push(
                        record.definition.id().clone(),
                        TrajectoryEventKind::ProviderRuntimeLost {
                            capability: capability.clone(),
                            provider,
                            runtime_id: provider_runtime,
                        },
                    );
                }
            }
        }
    }

    fn deactivate_unavailable_consumers(&mut self) {
        let consumers: Vec<RuntimeId> = self
            .runtimes
            .values()
            .filter(|runtime| runtime.metadata.state == RuntimeState::Active)
            .filter_map(|runtime| {
                let record = self.plugins.get(&runtime.metadata.installation_id)?;
                let missing = record
                    .definition
                    .dependencies()
                    .iter()
                    .any(|(capability, kind)| {
                        *kind == crate::DependencyKind::Required
                            && !self.registry.has_provider(capability)
                    });
                missing.then_some(runtime.metadata.runtime_id)
            })
            .collect();
        for runtime_id in consumers {
            self.deactivate_runtime_sync(runtime_id, RuntimeState::Pending);
        }
    }

    fn revoke_runtime_if_needed(&self, runtime_id: RuntimeId) {
        if let Some(principal) = self
            .runtimes
            .get(&runtime_id)
            .map(|runtime| runtime.metadata.principal.clone())
        {
            self.revoke_runtime_authority(&principal);
        }
    }

    fn revoke_runtime_authority(&self, principal: &PrincipalId) {
        let (direct_grants, descendant_grants) = self.security.revoke_subject(principal);
        self.log_grant_revocations(direct_grants, descendant_grants);
    }

    fn log_grant_revocations(&self, direct: Vec<GrantId>, descendants: Vec<GrantId>) {
        for grant in direct {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantRevoked { grant });
        }
        for grant in descendants {
            self.trajectory
                .push_security(TrajectoryEventKind::GrantAutoRevoked { grant });
        }
    }

    fn installation_for_plugin(&self, plugin: &PluginId) -> Option<InstallationId> {
        self.default_installations
            .get(plugin)
            .filter(|installation| self.plugins.contains_key(*installation))
            .cloned()
            .or_else(|| {
                self.plugins
                    .values()
                    .find(|record| record.definition.id() == plugin)
                    .map(|record| record.installation_id.clone())
            })
    }

    fn transition_runtime(
        &mut self,
        runtime_id: RuntimeId,
        next: RuntimeState,
    ) -> Result<(), KernelError> {
        let runtime = self
            .runtimes
            .get_mut(&runtime_id)
            .ok_or(KernelError::UnknownRuntime {
                runtime: runtime_id,
            })?;
        let from = runtime.metadata.state;
        if !from.can_transition_to(next) {
            return Err(KernelError::InvalidRuntimeTransition {
                runtime: runtime_id,
                from,
                to: next,
            });
        }
        runtime.metadata.state = next;
        Ok(())
    }

    fn set_runtime_terminal(
        &mut self,
        runtime_id: RuntimeId,
        state: RuntimeState,
        failure: Option<RuntimeFailureClass>,
    ) {
        let installation = if let Some(runtime) = self.runtimes.get_mut(&runtime_id) {
            runtime.metadata.state = state;
            runtime.metadata.last_failure = failure;
            runtime.operation_id = None;
            Some(runtime.metadata.installation_id.clone())
        } else {
            None
        };
        if let Some(installation) = installation
            && let Some(record) = self.plugins.get_mut(&installation)
            && record.current_runtime == Some(runtime_id)
            && !matches!(record.state, RuntimeState::Pending | RuntimeState::Stopped)
        {
            record.state = state;
        }
    }

    fn note_failure(&mut self, installation: &InstallationId, runtime_id: RuntimeId) {
        let (quarantine, schedule, attempt, backoff, plugin_id) = {
            let Some(record) = self.plugins.get_mut(installation) else {
                return;
            };
            record.failure_count = record.failure_count.saturating_add(1);
            let policy = record.policy.restart_policy();
            let quarantine = policy
                .quarantine_after()
                .is_some_and(|threshold| record.failure_count >= threshold);
            let schedule = !quarantine
                && policy.mode() == RestartMode::OnFailure
                && record.activation_attempt < policy.max_attempts();
            let backoff = if schedule {
                backoff_for(policy, record.failure_count)
            } else {
                Duration::ZERO
            };
            if schedule {
                record.next_restart_at = Some(Instant::now() + backoff);
            }
            (
                quarantine,
                schedule,
                record.activation_attempt,
                backoff,
                record.definition.id().clone(),
            )
        };
        if quarantine {
            if let Some(runtime) = self.runtimes.get_mut(&runtime_id) {
                runtime.metadata.state = RuntimeState::Quarantined;
            }
            if let Some(record) = self.plugins.get_mut(installation) {
                record.state = RuntimeState::Quarantined;
            }
            self.trajectory.push(
                plugin_id,
                TrajectoryEventKind::RuntimeQuarantined {
                    runtime_id,
                    installation: installation.clone(),
                },
            );
        } else if schedule {
            self.trajectory
                .push_security(TrajectoryEventKind::RuntimeRestartScheduled {
                    installation: installation.clone(),
                    attempt: attempt.saturating_add(1),
                });
            if backoff.is_zero()
                && let Some(record) = self.plugins.get_mut(installation)
            {
                record.state = RuntimeState::Registered;
                record.next_activation_reason = ActivationReason::Restart;
            }
        }
    }

    fn prepare_restarts(&mut self) {
        let now = Instant::now();
        let installations: Vec<InstallationId> = self.plugins.keys().cloned().collect();
        for installation in installations {
            let Some(record) = self.plugins.get(&installation) else {
                continue;
            };
            if !matches!(
                record.state,
                RuntimeState::Failed | RuntimeState::Crashed | RuntimeState::Hung
            ) || record.policy.restart_policy().mode() != RestartMode::OnFailure
                || record.activation_attempt >= record.policy.restart_policy().max_attempts()
                || record
                    .next_restart_at
                    .is_some_and(|deadline| deadline > now)
            {
                continue;
            }
            if let Some(record) = self.plugins.get_mut(&installation) {
                record.state = RuntimeState::Registered;
                record.next_restart_at = None;
                record.next_activation_reason = ActivationReason::Restart;
            }
        }
    }

    fn poll_lifecycle_internal(&mut self) {
        let now = Instant::now();
        let activation_deadlines: Vec<RuntimeId> = self
            .pending_activations
            .values()
            .filter(|pending| {
                pending
                    .deadline_at
                    .is_some_and(|deadline| now >= deadline && !pending.cancellation.is_cancelled())
            })
            .map(|pending| pending.runtime_id)
            .collect();
        for runtime_id in activation_deadlines {
            self.mark_runtime_hung_internal(runtime_id);
        }
        let deactivation_deadlines: Vec<RuntimeId> = self
            .pending_deactivations
            .values()
            .filter(|pending| {
                pending
                    .deadline_at
                    .is_some_and(|deadline| now >= deadline && !pending.cancellation.is_cancelled())
            })
            .map(|pending| pending.runtime_id)
            .collect();
        for runtime_id in deactivation_deadlines {
            self.mark_runtime_hung_internal(runtime_id);
        }
        let cancelled: Vec<LifecycleOperationId> = self
            .pending_activations
            .values()
            .filter(|pending| pending.cancellation.is_cancelled())
            .map(|pending| pending.operation_id)
            .chain(
                self.pending_deactivations
                    .values()
                    .filter(|pending| pending.cancellation.is_cancelled())
                    .map(|pending| pending.operation_id),
            )
            .collect();
        for operation in cancelled {
            self.apply_cancellation(operation);
        }

        let mut activation_completions = Vec::new();
        let activation_ids: Vec<RuntimeId> = self.pending_activations.keys().copied().collect();
        let mut disconnected_activations = Vec::new();
        for runtime_id in activation_ids {
            let Some(pending) = self.pending_activations.get(&runtime_id) else {
                continue;
            };
            match pending.receiver.try_recv() {
                Ok(completion) => activation_completions.push((runtime_id, completion)),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    disconnected_activations.push((runtime_id, pending.operation_id));
                }
            }
        }
        for (runtime_id, operation) in disconnected_activations {
            self.record_stale_completion(runtime_id, operation);
            self.pending_activations.remove(&runtime_id);
        }
        for (runtime_id, completion) in activation_completions {
            if let Some(pending) = self.pending_activations.remove(&runtime_id) {
                self.process_activation_completion(pending, completion);
            }
        }

        let mut deactivation_completions = Vec::new();
        let deactivation_ids: Vec<RuntimeId> = self.pending_deactivations.keys().copied().collect();
        let mut disconnected_deactivations = Vec::new();
        for runtime_id in deactivation_ids {
            let Some(pending) = self.pending_deactivations.get(&runtime_id) else {
                continue;
            };
            match pending.receiver.try_recv() {
                Ok(completion) => deactivation_completions.push((runtime_id, completion)),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    disconnected_deactivations.push((runtime_id, pending.operation_id));
                }
            }
        }
        for (runtime_id, operation) in disconnected_deactivations {
            self.record_stale_completion(runtime_id, operation);
            self.pending_deactivations.remove(&runtime_id);
        }
        for (runtime_id, completion) in deactivation_completions {
            if let Some(pending) = self.pending_deactivations.remove(&runtime_id) {
                self.process_deactivation_completion(pending, completion);
            }
        }
    }

    fn process_activation_completion(
        &mut self,
        pending: PendingActivation,
        completion: ActivationCompletion,
    ) {
        let valid = self.runtime_is_current(
            completion.runtime_id,
            &pending.installation_id,
            completion.operation_id,
        ) && self
            .runtimes
            .get(&completion.runtime_id)
            .is_some_and(|runtime| runtime.metadata.state == RuntimeState::Activating);
        if valid {
            let _ = self.finish_activation_result(
                completion.runtime_id,
                completion.operation_id,
                pending.plugin_id,
                pending.installation_id,
                completion.result,
                completion.scope,
                completion.publications,
                completion.state_lease,
                false,
            );
        } else {
            let runtime = match completion.result {
                ActivationResult::Success(runtime) => Some(runtime),
                ActivationResult::Failed(_) | ActivationResult::Crashed(_) => None,
            };
            self.cleanup_activation_resources(
                completion.runtime_id,
                &pending.plugin_id,
                runtime,
                completion.scope,
                completion.state_lease,
            );
            self.record_stale_completion(completion.runtime_id, completion.operation_id);
        }
    }

    fn process_deactivation_completion(
        &mut self,
        pending: PendingDeactivation,
        completion: DeactivationCompletion,
    ) {
        let valid = self.runtime_is_current(
            completion.runtime_id,
            &pending.installation_id,
            completion.operation_id,
        );
        if !valid {
            self.drop_runtime(
                &pending.plugin_id,
                completion.runtime,
                LifecyclePhase::Deactivation,
            );
            self.cleanup_scope(&pending.plugin_id, completion.scope);
            self.revoke_runtime_authority(&pending.principal);
            self.record_stale_completion(completion.runtime_id, completion.operation_id);
            return;
        }
        let result = completion.result;
        let scope = completion.scope;
        match &result {
            DeactivationResult::Success => {}
            DeactivationResult::Failed(error) => self.trajectory.push(
                pending.plugin_id.clone(),
                TrajectoryEventKind::PluginFailure {
                    phase: LifecyclePhase::Deactivation,
                    message: error.to_string(),
                },
            ),
            DeactivationResult::Crashed(message) => self.trajectory.push(
                pending.plugin_id.clone(),
                TrajectoryEventKind::PluginCrashed {
                    phase: LifecyclePhase::Deactivation,
                    message: message.clone(),
                },
            ),
        }
        self.drop_runtime(
            &pending.plugin_id,
            completion.runtime,
            LifecyclePhase::Deactivation,
        );
        self.cleanup_scope(&pending.plugin_id, scope);
        self.revoke_runtime_authority(&pending.principal);
        let current_state = self
            .runtimes
            .get(&pending.runtime_id)
            .map(|record| record.metadata.state)
            .unwrap_or(RuntimeState::Stopped);
        let terminal = if current_state == RuntimeState::Hung {
            RuntimeState::Hung
        } else {
            match result {
                DeactivationResult::Success => RuntimeState::Stopped,
                DeactivationResult::Failed(_) => RuntimeState::Failed,
                DeactivationResult::Crashed(_) => RuntimeState::Crashed,
            }
        };
        self.set_runtime_terminal(
            pending.runtime_id,
            terminal,
            match terminal {
                RuntimeState::Stopped => None,
                RuntimeState::Hung => Some(RuntimeFailureClass::Hung),
                RuntimeState::Failed => Some(RuntimeFailureClass::PluginError),
                RuntimeState::Crashed => Some(RuntimeFailureClass::Panic),
                _ => None,
            },
        );
        if let Some(record) = self.plugins.get_mut(&pending.installation_id) {
            record.state = terminal;
        }
        self.trajectory.push(
            pending.plugin_id.clone(),
            if terminal == RuntimeState::Stopped {
                TrajectoryEventKind::RuntimeStopped {
                    runtime_id: pending.runtime_id,
                    installation: pending.installation_id.clone(),
                }
            } else if terminal == RuntimeState::Hung {
                TrajectoryEventKind::RuntimeHung {
                    runtime_id: pending.runtime_id,
                    installation: pending.installation_id.clone(),
                }
            } else if terminal == RuntimeState::Crashed {
                TrajectoryEventKind::RuntimeCrashed {
                    runtime_id: pending.runtime_id,
                    installation: pending.installation_id.clone(),
                    message: "deactivation crashed".to_owned(),
                }
            } else {
                TrajectoryEventKind::RuntimeFailed {
                    runtime_id: pending.runtime_id,
                    installation: pending.installation_id.clone(),
                    classification: RuntimeFailureClass::PluginError,
                    message: "deactivation failed".to_owned(),
                }
            },
        );
        self.trajectory
            .push(pending.plugin_id, TrajectoryEventKind::Deactivated);
    }

    fn runtime_is_current(
        &self,
        runtime_id: RuntimeId,
        installation: &InstallationId,
        operation_id: LifecycleOperationId,
    ) -> bool {
        self.runtimes.get(&runtime_id).is_some_and(|runtime| {
            runtime.metadata.installation_id == *installation
                && runtime.operation_id == Some(operation_id)
                && self
                    .plugins
                    .get(installation)
                    .is_some_and(|record| record.current_runtime == Some(runtime_id))
        })
    }

    fn next_operation_id(&mut self) -> LifecycleOperationId {
        self.next_operation_sequence = self.next_operation_sequence.saturating_add(1);
        LifecycleOperationId::new(self.next_operation_sequence)
    }

    fn apply_cancellation(&mut self, operation_id: LifecycleOperationId) {
        if let Some((runtime_id, plugin_id, installation)) = self
            .pending_activations
            .values()
            .find(|pending| pending.operation_id == operation_id)
            .map(|pending| {
                (
                    pending.runtime_id,
                    pending.plugin_id.clone(),
                    pending.installation_id.clone(),
                )
            })
        {
            if let Some(pending) = self.pending_activations.get(&runtime_id) {
                pending.cancellation.cancel();
                pending.state_lease.revoke();
            }
            if self
                .runtimes
                .get(&runtime_id)
                .is_some_and(|runtime| runtime.metadata.state == RuntimeState::Activating)
            {
                if let Some(runtime) = self.runtimes.get_mut(&runtime_id) {
                    runtime.metadata.state = RuntimeState::Cancelled;
                    runtime.metadata.last_failure = Some(RuntimeFailureClass::Cancelled);
                    runtime.operation_id = None;
                }
                if let Some(record) = self.plugins.get_mut(&installation) {
                    record.state = RuntimeState::Cancelled;
                }
                if let Some(scope) = self.runtimes.get(&runtime_id).map(|r| r.metadata.scope_id) {
                    self.revoke_lifecycle_scope(scope);
                }
                if let Some(principal) = self
                    .runtimes
                    .get(&runtime_id)
                    .map(|runtime| runtime.metadata.principal.clone())
                {
                    self.revoke_runtime_authority(&principal);
                }
                self.trajectory.push(
                    plugin_id,
                    TrajectoryEventKind::RuntimeCancelled {
                        runtime_id,
                        installation,
                    },
                );
            }
            return;
        }
        if let Some((runtime_id, plugin_id, installation, scope_id, principal)) = self
            .pending_deactivations
            .values()
            .find(|pending| pending.operation_id == operation_id)
            .map(|pending| {
                (
                    pending.runtime_id,
                    pending.plugin_id.clone(),
                    pending.installation_id.clone(),
                    pending.scope_id,
                    pending.principal.clone(),
                )
            })
        {
            if let Some(pending) = self.pending_deactivations.get(&runtime_id) {
                pending.cancellation.cancel();
                pending.state_lease.revoke();
            }
            self.revoke_lifecycle_scope(scope_id);
            self.revoke_runtime_authority(&principal);
            self.trajectory.push(
                plugin_id,
                TrajectoryEventKind::RuntimeCancelled {
                    runtime_id,
                    installation,
                },
            );
        }
    }

    fn cancel_pending_for_installation(&mut self, installation: &InstallationId) {
        let operations: Vec<LifecycleOperationId> = self
            .pending_activations
            .values()
            .filter(|pending| &pending.installation_id == installation)
            .map(|pending| pending.operation_id)
            .chain(
                self.pending_deactivations
                    .values()
                    .filter(|pending| &pending.installation_id == installation)
                    .map(|pending| pending.operation_id),
            )
            .collect();
        for operation in operations {
            self.apply_cancellation(operation);
        }
    }

    fn mark_runtime_hung_internal(&mut self, runtime_id: RuntimeId) {
        let Some(runtime) = self.runtimes.get(&runtime_id) else {
            return;
        };
        if runtime.metadata.state.is_terminal() {
            return;
        }
        let plugin_id = runtime.metadata.plugin_id.clone();
        let installation = runtime.metadata.installation_id.clone();
        let principal = runtime.metadata.principal.clone();
        let scope_id = runtime.metadata.scope_id;
        if let Some(runtime) = self.runtimes.get_mut(&runtime_id) {
            runtime.metadata.state = RuntimeState::Hung;
            runtime.metadata.last_failure = Some(RuntimeFailureClass::Hung);
            runtime.operation_id = None;
        }
        if let Some(record) = self.plugins.get_mut(&installation) {
            record.state = RuntimeState::Hung;
        }
        if let Some(pending) = self.pending_activations.get(&runtime_id) {
            pending.cancellation.cancel();
            pending.state_lease.revoke();
        }
        if let Some(pending) = self.pending_deactivations.get(&runtime_id) {
            pending.cancellation.cancel();
            pending.state_lease.revoke();
        }
        if let Some(runtime) = self.runtimes.get_mut(&runtime_id)
            && let Some(instance) = runtime.instance.take()
        {
            for publication in &instance.publications {
                self.registry.unpublish(&runtime_id, &publication.id);
            }
            instance.state_lease.revoke();
            self.drop_runtime(&plugin_id, instance.runtime, LifecyclePhase::Deactivation);
            self.cleanup_scope(&plugin_id, instance.scope);
        }
        self.revoke_lifecycle_scope(scope_id);
        self.revoke_runtime_authority(&principal);
        self.trajectory.push(
            plugin_id,
            TrajectoryEventKind::RuntimeHung {
                runtime_id,
                installation: installation.clone(),
            },
        );
        self.note_failure(&installation, runtime_id);
    }

    fn record_stale_completion(&mut self, runtime_id: RuntimeId, operation: LifecycleOperationId) {
        self.stale_completions.push(runtime_id);
        let plugin = self
            .runtimes
            .get(&runtime_id)
            .map(|runtime| runtime.metadata.plugin_id.clone())
            .unwrap_or_else(|| PluginId::new("kernel-runtime"));
        self.trajectory.push(
            plugin,
            TrajectoryEventKind::LifecycleCompletionRejected {
                runtime_id,
                operation,
                classification: RuntimeFailureClass::StaleCompletion,
            },
        );
    }

    fn extend_report_from_state(&self, report: &mut ReconcileReport) {
        for record in self.plugins.values() {
            match record.state {
                RuntimeState::Pending | RuntimeState::WaitingDependencies => {
                    report.pending.push(record.definition.id().clone());
                    report.waiting.push(record.installation_id.clone());
                }
                RuntimeState::Failed => {
                    report.failed.push(record.definition.id().clone());
                    if let Some(runtime_id) = record.current_runtime {
                        report.failed_runtime_ids.push(runtime_id);
                    }
                }
                RuntimeState::Crashed => {
                    report.crashed.push(record.definition.id().clone());
                    if let Some(runtime_id) = record.current_runtime {
                        report.crashed_runtime_ids.push(runtime_id);
                    }
                }
                RuntimeState::Hung => {
                    if let Some(runtime_id) = record.current_runtime {
                        report.hung_runtime_ids.push(runtime_id);
                    }
                }
                RuntimeState::Quarantined => {
                    report
                        .quarantined_installations
                        .push(record.installation_id.clone());
                }
                _ => {}
            }
            if matches!(
                record.state,
                RuntimeState::Failed
                    | RuntimeState::Crashed
                    | RuntimeState::Hung
                    | RuntimeState::Quarantined
            ) {
                let criticality = match record.policy.criticality() {
                    crate::RuntimeCriticality::Required => "required",
                    crate::RuntimeCriticality::Optional => "optional",
                };
                report.degraded_reasons.push(format!(
                    "{criticality} installation '{}' is {:?}",
                    record.installation_id, record.state
                ));
            }
        }
        for runtime in self.runtimes.values() {
            if runtime.metadata.state == RuntimeState::Active {
                report.active.push(runtime.metadata.runtime_id);
            }
            if runtime.metadata.state == RuntimeState::Hung {
                report.hung_runtime_ids.push(runtime.metadata.runtime_id);
                report.degraded_reasons.push(format!(
                    "runtime '{}' for installation '{}' is Hung",
                    runtime.metadata.runtime_id, runtime.metadata.installation_id
                ));
            }
        }
        dedup_vec(&mut report.activated);
        dedup_vec(&mut report.pending);
        dedup_vec(&mut report.failed);
        dedup_vec(&mut report.crashed);
        dedup_vec(&mut report.activated_installations);
        dedup_vec(&mut report.active);
        dedup_vec(&mut report.waiting);
        dedup_vec(&mut report.failed_runtime_ids);
        dedup_vec(&mut report.crashed_runtime_ids);
        dedup_vec(&mut report.hung_runtime_ids);
        dedup_vec(&mut report.quarantined_installations);
        dedup_vec(&mut report.degraded_reasons);
    }
}

enum ActivationOutcome {
    Activated {
        plugin_id: PluginId,
        installation: InstallationId,
        runtime_id: Option<RuntimeId>,
    },
    Failed {
        plugin_id: PluginId,
        runtime_id: Option<RuntimeId>,
        installation: InstallationId,
    },
    Crashed {
        plugin_id: PluginId,
        runtime_id: Option<RuntimeId>,
        installation: InstallationId,
    },
    Hung {
        plugin_id: PluginId,
        runtime_id: Option<RuntimeId>,
        installation: InstallationId,
    },
}

fn min_duration(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn backoff_for(policy: crate::RestartPolicy, failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(31);
    let multiplier = 1u32 << exponent;
    let value = policy
        .backoff_base()
        .checked_mul(multiplier)
        .unwrap_or(policy.backoff_max());
    value.min(policy.backoff_max())
}

fn dedup_vec<T: Ord>(values: &mut Vec<T>) {
    values.sort();
    values.dedup();
}

fn installation_principal(record: &InstallationRecord) -> Principal {
    Principal::new(
        PrincipalId::plugin_installation(record.installation_id().as_str()),
        PrincipalKind::PluginInstallation,
    )
}
