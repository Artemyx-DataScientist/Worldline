use crate::{CapabilityId, PluginId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecyclePhase {
    Activation,
    Deactivation,
    RuntimeDrop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrajectoryEventKind {
    Registered,
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

#[derive(Default)]
pub(crate) struct Trajectory {
    events: Vec<TrajectoryEvent>,
}

impl Trajectory {
    pub(crate) fn push(&mut self, plugin: PluginId, kind: TrajectoryEventKind) {
        let sequence = self.events.len() as u64 + 1;
        self.events.push(TrajectoryEvent {
            sequence,
            plugin,
            kind,
        });
    }

    pub(crate) fn events(&self) -> &[TrajectoryEvent] {
        &self.events
    }
}
