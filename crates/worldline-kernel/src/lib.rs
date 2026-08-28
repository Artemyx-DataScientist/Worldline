//! Minimal, browser-agnostic plugin kernel for Worldline.
//!
//! The kernel deliberately knows only about plugin definitions, capability
//! contracts, lifecycle scopes, owned effects, and an append-only trajectory.
//! Browser engines, inference providers, UI components, and agent runtimes
//! belong in plugins outside this crate.

#![forbid(unsafe_code)]

mod capability;
mod effect;
mod error;
mod kernel;
mod plugin;
mod trajectory;

pub use capability::{
    CapabilityDependency, CapabilityHandle, CapabilityId, CapabilityService, DependencyKind,
    InterfaceVersion,
};
pub use effect::OwnedEffect;
pub use error::{CapabilityError, EffectCleanupError, KernelError, PluginError};
pub use kernel::{Kernel, ReconcileReport, RuntimeState};
pub use plugin::{
    ActivationContext, NoopRuntime, Plugin, PluginDefinition, PluginId, PluginRuntime,
};
pub use trajectory::{LifecyclePhase, TrajectoryEvent, TrajectoryEventKind};
