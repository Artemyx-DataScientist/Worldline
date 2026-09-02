//! Engine-neutral bounded request-policy broker.
//!
//! This module owns registration, authority, lifecycle and resource limits for
//! request-policy evaluators. It deliberately knows nothing about engine or
//! rule-list syntax. Failure semantics are selected by each registration; the
//! generic broker does not impose a global fail-open policy.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use worldline_browser_contract::authority::BrowserAuthoritySet;
use worldline_browser_contract::identity::{BrowserContextId, PageId};
use worldline_browser_contract::request_policy::{
    CONTRACT_REQUEST_POLICY, OP_REQUEST_POLICY_DECIDE, OP_REQUEST_POLICY_OBSERVE,
    OP_REQUEST_POLICY_REGISTER, OP_REQUEST_POLICY_UNREGISTER, RequestPolicyAction,
    RequestPolicyFailureMode, RequestPolicyObservation, RequestPolicyOutcome,
    RequestPolicyRegistration, RequestPolicyRequest, RequestPolicyResult,
};

/// Bounded resource limits for one request-policy broker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestPolicyBrokerLimits {
    pub max_registrations: usize,
    pub max_total_in_flight: usize,
    pub max_observations: usize,
}

impl Default for RequestPolicyBrokerLimits {
    fn default() -> Self {
        Self {
            max_registrations: 64,
            max_total_in_flight: 64,
            max_observations: 1024,
        }
    }
}

/// Identity and authority supplied by the caller of the broker. IDs are only
/// scope selectors; the authority set is independently required for every
/// operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestPolicyCaller {
    authorities: BrowserAuthoritySet,
    context_id: BrowserContextId,
    page_id: Option<PageId>,
}

impl RequestPolicyCaller {
    pub fn new(
        authorities: BrowserAuthoritySet,
        context_id: BrowserContextId,
        page_id: Option<PageId>,
    ) -> Self {
        Self {
            authorities,
            context_id,
            page_id,
        }
    }

    pub fn context_id(&self) -> &BrowserContextId {
        &self.context_id
    }

    pub fn page_id(&self) -> Option<&PageId> {
        self.page_id.as_ref()
    }

    pub fn authorities(&self) -> &BrowserAuthoritySet {
        &self.authorities
    }
}

/// Cooperative cancellation signal supplied to evaluators. The broker also
/// enforces a hard wait deadline; evaluators should check this signal in any
/// bounded loop.
#[derive(Clone, Debug)]
pub struct RequestPolicyCancellation {
    cancelled: Arc<AtomicBool>,
}

impl RequestPolicyCancellation {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// Failure returned by an evaluator. The broker maps it through the
/// registration's declared failure mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestPolicyEvaluatorError {
    Rejected(String),
    Unavailable(String),
    Cancelled,
}

impl fmt::Display for RequestPolicyEvaluatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(reason) => write!(formatter, "policy rejected request: {reason}"),
            Self::Unavailable(reason) => write!(formatter, "policy unavailable: {reason}"),
            Self::Cancelled => formatter.write_str("policy evaluation cancelled"),
        }
    }
}

impl std::error::Error for RequestPolicyEvaluatorError {}

/// Engine-neutral policy implementation boundary.
pub trait RequestPolicyEvaluator: Send + Sync {
    fn provider_id(&self) -> &str;

    fn decide(
        &self,
        request: &RequestPolicyRequest,
        cancellation: &RequestPolicyCancellation,
    ) -> Result<RequestPolicyResult, RequestPolicyEvaluatorError>;
}

/// Bounded failures reported by a physical provider-process request/result
/// transport. The transport does not select Allow or Block; the registration
/// or policy profile that owns the request maps these failures through its
/// declared failure mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestPolicyTransportError {
    CapacityExceeded,
    DeadlineExceeded { deadline_ms: u64 },
    Cancelled,
    PayloadTooLarge { limit: usize, actual: usize },
    Unavailable(String),
    ProtocolViolation(String),
    TransportClosed,
}

impl fmt::Display for RequestPolicyTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExceeded => {
                formatter.write_str("request-policy transport capacity exceeded")
            }
            Self::DeadlineExceeded { deadline_ms } => write!(
                formatter,
                "request-policy transport exceeded {deadline_ms} ms deadline"
            ),
            Self::Cancelled => formatter.write_str("request-policy transport request cancelled"),
            Self::PayloadTooLarge { limit, actual } => write!(
                formatter,
                "request-policy transport frame of {actual} bytes exceeds the {limit} byte limit"
            ),
            Self::Unavailable(reason) => {
                write!(formatter, "request-policy transport unavailable: {reason}")
            }
            Self::ProtocolViolation(reason) => {
                write!(
                    formatter,
                    "request-policy transport protocol violation: {reason}"
                )
            }
            Self::TransportClosed => formatter.write_str("request-policy transport closed"),
        }
    }
}

impl std::error::Error for RequestPolicyTransportError {}

/// Engine-neutral hook used by an engine adapter to request one bounded
/// policy decision from its owning provider process. Implementations must not
/// expose engine objects or hold an engine/core lock while waiting.
pub trait RequestPolicyTransport: Send + Sync {
    fn decide(
        &self,
        request: RequestPolicyRequest,
    ) -> Result<RequestPolicyResult, RequestPolicyTransportError>;
}

/// Broker admission and lifecycle failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestPolicyBrokerError {
    PermissionDenied(&'static str),
    Invalid(String),
    AlreadyRegistered(String),
    NotFound(String),
    ScopeDenied,
    CapacityExceeded,
}

impl fmt::Display for RequestPolicyBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied(operation) => {
                write!(formatter, "request-policy authority denied for {operation}")
            }
            Self::Invalid(reason) => write!(formatter, "invalid request-policy value: {reason}"),
            Self::AlreadyRegistered(id) => {
                write!(
                    formatter,
                    "request-policy registration already exists: {id}"
                )
            }
            Self::NotFound(id) => write!(formatter, "request-policy registration not found: {id}"),
            Self::ScopeDenied => formatter.write_str("request-policy scope denied"),
            Self::CapacityExceeded => {
                formatter.write_str("request-policy broker capacity exceeded")
            }
        }
    }
}

impl std::error::Error for RequestPolicyBrokerError {}

struct RegistrationState {
    registration: RequestPolicyRegistration,
    evaluator: Arc<dyn RequestPolicyEvaluator>,
    in_flight: usize,
}

struct ActiveDecision {
    registration_id: String,
    cancellation: RequestPolicyCancellation,
}

struct BrokerState {
    registrations: BTreeMap<String, RegistrationState>,
    active: BTreeMap<u64, ActiveDecision>,
    observations: VecDeque<RequestPolicyObservation>,
    next_decision_id: u64,
    total_in_flight: usize,
}

/// Cloneable handle to one bounded request-policy broker.
#[derive(Clone)]
pub struct RequestPolicyBroker {
    state: Arc<Mutex<BrokerState>>,
    limits: RequestPolicyBrokerLimits,
}

impl fmt::Debug for RequestPolicyBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestPolicyBroker")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Default for RequestPolicyBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestPolicyBroker {
    pub fn new() -> Self {
        Self::with_limits(RequestPolicyBrokerLimits::default())
    }

    pub fn with_limits(limits: RequestPolicyBrokerLimits) -> Self {
        Self {
            state: Arc::new(Mutex::new(BrokerState {
                registrations: BTreeMap::new(),
                active: BTreeMap::new(),
                observations: VecDeque::new(),
                next_decision_id: 1,
                total_in_flight: 0,
            })),
            limits,
        }
    }

    pub fn limits(&self) -> &RequestPolicyBrokerLimits {
        &self.limits
    }

    /// Registers an evaluator for an exact context or context/page scope.
    pub fn register(
        &self,
        registration: RequestPolicyRegistration,
        evaluator: Arc<dyn RequestPolicyEvaluator>,
        caller: &RequestPolicyCaller,
    ) -> Result<(), RequestPolicyBrokerError> {
        require_authority(caller, OP_REQUEST_POLICY_REGISTER)?;
        registration
            .validate()
            .map_err(RequestPolicyBrokerError::Invalid)?;
        if evaluator.provider_id() != registration.provider_id {
            return Err(RequestPolicyBrokerError::Invalid(
                "registration provider_id does not match evaluator".to_string(),
            ));
        }
        if !caller_can_manage_registration(caller, &registration) {
            return Err(RequestPolicyBrokerError::ScopeDenied);
        }

        let mut state = lock_state(&self.state);
        if state
            .registrations
            .contains_key(&registration.registration_id)
        {
            return Err(RequestPolicyBrokerError::AlreadyRegistered(
                registration.registration_id,
            ));
        }
        if state.registrations.len() >= self.limits.max_registrations {
            return Err(RequestPolicyBrokerError::CapacityExceeded);
        }
        state.registrations.insert(
            registration.registration_id.clone(),
            RegistrationState {
                registration,
                evaluator,
                in_flight: 0,
            },
        );
        Ok(())
    }

    /// Unregisters one policy and cooperatively cancels its active calls.
    pub fn unregister(
        &self,
        registration_id: &str,
        caller: &RequestPolicyCaller,
    ) -> Result<(), RequestPolicyBrokerError> {
        require_authority(caller, OP_REQUEST_POLICY_UNREGISTER)?;
        let mut state = lock_state(&self.state);
        let registration = state
            .registrations
            .get(registration_id)
            .ok_or_else(|| RequestPolicyBrokerError::NotFound(registration_id.to_string()))?;
        if !caller_can_manage_registration(caller, &registration.registration) {
            return Err(RequestPolicyBrokerError::ScopeDenied);
        }
        state.registrations.remove(registration_id);
        cancel_registration_locked(&mut state, registration_id);
        Ok(())
    }

    /// Invalidates every policy attached to a context. This is an internal
    /// lifecycle operation and therefore does not use caller-supplied IDs as
    /// authority.
    pub fn invalidate_context(&self, context_id: &BrowserContextId) {
        let mut state = lock_state(&self.state);
        let ids: Vec<String> = state
            .registrations
            .iter()
            .filter(|(_, registration)| registration.registration.context_id == *context_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            state.registrations.remove(&id);
            cancel_registration_locked(&mut state, &id);
        }
    }

    /// Invalidates page-scoped policies when a page closes. Context-scoped
    /// policies remain valid for the other pages in that context.
    pub fn invalidate_page(&self, page_id: &PageId) {
        let mut state = lock_state(&self.state);
        let ids: Vec<String> = state
            .registrations
            .iter()
            .filter(|(_, registration)| registration.registration.page_id.as_ref() == Some(page_id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            state.registrations.remove(&id);
            cancel_registration_locked(&mut state, &id);
        }
    }

    /// Invalidates all policies and pending calls when a provider/runtime is
    /// shut down or restarted.
    pub fn invalidate_all(&self) {
        let mut state = lock_state(&self.state);
        for active in state.active.values() {
            active.cancellation.cancel();
        }
        state.registrations.clear();
    }

    /// Evaluates one request under the caller's exact authority and scope.
    pub fn decide(
        &self,
        caller: &RequestPolicyCaller,
        request: RequestPolicyRequest,
    ) -> Result<RequestPolicyResult, RequestPolicyBrokerError> {
        require_authority(caller, OP_REQUEST_POLICY_DECIDE)?;
        request
            .validate()
            .map_err(RequestPolicyBrokerError::Invalid)?;

        let (registration, evaluator, decision_id, cancellation) = {
            let mut state = lock_state(&self.state);
            let (registration, evaluator) = state
                .registrations
                .get(&request.registration_id)
                .ok_or_else(|| RequestPolicyBrokerError::NotFound(request.registration_id.clone()))
                .map(|registration_state| {
                    (
                        registration_state.registration.clone(),
                        Arc::clone(&registration_state.evaluator),
                    )
                })?;
            if !caller_can_access_request(caller, &registration, &request) {
                return Err(RequestPolicyBrokerError::ScopeDenied);
            }

            let capacity_exceeded = state
                .registrations
                .get(&request.registration_id)
                .is_some_and(|registration_state| {
                    registration_state.in_flight
                        >= usize::from(registration_state.registration.max_in_flight)
                })
                || state.total_in_flight >= self.limits.max_total_in_flight;
            if capacity_exceeded {
                let result = fallback_result(&registration, RequestPolicyOutcome::FailureFallback);
                record_observation_locked(
                    &mut state,
                    &registration,
                    &result,
                    0,
                    self.limits.max_observations,
                );
                return Ok(result);
            }

            let decision_id = state.next_decision_id;
            state.next_decision_id = state.next_decision_id.saturating_add(1).max(1);
            let cancellation = RequestPolicyCancellation::new();
            state
                .registrations
                .get_mut(&request.registration_id)
                .expect("registration remains present while broker state is locked")
                .in_flight += 1;
            state.total_in_flight += 1;
            state.active.insert(
                decision_id,
                ActiveDecision {
                    registration_id: request.registration_id.clone(),
                    cancellation: cancellation.clone(),
                },
            );
            (registration, evaluator, decision_id, cancellation)
        };

        let started = Instant::now();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let thread_state = Arc::clone(&self.state);
        let thread_request = request.clone();
        let thread_cancellation = cancellation.clone();
        let spawn = std::thread::Builder::new()
            .name("worldline-request-policy".to_string())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    evaluator.decide(&thread_request, &thread_cancellation)
                }));
                let _ = sender.send(result);
                finish_decision(&thread_state, decision_id);
            });

        if spawn.is_err() {
            finish_decision(&self.state, decision_id);
            let result = fallback_result(&registration, RequestPolicyOutcome::Unavailable);
            record_observation(
                &self.state,
                &registration,
                &result,
                started.elapsed(),
                self.limits.max_observations,
            );
            return Ok(result);
        }

        let received = receiver.recv_timeout(Duration::from_millis(request.deadline_ms));
        let timed_out = matches!(&received, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
        let latency = started.elapsed();
        let result = match received {
            Ok(Ok(Ok(result))) => {
                if result.validate().is_err() {
                    fallback_result(&registration, RequestPolicyOutcome::Unavailable)
                } else {
                    let mut result = result;
                    // The registration is the authoritative provider identity;
                    // evaluator output cannot impersonate another provider.
                    result.provider_id = Some(registration.provider_id.clone());
                    result
                }
            }
            Ok(Ok(Err(_error))) => {
                fallback_result(&registration, RequestPolicyOutcome::FailureFallback)
            }
            Ok(Err(_panic)) => fallback_result(&registration, RequestPolicyOutcome::Unavailable),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                cancellation.cancel();
                fallback_result(&registration, RequestPolicyOutcome::DeadlineExceeded)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                fallback_result(&registration, RequestPolicyOutcome::Unavailable)
            }
        };

        // The worker also performs this cleanup. Doing it here for completed
        // calls makes the capacity visible immediately; the second cleanup is
        // idempotent. Timed-out workers retain their slot until they exit so a
        // hung evaluator can never create unbounded threads.
        if !timed_out {
            finish_decision(&self.state, decision_id);
        }
        record_observation(
            &self.state,
            &registration,
            &result,
            latency,
            self.limits.max_observations,
        );
        Ok(result)
    }

    /// Drains safe post-outcome observations for an authorized exact scope.
    pub fn drain_observations(
        &self,
        caller: &RequestPolicyCaller,
    ) -> Result<Vec<RequestPolicyObservation>, RequestPolicyBrokerError> {
        require_authority(caller, OP_REQUEST_POLICY_OBSERVE)?;
        let mut state = lock_state(&self.state);
        let mut visible = Vec::new();
        let mut retained = VecDeque::with_capacity(state.observations.len());
        while let Some(observation) = state.observations.pop_front() {
            if caller_can_access_observation(caller, &observation) {
                visible.push(observation);
            } else {
                retained.push_back(observation);
            }
        }
        state.observations = retained;
        Ok(visible)
    }
}

fn require_authority(
    caller: &RequestPolicyCaller,
    operation: &'static str,
) -> Result<(), RequestPolicyBrokerError> {
    if caller
        .authorities
        .permits(CONTRACT_REQUEST_POLICY, operation)
    {
        Ok(())
    } else {
        Err(RequestPolicyBrokerError::PermissionDenied(operation))
    }
}

fn caller_can_manage_registration(
    caller: &RequestPolicyCaller,
    registration: &RequestPolicyRegistration,
) -> bool {
    if caller.context_id != registration.context_id {
        return false;
    }
    match (&caller.page_id, &registration.page_id) {
        (None, _) => true,
        (Some(caller_page), Some(registration_page)) => caller_page == registration_page,
        (Some(_), None) => false,
    }
}

fn caller_can_access_request(
    caller: &RequestPolicyCaller,
    registration: &RequestPolicyRegistration,
    request: &RequestPolicyRequest,
) -> bool {
    if caller.context_id != request.metadata.context_id
        || registration.context_id != request.metadata.context_id
    {
        return false;
    }
    if caller
        .page_id
        .as_ref()
        .is_some_and(|page| request.metadata.page_id.as_ref() != Some(page))
    {
        return false;
    }
    registration
        .page_id
        .as_ref()
        .is_none_or(|page| request.metadata.page_id.as_ref() == Some(page))
}

fn caller_can_access_observation(
    caller: &RequestPolicyCaller,
    observation: &RequestPolicyObservation,
) -> bool {
    caller.context_id == observation.context_id
        && caller
            .page_id
            .as_ref()
            .is_none_or(|page| observation.page_id.as_ref() == Some(page))
}

fn fallback_result(
    registration: &RequestPolicyRegistration,
    outcome: RequestPolicyOutcome,
) -> RequestPolicyResult {
    RequestPolicyResult {
        action: match registration.failure_mode {
            RequestPolicyFailureMode::FailOpen => RequestPolicyAction::Allow,
            RequestPolicyFailureMode::FailClosed => RequestPolicyAction::Block,
        },
        outcome,
        provider_id: Some(registration.provider_id.clone()),
        opaque_rule_ref: None,
    }
}

fn record_observation(
    state: &Arc<Mutex<BrokerState>>,
    registration: &RequestPolicyRegistration,
    result: &RequestPolicyResult,
    latency: Duration,
    max_observations: usize,
) {
    let mut state = lock_state(state);
    record_observation_locked(
        &mut state,
        registration,
        result,
        latency.as_millis() as u64,
        max_observations,
    );
}

fn record_observation_locked(
    state: &mut BrokerState,
    registration: &RequestPolicyRegistration,
    result: &RequestPolicyResult,
    latency_ms: u64,
    max_observations: usize,
) {
    if max_observations == 0 {
        return;
    }
    if state.observations.len() >= max_observations {
        state.observations.pop_front();
    }
    let observation = RequestPolicyObservation {
        registration_id: registration.registration_id.clone(),
        context_id: registration.context_id.clone(),
        page_id: registration.page_id.clone(),
        action: result.action,
        outcome: result.outcome,
        provider_id: result.provider_id.clone(),
        opaque_rule_ref: result.opaque_rule_ref.clone(),
        latency_ms,
    };
    if observation.validate().is_ok() {
        state.observations.push_back(observation);
    }
}

fn cancel_registration_locked(state: &mut BrokerState, registration_id: &str) {
    for active in state.active.values() {
        if active.registration_id == registration_id {
            active.cancellation.cancel();
        }
    }
}

fn finish_decision(state: &Arc<Mutex<BrokerState>>, decision_id: u64) {
    let mut state = lock_state(state);
    if let Some(active) = state.active.remove(&decision_id) {
        state.total_in_flight = state.total_in_flight.saturating_sub(1);
        if let Some(registration) = state.registrations.get_mut(&active.registration_id) {
            registration.in_flight = registration.in_flight.saturating_sub(1);
        }
    }
}

fn lock_state(state: &Mutex<BrokerState>) -> std::sync::MutexGuard<'_, BrokerState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;
    use worldline_browser_contract::authority::BrowserAuthority;
    use worldline_browser_contract::request_policy::{
        DEFAULT_REQUEST_POLICY_DEADLINE_MS, RequestPolicyMetadata, RequestResourceType,
    };

    struct TestEvaluator {
        id: &'static str,
        action: RequestPolicyAction,
        delay: Duration,
        calls: AtomicUsize,
    }

    impl RequestPolicyEvaluator for TestEvaluator {
        fn provider_id(&self) -> &str {
            self.id
        }

        fn decide(
            &self,
            _request: &RequestPolicyRequest,
            cancellation: &RequestPolicyCancellation,
        ) -> Result<RequestPolicyResult, RequestPolicyEvaluatorError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let started = Instant::now();
            while started.elapsed() < self.delay {
                if cancellation.is_cancelled() {
                    return Err(RequestPolicyEvaluatorError::Cancelled);
                }
                thread::sleep(Duration::from_millis(1));
            }
            Ok(RequestPolicyResult {
                action: self.action,
                outcome: RequestPolicyOutcome::Evaluated,
                provider_id: None,
                opaque_rule_ref: Some("opaque-rule-1".to_string()),
            })
        }
    }

    fn caller(
        context: &str,
        page: Option<&str>,
        decide: bool,
        observe: bool,
    ) -> RequestPolicyCaller {
        let mut authorities = BrowserAuthoritySet::new();
        if decide {
            authorities.grant(BrowserAuthority::DecideRequestPolicy);
        }
        if observe {
            authorities.grant(BrowserAuthority::ObserveRequestPolicy);
        }
        RequestPolicyCaller::new(
            authorities,
            BrowserContextId::new(context),
            page.map(PageId::new),
        )
    }

    fn request(
        registration_id: &str,
        context: &str,
        page: Option<&str>,
        deadline_ms: u64,
    ) -> RequestPolicyRequest {
        RequestPolicyRequest {
            registration_id: registration_id.to_string(),
            metadata: RequestPolicyMetadata {
                context_id: BrowserContextId::new(context),
                page_id: page.map(PageId::new),
                url: "http://127.0.0.1/asset.js".to_string(),
                method: "GET".to_string(),
                resource_type: RequestResourceType::Script,
                initiator_origin: Some("http://127.0.0.1".to_string()),
                top_level_origin: Some("http://127.0.0.1".to_string()),
            },
            deadline_ms,
        }
    }

    fn registration(
        id: &str,
        context: &str,
        page: Option<&str>,
        mode: RequestPolicyFailureMode,
        max_in_flight: u16,
    ) -> RequestPolicyRegistration {
        RequestPolicyRegistration {
            registration_id: id.to_string(),
            context_id: BrowserContextId::new(context),
            page_id: page.map(PageId::new),
            failure_mode: mode,
            max_in_flight,
            provider_id: "test-policy".to_string(),
        }
    }

    #[test]
    fn exact_scope_and_separate_authorities_are_enforced() {
        let broker = RequestPolicyBroker::new();
        let registration = registration(
            "reg-a",
            "ctx-a",
            Some("page-a"),
            RequestPolicyFailureMode::FailOpen,
            2,
        );
        let evaluator = Arc::new(TestEvaluator {
            id: "test-policy",
            action: RequestPolicyAction::Block,
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        });
        let owner = caller("ctx-a", Some("page-a"), true, false);
        broker
            .register(registration, evaluator, &owner)
            .expect("register");

        let wrong_page = caller("ctx-a", Some("page-b"), true, false);
        assert_eq!(
            broker.decide(&wrong_page, request("reg-a", "ctx-a", Some("page-b"), 50)),
            Err(RequestPolicyBrokerError::ScopeDenied)
        );
        let wrong_context = caller("ctx-b", Some("page-a"), true, false);
        assert_eq!(
            broker.decide(
                &wrong_context,
                request("reg-a", "ctx-b", Some("page-a"), 50)
            ),
            Err(RequestPolicyBrokerError::ScopeDenied)
        );
        let observation_only = caller("ctx-a", Some("page-a"), false, true);
        assert!(matches!(
            broker.decide(
                &observation_only,
                request("reg-a", "ctx-a", Some("page-a"), 50)
            ),
            Err(RequestPolicyBrokerError::PermissionDenied(_))
        ));
    }

    #[test]
    fn registration_failure_mode_controls_timeout_result() {
        let broker = RequestPolicyBroker::new();
        let evaluator = Arc::new(TestEvaluator {
            id: "test-policy",
            action: RequestPolicyAction::Block,
            delay: Duration::from_millis(40),
            calls: AtomicUsize::new(0),
        });
        let owner = caller("ctx-a", None, true, false);
        broker
            .register(
                registration("open", "ctx-a", None, RequestPolicyFailureMode::FailOpen, 2),
                Arc::clone(&evaluator) as Arc<dyn RequestPolicyEvaluator>,
                &owner,
            )
            .expect("register fail-open");
        broker
            .register(
                registration(
                    "closed",
                    "ctx-a",
                    None,
                    RequestPolicyFailureMode::FailClosed,
                    2,
                ),
                Arc::clone(&evaluator) as Arc<dyn RequestPolicyEvaluator>,
                &owner,
            )
            .expect("register fail-closed");

        let open = broker
            .decide(&owner, request("open", "ctx-a", None, 5))
            .expect("open result");
        assert_eq!(open.action, RequestPolicyAction::Allow);
        assert_eq!(open.outcome, RequestPolicyOutcome::DeadlineExceeded);
        let closed = broker
            .decide(&owner, request("closed", "ctx-a", None, 5))
            .expect("closed result");
        assert_eq!(closed.action, RequestPolicyAction::Block);
        assert_eq!(closed.outcome, RequestPolicyOutcome::DeadlineExceeded);
    }

    #[test]
    fn observations_are_filtered_and_lifecycle_invalidates_stale_registration() {
        let broker = RequestPolicyBroker::new();
        let evaluator = Arc::new(TestEvaluator {
            id: "test-policy",
            action: RequestPolicyAction::Allow,
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        });
        let owner_a = caller("ctx-a", Some("page-a"), true, true);
        let owner_b = caller("ctx-b", Some("page-b"), true, true);
        broker
            .register(
                registration(
                    "reg-a",
                    "ctx-a",
                    Some("page-a"),
                    RequestPolicyFailureMode::FailOpen,
                    1,
                ),
                evaluator,
                &owner_a,
            )
            .expect("register");
        broker
            .decide(
                &owner_a,
                request(
                    "reg-a",
                    "ctx-a",
                    Some("page-a"),
                    DEFAULT_REQUEST_POLICY_DEADLINE_MS,
                ),
            )
            .expect("decision");
        assert!(
            broker
                .drain_observations(&owner_b)
                .expect("observe")
                .is_empty()
        );
        assert_eq!(
            broker.drain_observations(&owner_a).expect("observe").len(),
            1
        );
        broker.invalidate_page(&PageId::new("page-a"));
        assert!(matches!(
            broker.decide(&owner_a, request("reg-a", "ctx-a", Some("page-a"), 50)),
            Err(RequestPolicyBrokerError::NotFound(_))
        ));
    }

    #[test]
    fn capacity_is_bounded_without_unbounded_worker_creation() {
        let broker = RequestPolicyBroker::with_limits(RequestPolicyBrokerLimits {
            max_registrations: 4,
            max_total_in_flight: 1,
            max_observations: 4,
        });
        let evaluator = Arc::new(TestEvaluator {
            id: "test-policy",
            action: RequestPolicyAction::Block,
            delay: Duration::from_millis(30),
            calls: AtomicUsize::new(0),
        });
        let owner = caller("ctx-a", None, true, false);
        broker
            .register(
                registration(
                    "reg-a",
                    "ctx-a",
                    None,
                    RequestPolicyFailureMode::FailOpen,
                    1,
                ),
                evaluator,
                &owner,
            )
            .expect("register");
        let barrier = Arc::new(Barrier::new(2));
        let thread_broker = broker.clone();
        let thread_owner = owner.clone();
        let thread_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            thread_barrier.wait();
            thread_broker
                .decide(&thread_owner, request("reg-a", "ctx-a", None, 100))
                .expect("first decision")
        });
        barrier.wait();
        thread::sleep(Duration::from_millis(2));
        let second = broker
            .decide(&owner, request("reg-a", "ctx-a", None, 100))
            .expect("bounded fallback");
        assert_eq!(second.action, RequestPolicyAction::Allow);
        assert_eq!(second.outcome, RequestPolicyOutcome::FailureFallback);
        let _ = first.join().expect("first thread");
    }
}
