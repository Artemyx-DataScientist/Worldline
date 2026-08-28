use crate::{EffectCleanupError, LifecycleScopeId};

pub struct OwnedEffect {
    label: String,
    disposer: Option<Box<dyn FnOnce() -> Result<(), EffectCleanupError> + Send + 'static>>,
}

impl OwnedEffect {
    pub fn new(
        label: impl Into<String>,
        disposer: impl FnOnce() -> Result<(), EffectCleanupError> + Send + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            disposer: Some(Box::new(disposer)),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn cleanup(mut self) -> Result<(), EffectCleanupError> {
        self.disposer
            .take()
            .expect("owned effect disposer was consumed exactly once")()
    }
}

pub(crate) struct LifecycleScope {
    id: LifecycleScopeId,
    effects: Vec<OwnedEffect>,
}

impl LifecycleScope {
    pub(crate) fn new(id: LifecycleScopeId, effects: Vec<OwnedEffect>) -> Self {
        Self { id, effects }
    }

    pub(crate) fn id(&self) -> LifecycleScopeId {
        self.id
    }

    pub(crate) fn into_effects(self) -> Vec<OwnedEffect> {
        self.effects
    }
}
