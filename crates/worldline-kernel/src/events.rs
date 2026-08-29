use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    sync::{
        Arc, Condvar, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    CapabilityContract, CapabilityError, CapabilityId, CausationRef, CorrelationId,
    InterfaceVersion, InvocationId, OperationId, PrincipalId, ResourceId, RuntimeId, TraceContext,
    invocation::{CapabilityHandle, InvocationBroker},
    rpc::RpcOutcomeClass,
    security::SecurityStore,
    trajectory::{Trajectory, TrajectoryEventKind},
};

/// ABI-neutral event identity and concrete schema version.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventContract {
    namespace: String,
    name: String,
    version: InterfaceVersion,
}

impl EventContract {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: InterfaceVersion,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            version,
        }
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn version(&self) -> InterfaceVersion {
        self.version
    }

    pub const fn major(&self) -> u16 {
        self.version.major()
    }

    pub const fn minor(&self) -> u16 {
        self.version.minor()
    }

    pub fn is_well_formed(&self) -> bool {
        !self.namespace.trim().is_empty()
            && !self.name.trim().is_empty()
            && !self.namespace.contains('/')
            && !self.name.contains('/')
    }

    /// The generic grant contract used for both Publish and Subscribe.  The
    /// operation name keeps the two authority edges distinct.
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::new(
            "worldline.event",
            format!("{}/{}", self.namespace, self.name),
            self.version,
        )
    }

    pub fn is_compatible_with(&self, required: &Self) -> bool {
        self.namespace == required.namespace
            && self.name == required.name
            && self.major() == required.major()
            && self.minor() >= required.minor()
    }
}

impl fmt::Display for EventContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}@{}",
            self.namespace, self.name, self.version
        )
    }
}

/// Kernel-generated identity for one event publication.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventId(String);

impl EventId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque identity of one live subscription.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubscriptionId(String);

impl SubscriptionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Trusted metadata attached to the generic control-plane invocation-completed
/// event.  It contains no request or result payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationCompletedMetadata {
    request_id: crate::RpcRequestId,
    invocation_id: InvocationId,
    caller: PrincipalId,
    provider_runtime_id: RuntimeId,
    capability: CapabilityContract,
    operation: OperationId,
    outcome: RpcOutcomeClass,
}

impl InvocationCompletedMetadata {
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        request_id: crate::RpcRequestId,
        invocation_id: InvocationId,
        caller: PrincipalId,
        provider_runtime_id: RuntimeId,
        capability: CapabilityContract,
        operation: OperationId,
        outcome: RpcOutcomeClass,
    ) -> Self {
        Self {
            request_id,
            invocation_id,
            caller,
            provider_runtime_id,
            capability,
            operation,
            outcome,
        }
    }

    /// Reconstructs invocation metadata from trusted durable runtime fields.
    /// The reconstructed identity is metadata only and does not restore live
    /// runtime authority.
    #[allow(clippy::too_many_arguments)]
    pub fn from_storage_parts(
        request_id: crate::RpcRequestId,
        invocation_id: InvocationId,
        caller: PrincipalId,
        provider_runtime_incarnation: u64,
        provider_runtime_sequence: u64,
        capability: CapabilityContract,
        operation: OperationId,
        outcome: RpcOutcomeClass,
    ) -> Self {
        Self::from_parts(
            request_id,
            invocation_id,
            caller,
            RuntimeId::new(provider_runtime_incarnation, provider_runtime_sequence),
            capability,
            operation,
            outcome,
        )
    }

    pub(crate) fn new(
        request_id: crate::RpcRequestId,
        invocation_id: InvocationId,
        caller: PrincipalId,
        provider_runtime_id: RuntimeId,
        capability: CapabilityContract,
        operation: OperationId,
        outcome: RpcOutcomeClass,
    ) -> Self {
        Self {
            request_id,
            invocation_id,
            caller,
            provider_runtime_id,
            capability,
            operation,
            outcome,
        }
    }

    pub fn request_id(&self) -> &crate::RpcRequestId {
        &self.request_id
    }

    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub const fn provider_runtime_id(&self) -> RuntimeId {
        self.provider_runtime_id
    }

    pub fn capability(&self) -> &CapabilityContract {
        &self.capability
    }

    pub fn operation(&self) -> &OperationId {
        &self.operation
    }

    pub const fn outcome(&self) -> RpcOutcomeClass {
        self.outcome
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventEnvelope {
    event_id: EventId,
    contract: EventContract,
    producer: PrincipalId,
    producer_runtime_id: Option<RuntimeId>,
    sequence: u64,
    correlation_id: CorrelationId,
    causation: Option<CausationRef>,
    delivery_mode: DeliveryMode,
    payload: Vec<u8>,
    invocation_completed: Option<InvocationCompletedMetadata>,
}

impl EventEnvelope {
    /// Reconstructs an envelope from trusted journal storage. The constructor
    /// creates metadata, not authority; publication still goes through the
    /// kernel event transport.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        event_id: EventId,
        contract: EventContract,
        producer: PrincipalId,
        producer_runtime_id: Option<RuntimeId>,
        sequence: u64,
        correlation_id: CorrelationId,
        causation: Option<CausationRef>,
        delivery_mode: DeliveryMode,
        payload: Vec<u8>,
        invocation_completed: Option<InvocationCompletedMetadata>,
    ) -> Self {
        Self {
            event_id,
            contract,
            producer,
            producer_runtime_id,
            sequence,
            correlation_id,
            causation,
            delivery_mode,
            payload,
            invocation_completed,
        }
    }

    /// Reconstructs an envelope from trusted durable runtime fields. The
    /// reconstructed identity is metadata only and does not restore authority.
    #[allow(clippy::too_many_arguments)]
    pub fn from_storage_parts(
        event_id: EventId,
        contract: EventContract,
        producer: PrincipalId,
        producer_runtime_parts: Option<(u64, u64)>,
        sequence: u64,
        correlation_id: CorrelationId,
        causation: Option<CausationRef>,
        delivery_mode: DeliveryMode,
        payload: Vec<u8>,
        invocation_completed: Option<InvocationCompletedMetadata>,
    ) -> Self {
        Self::from_parts(
            event_id,
            contract,
            producer,
            producer_runtime_parts
                .map(|(incarnation, sequence)| RuntimeId::new(incarnation, sequence)),
            sequence,
            correlation_id,
            causation,
            delivery_mode,
            payload,
            invocation_completed,
        )
    }

    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    pub fn contract(&self) -> &EventContract {
        &self.contract
    }

    pub fn producer(&self) -> &PrincipalId {
        &self.producer
    }

    pub const fn producer_runtime_id(&self) -> Option<RuntimeId> {
        self.producer_runtime_id
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    pub fn causation(&self) -> Option<&CausationRef> {
        self.causation.as_ref()
    }

    pub const fn delivery_mode(&self) -> DeliveryMode {
        self.delivery_mode
    }

    /// Opaque bytes owned by the event contract.  Trusted envelope metadata
    /// is never reconstructed from these bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn invocation_completed(&self) -> Option<&InvocationCompletedMetadata> {
        self.invocation_completed.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DeliveryMode {
    #[default]
    Ephemeral,
    Durable,
}

impl fmt::Display for DeliveryMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ephemeral => "Ephemeral",
            Self::Durable => "Durable",
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct EventPublishOptions {
    delivery_mode: DeliveryMode,
    trace_context: Option<TraceContext>,
}

impl EventPublishOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_delivery_mode(mut self, delivery_mode: DeliveryMode) -> Self {
        self.delivery_mode = delivery_mode;
        self
    }

    pub fn durable(self) -> Self {
        self.with_delivery_mode(DeliveryMode::Durable)
    }

    pub fn ephemeral(self) -> Self {
        self.with_delivery_mode(DeliveryMode::Ephemeral)
    }

    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = Some(trace_context);
        self
    }

    pub const fn delivery_mode(&self) -> DeliveryMode {
        self.delivery_mode
    }

    pub fn trace_context(&self) -> Option<&TraceContext> {
        self.trace_context.as_ref()
    }

    pub(crate) fn into_parts(self) -> (DeliveryMode, Option<TraceContext>) {
        (self.delivery_mode, self.trace_context)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OverflowPolicy {
    RejectForSubscriber,
    DropNewest,
    DropOldest,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EventQoS {
    BestEffort,
    Observed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionOptions {
    capacity: usize,
    overflow: OverflowPolicy,
    qos: EventQoS,
}

impl SubscriptionOptions {
    pub const fn new(capacity: usize, overflow: OverflowPolicy) -> Self {
        Self {
            capacity,
            overflow,
            qos: EventQoS::BestEffort,
        }
    }

    pub const fn with_qos(mut self, qos: EventQoS) -> Self {
        self.qos = qos;
        self
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    pub const fn overflow_policy(&self) -> OverflowPolicy {
        self.overflow
    }

    pub const fn qos(&self) -> EventQoS {
        self.qos
    }
}

impl Default for SubscriptionOptions {
    fn default() -> Self {
        Self::new(16, OverflowPolicy::DropNewest)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishReport {
    event_id: EventId,
    delivery_mode: DeliveryMode,
    eligible_subscribers: usize,
    enqueued: usize,
    dropped: usize,
    backpressured: usize,
    durably_recorded: bool,
}

impl PublishReport {
    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    pub const fn delivery_mode(&self) -> DeliveryMode {
        self.delivery_mode
    }

    pub const fn eligible_subscribers(&self) -> usize {
        self.eligible_subscribers
    }

    pub const fn enqueued(&self) -> usize {
        self.enqueued
    }

    pub const fn dropped(&self) -> usize {
        self.dropped
    }

    pub const fn backpressured(&self) -> usize {
        self.backpressured
    }

    pub const fn durably_recorded(&self) -> bool {
        self.durably_recorded
    }
}

impl Default for PublishReport {
    fn default() -> Self {
        Self {
            event_id: EventId::new("event-unassigned"),
            delivery_mode: DeliveryMode::Ephemeral,
            eligible_subscribers: 0,
            enqueued: 0,
            dropped: 0,
            backpressured: 0,
            durably_recorded: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventCursor(usize);

impl EventCursor {
    pub const fn new(position: usize) -> Self {
        Self(position)
    }

    pub const fn position(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventJournalError {
    CapacityExceeded,
    Failure(String),
}

impl fmt::Display for EventJournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExceeded => formatter.write_str("event journal capacity exceeded"),
            Self::Failure(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for EventJournalError {}

/// Logical durable transport seam.  It deliberately exposes no filesystem,
/// database, cursor persistence, or crash-recovery concepts to plugins.
pub trait EventJournal: Send + Sync {
    fn append(&self, event: &EventEnvelope) -> Result<(), EventJournalError>;
    fn read_from(&self, cursor: EventCursor) -> Result<Vec<EventEnvelope>, EventJournalError>;
}

#[derive(Default)]
struct InMemoryEventJournalState {
    events: std::sync::Mutex<Vec<EventEnvelope>>,
}

/// Deterministic in-memory journal for contract and acceptance tests.  It is
/// intentionally not presented as crash-safe persistence.
#[derive(Default)]
pub struct InMemoryEventJournal {
    state: InMemoryEventJournalState,
}

impl InMemoryEventJournal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<EventEnvelope> {
        self.state
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl EventJournal for InMemoryEventJournal {
    fn append(&self, event: &EventEnvelope) -> Result<(), EventJournalError> {
        self.state
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event.clone());
        Ok(())
    }

    fn read_from(&self, cursor: EventCursor) -> Result<Vec<EventEnvelope>, EventJournalError> {
        Ok(self
            .state
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .skip(cursor.position())
            .cloned()
            .collect())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventError {
    UnknownEventContract,
    EventSchemaIncompatible,
    EventPublishDenied,
    EventSubscribeDenied,
    UnknownSubscription { subscription: SubscriptionId },
    SubscriptionClosed { subscription: SubscriptionId },
    MailboxFull { subscription: SubscriptionId },
    DurableDeliveryUnavailable,
    EventJournalFailure { message: String },
    InvalidMailboxCapacity,
    PrincipalUnavailable { principal: PrincipalId },
}

impl fmt::Display for EventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEventContract => formatter.write_str("unknown event contract"),
            Self::EventSchemaIncompatible => formatter.write_str("event schema is incompatible"),
            Self::EventPublishDenied => formatter.write_str("event publication denied"),
            Self::EventSubscribeDenied => formatter.write_str("event subscription denied"),
            Self::UnknownSubscription { subscription } => {
                write!(formatter, "subscription '{subscription}' is unknown")
            }
            Self::SubscriptionClosed { subscription } => {
                write!(formatter, "subscription '{subscription}' is closed")
            }
            Self::MailboxFull { subscription } => {
                write!(
                    formatter,
                    "mailbox for subscription '{subscription}' is full"
                )
            }
            Self::DurableDeliveryUnavailable => {
                formatter.write_str("durable event delivery is unavailable")
            }
            Self::EventJournalFailure { message } => {
                write!(formatter, "event journal failure: {message}")
            }
            Self::InvalidMailboxCapacity => formatter.write_str("mailbox capacity is invalid"),
            Self::PrincipalUnavailable { principal } => {
                write!(formatter, "principal '{principal}' is unavailable")
            }
        }
    }
}

impl std::error::Error for EventError {}

struct Mailbox {
    queue: VecDeque<EventEnvelope>,
    closed: bool,
    options: SubscriptionOptions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnqueueOutcome {
    Enqueued,
    Dropped,
    EnqueuedAfterDrop,
    Backpressured,
    Closed,
}

struct SubscriptionState {
    id: SubscriptionId,
    subscriber: PrincipalId,
    runtime_id: Option<RuntimeId>,
    contract: EventContract,
    mailbox: std::sync::Mutex<Mailbox>,
    changed: Condvar,
    closed: AtomicBool,
}

impl SubscriptionState {
    fn new(
        id: SubscriptionId,
        subscriber: PrincipalId,
        runtime_id: Option<RuntimeId>,
        contract: EventContract,
        options: SubscriptionOptions,
    ) -> Self {
        Self {
            id,
            subscriber,
            runtime_id,
            contract,
            mailbox: std::sync::Mutex::new(Mailbox {
                queue: VecDeque::new(),
                closed: false,
                options,
            }),
            changed: Condvar::new(),
            closed: AtomicBool::new(false),
        }
    }

    fn close(&self) -> bool {
        if self.closed.swap(true, Ordering::SeqCst) {
            return false;
        }
        let mut mailbox = self
            .mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        mailbox.closed = true;
        mailbox.queue.clear();
        self.changed.notify_all();
        true
    }

    fn enqueue(&self, event: EventEnvelope) -> EnqueueOutcome {
        let mut mailbox = self
            .mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if mailbox.closed {
            return EnqueueOutcome::Closed;
        }
        if mailbox.queue.len() < mailbox.options.capacity() {
            mailbox.queue.push_back(event);
            self.changed.notify_one();
            return EnqueueOutcome::Enqueued;
        }
        match mailbox.options.overflow_policy() {
            OverflowPolicy::RejectForSubscriber => EnqueueOutcome::Backpressured,
            OverflowPolicy::DropNewest => EnqueueOutcome::Dropped,
            OverflowPolicy::DropOldest if mailbox.options.capacity() == 0 => {
                EnqueueOutcome::Dropped
            }
            OverflowPolicy::DropOldest => {
                let _ = mailbox.queue.pop_front();
                mailbox.queue.push_back(event);
                self.changed.notify_one();
                EnqueueOutcome::EnqueuedAfterDrop
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct SequenceKey {
    producer: PrincipalId,
    runtime_id: Option<RuntimeId>,
    namespace: String,
    name: String,
    major: u16,
}

struct EventTransportState {
    security: Arc<SecurityStore>,
    trajectory: Trajectory,
    next_event: AtomicU64,
    next_subscription: AtomicU64,
    sequences: std::sync::Mutex<BTreeMap<SequenceKey, u64>>,
    subscriptions: RwLock<BTreeMap<SubscriptionId, Arc<SubscriptionState>>>,
    journal: RwLock<Option<Arc<dyn EventJournal>>>,
    broker: RwLock<Option<Weak<InvocationBroker>>>,
}

/// Generic kernel event transport.  It has no dependency on capability
/// provider resolution and never invokes subscriber code on the publish path.
#[derive(Clone)]
pub(crate) struct EventTransport {
    state: Arc<EventTransportState>,
}

impl EventTransport {
    pub(crate) fn new(security: Arc<SecurityStore>, trajectory: Trajectory) -> Self {
        Self {
            state: Arc::new(EventTransportState {
                security,
                trajectory,
                next_event: AtomicU64::new(0),
                next_subscription: AtomicU64::new(0),
                sequences: std::sync::Mutex::new(BTreeMap::new()),
                subscriptions: RwLock::new(BTreeMap::new()),
                journal: RwLock::new(None),
                broker: RwLock::new(None),
            }),
        }
    }

    pub(crate) fn attach_broker(&self, broker: Weak<InvocationBroker>) {
        *self
            .state
            .broker
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(broker);
    }

    pub(crate) fn set_journal(&self, journal: Option<Arc<dyn EventJournal>>) {
        *self
            .state
            .journal
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = journal;
    }

    pub(crate) fn publish(
        &self,
        producer: PrincipalId,
        producer_runtime_id: Option<RuntimeId>,
        contract: EventContract,
        payload: &[u8],
        options: EventPublishOptions,
        trusted: bool,
    ) -> Result<PublishReport, EventError> {
        self.publish_with_metadata(
            producer,
            producer_runtime_id,
            contract,
            payload,
            options,
            trusted,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_with_metadata(
        &self,
        producer: PrincipalId,
        producer_runtime_id: Option<RuntimeId>,
        contract: EventContract,
        payload: &[u8],
        options: EventPublishOptions,
        trusted: bool,
        invocation_completed: Option<InvocationCompletedMetadata>,
    ) -> Result<PublishReport, EventError> {
        if !contract.is_well_formed() {
            return Err(EventError::UnknownEventContract);
        }
        if !self.state.security.principal_exists(&producer) {
            return Err(EventError::PrincipalUnavailable {
                principal: producer,
            });
        }
        let publish_capability = contract.capability_id();
        if !trusted
            && self
                .state
                .security
                .authorize_event(&producer, &publish_capability, &OperationId::new("publish"))
                .is_err()
        {
            return Err(EventError::EventPublishDenied);
        }
        let (delivery_mode, trace_context) = options.into_parts();
        let trace_context = trace_context
            .unwrap_or_else(|| TraceContext::new(self.state.security.allocate_correlation()));
        let event_id = EventId::new(format!(
            "event-{}",
            self.state.next_event.fetch_add(1, Ordering::SeqCst) + 1
        ));
        let sequence_key = SequenceKey {
            producer: producer.clone(),
            runtime_id: producer_runtime_id,
            namespace: contract.namespace.clone(),
            name: contract.name.clone(),
            major: contract.major(),
        };
        let sequence = {
            let mut sequences = self
                .state
                .sequences
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let next = sequences.entry(sequence_key).or_insert(0);
            *next += 1;
            *next
        };
        let envelope = EventEnvelope {
            event_id: event_id.clone(),
            contract: contract.clone(),
            producer: producer.clone(),
            producer_runtime_id,
            sequence,
            correlation_id: trace_context.correlation_id().clone(),
            causation: trace_context.causation().cloned(),
            delivery_mode,
            payload: payload.to_vec(),
            invocation_completed,
        };

        let mut report = PublishReport {
            event_id: event_id.clone(),
            delivery_mode,
            ..PublishReport::default()
        };
        if delivery_mode == DeliveryMode::Durable {
            let journal = self
                .state
                .journal
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
                .ok_or(EventError::DurableDeliveryUnavailable)?;
            journal
                .append(&envelope)
                .map_err(|error| EventError::EventJournalFailure {
                    message: error.to_string(),
                })?;
            report.durably_recorded = true;
        }

        let subscriptions: Vec<Arc<SubscriptionState>> = self
            .state
            .subscriptions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect();
        for subscription in subscriptions {
            if subscription.closed.load(Ordering::SeqCst)
                || !contract.is_compatible_with(&subscription.contract)
                || self
                    .state
                    .security
                    .authorize_event(
                        &subscription.subscriber,
                        &subscription.contract.capability_id(),
                        &OperationId::new("subscribe"),
                    )
                    .is_err()
            {
                continue;
            }
            report.eligible_subscribers += 1;
            match subscription.enqueue(envelope.clone()) {
                EnqueueOutcome::Enqueued => {
                    report.enqueued += 1;
                    self.state.trajectory.push_security(
                        TrajectoryEventKind::EventDeliveryEnqueued {
                            subscription: subscription.id.clone(),
                            event_id: event_id.clone(),
                        },
                    );
                }
                EnqueueOutcome::EnqueuedAfterDrop => {
                    report.enqueued += 1;
                    report.dropped += 1;
                    self.state
                        .trajectory
                        .push_security(TrajectoryEventKind::EventDropped {
                            subscription: subscription.id.clone(),
                            event_id: event_id.clone(),
                        });
                }
                EnqueueOutcome::Dropped => {
                    report.dropped += 1;
                    self.state
                        .trajectory
                        .push_security(TrajectoryEventKind::EventDropped {
                            subscription: subscription.id.clone(),
                            event_id: event_id.clone(),
                        });
                }
                EnqueueOutcome::Backpressured => {
                    report.backpressured += 1;
                    self.state
                        .trajectory
                        .push_security(TrajectoryEventKind::EventBackpressured {
                            subscription: subscription.id.clone(),
                            event_id: event_id.clone(),
                        });
                }
                EnqueueOutcome::Closed => {}
            }
        }
        self.state
            .trajectory
            .push_security(TrajectoryEventKind::EventPublished {
                event_id,
                contract,
                producer,
                producer_runtime_id,
                sequence,
                correlation_id: envelope.correlation_id.clone(),
                causation: envelope.causation.clone(),
                delivery_mode,
            });
        Ok(report)
    }

    pub(crate) fn subscribe(
        &self,
        subscriber: PrincipalId,
        runtime_id: Option<RuntimeId>,
        contract: EventContract,
        options: SubscriptionOptions,
    ) -> Result<SubscriptionHandle, EventError> {
        if !contract.is_well_formed() {
            return Err(EventError::UnknownEventContract);
        }
        if options.capacity() == 0 {
            return Err(EventError::InvalidMailboxCapacity);
        }
        if !self.state.security.principal_exists(&subscriber) {
            return Err(EventError::PrincipalUnavailable {
                principal: subscriber,
            });
        }
        if self
            .state
            .security
            .authorize_event(
                &subscriber,
                &contract.capability_id(),
                &OperationId::new("subscribe"),
            )
            .is_err()
        {
            return Err(EventError::EventSubscribeDenied);
        }
        let id = SubscriptionId::new(format!(
            "subscription-{}",
            self.state.next_subscription.fetch_add(1, Ordering::SeqCst) + 1
        ));
        let state = Arc::new(SubscriptionState::new(
            id.clone(),
            subscriber.clone(),
            runtime_id,
            contract.clone(),
            options,
        ));
        self.state
            .subscriptions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.clone(), Arc::clone(&state));
        self.state
            .trajectory
            .push_security(TrajectoryEventKind::SubscriptionCreated {
                subscription: id.clone(),
                subscriber: subscriber.clone(),
                runtime_id,
                contract,
            });
        Ok(SubscriptionHandle {
            id,
            subscriber,
            runtime_id,
            state,
            transport: self.clone(),
        })
    }

    pub(crate) fn close_subscription(&self, id: &SubscriptionId) -> bool {
        let state = self
            .state
            .subscriptions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
        let Some(state) = state else {
            return false;
        };
        let closed = state.close();
        if closed {
            self.state
                .trajectory
                .push_security(TrajectoryEventKind::SubscriptionClosed {
                    subscription: id.clone(),
                    subscriber: state.subscriber.clone(),
                    runtime_id: state.runtime_id,
                });
        }
        closed
    }

    pub(crate) fn close_runtime(&self, runtime_id: RuntimeId) {
        let ids: Vec<SubscriptionId> = self
            .state
            .subscriptions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .filter(|subscription| subscription.runtime_id == Some(runtime_id))
            .map(|subscription| subscription.id.clone())
            .collect();
        for id in ids {
            let _ = self.close_subscription(&id);
        }
    }

    pub(crate) fn publish_invocation_completed(
        &self,
        provider: PrincipalId,
        runtime_id: RuntimeId,
        metadata: InvocationCompletedMetadata,
        options: EventPublishOptions,
    ) -> Result<PublishReport, EventError> {
        self.publish_with_metadata(
            provider,
            Some(runtime_id),
            invocation_completed_event_contract(),
            &[],
            options,
            true,
            Some(metadata),
        )
    }
}

/// Pull-oriented subscription handle.  It contains subscriber identity and a
/// mailbox, never producer authority or a provider service object.
pub struct SubscriptionHandle {
    id: SubscriptionId,
    subscriber: PrincipalId,
    runtime_id: Option<RuntimeId>,
    state: Arc<SubscriptionState>,
    transport: EventTransport,
}

impl SubscriptionHandle {
    pub fn id(&self) -> &SubscriptionId {
        &self.id
    }

    pub fn subscriber(&self) -> &PrincipalId {
        &self.subscriber
    }

    pub const fn runtime_id(&self) -> Option<RuntimeId> {
        self.runtime_id
    }

    pub fn try_recv(&self) -> Result<Option<EventEnvelope>, EventError> {
        self.ensure_authorized()?;
        let mut mailbox = self
            .state
            .mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if mailbox.closed {
            return Err(EventError::SubscriptionClosed {
                subscription: self.id.clone(),
            });
        }
        Ok(mailbox.queue.pop_front())
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<EventEnvelope>, EventError> {
        let deadline = Instant::now() + timeout;
        let mut mailbox = self
            .state
            .mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            drop(mailbox);
            self.ensure_authorized()?;
            mailbox = self
                .state
                .mailbox
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if mailbox.closed {
                return Err(EventError::SubscriptionClosed {
                    subscription: self.id.clone(),
                });
            }
            if let Some(event) = mailbox.queue.pop_front() {
                return Ok(Some(event));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let (next_mailbox, _) = self
                .state
                .changed
                .wait_timeout(mailbox, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox = next_mailbox;
        }
    }

    pub fn close(&self) -> bool {
        self.transport.close_subscription(&self.id)
    }

    /// Builds a context for an event already received by this subscriber.
    /// Follow-up calls made through it use the subscriber's own authority.
    pub fn context(&self, event: EventEnvelope) -> EventContext {
        let broker = self
            .transport
            .state
            .broker
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned()
            .unwrap_or_default();
        EventContext {
            event,
            subscriber: self.subscriber.clone(),
            runtime_id: self.runtime_id,
            broker,
        }
    }

    fn ensure_authorized(&self) -> Result<(), EventError> {
        if self.state.closed.load(Ordering::SeqCst) {
            return Err(EventError::SubscriptionClosed {
                subscription: self.id.clone(),
            });
        }
        if self
            .transport
            .state
            .security
            .authorize_event(
                &self.subscriber,
                &self.state.contract.capability_id(),
                &OperationId::new("subscribe"),
            )
            .is_err()
        {
            let _ = self.transport.close_subscription(&self.id);
            return Err(EventError::EventSubscribeDenied);
        }
        Ok(())
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        let _ = self.transport.close_subscription(&self.id);
    }
}

impl fmt::Debug for SubscriptionHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionHandle")
            .field("id", &self.id)
            .field("subscriber", &self.subscriber)
            .field("runtime_id", &self.runtime_id)
            .finish_non_exhaustive()
    }
}

/// Event-side context used to derive a new trace and optional follow-up RPC.
pub struct EventContext {
    event: EventEnvelope,
    subscriber: PrincipalId,
    runtime_id: Option<RuntimeId>,
    broker: Weak<InvocationBroker>,
}

impl EventContext {
    pub fn event(&self) -> &EventEnvelope {
        &self.event
    }

    pub fn subscriber(&self) -> &PrincipalId {
        &self.subscriber
    }

    pub const fn runtime_id(&self) -> Option<RuntimeId> {
        self.runtime_id
    }

    pub fn trace_context(&self) -> TraceContext {
        TraceContext::new(self.event.correlation_id().clone())
            .with_causation(CausationRef::Event(self.event.event_id().clone()))
    }

    pub fn capability(
        &self,
        capability: impl Into<CapabilityId>,
    ) -> Result<CapabilityHandle, CapabilityError> {
        let capability = capability.into();
        let broker = self
            .broker
            .upgrade()
            .ok_or_else(|| CapabilityError::InvocationFailed {
                capability: capability.clone(),
                message: "invocation broker is unavailable".to_owned(),
            })?;
        Ok(CapabilityHandle::new(
            capability,
            self.subscriber.clone(),
            broker,
        ))
    }

    pub fn invoke(
        &self,
        capability: impl Into<CapabilityId>,
        operation: impl Into<OperationId>,
        resource: impl Into<ResourceId>,
        payload: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        let capability = capability.into();
        let broker = self
            .broker
            .upgrade()
            .ok_or_else(|| CapabilityError::InvocationFailed {
                capability: capability.clone(),
                message: "invocation broker is unavailable".to_owned(),
            })?;
        broker.invoke(
            crate::InvocationRequest::new(
                self.subscriber.clone(),
                capability,
                operation,
                resource,
                payload,
            )
            .with_trace_context(self.trace_context()),
        )
    }
}

pub fn invocation_completed_event_contract() -> EventContract {
    EventContract::new(
        "worldline.control",
        "invocation-completed",
        InterfaceVersion::new(1, 0),
    )
}
