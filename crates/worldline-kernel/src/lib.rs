//! Minimal, browser-agnostic plugin kernel for Worldline.
//!
//! The kernel deliberately knows only about plugin definitions, capability
//! contracts, lifecycle scopes, owned effects, and an append-only trajectory.
//! Browser engines, inference providers, UI components, and agent runtimes
//! belong in plugins outside this crate.

#![forbid(unsafe_code)]

mod bisect;
mod capability;
mod diagnostics;
mod effect;
mod error;
mod events;
mod external;
mod invocation;
mod kernel;
mod persistence;
mod plugin;
mod quarantine;
mod rpc;
mod runtime;
mod safe_mode;
mod security;
mod side_effect;
mod state;
mod telemetry;
mod trajectory;
mod upgrade;

pub use bisect::{BisectEngine, BisectOutcome, BisectTrialRecord};
pub use capability::{
    CapabilityDependency, CapabilityDiscoveryDescriptor, CapabilityId, CapabilityService,
    ContractStability, DependencyKind, InterfaceVersion, ProviderDescriptor,
    ProviderSelectionDiagnostic,
};
pub use diagnostics::{
    CausalFact, CausalFactKind, DiagnosticCausalityChain, DiagnosticCausalityGraph,
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
pub use external::ExternalHandleView;
pub use invocation::{CapabilityHandle, InvocationContext, MAX_NESTED_INVOCATION_DEPTH};
pub use kernel::{Kernel, ReconcileReport, RuntimeMetadata};
pub use persistence::{
    AuditOutcome, AuditRecord, AuditStore, BlobId, BlobStore, CURRENT_STORAGE_FORMAT_VERSION,
    JobId, JobRecord, JobRecoveryPolicy, JobState, JobStore, OutboxId, OutboxRecord, OutboxStatus,
    OutboxStore, PersistenceError, StorageFormatVersion,
};
pub use plugin::{
    ActivationContext, NoopRuntime, Plugin, PluginDefinition, PluginId, PluginRuntime,
};
pub use quarantine::{QuarantineManager, QuarantineReason, QuarantineRecord};
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
pub use safe_mode::{SafeModeManager, SafeModeReason, SafeModeState};
pub use security::{
    AuthoritySet, AuthoritySource, CapabilityContract, CapabilityGrant, DenialReason, GrantId,
    GrantLifetime, GrantRequest, GrantStatus, InvocationId, InvocationRequest, LifecycleScopeId,
    OperationId, Principal, PrincipalId, PrincipalKind, ResourceId, ResourceScope, SecurityError,
};
pub use side_effect::{SideEffectOutcome, SideEffectRecord};
pub use state::{
    BackendState, InMemoryStateBackend, InstallationId, InstallationRecord, InstallationStatus,
    MigrationContext, MigrationError, MigrationId, MigrationPlan, RuntimeStateHandle, StateBackend,
    StateError, StateHandle, StateKey, StateMigration, StateRevision, StateSchemaVersion,
    StateTransaction, StateTransactionId, StateTransactionKind, StateValue,
};
pub use telemetry::{RuntimeOperationalMetrics, TelemetryRegistry};
pub use trajectory::{LifecyclePhase, TrajectoryEvent, TrajectoryEventKind};
pub use upgrade::{
    HealthProbeStatus, InstallationUpgradeRecord, MigrationProvenance, PackageRevisionId,
    UpgradeError, UpgradeManager, UpgradeState,
};
