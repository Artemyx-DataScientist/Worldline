use std::{any::Any, error::Error, fmt};

use crate::{CapabilityId, PluginId};

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
    InvocationFailed {
        capability: CapabilityId,
        message: String,
    },
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
    DuplicatePlugin { id: PluginId },
    UnknownPlugin { id: PluginId },
    InvalidDefinition { id: PluginId, reason: String },
    PluginDefinitionPanicked { message: String },
}

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePlugin { id } => {
                write!(formatter, "plugin '{id}' is already registered")
            }
            Self::UnknownPlugin { id } => write!(formatter, "plugin '{id}' is not registered"),
            Self::InvalidDefinition { id, reason } => {
                write!(formatter, "invalid definition for plugin '{id}': {reason}")
            }
            Self::PluginDefinitionPanicked { message } => {
                write!(formatter, "plugin definition panicked: {message}")
            }
        }
    }
}

impl Error for KernelError {}

pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}
