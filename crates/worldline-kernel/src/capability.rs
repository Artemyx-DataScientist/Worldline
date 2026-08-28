use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, RwLock},
};

use crate::{InstallationId, RuntimeId};
use crate::{PluginId, PrincipalId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InterfaceVersion {
    major: u16,
    minor: u16,
}

impl InterfaceVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    pub const fn major(self) -> u16 {
        self.major
    }

    pub const fn minor(self) -> u16 {
        self.minor
    }
}

impl fmt::Display for InterfaceVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityId {
    namespace: String,
    name: String,
    interface_version: InterfaceVersion,
}

impl CapabilityId {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        interface_version: InterfaceVersion,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            interface_version,
        }
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn interface_version(&self) -> InterfaceVersion {
        self.interface_version
    }

    pub fn contract(&self) -> crate::CapabilityContract {
        crate::CapabilityContract::from(self)
    }

    pub fn is_well_formed(&self) -> bool {
        !self.namespace.trim().is_empty() && !self.name.trim().is_empty()
    }

    pub fn is_compatible_with(&self, required: &Self) -> bool {
        self.namespace == required.namespace
            && self.name == required.name
            && self.interface_version.major == required.interface_version.major
            && self.interface_version.minor >= required.interface_version.minor
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}@{}",
            self.namespace, self.name, self.interface_version
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyKind {
    Required,
    Optional,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityDependency {
    capability: CapabilityId,
    kind: DependencyKind,
}

impl CapabilityDependency {
    pub fn required(capability: CapabilityId) -> Self {
        Self {
            capability,
            kind: DependencyKind::Required,
        }
    }

    pub fn optional(capability: CapabilityId) -> Self {
        Self {
            capability,
            kind: DependencyKind::Optional,
        }
    }

    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    pub const fn kind(&self) -> DependencyKind {
        self.kind
    }
}

pub trait CapabilityService: Send + Sync {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String>;

    fn invoke_with_context(
        &self,
        context: &crate::InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.invoke(context.operation().as_str(), payload)
    }
}

pub(crate) struct CapabilityPublication {
    pub(crate) id: CapabilityId,
    pub(crate) service: Arc<dyn CapabilityService>,
}

/// Describes one currently published provider without exposing its service
/// object.  Provider identity is runtime-scoped, not definition-scoped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDescriptor {
    capability: CapabilityId,
    plugin: PluginId,
    installation_id: InstallationId,
    runtime_id: RuntimeId,
    principal: PrincipalId,
}

impl ProviderDescriptor {
    pub(crate) fn new(
        capability: CapabilityId,
        plugin: PluginId,
        installation_id: InstallationId,
        runtime_id: RuntimeId,
        principal: PrincipalId,
    ) -> Self {
        Self {
            capability,
            plugin,
            installation_id,
            runtime_id,
            principal,
        }
    }

    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub fn installation_id(&self) -> &InstallationId {
        &self.installation_id
    }

    pub const fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }
}

/// Observable explanation for deterministic provider selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderSelectionDiagnostic {
    requested: CapabilityId,
    compatible_candidate_count: usize,
    selected_runtime_id: Option<RuntimeId>,
    selected_installation_id: Option<InstallationId>,
    policy: String,
    reason: String,
    negotiated_capability: Option<CapabilityId>,
}

impl ProviderSelectionDiagnostic {
    pub(crate) fn new(
        requested: CapabilityId,
        compatible_candidate_count: usize,
        selected: Option<&ProviderDescriptor>,
        policy: &str,
        reason: &str,
    ) -> Self {
        Self {
            requested,
            compatible_candidate_count,
            selected_runtime_id: selected.map(ProviderDescriptor::runtime_id),
            selected_installation_id: selected.map(|provider| provider.installation_id.clone()),
            policy: policy.to_owned(),
            reason: reason.to_owned(),
            negotiated_capability: selected.map(|provider| provider.capability.clone()),
        }
    }

    pub fn requested(&self) -> &CapabilityId {
        &self.requested
    }

    pub const fn compatible_candidate_count(&self) -> usize {
        self.compatible_candidate_count
    }

    pub const fn selected_runtime_id(&self) -> Option<RuntimeId> {
        self.selected_runtime_id
    }

    pub fn selected_installation_id(&self) -> Option<&InstallationId> {
        self.selected_installation_id.as_ref()
    }

    pub fn policy(&self) -> &str {
        &self.policy
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn negotiated_capability(&self) -> Option<&CapabilityId> {
        self.negotiated_capability.as_ref()
    }
}

/// Read-only provider metadata returned by capability discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityDiscoveryDescriptor {
    capability: CapabilityId,
    plugin: PluginId,
    installation_id: InstallationId,
    runtime_id: Option<RuntimeId>,
    lifecycle_state: crate::RuntimeState,
    activation_mode: crate::ActivationMode,
}

impl CapabilityDiscoveryDescriptor {
    pub(crate) fn new(
        capability: CapabilityId,
        plugin: PluginId,
        installation_id: InstallationId,
        runtime_id: Option<RuntimeId>,
        lifecycle_state: crate::RuntimeState,
        activation_mode: crate::ActivationMode,
    ) -> Self {
        Self {
            capability,
            plugin,
            installation_id,
            runtime_id,
            lifecycle_state,
            activation_mode,
        }
    }

    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub fn installation_id(&self) -> &InstallationId {
        &self.installation_id
    }

    pub const fn runtime_id(&self) -> Option<RuntimeId> {
        self.runtime_id
    }

    pub const fn lifecycle_state(&self) -> crate::RuntimeState {
        self.lifecycle_state
    }

    pub const fn activation_mode(&self) -> crate::ActivationMode {
        self.activation_mode
    }
}

struct RegisteredProvider {
    descriptor: ProviderDescriptor,
    service: Arc<dyn CapabilityService>,
}

type ProviderMap = BTreeMap<RuntimeId, RegisteredProvider>;
type ProviderRegistry = BTreeMap<CapabilityId, ProviderMap>;

pub(crate) struct ResolvedProvider {
    pub(crate) descriptor: ProviderDescriptor,
    pub(crate) service: Arc<dyn CapabilityService>,
}

#[derive(Default)]
pub(crate) struct CapabilityRegistry {
    providers: RwLock<ProviderRegistry>,
}

impl CapabilityRegistry {
    pub(crate) fn publish(
        &self,
        provider: PluginId,
        installation_id: InstallationId,
        runtime_id: RuntimeId,
        principal: PrincipalId,
        capability: CapabilityId,
        service: Arc<dyn CapabilityService>,
    ) {
        let descriptor = ProviderDescriptor::new(
            capability.clone(),
            provider,
            installation_id,
            runtime_id,
            principal,
        );
        let mut providers = self
            .providers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        providers.entry(capability).or_default().insert(
            runtime_id,
            RegisteredProvider {
                descriptor,
                service,
            },
        );
    }

    pub(crate) fn unpublish(&self, provider: &RuntimeId, capability: &CapabilityId) {
        let mut providers = self
            .providers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let remove_capability = if let Some(provider_map) = providers.get_mut(capability) {
            provider_map.remove(provider);
            provider_map.is_empty()
        } else {
            false
        };
        if remove_capability {
            providers.remove(capability);
        }
    }

    pub(crate) fn has_provider(&self, required: &CapabilityId) -> bool {
        self.resolve(required, &BTreeSet::new()).is_some()
    }

    pub(crate) fn provider_runtime_for(&self, required: &CapabilityId) -> Option<RuntimeId> {
        self.resolve(required, &BTreeSet::new())
            .map(|resolved| resolved.descriptor.runtime_id)
    }

    pub(crate) fn provider_runtime_for_except(
        &self,
        required: &CapabilityId,
        excluded: &BTreeSet<RuntimeId>,
    ) -> Option<RuntimeId> {
        self.resolve(required, excluded)
            .map(|resolved| resolved.descriptor.runtime_id)
    }

    pub(crate) fn resolve(
        &self,
        required: &CapabilityId,
        excluded: &BTreeSet<RuntimeId>,
    ) -> Option<ResolvedProvider> {
        let providers = self
            .providers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut candidates = Vec::new();

        for (provided_id, provider_map) in providers.iter() {
            if !provided_id.is_compatible_with(required) {
                continue;
            }
            for (provider_id, service) in provider_map {
                if excluded.contains(provider_id) {
                    continue;
                }
                candidates.push((provided_id.clone(), *provider_id, service));
            }
        }

        candidates.sort_by(
            |(left_capability, left_runtime, left), (right_capability, right_runtime, right)| {
                right_capability
                    .interface_version()
                    .minor()
                    .cmp(&left_capability.interface_version().minor())
                    .then_with(|| {
                        left.descriptor
                            .installation_id
                            .cmp(&right.descriptor.installation_id)
                    })
                    .then_with(|| left_runtime.cmp(right_runtime))
                    .then_with(|| left_capability.cmp(right_capability))
            },
        );
        candidates
            .into_iter()
            .next()
            .map(|(capability, runtime_id, provider)| {
                let mut descriptor = provider.descriptor.clone();
                descriptor.capability = capability;
                debug_assert_eq!(descriptor.runtime_id, runtime_id);
                ResolvedProvider {
                    descriptor,
                    service: Arc::clone(&provider.service),
                }
            })
    }

    pub(crate) fn selection(
        &self,
        required: &CapabilityId,
        excluded: &BTreeSet<RuntimeId>,
    ) -> (Option<ResolvedProvider>, ProviderSelectionDiagnostic) {
        let resolved = self.resolve(required, excluded);
        let candidate_count = self.compatible_candidate_count(required, excluded);
        let diagnostic = ProviderSelectionDiagnostic::new(
            required.clone(),
            candidate_count,
            resolved.as_ref().map(|provider| &provider.descriptor),
            "highest-compatible-minor; installation-id; runtime-id",
            if resolved.is_some() {
                "selected deterministic compatible active provider"
            } else {
                "no compatible active provider"
            },
        );
        (resolved, diagnostic)
    }

    fn compatible_candidate_count(
        &self,
        required: &CapabilityId,
        excluded: &BTreeSet<RuntimeId>,
    ) -> usize {
        let providers = self
            .providers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        providers
            .iter()
            .filter(|(provided, _)| provided.is_compatible_with(required))
            .map(|(_, candidates)| {
                candidates
                    .keys()
                    .filter(|runtime_id| !excluded.contains(runtime_id))
                    .count()
            })
            .sum()
    }
}
