use std::sync::{Arc, Mutex};

use crate::{
    CapabilityContract, CapabilityId, GrantId, GrantLifetime, InvocationId, OperationId, PluginId,
    PrincipalId, PrincipalKind, ResourceId, ResourceScope, security::AuthoritySet,
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
    PrincipalRegistered {
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
