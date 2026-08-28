use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, RwLock},
};

use crate::PluginId;

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

type ProviderMap = BTreeMap<PluginId, Arc<dyn CapabilityService>>;
type ProviderRegistry = BTreeMap<CapabilityId, ProviderMap>;

#[derive(Default)]
pub(crate) struct CapabilityRegistry {
    providers: RwLock<ProviderRegistry>,
}

impl CapabilityRegistry {
    pub(crate) fn publish(
        &self,
        provider: PluginId,
        capability: CapabilityId,
        service: Arc<dyn CapabilityService>,
    ) {
        let mut providers = self
            .providers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        providers
            .entry(capability)
            .or_default()
            .insert(provider, service);
    }

    pub(crate) fn unpublish(&self, provider: &PluginId, capability: &CapabilityId) {
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

    pub(crate) fn provider_for(&self, required: &CapabilityId) -> Option<PluginId> {
        self.resolve(required, &BTreeSet::new())
            .map(|(_, provider, _)| provider)
    }

    pub(crate) fn provider_for_except(
        &self,
        required: &CapabilityId,
        excluded: &BTreeSet<PluginId>,
    ) -> Option<PluginId> {
        self.resolve(required, excluded)
            .map(|(_, provider, _)| provider)
    }

    pub(crate) fn resolve(
        &self,
        required: &CapabilityId,
        excluded: &BTreeSet<PluginId>,
    ) -> Option<(CapabilityId, PluginId, Arc<dyn CapabilityService>)> {
        let providers = self
            .providers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut selected: Option<(CapabilityId, PluginId, Arc<dyn CapabilityService>)> = None;

        for (provided_id, provider_map) in providers.iter() {
            if !provided_id.is_compatible_with(required) {
                continue;
            }
            for (provider_id, service) in provider_map {
                if excluded.contains(provider_id) {
                    continue;
                }
                let should_replace =
                    selected
                        .as_ref()
                        .is_none_or(|(selected_capability, selected_provider, _)| {
                            (provider_id, provided_id) < (selected_provider, selected_capability)
                        });
                if should_replace {
                    selected = Some((
                        provided_id.clone(),
                        provider_id.clone(),
                        Arc::clone(service),
                    ));
                }
            }
        }

        selected
    }
}
