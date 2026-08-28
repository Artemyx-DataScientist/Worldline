use std::{any::Any, error::Error, fmt};

use crate::{
    CapabilityId, DenialReason, InstallationId, InvocationId, LifecycleOperationId, OperationId,
    PluginId, PrincipalId, ResourceId, RuntimeId, RuntimeLifecycleState, SecurityError, StateError,
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
            Self::Unavailable { capability } | Self::InvocationFailed { capability, .. } => {
                Some(capability)
            }
            Self::Denied { capability, .. } => Some(capability),
            Self::UndeclaredDependency { capability, .. } => Some(capability),
            Self::PrincipalUnavailable { .. } => None,
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
