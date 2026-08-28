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
mod events;
mod invocation;
mod kernel;
mod plugin;
mod rpc;
mod runtime;
mod security;
mod state;
mod trajectory;

pub use capability::{
    CapabilityDependency, CapabilityDiscoveryDescriptor, CapabilityId, CapabilityService,
    DependencyKind, InterfaceVersion, ProviderDescriptor, ProviderSelectionDiagnostic,
};
pub use effect::OwnedEffect;
pub use error::{
    CapabilityError, EffectCleanupError, KernelError, PluginError, RuntimeLifecycleError,
};
pub use events::{
    DeliveryMode, EventContext, EventContract, EventCursor, EventEnvelope, EventError, EventId,
    EventJournal, EventJournalError, EventPublishOptions, EventQoS, InMemoryEventJournal,
    InvocationCompletedMetadata, OverflowPolicy, PublishReport, SubscriptionHandle, SubscriptionId,
    SubscriptionOptions, invocation_completed_event_contract,
};
pub use invocation::{CapabilityHandle, InvocationContext, MAX_NESTED_INVOCATION_DEPTH};
pub use kernel::{Kernel, ReconcileReport, RuntimeMetadata};
pub use plugin::{
    ActivationContext, NoopRuntime, Plugin, PluginDefinition, PluginId, PluginRuntime,
};
pub use rpc::{
    CausationRef, CorrelationId, DEFAULT_RPC_DEADLINE, ProviderLimits, RpcCallOptions,
    RpcCancellationToken, RpcOperationContract, RpcOutcomeClass, RpcRequestId, RpcRetryClass,
    TraceContext,
};
pub use runtime::{
    ActivationMode, ActivationReason, LifecycleCancellationToken, LifecycleContext,
    LifecycleOperation, LifecycleOperationId, RestartMode, RestartPolicy, RuntimeCriticality,
    RuntimeFailureClass, RuntimeId, RuntimeLaunchPolicy, RuntimeLifecycleState, RuntimeState,
    StartupBudget,
};
pub use security::{
    AuthoritySet, AuthoritySource, CapabilityContract, CapabilityGrant, DenialReason, GrantId,
    GrantLifetime, GrantRequest, GrantStatus, InvocationId, InvocationRequest, LifecycleScopeId,
    OperationId, Principal, PrincipalId, PrincipalKind, ResourceId, ResourceScope, SecurityError,
};
pub use state::{
    BackendState, InMemoryStateBackend, InstallationId, InstallationRecord, InstallationStatus,
    MigrationContext, MigrationError, MigrationId, MigrationPlan, RuntimeStateHandle, StateBackend,
    StateError, StateHandle, StateKey, StateMigration, StateRevision, StateSchemaVersion,
    StateTransaction, StateTransactionId, StateTransactionKind, StateValue,
};
pub use trajectory::{LifecyclePhase, TrajectoryEvent, TrajectoryEventKind};
