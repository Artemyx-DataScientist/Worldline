use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{CapabilityId, EventId, InvocationId, OperationId, RuntimeId};

/// Host default keeps ordinary calls bounded.  A host that explicitly allows
/// an unbounded deadline must opt into the no-deadline builder.
pub const DEFAULT_RPC_DEADLINE: Duration = Duration::from_secs(30);

/// Opaque logical identity of one caller intent.  A retry may reuse this
/// identity while receiving a fresh [`InvocationId`] for every attempt.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RpcRequestId(String);

impl RpcRequestId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for RpcRequestId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for RpcRequestId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for RpcRequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque correlation identity shared by related RPC and event activity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CorrelationId(String);

impl CorrelationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CorrelationId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for CorrelationId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for CorrelationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Immediate cause of an RPC or event.  This is observability metadata only;
/// it is never an authority handle or an object lookup capability.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CausationRef {
    Invocation(InvocationId),
    Event(EventId),
}

/// Trusted correlation/causation context carried across kernel boundaries.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraceContext {
    correlation_id: CorrelationId,
    causation: Option<CausationRef>,
}

impl TraceContext {
    pub fn new(correlation_id: impl Into<CorrelationId>) -> Self {
        Self {
            correlation_id: correlation_id.into(),
            causation: None,
        }
    }

    pub fn with_causation(mut self, causation: CausationRef) -> Self {
        self.causation = Some(causation);
        self
    }

    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    pub fn causation(&self) -> Option<&CausationRef> {
        self.causation.as_ref()
    }
}

/// Cooperative cancellation token for one RPC request.
#[derive(Clone, Debug, Default)]
pub struct RpcCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl PartialEq for RpcCancellationToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}

impl Eq for RpcCancellationToken {}

impl RpcCancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true only for the transition that changed the state.
    pub fn cancel(&self) -> bool {
        !self.cancelled.swap(true, Ordering::SeqCst)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Contract-owned retry safety classification.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RpcRetryClass {
    #[default]
    NeverRetry,
    Safe,
    Idempotent,
}

impl RpcRetryClass {
    pub const fn rank(self) -> u8 {
        match self {
            Self::NeverRetry => 0,
            Self::Safe => 1,
            Self::Idempotent => 2,
        }
    }

    pub const fn is_at_least(self, declared: Self) -> bool {
        self.rank() >= declared.rank()
    }
}

impl fmt::Display for RpcRetryClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NeverRetry => "NeverRetry",
            Self::Safe => "Safe",
            Self::Idempotent => "Idempotent",
        })
    }
}

/// Provider-declared operation contract.  Callers may request less retry
/// power than this contract, but can never escalate it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcOperationContract {
    operation: OperationId,
    retry_class: RpcRetryClass,
    idempotency_key_required: bool,
}

impl RpcOperationContract {
    pub fn new(operation: impl Into<OperationId>, retry_class: RpcRetryClass) -> Self {
        Self {
            operation: operation.into(),
            retry_class,
            idempotency_key_required: matches!(retry_class, RpcRetryClass::Idempotent),
        }
    }

    pub fn never_retry(operation: impl Into<OperationId>) -> Self {
        Self::new(operation, RpcRetryClass::NeverRetry)
    }

    pub fn safe(operation: impl Into<OperationId>) -> Self {
        Self::new(operation, RpcRetryClass::Safe)
    }

    pub fn idempotent(operation: impl Into<OperationId>) -> Self {
        Self::new(operation, RpcRetryClass::Idempotent)
    }

    pub fn with_idempotency_key_required(mut self, required: bool) -> Self {
        self.idempotency_key_required = required;
        self
    }

    pub fn operation(&self) -> &OperationId {
        &self.operation
    }

    pub const fn retry_class(&self) -> RpcRetryClass {
        self.retry_class
    }

    pub const fn idempotency_key_required(&self) -> bool {
        self.idempotency_key_required
    }
}

/// Per-call options.  The handle still supplies the trusted caller and
/// capability identity; these options only describe request behavior.
#[derive(Clone, Debug)]
pub struct RpcCallOptions {
    request_id: Option<RpcRequestId>,
    deadline: Option<Duration>,
    cancellation: RpcCancellationToken,
    retry_class: RpcRetryClass,
    retry: bool,
    idempotency_key: Option<String>,
    trace_context: Option<TraceContext>,
}

impl Default for RpcCallOptions {
    fn default() -> Self {
        Self {
            request_id: None,
            deadline: Some(DEFAULT_RPC_DEADLINE),
            cancellation: RpcCancellationToken::new(),
            retry_class: RpcRetryClass::NeverRetry,
            retry: false,
            idempotency_key: None,
            trace_context: None,
        }
    }
}

impl RpcCallOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_request_id(mut self, request_id: impl Into<RpcRequestId>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn with_no_deadline(mut self) -> Self {
        self.deadline = None;
        self
    }

    pub fn with_cancellation(mut self, cancellation: RpcCancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn with_retry_classification(mut self, retry_class: RpcRetryClass) -> Self {
        self.retry_class = retry_class;
        self
    }

    pub fn with_retry(mut self) -> Self {
        self.retry = true;
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = Some(trace_context);
        self
    }

    pub fn request_id(&self) -> Option<&RpcRequestId> {
        self.request_id.as_ref()
    }

    pub const fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    pub fn cancellation(&self) -> RpcCancellationToken {
        self.cancellation.clone()
    }

    pub const fn retry_classification(&self) -> RpcRetryClass {
        self.retry_class
    }

    pub const fn is_retry(&self) -> bool {
        self.retry
    }

    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    pub fn trace_context(&self) -> Option<&TraceContext> {
        self.trace_context.as_ref()
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn into_parts(
        self,
    ) -> (
        Option<RpcRequestId>,
        Option<Duration>,
        RpcCancellationToken,
        RpcRetryClass,
        bool,
        Option<String>,
        Option<TraceContext>,
    ) {
        (
            self.request_id,
            self.deadline,
            self.cancellation,
            self.retry_class,
            self.retry,
            self.idempotency_key,
            self.trace_context,
        )
    }
}

/// Bounded concurrency configuration for one provider runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderLimits {
    max_in_flight: usize,
    queue_capacity: usize,
}

impl ProviderLimits {
    pub const fn new(max_in_flight: usize, queue_capacity: usize) -> Self {
        Self {
            max_in_flight,
            queue_capacity,
        }
    }

    pub const fn max_in_flight(self) -> usize {
        self.max_in_flight
    }

    pub const fn queue_capacity(self) -> usize {
        self.queue_capacity
    }

    pub const fn is_valid(self) -> bool {
        self.max_in_flight > 0
    }
}

impl Default for ProviderLimits {
    fn default() -> Self {
        Self::new(8, 32)
    }
}

/// Stable class used by trajectory/control-plane observations.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RpcOutcomeClass {
    Success,
    AuthorizationDenied,
    NoCompatibleProvider,
    ProviderBusy,
    QueueFull,
    DeadlineExceeded,
    Cancelled,
    ProviderReturnedError,
    ProviderCrashed,
    RuntimeUnavailable,
    InvalidRetryClassification,
    InvalidIdempotencyKey,
}

impl fmt::Display for RpcOutcomeClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Success => "Success",
            Self::AuthorizationDenied => "AuthorizationDenied",
            Self::NoCompatibleProvider => "NoCompatibleProvider",
            Self::ProviderBusy => "ProviderBusy",
            Self::QueueFull => "QueueFull",
            Self::DeadlineExceeded => "DeadlineExceeded",
            Self::Cancelled => "Cancelled",
            Self::ProviderReturnedError => "ProviderReturnedError",
            Self::ProviderCrashed => "ProviderCrashed",
            Self::RuntimeUnavailable => "RuntimeUnavailable",
            Self::InvalidRetryClassification => "InvalidRetryClassification",
            Self::InvalidIdempotencyKey => "InvalidIdempotencyKey",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FlowFailure {
    ProviderBusy,
    QueueFull,
    DeadlineExceeded,
    Cancelled,
    RuntimeUnavailable,
}

struct LimiterState {
    in_flight: usize,
    queued: usize,
}

struct ProviderLimiter {
    limits: ProviderLimits,
    state: Mutex<LimiterState>,
    changed: Condvar,
    closed: AtomicBool,
}

impl ProviderLimiter {
    fn new(limits: ProviderLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(LimiterState {
                in_flight: 0,
                queued: 0,
            }),
            changed: Condvar::new(),
            closed: AtomicBool::new(false),
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }

    fn acquire(
        self: &Arc<Self>,
        deadline: Option<Instant>,
        cancellation: &RpcCancellationToken,
    ) -> Result<ProviderPermit, FlowFailure> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.closed.load(Ordering::SeqCst) {
            return Err(FlowFailure::RuntimeUnavailable);
        }
        if state.in_flight < self.limits.max_in_flight && state.queued == 0 {
            state.in_flight += 1;
            return Ok(ProviderPermit {
                limiter: Some(Arc::clone(self)),
            });
        }
        if self.limits.queue_capacity == 0 {
            return Err(FlowFailure::ProviderBusy);
        }
        if state.queued >= self.limits.queue_capacity {
            return Err(FlowFailure::QueueFull);
        }
        state.queued += 1;
        loop {
            if self.closed.load(Ordering::SeqCst) {
                state.queued -= 1;
                self.changed.notify_all();
                return Err(FlowFailure::RuntimeUnavailable);
            }
            if cancellation.is_cancelled() {
                state.queued -= 1;
                self.changed.notify_all();
                return Err(FlowFailure::Cancelled);
            }
            if deadline.is_some_and(|limit| Instant::now() >= limit) {
                state.queued -= 1;
                self.changed.notify_all();
                return Err(FlowFailure::DeadlineExceeded);
            }
            if state.in_flight < self.limits.max_in_flight {
                state.in_flight += 1;
                state.queued -= 1;
                return Ok(ProviderPermit {
                    limiter: Some(Arc::clone(self)),
                });
            }
            let wait_for = deadline
                .map(|limit| limit.saturating_duration_since(Instant::now()))
                .map(|remaining| remaining.min(Duration::from_millis(10)))
                .unwrap_or(Duration::from_millis(10));
            let (next_state, _) = self
                .changed
                .wait_timeout(state, wait_for)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next_state;
        }
    }
}

pub(crate) struct ProviderPermit {
    limiter: Option<Arc<ProviderLimiter>>,
}

impl ProviderPermit {
    pub(crate) fn reentrant() -> Self {
        Self { limiter: None }
    }
}

impl Drop for ProviderPermit {
    fn drop(&mut self) {
        let Some(limiter) = self.limiter.as_ref() else {
            return;
        };
        let mut state = limiter
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.in_flight = state.in_flight.saturating_sub(1);
        limiter.changed.notify_all();
    }
}

/// Provider-runtime-local bounded flow-control registry.
#[derive(Default)]
pub(crate) struct ProviderFlowControl {
    state: Mutex<FlowRegistryState>,
}

#[derive(Default)]
struct FlowRegistryState {
    providers: BTreeMap<ProviderKey, Arc<ProviderLimiter>>,
    retired_runtimes: BTreeSet<RuntimeId>,
}

/// Flow control belongs to one concrete runtime publication, not to the
/// plugin definition or to the whole runtime. A runtime may publish multiple
/// capabilities with intentionally different concurrency/queue budgets.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct ProviderKey {
    runtime_id: RuntimeId,
    capability: CapabilityId,
}

impl ProviderFlowControl {
    pub(crate) fn register(
        &self,
        runtime_id: RuntimeId,
        capability: CapabilityId,
        limits: ProviderLimits,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.retired_runtimes.remove(&runtime_id);
        state.providers.insert(
            ProviderKey {
                runtime_id,
                capability,
            },
            Arc::new(ProviderLimiter::new(limits)),
        );
    }

    pub(crate) fn unregister(&self, runtime_id: RuntimeId) {
        let limiters = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.retired_runtimes.insert(runtime_id);
            let keys: Vec<ProviderKey> = state
                .providers
                .keys()
                .filter(|key| key.runtime_id == runtime_id)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| state.providers.remove(&key))
                .collect::<Vec<_>>()
        };
        for limiter in limiters {
            limiter.close();
        }
    }

    pub(crate) fn acquire(
        &self,
        runtime_id: RuntimeId,
        capability: &CapabilityId,
        deadline: Option<Instant>,
        cancellation: &RpcCancellationToken,
        reentrant: bool,
    ) -> Result<ProviderPermit, FlowFailure> {
        if reentrant {
            return Ok(ProviderPermit::reentrant());
        }
        let limiter = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.retired_runtimes.contains(&runtime_id) {
                return Err(FlowFailure::RuntimeUnavailable);
            }
            state
                .providers
                .entry(ProviderKey {
                    runtime_id,
                    capability: capability.clone(),
                })
                .or_insert_with(|| Arc::new(ProviderLimiter::new(ProviderLimits::default())))
                .clone()
        };
        limiter.acquire(deadline, cancellation)
    }

    pub(crate) fn is_saturated(&self, runtime_id: RuntimeId, capability: &CapabilityId) -> bool {
        let limiter = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .providers
            .get(&ProviderKey {
                runtime_id,
                capability: capability.clone(),
            })
            .cloned();
        limiter.is_some_and(|limiter| {
            let state = limiter
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.in_flight >= limiter.limits.max_in_flight || state.queued > 0
        })
    }

    pub(crate) fn queue_depth(&self, runtime_id: RuntimeId, capability: &CapabilityId) -> usize {
        let limiter = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .providers
            .get(&ProviderKey {
                runtime_id,
                capability: capability.clone(),
            })
            .cloned();
        limiter.map_or(0, |limiter| {
            limiter
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .queued
        })
    }
}
