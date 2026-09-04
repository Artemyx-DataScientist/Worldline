use std::{any::Any, error::Error, fmt};

use crate::{
    CapabilityId, DenialReason, InstallationId, InvocationId, LifecycleOperationId, OperationId,
    PluginId, PrincipalId, ResourceId, RpcRequestId, RpcRetryClass, RuntimeId,
    RuntimeLifecycleState, SecurityError, StateError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginError {
    message: String,
}

impl PluginError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<&str> for PluginError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for PluginError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PluginError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    UndeclaredDependency {
        capability: CapabilityId,
        consumer: PluginId,
    },
    Unavailable {
        capability: CapabilityId,
    },
    NoCompatibleProvider {
        request_id: RpcRequestId,
        invocation: InvocationId,
        capability: CapabilityId,
    },
    TargetUnavailable {
        request_id: RpcRequestId,
        invocation: InvocationId,
        capability: CapabilityId,
        target: InstallationId,
    },
    PrincipalUnavailable {
        principal: PrincipalId,
    },
    Denied {
        invocation: InvocationId,
        caller: PrincipalId,
        capability: Box<CapabilityId>,
        operation: OperationId,
        resource: Box<ResourceId>,
        reason: DenialReason,
    },
    InvocationFailed {
        capability: CapabilityId,
        message: String,
    },
    RpcDeadlineExceeded {
        request_id: RpcRequestId,
        invocation: InvocationId,
    },
    RpcCancelled {
        request_id: RpcRequestId,
        invocation: InvocationId,
    },
    RpcProviderBusy {
        request_id: RpcRequestId,
        invocation: InvocationId,
        runtime: RuntimeId,
    },
    RpcQueueFull {
        request_id: RpcRequestId,
        invocation: InvocationId,
        runtime: RuntimeId,
    },
    RpcRuntimeUnavailable {
        request_id: RpcRequestId,
        invocation: InvocationId,
        runtime: RuntimeId,
    },
    RpcInvalidRetry {
        request_id: RpcRequestId,
        invocation: InvocationId,
        requested: RpcRetryClass,
        declared: RpcRetryClass,
    },
    RpcMissingIdempotencyKey {
        request_id: RpcRequestId,
        invocation: InvocationId,
    },
    RpcStaleCompletion {
        request_id: RpcRequestId,
        invocation: InvocationId,
    },
}

impl CapabilityError {
    pub fn denial_reason(&self) -> Option<DenialReason> {
        match self {
            Self::Denied { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    pub fn invocation_id(&self) -> Option<&InvocationId> {
        match self {
            Self::Denied { invocation, .. } => Some(invocation),
            _ => None,
        }
    }

    pub fn caller(&self) -> Option<&PrincipalId> {
        match self {
            Self::Denied { caller, .. } => Some(caller),
            _ => None,
        }
    }

    pub fn capability(&self) -> Option<&CapabilityId> {
        match self {
            Self::Unavailable { capability }
            | Self::NoCompatibleProvider { capability, .. }
            | Self::TargetUnavailable { capability, .. }
            | Self::InvocationFailed { capability, .. } => Some(capability),
            Self::Denied { capability, .. } => Some(capability),
            Self::UndeclaredDependency { capability, .. } => Some(capability),
            Self::PrincipalUnavailable { .. } => None,
            Self::RpcDeadlineExceeded { .. }
            | Self::RpcCancelled { .. }
            | Self::RpcProviderBusy { .. }
            | Self::RpcQueueFull { .. }
            | Self::RpcRuntimeUnavailable { .. }
            | Self::RpcInvalidRetry { .. }
            | Self::RpcMissingIdempotencyKey { .. }
            | Self::RpcStaleCompletion { .. } => None,
        }
    }

    pub fn request_id(&self) -> Option<&RpcRequestId> {
        match self {
            Self::NoCompatibleProvider { request_id, .. }
            | Self::TargetUnavailable { request_id, .. }
            | Self::RpcDeadlineExceeded { request_id, .. }
            | Self::RpcCancelled { request_id, .. }
            | Self::RpcProviderBusy { request_id, .. }
            | Self::RpcQueueFull { request_id, .. }
            | Self::RpcInvalidRetry { request_id, .. }
            | Self::RpcMissingIdempotencyKey { request_id, .. }
            | Self::RpcStaleCompletion { request_id, .. } => Some(request_id),
            _ => None,
        }
    }

    pub fn attempt_id(&self) -> Option<&InvocationId> {
        match self {
            Self::Denied { invocation, .. }
            | Self::NoCompatibleProvider { invocation, .. }
            | Self::TargetUnavailable { invocation, .. }
            | Self::RpcDeadlineExceeded { invocation, .. }
            | Self::RpcCancelled { invocation, .. }
            | Self::RpcProviderBusy { invocation, .. }
            | Self::RpcQueueFull { invocation, .. }
            | Self::RpcRuntimeUnavailable { invocation, .. }
            | Self::RpcInvalidRetry { invocation, .. }
            | Self::RpcMissingIdempotencyKey { invocation, .. }
            | Self::RpcStaleCompletion { invocation, .. } => Some(invocation),
            _ => None,
        }
    }

    pub fn target_installation(&self) -> Option<&InstallationId> {
        match self {
            Self::TargetUnavailable { target, .. } => Some(target),
            _ => None,
        }
    }
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndeclaredDependency {
                capability,
                consumer,
            } => write!(
                formatter,
                "plugin '{consumer}' has not declared dependency on capability '{capability}'"
            ),
            Self::Unavailable { capability } => {
                write!(formatter, "capability '{capability}' is unavailable")
            }
            Self::NoCompatibleProvider {
                request_id,
                invocation,
                capability,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' has no compatible provider for capability '{capability}'"
            ),
            Self::TargetUnavailable {
                request_id,
                invocation,
                capability,
                target,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' target installation '{target}' is unavailable or incompatible for capability '{capability}'"
            ),
            Self::PrincipalUnavailable { principal } => {
                write!(formatter, "principal '{principal}' is unavailable")
            }
            Self::Denied {
                caller,
                capability,
                operation,
                resource,
                reason,
                ..
            } => write!(
                formatter,
                "principal '{caller}' is denied operation '{operation}' on capability '{capability}' for resource '{resource}': {reason}"
            ),
            Self::InvocationFailed {
                capability,
                message,
            } => write!(
                formatter,
                "capability '{capability}' invocation failed: {message}"
            ),
            Self::RpcDeadlineExceeded {
                request_id,
                invocation,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' exceeded its deadline"
            ),
            Self::RpcCancelled {
                request_id,
                invocation,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' was cancelled"
            ),
            Self::RpcProviderBusy {
                request_id,
                invocation,
                runtime,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' provider runtime '{runtime}' is busy"
            ),
            Self::RpcQueueFull {
                request_id,
                invocation,
                runtime,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' provider runtime '{runtime}' queue is full"
            ),
            Self::RpcRuntimeUnavailable {
                request_id,
                invocation,
                runtime,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' provider runtime '{runtime}' is unavailable"
            ),
            Self::RpcInvalidRetry {
                request_id,
                invocation,
                requested,
                declared,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' requested retry class '{requested}' above provider contract '{declared}'"
            ),
            Self::RpcMissingIdempotencyKey {
                request_id,
                invocation,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' requires an idempotency key"
            ),
            Self::RpcStaleCompletion {
                request_id,
                invocation,
            } => write!(
                formatter,
                "RPC request '{request_id}' invocation '{invocation}' completed after its caller-visible outcome was closed"
            ),
        }
    }
}

impl Error for CapabilityError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectCleanupError {
    message: String,
}

impl EffectCleanupError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<&str> for EffectCleanupError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for EffectCleanupError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl fmt::Display for EffectCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for EffectCleanupError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KernelError {
    DuplicatePlugin {
        id: PluginId,
    },
    UnknownPlugin {
        id: PluginId,
    },
    UnknownRuntime {
        runtime: RuntimeId,
    },
    RuntimeAlreadyActiveForInstallation {
        installation: InstallationId,
    },
    InvalidRuntimeTransition {
        runtime: RuntimeId,
        from: RuntimeLifecycleState,
        to: RuntimeLifecycleState,
    },
    RuntimeInstallationMismatch {
        runtime: RuntimeId,
        installation: InstallationId,
    },
    RuntimeActivationFailed {
        runtime: RuntimeId,
        message: String,
    },
    RuntimeActivationCancelled {
        runtime: RuntimeId,
    },
    RuntimeActivationDeadlineExceeded {
        runtime: RuntimeId,
    },
    RuntimeDeactivationFailed {
        runtime: RuntimeId,
        message: String,
    },
    RuntimeDeactivationDeadlineExceeded {
        runtime: RuntimeId,
    },
    RuntimeHung {
        runtime: RuntimeId,
    },
    RuntimeQuarantined {
        installation: InstallationId,
    },
    StartupBudgetExceeded,
    NoCompatibleProvider {
        capability: CapabilityId,
    },
    TargetUnavailable {
        capability: CapabilityId,
        target: InstallationId,
    },
    ProviderSelectionFailed {
        capability: CapabilityId,
        reason: String,
    },
    CapabilityVersionIncompatible {
        required: CapabilityId,
        provided: CapabilityId,
    },
    StaleLifecycleCompletion {
        runtime: RuntimeId,
        operation: LifecycleOperationId,
    },
    InvalidDefinition {
        id: PluginId,
        reason: String,
    },
    PluginDefinitionPanicked {
        message: String,
    },
    InvalidExternalHandle {
        handle: u64,
    },
    ExternalHandleRevoked {
        handle: u64,
    },
    ExternalHandleWrongRuntime {
        handle: u64,
        claimed: RuntimeId,
        owner: RuntimeId,
    },
    ExternalHandleScopeDenied {
        handle: u64,
        runtime: RuntimeId,
    },
    ExternalRuntimeNotActive {
        runtime: RuntimeId,
    },
    Security(SecurityError),
    State(StateError),
}

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePlugin { id } => {
                write!(formatter, "plugin '{id}' is already registered")
            }
            Self::UnknownPlugin { id } => write!(formatter, "plugin '{id}' is not registered"),
            Self::UnknownRuntime { runtime } => write!(formatter, "runtime '{runtime}' is unknown"),
            Self::RuntimeAlreadyActiveForInstallation { installation } => write!(
                formatter,
                "installation '{installation}' already has a live runtime"
            ),
            Self::InvalidRuntimeTransition { runtime, from, to } => write!(
                formatter,
                "runtime '{runtime}' cannot transition from '{from:?}' to '{to:?}'"
            ),
            Self::RuntimeInstallationMismatch {
                runtime,
                installation,
            } => write!(
                formatter,
                "runtime '{runtime}' is not bound to installation '{installation}'"
            ),
            Self::RuntimeActivationFailed { runtime, message } => write!(
                formatter,
                "runtime '{runtime}' activation failed: {message}"
            ),
            Self::RuntimeActivationCancelled { runtime } => {
                write!(formatter, "runtime '{runtime}' activation was cancelled")
            }
            Self::RuntimeActivationDeadlineExceeded { runtime } => write!(
                formatter,
                "runtime '{runtime}' activation deadline exceeded"
            ),
            Self::RuntimeDeactivationFailed { runtime, message } => write!(
                formatter,
                "runtime '{runtime}' deactivation failed: {message}"
            ),
            Self::RuntimeDeactivationDeadlineExceeded { runtime } => write!(
                formatter,
                "runtime '{runtime}' deactivation deadline exceeded"
            ),
            Self::RuntimeHung { runtime } => write!(formatter, "runtime '{runtime}' is hung"),
            Self::RuntimeQuarantined { installation } => {
                write!(formatter, "installation '{installation}' is quarantined")
            }
            Self::StartupBudgetExceeded => formatter.write_str("startup budget exceeded"),
            Self::NoCompatibleProvider { capability } => write!(
                formatter,
                "no compatible provider exists for capability '{capability}'"
            ),
            Self::TargetUnavailable { capability, target } => write!(
                formatter,
                "target installation '{target}' is unavailable or incompatible for capability '{capability}'"
            ),
            Self::ProviderSelectionFailed { capability, reason } => write!(
                formatter,
                "provider selection for '{capability}' failed: {reason}"
            ),
            Self::CapabilityVersionIncompatible { required, provided } => write!(
                formatter,
                "provider capability '{provided}' is incompatible with '{required}'"
            ),
            Self::StaleLifecycleCompletion { runtime, operation } => write!(
                formatter,
                "lifecycle completion for runtime '{runtime}' and operation '{operation}' is stale"
            ),
            Self::InvalidDefinition { id, reason } => {
                write!(formatter, "invalid definition for plugin '{id}': {reason}")
            }
            Self::PluginDefinitionPanicked { message } => {
                write!(formatter, "plugin definition panicked: {message}")
            }
            Self::InvalidExternalHandle { handle } => {
                write!(formatter, "external handle '{handle}' does not exist")
            }
            Self::ExternalHandleRevoked { handle } => {
                write!(formatter, "external handle '{handle}' is revoked")
            }
            Self::ExternalHandleWrongRuntime {
                handle,
                claimed,
                owner,
            } => write!(
                formatter,
                "external handle '{handle}' is owned by runtime '{owner}', not '{claimed}'"
            ),
            Self::ExternalHandleScopeDenied { handle, runtime } => write!(
                formatter,
                "external handle '{handle}' does not delegate the requested scope to runtime '{runtime}'"
            ),
            Self::ExternalRuntimeNotActive { runtime } => {
                write!(formatter, "runtime '{runtime}' is not active")
            }
            Self::Security(error) => error.fmt(formatter),
            Self::State(error) => error.fmt(formatter),
        }
    }
}

impl Error for KernelError {}

/// Explicit name for lifecycle transition/operation failures.  The kernel
/// keeps one error enum so existing callers can continue matching
/// `KernelError`, while lifecycle APIs can document this narrower contract.
pub type RuntimeLifecycleError = KernelError;

impl From<SecurityError> for KernelError {
    fn from(error: SecurityError) -> Self {
        Self::Security(error)
    }
}

impl From<StateError> for KernelError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}
