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
    plugin::{ActivationContext, Plugin, PluginDefinition, PluginId, PluginRuntime},
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
}

struct PluginRecord {
    plugin: Arc<dyn Plugin>,
    definition: PluginDefinition,
    state: RuntimeState,
    runtime: Option<RuntimeInstance>,
    last_missing: Option<Vec<crate::CapabilityId>>,
}

pub struct Kernel {
    registry: Arc<CapabilityRegistry>,
    plugins: std::collections::BTreeMap<PluginId, PluginRecord>,
    trajectory: Trajectory,
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(CapabilityRegistry::default()),
            plugins: std::collections::BTreeMap::new(),
            trajectory: Trajectory::default(),
        }
    }

    pub fn register<P>(&mut self, plugin: P) -> Result<PluginId, KernelError>
    where
        P: Plugin + 'static,
    {
        self.register_arc(Arc::new(plugin))
    }

    pub fn register_arc(&mut self, plugin: Arc<dyn Plugin>) -> Result<PluginId, KernelError> {
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
        if self.plugins.contains_key(definition.id()) {
            return Err(KernelError::DuplicatePlugin {
                id: definition.id().clone(),
            });
        }

        let id = definition.id().clone();
        self.plugins.insert(
            id.clone(),
            PluginRecord {
                plugin,
                definition,
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

    pub fn trajectory(&self) -> &[TrajectoryEvent] {
        self.trajectory.events()
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
        let (plugin, definition) = {
            let record = self
                .plugins
                .get(id)
                .expect("eligible plugin must remain registered");
            (Arc::clone(&record.plugin), record.definition.clone())
        };
        self.trajectory
            .push(id.clone(), TrajectoryEventKind::ActivationStarted);
        let mut context = ActivationContext::new(&definition, Arc::clone(&self.registry));
        let activation = catch_unwind(AssertUnwindSafe(|| plugin.activate(&mut context)));
        let (scope, publications) = context.into_parts();

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
                    self.record_activation_failure(
                        id,
                        LifecyclePhase::Activation,
                        error,
                        scope,
                        runtime,
                    );
                    ActivationOutcome::Failed
                } else {
                    for publication in &publications {
                        self.registry.publish(
                            id.clone(),
                            publication.id.clone(),
                            Arc::clone(&publication.service),
                        );
                    }
                    if let Some(record) = self.plugins.get_mut(id) {
                        record.runtime = Some(RuntimeInstance {
                            runtime,
                            scope,
                            publications,
                        });
                        record.state = RuntimeState::Active;
                    }
                    self.trajectory
                        .push(id.clone(), TrajectoryEventKind::Activated);
                    ActivationOutcome::Activated
                }
            }
            Ok(Err(error)) => {
                self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::PluginFailure {
                        phase: LifecyclePhase::Activation,
                        message: error.to_string(),
                    },
                );
                self.cleanup_scope(id, scope);
                if let Some(record) = self.plugins.get_mut(id) {
                    record.state = RuntimeState::Failed;
                }
                ActivationOutcome::Failed
            }
            Err(payload) => {
                let message = panic_message(payload.as_ref());
                self.trajectory.push(
                    id.clone(),
                    TrajectoryEventKind::PluginCrashed {
                        phase: LifecyclePhase::Activation,
                        message,
                    },
                );
                self.cleanup_scope(id, scope);
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
        if let Some(record) = self.plugins.get_mut(id) {
            record.state = RuntimeState::Failed;
        }
    }

    fn cleanup_scope(&mut self, id: &PluginId, scope: LifecycleScope) {
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
        for publication in &instance.publications {
            self.registry.unpublish(id, &publication.id);
        }
        if let Some(record) = self.plugins.get_mut(id) {
            record.state = inactive_state;
        }
        self.trajectory
            .push(id.clone(), TrajectoryEventKind::Deactivated);
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
