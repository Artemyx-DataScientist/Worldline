use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::RwLock,
};

use crate::CapabilityId;

/// The category of a subject in the kernel security model.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PrincipalKind {
    System,
    User,
    PluginRuntime,
    Agent,
    Workspace,
}

/// An opaque, stable identity of a security subject.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PrincipalId(String);

impl PrincipalId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_well_formed(&self) -> bool {
        !self.0.trim().is_empty()
    }

    pub(crate) fn plugin_runtime(plugin_id: &str) -> Self {
        Self::new(format!("plugin-runtime:{plugin_id}"))
    }
}

impl From<&str> for PrincipalId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PrincipalId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Registered metadata for a PrincipalId.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Principal {
    id: PrincipalId,
    kind: PrincipalKind,
    display_name: Option<String>,
}

impl Principal {
    pub fn new(id: impl Into<PrincipalId>, kind: PrincipalKind) -> Self {
        Self {
            id: id.into(),
            kind,
            display_name: None,
        }
    }

    pub fn with_display_name(
        id: impl Into<PrincipalId>,
        kind: PrincipalKind,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            display_name: Some(display_name.into()),
        }
    }

    pub fn id(&self) -> &PrincipalId {
        &self.id
    }

    pub const fn kind(&self) -> PrincipalKind {
        self.kind
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }
}

/// Opaque identifier of a capability grant.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GrantId(String);

impl GrantId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for GrantId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for GrantId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Stable identifier of an operation within a capability contract.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId(String);

impl OperationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_well_formed(&self) -> bool {
        !self.0.trim().is_empty()
    }
}

impl From<&str> for OperationId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for OperationId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque identity of one capability invocation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InvocationId(String);

impl InvocationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InvocationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Identity of a lifecycle scope which can own grants.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LifecycleScopeId(u64);

impl LifecycleScopeId {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LifecycleScopeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Logical capability identity used by grants.
///
/// Minor versions are intentionally not part of this identity. A grant for
/// namespace/name major version N remains valid for compatible providers on
/// the same major line.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityContract {
    namespace: String,
    name: String,
    interface_major: u16,
}

impl CapabilityContract {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        interface_major: u16,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            interface_major,
        }
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn interface_major(&self) -> u16 {
        self.interface_major
    }

    pub fn is_well_formed(&self) -> bool {
        !self.namespace.trim().is_empty() && !self.name.trim().is_empty()
    }
}

impl From<CapabilityId> for CapabilityContract {
    fn from(value: CapabilityId) -> Self {
        Self::from(&value)
    }
}

impl From<&CapabilityId> for CapabilityContract {
    fn from(value: &CapabilityId) -> Self {
        Self::new(
            value.namespace(),
            value.name(),
            value.interface_version().major(),
        )
    }
}

impl fmt::Display for CapabilityContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}@{}",
            self.namespace, self.name, self.interface_major
        )
    }
}

/// Structured hierarchical resource identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceId {
    namespace: String,
    segments: Vec<String>,
}

impl ResourceId {
    pub fn new<I, S>(namespace: impl Into<String>, segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            namespace: namespace.into(),
            segments: segments.into_iter().map(Into::into).collect(),
        }
    }

    pub fn root(namespace: impl Into<String>) -> Self {
        Self::new(namespace, std::iter::empty::<String>())
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let mut parts = value.split('/');
        let Some(namespace) = parts.next() else {
            return Err("resource namespace is missing".to_owned());
        };
        if namespace.trim().is_empty() {
            return Err("resource namespace must not be empty".to_owned());
        }
        let segments: Vec<String> = parts.map(str::to_owned).collect();
        if segments.iter().any(|segment| segment.is_empty()) {
            return Err("resource segments must not be empty".to_owned());
        }
        Ok(Self::new(namespace, segments))
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn child(&self, segment: impl Into<String>) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment.into());
        Self::new(self.namespace.clone(), segments)
    }

    pub fn is_well_formed(&self) -> bool {
        !self.namespace.trim().is_empty()
            && !self.namespace.contains('/')
            && self
                .segments
                .iter()
                .all(|segment| !segment.is_empty() && !segment.contains('/'))
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.namespace)?;
        for segment in &self.segments {
            write!(formatter, "/{segment}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for ResourceId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Resource restriction attached to a grant.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ResourceScope {
    Any,
    Exact(ResourceId),
    Subtree(ResourceId),
}

impl ResourceScope {
    pub fn matches(&self, resource: &ResourceId) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(expected) => expected == resource,
            Self::Subtree(root) => {
                root.namespace == resource.namespace
                    && resource.segments.len() >= root.segments.len()
                    && resource.segments[..root.segments.len()] == root.segments[..]
            }
        }
    }

    /// Returns whether child is no broader than this scope.
    pub fn contains_scope(&self, child: &Self) -> bool {
        match (self, child) {
            (Self::Any, _) => true,
            (Self::Exact(parent), Self::Exact(candidate)) => parent == candidate,
            (Self::Exact(parent), _) => {
                matches!(child, Self::Exact(candidate) if parent == candidate)
            }
            (Self::Subtree(parent), Self::Exact(candidate)) => {
                Self::Subtree(parent.clone()).matches(candidate)
            }
            (Self::Subtree(parent), Self::Subtree(candidate)) => {
                Self::Subtree(parent.clone()).matches(candidate)
            }
            (Self::Subtree(_), Self::Any) => false,
        }
    }

    pub fn is_attenuated_from(&self, parent: &Self) -> bool {
        parent.contains_scope(self)
    }

    fn is_well_formed(&self) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(resource) | Self::Subtree(resource) => resource.is_well_formed(),
        }
    }
}

/// Grant ownership duration. Time-based expiration is intentionally not part
/// of this model.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GrantLifetime {
    Persistent,
    Lifecycle(LifecycleScopeId),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GrantStatus {
    Active,
    Revoked,
}

/// Machine-readable reasons for failed authorization or delegation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DenialReason {
    NoGrant,
    GrantRevoked,
    OperationNotAllowed,
    ResourceOutOfScope,
    CapabilityVersionMismatch,
    DelegationNotAllowed,
    DelegationWouldWidenAuthority,
    InvalidAuthoritySource,
    PrincipalUnavailable,
    InvocationDepthExceeded,
}

impl fmt::Display for DenialReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::NoGrant => "NoGrant",
            Self::GrantRevoked => "GrantRevoked",
            Self::OperationNotAllowed => "OperationNotAllowed",
            Self::ResourceOutOfScope => "ResourceOutOfScope",
            Self::CapabilityVersionMismatch => "CapabilityVersionMismatch",
            Self::DelegationNotAllowed => "DelegationNotAllowed",
            Self::DelegationWouldWidenAuthority => "DelegationWouldWidenAuthority",
            Self::InvalidAuthoritySource => "InvalidAuthoritySource",
            Self::PrincipalUnavailable => "PrincipalUnavailable",
            Self::InvocationDepthExceeded => "InvocationDepthExceeded",
        };
        formatter.write_str(name)
    }
}

/// References to authority available to one invocation. It deliberately
/// contains grant identities only, never provider implementations or service
/// objects.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthoritySet {
    grants: BTreeSet<GrantId>,
}

impl AuthoritySet {
    pub fn from_grant(grant: impl Into<GrantId>) -> Self {
        let mut grants = BTreeSet::new();
        grants.insert(grant.into());
        Self { grants }
    }

    pub fn from_grants<I, G>(grants: I) -> Self
    where
        I: IntoIterator<Item = G>,
        G: Into<GrantId>,
    {
        Self {
            grants: grants.into_iter().map(Into::into).collect(),
        }
    }

    pub fn grants(&self) -> &BTreeSet<GrantId> {
        &self.grants
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

/// Identifies which authority plane an invocation is using.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthoritySource {
    Caller,
    Delegated(AuthoritySet),
    ProviderSelf(PrincipalId),
}

/// A broker request with trusted security metadata separated from payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationRequest {
    caller: PrincipalId,
    capability: CapabilityId,
    operation: OperationId,
    resource: ResourceId,
    authority: AuthoritySource,
    payload: Vec<u8>,
    causal_parent: Option<InvocationId>,
    nested_depth: usize,
}

impl InvocationRequest {
    pub fn new<P>(
        caller: impl Into<PrincipalId>,
        capability: impl Into<CapabilityId>,
        operation: impl Into<OperationId>,
        resource: impl Into<ResourceId>,
        payload: P,
    ) -> Self
    where
        P: AsRef<[u8]>,
    {
        Self {
            caller: caller.into(),
            capability: capability.into(),
            operation: operation.into(),
            resource: resource.into(),
            authority: AuthoritySource::Caller,
            payload: payload.as_ref().to_vec(),
            causal_parent: None,
            nested_depth: 0,
        }
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    pub fn operation(&self) -> &OperationId {
        &self.operation
    }

    pub fn resource(&self) -> &ResourceId {
        &self.resource
    }

    pub fn authority(&self) -> &AuthoritySource {
        &self.authority
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn causal_parent(&self) -> Option<&InvocationId> {
        self.causal_parent.as_ref()
    }

    pub fn with_authority(mut self, authority: AuthoritySource) -> Self {
        self.authority = authority;
        self
    }

    pub fn with_causal_parent(mut self, parent: InvocationId) -> Self {
        self.causal_parent = Some(parent);
        self
    }

    pub(crate) fn with_nested_depth(mut self, depth: usize) -> Self {
        self.nested_depth = depth;
        self
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PrincipalId,
        CapabilityId,
        OperationId,
        ResourceId,
        AuthoritySource,
        Vec<u8>,
        Option<InvocationId>,
        usize,
    ) {
        (
            self.caller,
            self.capability,
            self.operation,
            self.resource,
            self.authority,
            self.payload,
            self.causal_parent,
            self.nested_depth,
        )
    }
}

/// A grant request accepted by the kernel security authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantRequest {
    issuer: PrincipalId,
    subject: PrincipalId,
    capability_contract: CapabilityContract,
    allowed_operations: BTreeSet<OperationId>,
    resource_scope: ResourceScope,
    delegable: bool,
    parent_grant: Option<GrantId>,
    lifetime: GrantLifetime,
}

impl GrantRequest {
    pub fn new(
        issuer: impl Into<PrincipalId>,
        subject: impl Into<PrincipalId>,
        capability_contract: impl Into<CapabilityContract>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            subject: subject.into(),
            capability_contract: capability_contract.into(),
            allowed_operations: BTreeSet::new(),
            resource_scope: ResourceScope::Any,
            delegable: false,
            parent_grant: None,
            lifetime: GrantLifetime::Persistent,
        }
    }

    pub fn issuer(&self) -> &PrincipalId {
        &self.issuer
    }

    pub fn subject(&self) -> &PrincipalId {
        &self.subject
    }

    pub fn capability_contract(&self) -> &CapabilityContract {
        &self.capability_contract
    }

    pub fn allowed_operations(&self) -> &BTreeSet<OperationId> {
        &self.allowed_operations
    }

    pub fn resource_scope(&self) -> &ResourceScope {
        &self.resource_scope
    }

    pub const fn delegable(&self) -> bool {
        self.delegable
    }

    pub fn parent_grant(&self) -> Option<&GrantId> {
        self.parent_grant.as_ref()
    }

    pub const fn lifetime(&self) -> GrantLifetime {
        self.lifetime
    }

    pub fn allow_operation(mut self, operation: impl Into<OperationId>) -> Self {
        self.allowed_operations.insert(operation.into());
        self
    }

    pub fn allow_operations<I, O>(mut self, operations: I) -> Self
    where
        I: IntoIterator<Item = O>,
        O: Into<OperationId>,
    {
        self.allowed_operations
            .extend(operations.into_iter().map(Into::into));
        self
    }

    pub fn with_resource_scope(mut self, scope: ResourceScope) -> Self {
        self.resource_scope = scope;
        self
    }

    pub fn with_delegable(mut self, value: bool) -> Self {
        self.delegable = value;
        self
    }

    pub fn with_parent_grant(mut self, parent: impl Into<GrantId>) -> Self {
        self.parent_grant = Some(parent.into());
        self
    }

    pub fn with_lifetime(mut self, lifetime: GrantLifetime) -> Self {
        self.lifetime = lifetime;
        self
    }

    pub(crate) fn into_grant(self, id: GrantId) -> CapabilityGrant {
        CapabilityGrant {
            id,
            issuer: self.issuer,
            subject: self.subject,
            capability_contract: self.capability_contract,
            allowed_operations: self.allowed_operations,
            resource_scope: self.resource_scope,
            delegable: self.delegable,
            parent_grant: self.parent_grant,
            lifetime: self.lifetime,
            status: GrantStatus::Active,
        }
    }
}

/// Explicit authority assigned to one subject for one capability contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityGrant {
    id: GrantId,
    issuer: PrincipalId,
    subject: PrincipalId,
    capability_contract: CapabilityContract,
    allowed_operations: BTreeSet<OperationId>,
    resource_scope: ResourceScope,
    delegable: bool,
    parent_grant: Option<GrantId>,
    lifetime: GrantLifetime,
    status: GrantStatus,
}

impl CapabilityGrant {
    pub fn id(&self) -> &GrantId {
        &self.id
    }

    pub fn issuer(&self) -> &PrincipalId {
        &self.issuer
    }

    pub fn subject(&self) -> &PrincipalId {
        &self.subject
    }

    pub fn capability_contract(&self) -> &CapabilityContract {
        &self.capability_contract
    }

    pub fn allowed_operations(&self) -> &BTreeSet<OperationId> {
        &self.allowed_operations
    }

    pub fn resource_scope(&self) -> &ResourceScope {
        &self.resource_scope
    }

    pub const fn delegable(&self) -> bool {
        self.delegable
    }

    pub fn parent_grant(&self) -> Option<&GrantId> {
        self.parent_grant.as_ref()
    }

    pub const fn lifetime(&self) -> GrantLifetime {
        self.lifetime
    }

    pub const fn status(&self) -> GrantStatus {
        self.status
    }

    pub const fn is_active(&self) -> bool {
        matches!(self.status, GrantStatus::Active)
    }
}

/// Errors returned by the kernel security authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecurityError {
    PrincipalUnavailable { principal: PrincipalId },
    DuplicatePrincipal { principal: PrincipalId },
    UnknownGrant { grant: GrantId },
    InvalidGrant { reason: String },
    Denied { reason: DenialReason },
}

impl fmt::Display for SecurityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrincipalUnavailable { principal } => {
                write!(formatter, "principal '{principal}' is not registered")
            }
            Self::DuplicatePrincipal { principal } => {
                write!(formatter, "principal '{principal}' is already registered")
            }
            Self::UnknownGrant { grant } => write!(formatter, "grant '{grant}' is unknown"),
            Self::InvalidGrant { reason } => write!(formatter, "invalid grant: {reason}"),
            Self::Denied { reason } => write!(formatter, "security operation denied: {reason}"),
        }
    }
}

impl std::error::Error for SecurityError {}

#[derive(Default)]
struct SecurityState {
    principals: BTreeMap<PrincipalId, Principal>,
    grants: BTreeMap<GrantId, CapabilityGrant>,
    active_scopes: BTreeSet<LifecycleScopeId>,
    next_grant: u64,
    next_invocation: u64,
    next_scope: u64,
}

/// In-memory security authority shared by kernel handles and the invocation
/// broker. It contains no provider service references.
#[derive(Default)]
pub(crate) struct SecurityStore {
    state: RwLock<SecurityState>,
}

impl SecurityStore {
    pub(crate) fn new() -> Self {
        let store = Self::default();
        let system = Principal::new(PrincipalId::new("worldline-system"), PrincipalKind::System);
        store
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .principals
            .insert(system.id.clone(), system);
        store
    }

    pub(crate) fn system_principal(&self) -> PrincipalId {
        PrincipalId::new("worldline-system")
    }

    pub(crate) fn register_principal(&self, principal: Principal) -> Result<(), SecurityError> {
        if !principal.id.is_well_formed() {
            return Err(SecurityError::InvalidGrant {
                reason: "principal id must not be empty".to_owned(),
            });
        }
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.principals.contains_key(&principal.id) {
            return Err(SecurityError::DuplicatePrincipal {
                principal: principal.id,
            });
        }
        state.principals.insert(principal.id.clone(), principal);
        Ok(())
    }

    pub(crate) fn ensure_principal(&self, principal: Principal) -> Result<bool, SecurityError> {
        if !principal.id.is_well_formed() {
            return Err(SecurityError::InvalidGrant {
                reason: "principal id must not be empty".to_owned(),
            });
        }
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.principals.get(&principal.id) {
            Some(existing) if existing == &principal => Ok(false),
            Some(_) => Err(SecurityError::DuplicatePrincipal {
                principal: principal.id,
            }),
            None => {
                state.principals.insert(principal.id.clone(), principal);
                Ok(true)
            }
        }
    }

    pub(crate) fn principal(&self, id: &PrincipalId) -> Option<Principal> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .principals
            .get(id)
            .cloned()
    }

    pub(crate) fn principal_exists(&self, id: &PrincipalId) -> bool {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .principals
            .contains_key(id)
    }

    pub(crate) fn allocate_scope(&self) -> LifecycleScopeId {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.next_scope += 1;
        let scope = LifecycleScopeId::new(state.next_scope);
        state.active_scopes.insert(scope);
        scope
    }

    pub(crate) fn allocate_invocation(&self) -> InvocationId {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.next_invocation += 1;
        InvocationId::new(format!("invocation-{}", state.next_invocation))
    }

    pub(crate) fn issue(&self, request: GrantRequest) -> Result<CapabilityGrant, SecurityError> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if !state.principals.contains_key(&request.issuer)
            || !state.principals.contains_key(&request.subject)
        {
            let principal = if !state.principals.contains_key(&request.issuer) {
                request.issuer.clone()
            } else {
                request.subject.clone()
            };
            return Err(SecurityError::PrincipalUnavailable { principal });
        }
        if !request.capability_contract.is_well_formed() {
            return Err(SecurityError::InvalidGrant {
                reason: "capability contract identity must have namespace and name".to_owned(),
            });
        }
        if request.allowed_operations.is_empty()
            || request
                .allowed_operations
                .iter()
                .any(|operation| !operation.is_well_formed())
        {
            return Err(SecurityError::InvalidGrant {
                reason: "grant must contain at least one well-formed operation".to_owned(),
            });
        }
        if !request.resource_scope.is_well_formed() {
            return Err(SecurityError::InvalidGrant {
                reason: "resource scope must be well formed".to_owned(),
            });
        }
        if let GrantLifetime::Lifecycle(scope) = request.lifetime
            && !state.active_scopes.contains(&scope)
        {
            return Err(SecurityError::InvalidGrant {
                reason: format!("lifecycle scope '{scope}' is no longer active"),
            });
        }

        if let Some(parent_id) = &request.parent_grant {
            let Some(parent) = state.grants.get(parent_id) else {
                return Err(SecurityError::UnknownGrant {
                    grant: parent_id.clone(),
                });
            };
            if !Self::grant_is_active(&state, parent_id) {
                return Err(SecurityError::Denied {
                    reason: DenialReason::GrantRevoked,
                });
            }
            if parent.subject != request.issuer || !parent.delegable {
                return Err(SecurityError::Denied {
                    reason: DenialReason::DelegationNotAllowed,
                });
            }
            if request.delegable && !parent.delegable {
                return Err(SecurityError::Denied {
                    reason: DenialReason::DelegationWouldWidenAuthority,
                });
            }
            if parent.capability_contract != request.capability_contract {
                return Err(SecurityError::Denied {
                    reason: DenialReason::CapabilityVersionMismatch,
                });
            }
            if !request
                .allowed_operations
                .is_subset(&parent.allowed_operations)
                || !request
                    .resource_scope
                    .is_attenuated_from(&parent.resource_scope)
            {
                return Err(SecurityError::Denied {
                    reason: DenialReason::DelegationWouldWidenAuthority,
                });
            }
            if matches!(parent.lifetime, GrantLifetime::Lifecycle(_))
                && request.lifetime != parent.lifetime
            {
                return Err(SecurityError::Denied {
                    reason: DenialReason::DelegationWouldWidenAuthority,
                });
            }
        }

        state.next_grant += 1;
        let id = GrantId::new(format!("grant-{}", state.next_grant));
        let grant = request.into_grant(id.clone());
        state.grants.insert(id, grant.clone());
        Ok(grant)
    }

    pub(crate) fn grant(&self, id: &GrantId) -> Option<CapabilityGrant> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .grants
            .get(id)
            .cloned()
    }

    pub(crate) fn revoke(&self, id: &GrantId) -> Result<Vec<GrantId>, SecurityError> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.grants.contains_key(id) {
            return Err(SecurityError::UnknownGrant { grant: id.clone() });
        }

        let mut revoked = BTreeSet::new();
        let mut pending = vec![id.clone()];
        while let Some(candidate) = pending.pop() {
            let Some(grant) = state.grants.get_mut(&candidate) else {
                continue;
            };
            if grant.status == GrantStatus::Revoked {
                continue;
            }
            grant.status = GrantStatus::Revoked;
            revoked.insert(candidate.clone());
            let descendants: Vec<GrantId> = state
                .grants
                .values()
                .filter(|child| child.parent_grant.as_ref() == Some(&candidate))
                .map(|child| child.id.clone())
                .collect();
            pending.extend(descendants);
        }
        let mut result = Vec::with_capacity(revoked.len());
        if revoked.remove(id) {
            result.push(id.clone());
        }
        result.extend(revoked);
        Ok(result)
    }

    pub(crate) fn revoke_subject(&self, subject: &PrincipalId) -> (Vec<GrantId>, Vec<GrantId>) {
        let direct_ids: Vec<GrantId> = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .grants
            .values()
            .filter(|grant| grant.subject == *subject && grant.status == GrantStatus::Active)
            .map(|grant| grant.id.clone())
            .collect();

        let direct_set: BTreeSet<GrantId> = direct_ids.iter().cloned().collect();
        let mut changed = BTreeSet::new();
        for id in direct_ids {
            if let Ok(revoked) = self.revoke(&id) {
                changed.extend(revoked);
            }
        }

        let mut direct = Vec::new();
        let mut descendants = Vec::new();
        for id in changed {
            if direct_set.contains(&id) {
                direct.push(id);
            } else {
                descendants.push(id);
            }
        }
        (direct, descendants)
    }

    pub(crate) fn revoke_lifecycle_scope(&self, scope: LifecycleScopeId) -> Vec<GrantId> {
        let ids: Vec<GrantId> = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.active_scopes.remove(&scope);
            state
                .grants
                .values()
                .filter(|grant| grant.lifetime == GrantLifetime::Lifecycle(scope))
                .map(|grant| grant.id.clone())
                .collect()
        };
        let mut revoked = BTreeSet::new();
        for id in ids {
            if let Ok(ids) = self.revoke(&id) {
                revoked.extend(ids);
            }
        }
        revoked.into_iter().collect()
    }

    pub(crate) fn authorize(
        &self,
        caller: &PrincipalId,
        capability: &CapabilityId,
        operation: &OperationId,
        resource: &ResourceId,
        source: &AuthoritySource,
        enclosing_provider: Option<&PrincipalId>,
    ) -> Result<AuthoritySet, DenialReason> {
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.principals.contains_key(caller) {
            return Err(DenialReason::PrincipalUnavailable);
        }
        if !capability.is_well_formed() {
            return Err(DenialReason::NoGrant);
        }
        if !operation.is_well_formed() {
            return Err(DenialReason::OperationNotAllowed);
        }
        if !resource.is_well_formed() {
            return Err(DenialReason::ResourceOutOfScope);
        }

        let contract = CapabilityContract::from(capability);
        let candidates: Vec<&CapabilityGrant> = match source {
            AuthoritySource::Caller => state
                .grants
                .values()
                .filter(|grant| grant.subject == *caller)
                .collect(),
            AuthoritySource::ProviderSelf(provider) => {
                let is_runtime_provider = state
                    .principals
                    .get(provider)
                    .is_some_and(|principal| principal.kind() == PrincipalKind::PluginRuntime);
                if provider != caller
                    || enclosing_provider != Some(provider)
                    || !is_runtime_provider
                {
                    return Err(DenialReason::InvalidAuthoritySource);
                }
                state
                    .grants
                    .values()
                    .filter(|grant| grant.subject == *provider)
                    .collect()
            }
            AuthoritySource::Delegated(authority) => {
                if authority.is_empty() {
                    return Err(DenialReason::InvalidAuthoritySource);
                }
                let mut candidates = Vec::with_capacity(authority.grants.len());
                for id in &authority.grants {
                    let Some(grant) = state.grants.get(id) else {
                        return Err(DenialReason::InvalidAuthoritySource);
                    };
                    if grant.subject != *caller {
                        return Err(DenialReason::InvalidAuthoritySource);
                    }
                    candidates.push(grant);
                }
                candidates
            }
        };

        if candidates.is_empty() {
            return Err(DenialReason::NoGrant);
        }

        let same_name = candidates.iter().any(|grant| {
            grant.capability_contract.namespace == contract.namespace
                && grant.capability_contract.name == contract.name
        });
        let mut matching_contract = Vec::new();
        let mut revoked_matching = false;
        for grant in candidates {
            if grant.capability_contract != contract {
                continue;
            }
            if !Self::grant_is_active(&state, grant.id()) {
                revoked_matching = true;
                continue;
            }
            matching_contract.push(grant);
        }

        if matching_contract.is_empty() {
            if revoked_matching {
                return Err(DenialReason::GrantRevoked);
            }
            if same_name {
                return Err(DenialReason::CapabilityVersionMismatch);
            }
            return Err(DenialReason::NoGrant);
        }

        let mut operation_allowed = false;
        for grant in &matching_contract {
            if !grant.allowed_operations.contains(operation) {
                continue;
            }
            operation_allowed = true;
            if grant.resource_scope.matches(resource) {
                return Ok(AuthoritySet::from_grant(grant.id.clone()));
            }
        }
        if !operation_allowed {
            return Err(DenialReason::OperationNotAllowed);
        }
        Err(DenialReason::ResourceOutOfScope)
    }

    fn grant_is_active(state: &SecurityState, id: &GrantId) -> bool {
        let Some(grant) = state.grants.get(id) else {
            return false;
        };
        if grant.status != GrantStatus::Active {
            return false;
        }
        grant
            .parent_grant
            .as_ref()
            .is_none_or(|parent| Self::grant_is_active(state, parent))
    }
}
