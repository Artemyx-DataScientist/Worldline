use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, RwLock},
};

use crate::rpc::{ProviderLimits, RpcOperationContract};
use crate::security::OperationId;
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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ContractStability {
    Stable,
    Experimental,
}

impl fmt::Display for ContractStability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stable => formatter.write_str("stable"),
            Self::Experimental => formatter.write_str("experimental"),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityId {
    namespace: String,
    name: String,
    interface_version: InterfaceVersion,
    stability: ContractStability,
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
            stability: ContractStability::Stable,
        }
    }

    pub fn with_stability(
        namespace: impl Into<String>,
        name: impl Into<String>,
        interface_version: InterfaceVersion,
        stability: ContractStability,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            interface_version,
            stability,
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

    pub const fn stability(&self) -> ContractStability {
        self.stability
    }

    pub fn contract(&self) -> crate::CapabilityContract {
        crate::CapabilityContract::from(self)
    }

    pub fn is_well_formed(&self) -> bool {
        !self.namespace.trim().is_empty() && !self.name.trim().is_empty()
    }

    pub fn is_compatible_with(&self, required: &Self) -> bool {
        if self.namespace != required.namespace
            || self.name != required.name
            || self.stability != required.stability
        {
            return false;
        }

        match self.stability {
            ContractStability::Stable => {
                self.interface_version.major == required.interface_version.major
                    && self.interface_version.minor >= required.interface_version.minor
            }
            ContractStability::Experimental => {
                self.interface_version.major == required.interface_version.major
                    && self.interface_version.minor == required.interface_version.minor
            }
        }
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.stability == ContractStability::Experimental {
            write!(
                formatter,
                "experimental:{}/{}@{}",
                self.namespace, self.name, self.interface_version
            )
        } else {
            write!(
                formatter,
                "{}/{}@{}",
                self.namespace, self.name, self.interface_version
            )
        }
    }
}

/// Target resolution policy for capability discovery and invocation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CapabilityTarget {
    /// Select any compatible active provider according to deterministic resolution order.
    AnyCompatible,
    /// Explicitly target an active provider published by the specified installation.
    Installation(InstallationId),
}

impl Default for CapabilityTarget {
    fn default() -> Self {
        Self::AnyCompatible
    }
}

impl From<InstallationId> for CapabilityTarget {
    fn from(installation_id: InstallationId) -> Self {
        Self::Installation(installation_id)
    }
}

impl fmt::Display for CapabilityTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AnyCompatible => formatter.write_str("any-compatible"),
            Self::Installation(installation_id) => {
                write!(formatter, "installation({installation_id})")
            }
        }
    }
}

impl CapabilityTarget {
    pub const fn is_any_compatible(&self) -> bool {
        matches!(self, Self::AnyCompatible)
    }

    pub fn target_installation(&self) -> Option<&InstallationId> {
        match self {
            Self::AnyCompatible => None,
            Self::Installation(installation_id) => Some(installation_id),
        }
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

    /// Provider-owned retry contract.  The broker never grants callers more
    /// retry power than this declaration.
    fn rpc_operation_contract(&self, operation: &OperationId) -> RpcOperationContract {
        RpcOperationContract::never_retry(operation.clone())
    }

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
    pub(crate) limits: ProviderLimits,
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
        self.has_targeted_provider(required, &CapabilityTarget::AnyCompatible)
    }

    pub(crate) fn has_targeted_provider(
        &self,
        required: &CapabilityId,
        target: &CapabilityTarget,
    ) -> bool {
        self.resolve_target(required, target, &BTreeSet::new()).is_some()
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
        self.resolve_target(required, &CapabilityTarget::AnyCompatible, excluded)
    }

    pub(crate) fn resolve_target(
        &self,
        required: &CapabilityId,
        target: &CapabilityTarget,
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
                if let CapabilityTarget::Installation(target_installation) = target {
                    if &service.descriptor.installation_id != target_installation {
                        continue;
                    }
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
        self.selection_target(required, &CapabilityTarget::AnyCompatible, excluded)
    }

    pub(crate) fn selection_target(
        &self,
        required: &CapabilityId,
        target: &CapabilityTarget,
        excluded: &BTreeSet<RuntimeId>,
    ) -> (Option<ResolvedProvider>, ProviderSelectionDiagnostic) {
        let resolved = self.resolve_target(required, target, excluded);
        let candidate_count = self.target_candidate_count(required, target, excluded);
        let policy = match target {
            CapabilityTarget::AnyCompatible => "highest-compatible-minor; installation-id; runtime-id",
            CapabilityTarget::Installation(_) => "explicit-installation-targeting",
        };
        let reason = match (resolved.is_some(), target) {
            (true, CapabilityTarget::AnyCompatible) => {
                "selected deterministic compatible active provider"
            }
            (true, CapabilityTarget::Installation(_)) => {
                "selected explicitly targeted active provider"
            }
            (false, CapabilityTarget::AnyCompatible) => "no compatible active provider",
            (false, CapabilityTarget::Installation(_)) => {
                "targeted provider unavailable or incompatible"
            }
        };
        let diagnostic = ProviderSelectionDiagnostic::new(
            required.clone(),
            candidate_count,
            resolved.as_ref().map(|provider| &provider.descriptor),
            policy,
            reason,
        );
        (resolved, diagnostic)
    }

    fn target_candidate_count(
        &self,
        required: &CapabilityId,
        target: &CapabilityTarget,
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
                    .iter()
                    .filter(|(runtime_id, registered)| {
                        if excluded.contains(runtime_id) {
                            return false;
                        }
                        if let CapabilityTarget::Installation(target_installation) = target {
                            if &registered.descriptor.installation_id != target_installation {
                                return false;
                            }
                        }
                        true
                    })
                    .count()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyService;
    impl CapabilityService for DummyService {
        fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn capability_target_basics() {
        let any = CapabilityTarget::default();
        assert_eq!(any, CapabilityTarget::AnyCompatible);
        assert!(any.is_any_compatible());
        assert_eq!(any.target_installation(), None);
        assert_eq!(any.to_string(), "any-compatible");

        let inst = InstallationId::new("inst-alpha");
        let targeted = CapabilityTarget::from(inst.clone());
        assert_eq!(targeted, CapabilityTarget::Installation(inst.clone()));
        assert!(!targeted.is_any_compatible());
        assert_eq!(targeted.target_installation(), Some(&inst));
        assert_eq!(targeted.to_string(), "installation(inst-alpha)");
    }

    #[test]
    fn capability_registry_targeting_and_selection() {
        let registry = CapabilityRegistry::default();
        let cap_v1 = CapabilityId::new("test", "search", InterfaceVersion::new(1, 0));
        let inst_a = InstallationId::new("inst-a");
        let inst_b = InstallationId::new("inst-b");
        let runtime_a = RuntimeId::new(1, 1);
        let runtime_b = RuntimeId::new(2, 1);
        let plugin = PluginId::new("search-plugin");
        let principal_a = PrincipalId::new("principal-a");
        let principal_b = PrincipalId::new("principal-b");

        registry.publish(
            plugin.clone(),
            inst_a.clone(),
            runtime_a,
            principal_a,
            cap_v1.clone(),
            Arc::new(DummyService),
        );
        registry.publish(
            plugin,
            inst_b.clone(),
            runtime_b,
            principal_b,
            cap_v1.clone(),
            Arc::new(DummyService),
        );

        // Untargeted resolve picks lowest installation_id (inst-a)
        let untargeted = registry.resolve(&cap_v1, &BTreeSet::new()).expect("resolved");
        assert_eq!(untargeted.descriptor.installation_id(), &inst_a);

        // Targeted resolve inst-b explicitly selects inst-b even though inst-a has lower ID
        let targeted_b = registry
            .resolve_target(
                &cap_v1,
                &CapabilityTarget::Installation(inst_b.clone()),
                &BTreeSet::new(),
            )
            .expect("resolved inst_b");
        assert_eq!(targeted_b.descriptor.installation_id(), &inst_b);
        assert_eq!(targeted_b.descriptor.runtime_id(), runtime_b);

        // Targeted resolve inst-a explicitly selects inst-a
        let targeted_a = registry
            .resolve_target(
                &cap_v1,
                &CapabilityTarget::Installation(inst_a.clone()),
                &BTreeSet::new(),
            )
            .expect("resolved inst_a");
        assert_eq!(targeted_a.descriptor.installation_id(), &inst_a);
        assert_eq!(targeted_a.descriptor.runtime_id(), runtime_a);

        // Targeted resolve unknown installation returns None without fallback
        let unknown = InstallationId::new("inst-unknown");
        assert!(
            registry
                .resolve_target(
                    &cap_v1,
                    &CapabilityTarget::Installation(unknown.clone()),
                    &BTreeSet::new()
                )
                .is_none()
        );

        // Targeted resolve with runtime excluded (e.g. quarantined/stopping) returns None without fallback
        let mut excluded = BTreeSet::new();
        excluded.insert(runtime_b);
        assert!(
            registry
                .resolve_target(
                    &cap_v1,
                    &CapabilityTarget::Installation(inst_b.clone()),
                    &excluded
                )
                .is_none()
        );

        // Selection diagnostics verify policy and candidate count
        let (resolved, diag) = registry.selection_target(
            &cap_v1,
            &CapabilityTarget::Installation(inst_b.clone()),
            &BTreeSet::new(),
        );
        assert!(resolved.is_some());
        assert_eq!(diag.compatible_candidate_count(), 1);
        assert_eq!(diag.selected_installation_id(), Some(&inst_b));
        assert_eq!(diag.policy(), "explicit-installation-targeting");
        assert_eq!(diag.reason(), "selected explicitly targeted active provider");

        let (resolved_unavail, diag_unavail) = registry.selection_target(
            &cap_v1,
            &CapabilityTarget::Installation(unknown),
            &BTreeSet::new(),
        );
        assert!(resolved_unavail.is_none());
        assert_eq!(diag_unavail.compatible_candidate_count(), 0);
        assert_eq!(
            diag_unavail.reason(),
            "targeted provider unavailable or incompatible"
        );
    }
}
