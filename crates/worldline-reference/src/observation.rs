use std::{
    collections::BTreeMap,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex, RwLock},
};

use worldline_kernel::{InvocationId, PrincipalId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObservationId(u64);

impl ObservationId {
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ObservationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "observation-{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubscriberId(u64);

impl SubscriberId {
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SubscriberId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "subscriber-{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    id: ObservationId,
    producer: PrincipalId,
    topic: String,
    payload: Vec<u8>,
    causation: Option<InvocationId>,
    correlation: Option<String>,
}

impl Observation {
    pub fn id(&self) -> ObservationId {
        self.id
    }

    pub fn producer(&self) -> &PrincipalId {
        &self.producer
    }

    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn causation(&self) -> Option<&InvocationId> {
        self.causation.as_ref()
    }

    pub fn correlation(&self) -> Option<&str> {
        self.correlation.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationDraft {
    producer: PrincipalId,
    topic: String,
    payload: Vec<u8>,
    causation: Option<InvocationId>,
    correlation: Option<String>,
}

impl ObservationDraft {
    pub fn new(
        producer: impl Into<PrincipalId>,
        topic: impl Into<String>,
        payload: impl AsRef<[u8]>,
    ) -> Self {
        Self {
            producer: producer.into(),
            topic: topic.into(),
            payload: payload.as_ref().to_vec(),
            causation: None,
            correlation: None,
        }
    }

    pub fn with_causation(mut self, causation: InvocationId) -> Self {
        self.causation = Some(causation);
        self
    }

    pub fn with_correlation(mut self, correlation: impl Into<String>) -> Self {
        self.correlation = Some(correlation.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationFailure {
    subscriber: SubscriberId,
    message: String,
}

impl ObservationFailure {
    pub fn subscriber(&self) -> SubscriberId {
        self.subscriber
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObservationDelivery {
    delivered: usize,
    failures: Vec<ObservationFailure>,
}

impl ObservationDelivery {
    pub const fn delivered(&self) -> usize {
        self.delivered
    }

    pub fn failures(&self) -> &[ObservationFailure] {
        &self.failures
    }

    pub const fn is_successful(&self) -> bool {
        self.failures.is_empty()
    }
}

type Subscriber = Arc<dyn Fn(&Observation) -> Result<(), String> + Send + Sync>;

#[derive(Default)]
struct ObservationState {
    subscribers: RwLock<BTreeMap<SubscriberId, Subscriber>>,
    history: Mutex<Vec<Observation>>,
    next_observation: AtomicU64,
    next_subscriber: AtomicU64,
}

/// Minimal host-local observation plane for the proving slice.
///
/// It intentionally has no provider resolution or command path. Publishing a
/// fact records it and independently offers it to zero or more subscribers;
/// subscriber errors are collected and never change the producer's RPC result.
#[derive(Clone, Default)]
pub struct ObservationBus {
    state: Arc<ObservationState>,
}

impl ObservationBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe<F>(&self, subscriber: F) -> SubscriberId
    where
        F: Fn(&Observation) -> Result<(), String> + Send + Sync + 'static,
    {
        let id = SubscriberId(self.state.next_subscriber.fetch_add(1, Ordering::SeqCst) + 1);
        self.state
            .subscribers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, Arc::new(subscriber));
        id
    }

    pub fn unsubscribe(&self, subscriber: SubscriberId) -> bool {
        self.state
            .subscribers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&subscriber)
            .is_some()
    }

    pub fn subscriber_count(&self) -> usize {
        self.state
            .subscribers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn publish(&self, draft: ObservationDraft) -> ObservationDelivery {
        let id = ObservationId(self.state.next_observation.fetch_add(1, Ordering::SeqCst) + 1);
        let observation = Observation {
            id,
            producer: draft.producer,
            topic: draft.topic,
            payload: draft.payload,
            causation: draft.causation,
            correlation: draft.correlation,
        };
        self.state
            .history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(observation.clone());

        let subscribers: Vec<(SubscriberId, Subscriber)> = self
            .state
            .subscribers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(id, subscriber)| (*id, Arc::clone(subscriber)))
            .collect();
        let mut delivery = ObservationDelivery::default();
        for (subscriber, callback) in subscribers {
            let result = catch_unwind(AssertUnwindSafe(|| callback(&observation)));
            match result {
                Ok(Ok(())) => delivery.delivered += 1,
                Ok(Err(message)) => delivery.failures.push(ObservationFailure {
                    subscriber,
                    message,
                }),
                Err(payload) => delivery.failures.push(ObservationFailure {
                    subscriber,
                    message: panic_message(payload.as_ref()),
                }),
            }
        }
        delivery
    }

    pub fn history(&self) -> Vec<Observation> {
        self.state
            .history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "subscriber panicked".to_owned()
    }
}
