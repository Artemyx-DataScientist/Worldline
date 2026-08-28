use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Opaque identity of one activation attempt.
///
/// The first component is the persisted installation incarnation and the
/// second component is a host-local sequence.  The pair is deliberately
/// allocated by the kernel and is never exposed as an installation identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RuntimeId {
    incarnation: u64,
    sequence: u64,
}

impl RuntimeId {
    pub(crate) const fn new(incarnation: u64, sequence: u64) -> Self {
        Self {
            incarnation,
            sequence,
        }
    }

    /// Returns a compact numeric representation useful for diagnostics.
    pub const fn value(self) -> u128 {
        ((self.incarnation as u128) << 64) | self.sequence as u128
    }

    pub const fn incarnation(self) -> u64 {
        self.incarnation
    }

    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

impl fmt::Display for RuntimeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "runtime-{}-{}", self.incarnation, self.sequence)
    }
}

/// Explicit lifecycle state for one runtime activation attempt.
///
/// `Registered` and `Pending` are compatibility spellings retained for the
/// M0.1/M0.2 host API.  New lifecycle diagnostics use `WaitingDependencies`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeLifecycleState {
    Created,
    WaitingDependencies,
    Activating,
    Active,
    Deactivating,
    Stopped,
    Failed,
    Crashed,
    Cancelled,
    Hung,
    Quarantined,
    Registered,
    Pending,
}

impl RuntimeLifecycleState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Stopped
                | Self::Failed
                | Self::Crashed
                | Self::Cancelled
                | Self::Hung
                | Self::Quarantined
        )
    }

    pub const fn is_waiting(self) -> bool {
        matches!(self, Self::WaitingDependencies | Self::Pending)
    }

    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Validates the kernel lifecycle transition graph.
    pub const fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Created | Self::Registered => {
                matches!(
                    next,
                    Self::WaitingDependencies | Self::Pending | Self::Activating
                )
            }
            Self::WaitingDependencies | Self::Pending => {
                matches!(next, Self::Activating | Self::Cancelled)
            }
            Self::Activating => matches!(
                next,
                Self::Active | Self::Failed | Self::Crashed | Self::Cancelled | Self::Hung
            ),
            Self::Active => matches!(next, Self::Deactivating | Self::Crashed),
            Self::Deactivating => {
                matches!(
                    next,
                    Self::Stopped | Self::Failed | Self::Crashed | Self::Hung
                )
            }
            Self::Failed | Self::Crashed | Self::Hung => matches!(next, Self::Quarantined),
            Self::Stopped | Self::Cancelled | Self::Quarantined => false,
        }
    }
}

impl fmt::Display for RuntimeLifecycleState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Created => "Created",
            Self::WaitingDependencies | Self::Pending => "WaitingDependencies",
            Self::Activating => "Activating",
            Self::Active => "Active",
            Self::Deactivating => "Deactivating",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
            Self::Crashed => "Crashed",
            Self::Cancelled => "Cancelled",
            Self::Hung => "Hung",
            Self::Quarantined => "Quarantined",
            Self::Registered => "Registered",
        };
        formatter.write_str(name)
    }
}

/// Backwards-compatible public name used by the bootstrap API.
pub type RuntimeState = RuntimeLifecycleState;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ActivationMode {
    Eager,
    Lazy,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeCriticality {
    Required,
    Optional,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RestartMode {
    Never,
    OnFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartPolicy {
    mode: RestartMode,
    max_attempts: u32,
    backoff_base: Duration,
    backoff_max: Duration,
    quarantine_after: Option<u32>,
}

impl RestartPolicy {
    pub const fn never() -> Self {
        Self {
            mode: RestartMode::Never,
            max_attempts: 0,
            backoff_base: Duration::ZERO,
            backoff_max: Duration::ZERO,
            quarantine_after: None,
        }
    }

    pub const fn on_failure(max_attempts: u32) -> Self {
        Self {
            mode: RestartMode::OnFailure,
            max_attempts,
            backoff_base: Duration::ZERO,
            backoff_max: Duration::ZERO,
            quarantine_after: None,
        }
    }

    pub const fn mode(self) -> RestartMode {
        self.mode
    }

    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }

    pub const fn backoff_base(self) -> Duration {
        self.backoff_base
    }

    pub const fn backoff_max(self) -> Duration {
        self.backoff_max
    }

    pub const fn quarantine_after(self) -> Option<u32> {
        self.quarantine_after
    }

    pub const fn with_backoff(mut self, base: Duration, maximum: Duration) -> Self {
        self.backoff_base = base;
        self.backoff_max = maximum;
        self
    }

    pub const fn with_quarantine_after(mut self, attempts: u32) -> Self {
        self.quarantine_after = Some(attempts);
        self
    }
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self::never()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLaunchPolicy {
    activation_mode: ActivationMode,
    criticality: RuntimeCriticality,
    restart: RestartPolicy,
    activation_deadline: Option<Duration>,
    deactivation_deadline: Option<Duration>,
}

impl RuntimeLaunchPolicy {
    pub const fn new(activation_mode: ActivationMode, criticality: RuntimeCriticality) -> Self {
        Self {
            activation_mode,
            criticality,
            restart: RestartPolicy::never(),
            activation_deadline: None,
            deactivation_deadline: None,
        }
    }

    pub const fn eager(criticality: RuntimeCriticality) -> Self {
        Self::new(ActivationMode::Eager, criticality)
    }

    pub const fn lazy(criticality: RuntimeCriticality) -> Self {
        Self::new(ActivationMode::Lazy, criticality)
    }

    pub const fn required_eager() -> Self {
        Self::eager(RuntimeCriticality::Required)
    }

    pub const fn optional_eager() -> Self {
        Self::eager(RuntimeCriticality::Optional)
    }

    pub const fn required_lazy() -> Self {
        Self::lazy(RuntimeCriticality::Required)
    }

    pub const fn optional_lazy() -> Self {
        Self::lazy(RuntimeCriticality::Optional)
    }

    pub const fn activation_mode(self) -> ActivationMode {
        self.activation_mode
    }

    pub const fn criticality(self) -> RuntimeCriticality {
        self.criticality
    }

    pub const fn restart_policy(self) -> RestartPolicy {
        self.restart
    }

    pub const fn activation_deadline(self) -> Option<Duration> {
        self.activation_deadline
    }

    pub const fn deactivation_deadline(self) -> Option<Duration> {
        self.deactivation_deadline
    }

    pub const fn with_restart_policy(mut self, restart: RestartPolicy) -> Self {
        self.restart = restart;
        self
    }

    pub const fn with_activation_deadline(mut self, deadline: Duration) -> Self {
        self.activation_deadline = Some(deadline);
        self
    }

    pub const fn with_deactivation_deadline(mut self, deadline: Duration) -> Self {
        self.deactivation_deadline = Some(deadline);
        self
    }
}

impl Default for RuntimeLaunchPolicy {
    fn default() -> Self {
        Self::new(ActivationMode::Eager, RuntimeCriticality::Required)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ActivationReason {
    Boot,
    DependencyDemand,
    Explicit,
    Restart,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeFailureClass {
    PluginError,
    Panic,
    Cancelled,
    DeadlineExceeded,
    Hung,
    StartupBudgetExceeded,
    StaleCompletion,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LifecycleOperationId(u64);

impl LifecycleOperationId {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LifecycleOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "lifecycle-operation-{}", self.0)
    }
}

/// Cooperative cancellation signal passed to lifecycle code.
#[derive(Clone, Debug, Default)]
pub struct LifecycleCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl LifecycleCancellationToken {
    pub fn cancel(&self) -> bool {
        !self.cancelled.swap(true, Ordering::SeqCst)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug)]
pub struct LifecycleOperation {
    operation_id: LifecycleOperationId,
    runtime_id: RuntimeId,
    cancellation: LifecycleCancellationToken,
    deadline: Option<Duration>,
}

impl LifecycleOperation {
    pub(crate) fn new(
        operation_id: LifecycleOperationId,
        runtime_id: RuntimeId,
        cancellation: LifecycleCancellationToken,
        deadline: Option<Duration>,
    ) -> Self {
        Self {
            operation_id,
            runtime_id,
            cancellation,
            deadline,
        }
    }

    pub const fn id(&self) -> LifecycleOperationId {
        self.operation_id
    }

    pub const fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    pub fn cancellation(&self) -> LifecycleCancellationToken {
        self.cancellation.clone()
    }

    pub fn cancel(&self) -> bool {
        self.cancellation.cancel()
    }

    pub const fn deadline(&self) -> Option<Duration> {
        self.deadline
    }
}

/// Read-only lifecycle metadata available to deactivation callbacks.
#[derive(Clone, Debug)]
pub struct LifecycleContext {
    runtime_id: RuntimeId,
    cancellation: LifecycleCancellationToken,
    deadline: Option<Duration>,
}

impl LifecycleContext {
    pub(crate) fn new(
        runtime_id: RuntimeId,
        cancellation: LifecycleCancellationToken,
        deadline: Option<Duration>,
    ) -> Self {
        Self {
            runtime_id,
            cancellation,
            deadline,
        }
    }

    pub const fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    pub fn cancellation(&self) -> LifecycleCancellationToken {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub const fn deadline(&self) -> Option<Duration> {
        self.deadline
    }
}

/// Limits applied to one eager/dependency-demand reconciliation pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupBudget {
    max_simultaneous_activations: usize,
    per_runtime_activation_deadline: Option<Duration>,
    overall_boot_deadline: Option<Duration>,
}

impl StartupBudget {
    pub const fn unlimited() -> Self {
        Self {
            max_simultaneous_activations: usize::MAX,
            per_runtime_activation_deadline: None,
            overall_boot_deadline: None,
        }
    }

    pub const fn new(
        max_simultaneous_activations: usize,
        per_runtime_activation_deadline: Option<Duration>,
        overall_boot_deadline: Option<Duration>,
    ) -> Self {
        Self {
            max_simultaneous_activations,
            per_runtime_activation_deadline,
            overall_boot_deadline,
        }
    }

    pub const fn max_simultaneous_activations(self) -> usize {
        self.max_simultaneous_activations
    }

    pub const fn per_runtime_activation_deadline(self) -> Option<Duration> {
        self.per_runtime_activation_deadline
    }

    pub const fn overall_boot_deadline(self) -> Option<Duration> {
        self.overall_boot_deadline
    }

    pub const fn with_max_simultaneous_activations(mut self, maximum: usize) -> Self {
        self.max_simultaneous_activations = maximum;
        self
    }

    pub const fn with_per_runtime_activation_deadline(mut self, deadline: Duration) -> Self {
        self.per_runtime_activation_deadline = Some(deadline);
        self
    }

    pub const fn with_overall_boot_deadline(mut self, deadline: Duration) -> Self {
        self.overall_boot_deadline = Some(deadline);
        self
    }
}

impl Default for StartupBudget {
    fn default() -> Self {
        Self::unlimited()
    }
}
