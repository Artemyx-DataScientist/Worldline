use std::sync::{Arc, Mutex};

use crate::{
    CapabilityContract, CapabilityId, GrantId, GrantLifetime, InvocationId, LifecycleScopeId,
    OperationId, PluginId, PrincipalId, PrincipalKind, ResourceId, ResourceScope,
    runtime::{ActivationReason, LifecycleOperationId, RuntimeFailureClass, RuntimeId},
    security::AuthoritySet,
    state::{InstallationId, MigrationId, StateSchemaVersion, StateTransactionId},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecyclePhase {
    Activation,
    Deactivation,
    RuntimeDrop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrajectoryEventKind {
    Registered,
    InstallationCreated {
        installation: InstallationId,
        plugin: PluginId,
        schema: StateSchemaVersion,
    },
    InstallationReady {
        installation: InstallationId,
        schema: StateSchemaVersion,
    },
    RuntimeBoundToInstallation {
        installation: InstallationId,
        runtime: PrincipalId,
    },
    RuntimeCreated {
        runtime_id: RuntimeId,
        installation: InstallationId,
        plugin: PluginId,
        principal: PrincipalId,
        scope: LifecycleScopeId,
        activation_reason: ActivationReason,
        activation_attempt: u32,
    },
    RuntimeWaitingDependencies {
        installation: InstallationId,
        plugin: PluginId,
        missing: Vec<CapabilityId>,
    },
    RuntimeActivationStarted {
        runtime_id: RuntimeId,
        installation: InstallationId,
    },
    RuntimeActivated {
        runtime_id: RuntimeId,
        installation: InstallationId,
    },
    RuntimeDeactivationStarted {
        runtime_id: RuntimeId,
        installation: InstallationId,
    },
    RuntimeStopped {
        runtime_id: RuntimeId,
        installation: InstallationId,
    },
    RuntimeFailed {
        runtime_id: RuntimeId,
        installation: InstallationId,
        classification: RuntimeFailureClass,
        message: String,
    },
    RuntimeCrashed {
        runtime_id: RuntimeId,
        installation: InstallationId,
        message: String,
    },
    RuntimeCancelled {
        runtime_id: RuntimeId,
        installation: InstallationId,
    },
    RuntimeHung {
        runtime_id: RuntimeId,
        installation: InstallationId,
    },
    RuntimeQuarantined {
        runtime_id: RuntimeId,
        installation: InstallationId,
    },
    RuntimeRestartScheduled {
        installation: InstallationId,
        attempt: u32,
    },
    RuntimeRestartAttempted {
        runtime_id: RuntimeId,
        installation: InstallationId,
        attempt: u32,
    },
    LifecycleCompletionRejected {
        runtime_id: RuntimeId,
        operation: LifecycleOperationId,
        classification: RuntimeFailureClass,
    },
    StateTransactionStarted {
        installation: InstallationId,
        transaction: StateTransactionId,
    },
    StateTransactionCommitted {
        installation: InstallationId,
        transaction: StateTransactionId,
        changed_key_count: usize,
        schema: StateSchemaVersion,
    },
    StateTransactionRolledBack {
        installation: InstallationId,
        transaction: StateTransactionId,
    },
    MigrationPlanned {
        installation: InstallationId,
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
        migrations: Vec<MigrationId>,
    },
    MigrationStarted {
        installation: InstallationId,
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
    },
    MigrationStepStarted {
        installation: InstallationId,
        migration: MigrationId,
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
    },
    MigrationStepCompleted {
        installation: InstallationId,
        migration: MigrationId,
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
    },
    MigrationCommitted {
        installation: InstallationId,
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
    },
    MigrationFailed {
        installation: InstallationId,
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
        migration: Option<MigrationId>,
    },
    InstallationUninstallStarted {
        installation: InstallationId,
    },
    InstallationUninstalled {
        installation: InstallationId,
    },
    InstallationUninstallFailed {
        installation: InstallationId,
    },
    InstallationRecoveryFailed {
        installation: InstallationId,
        operation: String,
    },
    PrincipalRegistered {
        principal: PrincipalId,
        kind: PrincipalKind,
    },
    PrincipalRetired {
        principal: PrincipalId,
        kind: PrincipalKind,
    },
    GrantCreated {
        grant: GrantId,
        issuer: PrincipalId,
        subject: PrincipalId,
        capability: CapabilityContract,
        allowed_operations: std::collections::BTreeSet<OperationId>,
        resource_scope: ResourceScope,
        delegable: bool,
        lifetime: GrantLifetime,
    },
    GrantDelegated {
        grant: GrantId,
        parent: GrantId,
        issuer: PrincipalId,
        subject: PrincipalId,
        capability: CapabilityContract,
        allowed_operations: std::collections::BTreeSet<OperationId>,
        resource_scope: ResourceScope,
        delegable: bool,
        lifetime: GrantLifetime,
    },
    GrantRevoked {
        grant: GrantId,
    },
    GrantAutoRevoked {
        grant: GrantId,
    },
    DependencyResolution {
        missing: Vec<CapabilityId>,
    },
    ActivationStarted,
    Activated,
    ProviderLost {
        capability: CapabilityId,
        provider: PluginId,
    },
    ProviderRuntimeLost {
        capability: CapabilityId,
        provider: PluginId,
        runtime_id: RuntimeId,
    },
    ProviderSelectionMade {
        requested: CapabilityId,
        compatible_candidate_count: usize,
        selected_runtime_id: Option<RuntimeId>,
        selected_installation: Option<InstallationId>,
        policy: String,
        reason: String,
    },
    CapabilityVersionNegotiated {
        requested: CapabilityId,
        selected: Option<CapabilityId>,
    },
    BootDegraded {
        reasons: Vec<String>,
    },
    DeactivationStarted,
    EffectCleanupStarted {
        effect: String,
    },
    EffectCleaned {
        effect: String,
    },
    EffectCleanupFailed {
        effect: String,
        error: String,
    },
    PluginFailure {
        phase: LifecyclePhase,
        message: String,
    },
    PluginCrashed {
        phase: LifecyclePhase,
        message: String,
    },
    Deactivated,
    Unregistered,
    InvocationAuthorized {
        invocation: InvocationId,
        caller: PrincipalId,
        capability: CapabilityContract,
        operation: OperationId,
        resource: ResourceId,
        authority: AuthoritySet,
        causal_parent: Option<InvocationId>,
    },
    InvocationDenied {
        invocation: InvocationId,
        caller: PrincipalId,
        capability: CapabilityContract,
        operation: OperationId,
        resource: ResourceId,
        reason: crate::DenialReason,
        causal_parent: Option<InvocationId>,
    },
    InvocationStarted {
        invocation: InvocationId,
        caller: PrincipalId,
        provider: PrincipalId,
        capability: CapabilityContract,
        operation: OperationId,
        resource: ResourceId,
        payload_size: usize,
        causal_parent: Option<InvocationId>,
    },
    InvocationCompleted {
        invocation: InvocationId,
        causal_parent: Option<InvocationId>,
    },
    InvocationFailed {
        invocation: InvocationId,
        causal_parent: Option<InvocationId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrajectoryEvent {
    sequence: u64,
    plugin: PluginId,
    kind: TrajectoryEventKind,
}

impl TrajectoryEvent {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub fn kind(&self) -> &TrajectoryEventKind {
        &self.kind
    }
}

#[derive(Clone, Default)]
pub(crate) struct Trajectory {
    events: Arc<Mutex<Vec<TrajectoryEvent>>>,
}

impl Trajectory {
    pub(crate) fn push(&mut self, plugin: PluginId, kind: TrajectoryEventKind) {
        self.push_shared(plugin, kind);
    }

    pub(crate) fn push_security(&self, kind: TrajectoryEventKind) {
        self.push_shared(PluginId::new("kernel-security"), kind);
    }

    fn push_shared(&self, plugin: PluginId, kind: TrajectoryEventKind) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sequence = events.len() as u64 + 1;
        events.push(TrajectoryEvent {
            sequence,
            plugin,
            kind,
        });
    }

    pub(crate) fn events(&self) -> Vec<TrajectoryEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}
