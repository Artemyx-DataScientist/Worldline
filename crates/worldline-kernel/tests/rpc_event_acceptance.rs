use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use worldline_kernel::{
    ActivationContext, CapabilityError, CapabilityId, CapabilityService, CausationRef,
    DeliveryMode, EventContract, EventJournal, EventJournalError, EventQoS, GrantLifetime,
    InMemoryEventJournal, InterfaceVersion, Kernel, NoopRuntime, OperationId, OverflowPolicy,
    Plugin, PluginDefinition, PluginError, PluginRuntime, Principal, PrincipalId, PrincipalKind,
    ProviderLimits, ResourceId, ResourceScope, RpcCallOptions, RpcCancellationToken,
    RpcOperationContract, RpcRequestId, RpcRetryClass, SubscriptionOptions, TraceContext,
    TrajectoryEventKind,
};

fn capability(name: &str) -> CapabilityId {
    CapabilityId::new("worldline.rpc-event", name, InterfaceVersion::new(1, 0))
}

fn event_contract(name: &str) -> EventContract {
    EventContract::new("worldline.test", name, InterfaceVersion::new(1, 0))
}

fn register_principal(kernel: &Kernel, id: &str, kind: PrincipalKind) -> PrincipalId {
    let principal = PrincipalId::new(id);
    kernel
        .register_principal(Principal::new(principal.clone(), kind))
        .expect("principal registration must succeed");
    principal
}

fn grant(
    kernel: &Kernel,
    subject: &PrincipalId,
    contract: &worldline_kernel::CapabilityContract,
    operation: &str,
) -> worldline_kernel::GrantId {
    kernel
        .create_root_grant(
            subject.clone(),
            contract.clone(),
            [operation],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .expect("grant must succeed")
}

struct ProviderPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
    limits: ProviderLimits,
}

impl Plugin for ProviderPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        context.publish_capability_with_limits(
            self.capability.clone(),
            Arc::clone(&self.service),
            self.limits,
        )?;
        Ok(Box::new(NoopRuntime))
    }
}

fn provider(
    kernel: &mut Kernel,
    plugin: &str,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
    limits: ProviderLimits,
) -> (
    worldline_kernel::PluginId,
    worldline_kernel::RuntimeId,
    PrincipalId,
) {
    let id = kernel
        .register(ProviderPlugin {
            definition: PluginDefinition::new(plugin).provides(capability.clone()),
            capability: capability.clone(),
            service,
            limits,
        })
        .expect("provider must activate");
    let runtime = kernel
        .runtime_id_for_plugin(&id)
        .expect("runtime must exist");
    let principal = kernel
        .principal_for_runtime(&runtime)
        .expect("runtime principal must exist");
    (id, runtime, principal)
}

struct EchoService {
    calls: Arc<AtomicUsize>,
    last_context: Arc<Mutex<Option<(RpcRequestId, usize, TraceContext)>>>,
    event: Option<EventContract>,
}

impl CapabilityService for EchoService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok([operation.as_bytes(), payload].concat())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self
            .last_context
            .lock()
            .expect("context lock is not poisoned") = Some((
            context.rpc_request_id().clone(),
            context.nested_depth(),
            context.trace_context(),
        ));
        if let Some(event) = &self.event {
            context
                .publish_event(event.clone(), payload, Default::default())
                .map_err(|error| error.to_string())?;
        }
        Ok(payload.to_vec())
    }
}

struct ContractService {
    contract: RpcOperationContract,
}

impl CapabilityService for ContractService {
    fn invoke(&self, _operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(payload.to_vec())
    }

    fn rpc_operation_contract(&self, _operation: &OperationId) -> RpcOperationContract {
        self.contract.clone()
    }
}

struct DeadlineService {
    observed: Arc<Mutex<Option<Option<Duration>>>>,
}

impl CapabilityService for DeadlineService {
    fn invoke(&self, _operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(payload.to_vec())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        *self.observed.lock().expect("deadline lock is not poisoned") = Some(context.deadline());
        Ok(payload.to_vec())
    }
}

#[test]
fn request_and_attempt_ids_are_distinct_and_trace_is_kernel_stamped() {
    let cap = capability("echo");
    let calls = Arc::new(AtomicUsize::new(0));
    let context = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();
    let (_, _, provider_principal) = provider(
        &mut kernel,
        "echo-provider",
        cap.clone(),
        Arc::new(EchoService {
            calls: Arc::clone(&calls),
            last_context: Arc::clone(&context),
            event: None,
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "rpc-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "echo");
    let request_id = RpcRequestId::new("logical-request-1");
    let trace = TraceContext::new("activity-1");
    let result = kernel
        .capability_for(caller, cap.clone())
        .expect("handle must exist")
        .invoke_with_options(
            "echo",
            b"ok",
            RpcCallOptions::new()
                .with_request_id(request_id.clone())
                .with_trace_context(trace.clone()),
        )
        .expect("RPC must succeed");
    assert_eq!(result, b"ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let context = context
        .lock()
        .expect("context lock is not poisoned")
        .clone()
        .expect("provider context must be captured");
    assert_eq!(context.0, request_id);
    assert_eq!(context.1, 0);
    assert_eq!(context.2.correlation_id().as_str(), "activity-1");
    assert!(matches!(
        context.2.causation(),
        Some(CausationRef::Invocation(_))
    ));
    assert_eq!(
        kernel
            .principal(&provider_principal)
            .expect("provider principal metadata")
            .kind(),
        PrincipalKind::PluginRuntime
    );
    let attempts = kernel
        .trajectory()
        .into_iter()
        .filter_map(|event| match event.kind() {
            TrajectoryEventKind::RpcRequestCreated { invocation, .. } => Some(invocation.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 1);
}

#[test]
fn invocation_completed_event_is_metadata_only_and_independent() {
    let cap = capability("control-observation");
    let control = worldline_kernel::invocation_completed_event_contract();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let (_, provider_runtime, _) = provider(
        &mut kernel,
        "control-provider",
        cap.clone(),
        Arc::new(EchoService {
            calls: Arc::clone(&calls),
            last_context: Arc::new(Mutex::new(None)),
            event: None,
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "control-caller", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "control-observer", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    grant(
        &kernel,
        &observer,
        &control.capability_id().contract(),
        "subscribe",
    );
    let subscription = kernel
        .subscribe(observer.clone(), control, SubscriptionOptions::default())
        .expect("control observer must be authorized");
    let request_id = RpcRequestId::new("control-logical");
    let result = kernel
        .capability_for(caller.clone(), cap.clone())
        .expect("handle must exist")
        .invoke_with_options(
            "run",
            b"secret-request-and-result",
            RpcCallOptions::new().with_request_id(request_id.clone()),
        )
        .expect("RPC must succeed without observer participation");
    assert_eq!(result, b"secret-request-and-result");
    let envelope = subscription
        .try_recv()
        .expect("control observation receive")
        .expect("control observation must be published");
    let metadata = envelope
        .invocation_completed()
        .expect("control event must expose trusted RPC metadata");
    assert_eq!(metadata.request_id(), &request_id);
    assert_eq!(metadata.caller(), &caller);
    assert_eq!(metadata.provider_runtime_id(), provider_runtime);
    assert_eq!(
        metadata.outcome(),
        worldline_kernel::RpcOutcomeClass::Success
    );
    assert!(
        envelope.payload().is_empty(),
        "raw request/result bytes must not be in control observation payload"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn explicit_retry_gets_new_attempt_and_cannot_escalate_contract() {
    let cap = capability("retry");
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "retry-provider",
        cap.clone(),
        Arc::new(ContractService {
            contract: RpcOperationContract::safe("run"),
        }),
        ProviderLimits::new(2, 0),
    );
    let caller = register_principal(&kernel, "retry-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap.clone())
        .expect("handle must exist");
    let request_id = RpcRequestId::new("retry-logical");
    handle
        .invoke_with_options(
            "run",
            b"first",
            RpcCallOptions::new()
                .with_request_id(request_id.clone())
                .with_retry_classification(RpcRetryClass::Safe),
        )
        .expect("first attempt must succeed");
    handle
        .invoke_with_options(
            "run",
            b"second",
            RpcCallOptions::new()
                .with_request_id(request_id.clone())
                .with_retry_classification(RpcRetryClass::Safe)
                .with_retry(),
        )
        .expect("explicit safe retry must succeed");
    let attempts = kernel
        .trajectory()
        .into_iter()
        .filter_map(|event| match event.kind() {
            TrajectoryEventKind::RpcRequestCreated {
                request_id: candidate,
                invocation,
                ..
            } if candidate == &request_id => Some(invocation.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2);

    let error = handle
        .invoke_with_options(
            "run",
            b"escalate",
            RpcCallOptions::new()
                .with_request_id("escalated")
                .with_retry_classification(RpcRetryClass::Idempotent),
        )
        .expect_err("caller cannot escalate safe contract");
    assert!(matches!(error, CapabilityError::RpcInvalidRetry { .. }));
}

#[test]
fn idempotent_retry_requires_key_and_never_retry_rejects_retry() {
    let cap = capability("idempotent");
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "idempotent-provider",
        cap.clone(),
        Arc::new(ContractService {
            contract: RpcOperationContract::idempotent("run"),
        }),
        ProviderLimits::new(2, 0),
    );
    let caller = register_principal(&kernel, "idempotent-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap.clone())
        .expect("handle must exist");
    let missing = handle
        .invoke_with_options(
            "run",
            b"retry",
            RpcCallOptions::new()
                .with_request_id("idempotent-request")
                .with_retry_classification(RpcRetryClass::Idempotent)
                .with_retry(),
        )
        .expect_err("idempotent retry without key must fail");
    assert!(matches!(
        missing,
        CapabilityError::RpcMissingIdempotencyKey { .. }
    ));

    let never_cap = capability("never");
    provider(
        &mut kernel,
        "never-provider",
        never_cap.clone(),
        Arc::new(EchoService {
            calls: Arc::new(AtomicUsize::new(0)),
            last_context: Arc::new(Mutex::new(None)),
            event: None,
        }),
        ProviderLimits::new(2, 0),
    );
    let caller = register_principal(&kernel, "never-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &never_cap.contract(), "run");
    let error = kernel
        .capability_for(caller, never_cap)
        .expect("handle must exist")
        .invoke_with_options(
            "run",
            b"retry",
            RpcCallOptions::new()
                .with_request_id("never-request")
                .with_retry(),
        )
        .expect_err("NeverRetry must reject explicit retry");
    assert!(matches!(error, CapabilityError::RpcInvalidRetry { .. }));
}

#[test]
fn no_deadline_is_explicit_and_provider_contract_can_require_a_key() {
    let cap = capability("deadline-policy");
    let observed = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "deadline-policy-provider",
        cap.clone(),
        Arc::new(DeadlineService {
            observed: Arc::clone(&observed),
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "deadline-policy-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    kernel
        .capability_for(caller, cap.clone())
        .expect("handle must exist")
        .invoke_with_options(
            "run",
            b"no-deadline",
            RpcCallOptions::new().with_no_deadline(),
        )
        .expect("explicit no-deadline policy must be accepted");
    assert_eq!(
        *observed.lock().expect("deadline lock is not poisoned"),
        Some(None)
    );

    let keyed_cap = capability("contract-key");
    provider(
        &mut kernel,
        "contract-key-provider",
        keyed_cap.clone(),
        Arc::new(ContractService {
            contract: RpcOperationContract::safe("run").with_idempotency_key_required(true),
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "contract-key-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &keyed_cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, keyed_cap)
        .expect("handle must exist");
    let missing = handle
        .invoke_with_options(
            "run",
            b"retry",
            RpcCallOptions::new()
                .with_request_id("contract-key-request")
                .with_retry(),
        )
        .expect_err("contract-required key must be enforced");
    assert!(matches!(
        missing,
        CapabilityError::RpcMissingIdempotencyKey { .. }
    ));
    handle
        .invoke_with_options(
            "run",
            b"retry",
            RpcCallOptions::new()
                .with_request_id("contract-key-request-2")
                .with_retry()
                .with_idempotency_key("key-2"),
        )
        .expect("explicit key must satisfy the provider contract");
}

struct BlockingService {
    started: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl CapabilityService for BlockingService {
    fn invoke(&self, _operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            thread::yield_now();
        }
        Ok(payload.to_vec())
    }
}

#[test]
fn provider_concurrency_and_queue_are_bounded() {
    let cap = capability("bounded");
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "bounded-provider",
        cap.clone(),
        Arc::new(BlockingService {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        }),
        ProviderLimits::new(1, 1),
    );
    let caller = register_principal(&kernel, "bounded-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap)
        .expect("handle must exist");
    let first_handle = handle.clone();
    let first = thread::spawn(move || first_handle.invoke("run", b"first"));
    for _ in 0..200 {
        if started.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        started.load(Ordering::Acquire),
        "first request must dispatch"
    );
    let second_handle = handle.clone();
    let second = thread::spawn(move || {
        second_handle.invoke_with_options(
            "run",
            b"second",
            RpcCallOptions::new().with_deadline(Duration::from_secs(2)),
        )
    });
    let queued = (0..200).any(|_| {
        let found = kernel
            .trajectory()
            .into_iter()
            .any(|event| matches!(event.kind(), TrajectoryEventKind::RpcQueued { .. }));
        if !found {
            thread::sleep(Duration::from_millis(1));
        }
        found
    });
    assert!(queued, "second request must enter the bounded queue");
    let third = handle
        .invoke_with_options(
            "run",
            b"third",
            RpcCallOptions::new().with_deadline(Duration::from_millis(100)),
        )
        .expect_err("bounded queue must reject a third request");
    assert!(matches!(third, CapabilityError::RpcQueueFull { .. }));
    release.store(true, Ordering::Release);
    assert_eq!(
        first.join().expect("first thread must join").unwrap(),
        b"first"
    );
    assert_eq!(
        second.join().expect("second thread must join").unwrap(),
        b"second"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn runtime_teardown_wakes_queued_rpc_with_explicit_unavailable_error() {
    let cap = capability("teardown-queue");
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let (plugin, _, _) = provider(
        &mut kernel,
        "teardown-queue-provider",
        cap.clone(),
        Arc::new(BlockingService {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        }),
        ProviderLimits::new(1, 1),
    );
    let caller = register_principal(&kernel, "teardown-queue-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap)
        .expect("handle must exist");
    let first_handle = handle.clone();
    let first = thread::spawn(move || first_handle.invoke("run", b"first"));
    for _ in 0..200 {
        if started.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        started.load(Ordering::Acquire),
        "first request must dispatch"
    );
    let second_handle = handle;
    let second = thread::spawn(move || {
        second_handle.invoke_with_options(
            "run",
            b"queued",
            RpcCallOptions::new().with_deadline(Duration::from_secs(5)),
        )
    });
    let queued = (0..200).any(|_| {
        let found = kernel
            .trajectory()
            .into_iter()
            .any(|event| matches!(event.kind(), TrajectoryEventKind::RpcQueued { .. }));
        if !found {
            thread::sleep(Duration::from_millis(1));
        }
        found
    });
    assert!(queued, "second request must enter the bounded queue");
    kernel
        .unregister(&plugin)
        .expect("runtime teardown must succeed");
    let second_result = second.join().expect("queued worker must join");
    assert!(matches!(
        second_result,
        Err(CapabilityError::RpcRuntimeUnavailable { .. })
    ));
    release.store(true, Ordering::Release);
    assert_eq!(
        first.join().expect("first worker must join").unwrap(),
        b"first"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn deadline_and_cancellation_prevent_dispatch_and_close_late_success() {
    let cap = capability("cancel");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "cancel-provider",
        cap.clone(),
        Arc::new(EchoService {
            calls: Arc::clone(&calls),
            last_context: Arc::new(Mutex::new(None)),
            event: None,
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "cancel-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap.clone())
        .expect("handle must exist");
    let token = RpcCancellationToken::new();
    token.cancel();
    let cancelled = handle
        .invoke_with_options(
            "run",
            b"before",
            RpcCallOptions::new().with_cancellation(token),
        )
        .expect_err("cancelled request must not dispatch");
    assert!(matches!(cancelled, CapabilityError::RpcCancelled { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let expired = handle
        .invoke_with_options(
            "run",
            b"expired",
            RpcCallOptions::new().with_deadline(Duration::ZERO),
        )
        .expect_err("expired request must not dispatch");
    assert!(matches!(
        expired,
        CapabilityError::RpcDeadlineExceeded { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_after_dispatch_is_cooperative_and_cannot_become_success() {
    let cap = capability("late-cancel");
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "late-cancel-provider",
        cap.clone(),
        Arc::new(BlockingService {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "late-cancel-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap)
        .expect("handle must exist");
    let token = RpcCancellationToken::new();
    let worker_token = token.clone();
    let worker_handle = handle.clone();
    let worker = thread::spawn(move || {
        worker_handle.invoke_with_options(
            "run",
            b"late",
            RpcCallOptions::new().with_cancellation(worker_token),
        )
    });
    for _ in 0..200 {
        if started.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(started.load(Ordering::Acquire), "request must dispatch");
    token.cancel();
    release.store(true, Ordering::Release);
    let result = worker.join().expect("worker must join");
    assert!(matches!(result, Err(CapabilityError::RpcCancelled { .. })));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        !kernel.trajectory().into_iter().any(|event| {
            matches!(
                event.kind(),
                TrajectoryEventKind::InvocationCompleted { .. }
            )
        }),
        "late cancellation cannot create a successful completion"
    );
}

#[test]
fn deadline_after_dispatch_cannot_become_success() {
    let cap = capability("late-deadline");
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "late-deadline-provider",
        cap.clone(),
        Arc::new(BlockingService {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "late-deadline-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap)
        .expect("handle must exist");
    let worker_handle = handle.clone();
    let worker = thread::spawn(move || {
        worker_handle.invoke_with_options(
            "run",
            b"late-deadline",
            RpcCallOptions::new().with_deadline(Duration::from_millis(20)),
        )
    });
    for _ in 0..200 {
        if started.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(started.load(Ordering::Acquire), "request must dispatch");
    thread::sleep(Duration::from_millis(30));
    release.store(true, Ordering::Release);
    let result = worker.join().expect("worker must join");
    assert!(matches!(
        result,
        Err(CapabilityError::RpcDeadlineExceeded { .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!kernel.trajectory().into_iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::InvocationCompleted { .. }
        )
    }));
}

#[test]
fn queued_request_expiry_is_removed_without_provider_dispatch() {
    let cap = capability("queued-expiry");
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "queued-expiry-provider",
        cap.clone(),
        Arc::new(BlockingService {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        }),
        ProviderLimits::new(1, 1),
    );
    let caller = register_principal(&kernel, "queued-expiry-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let handle = kernel
        .capability_for(caller, cap)
        .expect("handle must exist");
    let first_handle = handle.clone();
    let first = thread::spawn(move || first_handle.invoke("run", b"first"));
    for _ in 0..200 {
        if started.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        started.load(Ordering::Acquire),
        "first request must dispatch"
    );
    let second_handle = handle.clone();
    let second = thread::spawn(move || {
        second_handle.invoke_with_options(
            "run",
            b"expired-queued",
            RpcCallOptions::new().with_deadline(Duration::from_millis(20)),
        )
    });
    let second_result = second.join().expect("queued worker must join");
    assert!(matches!(
        second_result,
        Err(CapabilityError::RpcDeadlineExceeded { .. })
    ));
    release.store(true, Ordering::Release);
    assert_eq!(
        first.join().expect("first worker must join").unwrap(),
        b"first"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn event_authority_fanout_and_overflow_are_independent_from_rpc() {
    let contract = event_contract("observation");
    let producer_capability = contract.capability_id();
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "event-producer", PrincipalKind::Agent);
    let observer_a = register_principal(&kernel, "event-observer-a", PrincipalKind::Agent);
    let observer_b = register_principal(&kernel, "event-observer-b", PrincipalKind::Agent);
    grant(
        &kernel,
        &producer,
        &producer_capability.contract(),
        "publish",
    );
    grant(
        &kernel,
        &observer_a,
        &producer_capability.contract(),
        "subscribe",
    );
    grant(
        &kernel,
        &observer_b,
        &producer_capability.contract(),
        "subscribe",
    );
    let a = kernel
        .subscribe(
            observer_a,
            contract.clone(),
            SubscriptionOptions::new(1, OverflowPolicy::DropNewest),
        )
        .expect("subscriber A must be authorized");
    let b = kernel
        .subscribe(
            observer_b,
            contract.clone(),
            SubscriptionOptions::new(2, OverflowPolicy::RejectForSubscriber)
                .with_qos(EventQoS::Observed),
        )
        .expect("subscriber B must be authorized");
    let first = kernel
        .publish_event(
            producer.clone(),
            contract.clone(),
            b"first",
            Default::default(),
        )
        .expect("publication must succeed");
    assert_eq!(first.eligible_subscribers(), 2);
    assert_eq!(first.enqueued(), 2);
    let second = kernel
        .publish_event(
            producer.clone(),
            contract.clone(),
            b"second",
            Default::default(),
        )
        .expect("publication with full mailbox remains valid");
    assert_eq!(second.enqueued(), 1);
    assert_eq!(second.dropped(), 1);
    assert_eq!(second.backpressured(), 0);
    assert_eq!(
        a.try_recv()
            .expect("A receive must work")
            .unwrap()
            .payload(),
        b"first"
    );
    let third = kernel
        .publish_event(producer, contract, b"third", Default::default())
        .expect("third publication must succeed");
    assert_eq!(third.enqueued(), 1);
    assert_eq!(third.dropped(), 0);
    assert_eq!(third.backpressured(), 1);
    assert_eq!(
        a.try_recv()
            .expect("A receive must work")
            .unwrap()
            .payload(),
        b"third"
    );
    assert_eq!(
        b.try_recv()
            .expect("B receive must work")
            .unwrap()
            .payload(),
        b"first"
    );
    assert_eq!(
        b.try_recv()
            .expect("B receive must work")
            .unwrap()
            .payload(),
        b"second"
    );

    let denied = kernel.publish_event(
        PrincipalId::new("unregistered"),
        event_contract("denied"),
        b"payload",
        Default::default(),
    );
    assert!(matches!(
        denied,
        Err(worldline_kernel::EventError::PrincipalUnavailable { .. })
    ));
}

#[test]
fn full_event_mailbox_does_not_block_unrelated_rpc() {
    let event = event_contract("full-mailbox");
    let rpc = capability("unrelated-rpc");
    let mut kernel = Kernel::new();
    let (_, _, _) = provider(
        &mut kernel,
        "unrelated-rpc-provider",
        rpc.clone(),
        Arc::new(EchoService {
            calls: Arc::new(AtomicUsize::new(0)),
            last_context: Arc::new(Mutex::new(None)),
            event: None,
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "unrelated-rpc-caller", PrincipalKind::Agent);
    let producer = register_principal(&kernel, "full-mailbox-producer", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "full-mailbox-observer", PrincipalKind::Agent);
    grant(&kernel, &caller, &rpc.contract(), "run");
    grant(
        &kernel,
        &producer,
        &event.capability_id().contract(),
        "publish",
    );
    grant(
        &kernel,
        &observer,
        &event.capability_id().contract(),
        "subscribe",
    );
    let _subscription = kernel
        .subscribe(
            observer,
            event.clone(),
            SubscriptionOptions::new(1, OverflowPolicy::RejectForSubscriber),
        )
        .expect("subscription must be authorized");
    kernel
        .publish_event(
            producer.clone(),
            event.clone(),
            b"fills-mailbox",
            Default::default(),
        )
        .expect("first event must fill mailbox");
    let report = kernel
        .publish_event(producer, event, b"backpressured", Default::default())
        .expect("backpressure must be reported without blocking");
    assert_eq!(report.backpressured(), 1);
    assert_eq!(
        kernel
            .capability_for(caller, rpc)
            .expect("RPC handle must exist")
            .invoke("run", b"unrelated")
            .expect("unrelated RPC must remain available"),
        b"unrelated"
    );
}

#[test]
fn drop_oldest_replaces_the_oldest_mailbox_entry_deterministically() {
    let contract = event_contract("drop-oldest");
    let capability = contract.capability_id();
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "drop-oldest-producer", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "drop-oldest-observer", PrincipalKind::Agent);
    grant(&kernel, &producer, &capability.contract(), "publish");
    grant(&kernel, &observer, &capability.contract(), "subscribe");
    let subscription = kernel
        .subscribe(
            observer,
            contract.clone(),
            SubscriptionOptions::new(2, OverflowPolicy::DropOldest),
        )
        .expect("subscription must be authorized");

    let first = kernel
        .publish_event(
            producer.clone(),
            contract.clone(),
            b"first",
            Default::default(),
        )
        .expect("first publication must succeed");
    assert_eq!(first.enqueued(), 1);
    let second = kernel
        .publish_event(
            producer.clone(),
            contract.clone(),
            b"second",
            Default::default(),
        )
        .expect("second publication must succeed");
    assert_eq!(second.enqueued(), 1);
    let third = kernel
        .publish_event(producer, contract, b"third", Default::default())
        .expect("third publication must succeed");
    assert_eq!(third.enqueued(), 1);
    assert_eq!(third.dropped(), 1);
    assert_eq!(third.backpressured(), 0);

    assert_eq!(
        subscription
            .try_recv()
            .expect("first receive must work")
            .unwrap()
            .payload(),
        b"second"
    );
    assert_eq!(
        subscription
            .try_recv()
            .expect("second receive must work")
            .unwrap()
            .payload(),
        b"third"
    );
}

#[test]
fn event_default_deny_revocation_and_durable_journal_are_explicit() {
    let contract = event_contract("durable");
    let cap = contract.capability_id();
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "durable-producer", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "durable-observer", PrincipalKind::Agent);
    let denied_publish = kernel.publish_event(
        producer.clone(),
        contract.clone(),
        b"payload",
        Default::default(),
    );
    assert!(matches!(
        denied_publish,
        Err(worldline_kernel::EventError::EventPublishDenied)
    ));
    let denied_subscribe = kernel.subscribe(
        observer.clone(),
        contract.clone(),
        SubscriptionOptions::default(),
    );
    assert!(matches!(
        denied_subscribe,
        Err(worldline_kernel::EventError::EventSubscribeDenied)
    ));
    let publish_grant = grant(&kernel, &producer, &cap.contract(), "publish");
    let subscribe_grant = grant(&kernel, &observer, &cap.contract(), "subscribe");
    let journal = Arc::new(InMemoryEventJournal::new());
    kernel.set_event_journal(Arc::clone(&journal) as Arc<dyn EventJournal>);
    let subscription = kernel
        .subscribe(observer, contract.clone(), SubscriptionOptions::default())
        .expect("subscription must succeed after grant");
    let report = kernel
        .publish_event(
            producer,
            contract.clone(),
            b"durable-payload",
            worldline_kernel::EventPublishOptions::default()
                .with_delivery_mode(DeliveryMode::Durable),
        )
        .expect("durable publication must be journaled");
    assert!(report.durably_recorded());
    assert_eq!(journal.events().len(), 1);
    let replayed = journal
        .read_from(worldline_kernel::EventCursor::new(0))
        .expect("journal read must succeed");
    assert_eq!(replayed[0].payload(), b"durable-payload");
    assert_eq!(
        subscription
            .try_recv()
            .expect("delivery must succeed")
            .unwrap()
            .delivery_mode(),
        DeliveryMode::Durable
    );
    kernel
        .revoke_grant(&subscribe_grant)
        .expect("subscribe grant revoke must succeed");
    assert!(matches!(
        subscription.try_recv(),
        Err(worldline_kernel::EventError::EventSubscribeDenied)
    ));
    kernel
        .revoke_grant(&publish_grant)
        .expect("publish grant revoke must succeed");
    let denied = kernel.publish_event(
        PrincipalId::new("durable-producer"),
        contract,
        b"after-revoke",
        Default::default(),
    );
    assert!(matches!(
        denied,
        Err(worldline_kernel::EventError::EventPublishDenied)
    ));
}

#[test]
fn runtime_scoped_subscription_closes_with_runtime_lifecycle() {
    let source = capability("runtime-subscription-source");
    let event = event_contract("runtime-subscription");
    let mut kernel = Kernel::new();
    let (plugin, runtime, runtime_principal) = provider(
        &mut kernel,
        "runtime-subscription-provider",
        source,
        Arc::new(EchoService {
            calls: Arc::new(AtomicUsize::new(0)),
            last_context: Arc::new(Mutex::new(None)),
            event: None,
        }),
        ProviderLimits::new(1, 0),
    );
    grant(
        &kernel,
        &runtime_principal,
        &event.capability_id().contract(),
        "subscribe",
    );
    let subscription = kernel
        .subscribe_for_runtime(runtime, event.clone(), SubscriptionOptions::default())
        .expect("runtime subscription must be authorized");
    assert!(matches!(
        kernel.subscribe(runtime_principal, event, SubscriptionOptions::default()),
        Err(worldline_kernel::EventError::EventSubscribeDenied)
    ));
    kernel
        .unregister(&plugin)
        .expect("runtime unregister must succeed");
    assert!(matches!(
        subscription.try_recv(),
        Err(worldline_kernel::EventError::SubscriptionClosed { .. })
    ));
}

#[test]
fn event_trace_and_follow_up_rpc_use_subscriber_authority() {
    let event = event_contract("follow-up");
    let follow_up = capability("follow-up-target");
    let publish_calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let (_, provider_runtime, provider_principal) = provider(
        &mut kernel,
        "event-provider",
        capability("event-source"),
        Arc::new(EchoService {
            calls: Arc::clone(&publish_calls),
            last_context: Arc::new(Mutex::new(None)),
            event: Some(event.clone()),
        }),
        ProviderLimits::new(2, 0),
    );
    let (_, _, follow_up_provider_principal) = provider(
        &mut kernel,
        "follow-up-provider",
        follow_up.clone(),
        Arc::new(EchoService {
            calls: Arc::new(AtomicUsize::new(0)),
            last_context: Arc::new(Mutex::new(None)),
            event: None,
        }),
        ProviderLimits::new(2, 0),
    );
    let caller = register_principal(&kernel, "event-rpc-caller", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "event-rpc-observer", PrincipalKind::Agent);
    let source = capability("event-source");
    grant(&kernel, &caller, &source.contract(), "run");
    grant(
        &kernel,
        &provider_principal,
        &event.capability_id().contract(),
        "publish",
    );
    grant(
        &kernel,
        &observer,
        &event.capability_id().contract(),
        "subscribe",
    );
    grant(&kernel, &observer, &follow_up.contract(), "run");
    let subscription = kernel
        .subscribe(
            observer.clone(),
            event.clone(),
            SubscriptionOptions::default(),
        )
        .expect("observer must subscribe");
    let result = kernel
        .capability_for(caller, source.clone())
        .expect("source handle must exist")
        .invoke("run", b"source");
    assert_eq!(result.expect("source RPC must succeed"), b"source");
    assert_eq!(publish_calls.load(Ordering::SeqCst), 1);
    let envelope = subscription
        .try_recv()
        .expect("event receive must succeed")
        .expect("event must be delivered");
    assert_eq!(envelope.producer_runtime_id(), Some(provider_runtime));
    assert!(matches!(
        envelope.causation(),
        Some(CausationRef::Invocation(_))
    ));
    assert_eq!(envelope.payload(), b"source");
    let context = subscription.context(envelope.clone());
    assert_eq!(context.subscriber().as_str(), "event-rpc-observer");
    let denied = context
        .invoke(
            source.clone(),
            "run",
            ResourceId::root(source.namespace()),
            b"producer-authority-is-not-transferred",
        )
        .expect_err("receiving an event must not grant producer capability");
    assert!(matches!(
        denied,
        CapabilityError::Denied {
            reason: worldline_kernel::DenialReason::NoGrant,
            ..
        }
    ));
    let follow_up_result = context
        .invoke(
            follow_up.clone(),
            "run",
            ResourceId::root(follow_up.namespace()),
            b"follow-up",
        )
        .expect("observer-authorized follow-up must succeed");
    assert_eq!(follow_up_result, b"follow-up");
    assert_eq!(
        kernel
            .principal(&follow_up_provider_principal)
            .expect("follow-up principal metadata")
            .kind(),
        PrincipalKind::PluginRuntime
    );
    let has_event_causation = kernel.trajectory().into_iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::RpcRequestCreated {
                causation: Some(CausationRef::Event(_)),
                ..
            }
        )
    });
    assert!(has_event_causation);
}

#[test]
fn durable_without_journal_never_downgrades() {
    let contract = event_contract("no-journal");
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "no-journal-producer", PrincipalKind::Agent);
    grant(
        &kernel,
        &producer,
        &contract.capability_id().contract(),
        "publish",
    );
    let error = kernel
        .publish_event(
            producer,
            contract,
            b"payload",
            worldline_kernel::EventPublishOptions::default().durable(),
        )
        .expect_err("durable mode requires a journal");
    assert!(matches!(
        error,
        worldline_kernel::EventError::DurableDeliveryUnavailable
    ));
}

#[test]
fn recursive_provider_calls_stop_at_depth_limit() {
    struct RecursiveService {
        target: CapabilityId,
    }

    impl CapabilityService for RecursiveService {
        fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }

        fn invoke_with_context(
            &self,
            context: &worldline_kernel::InvocationContext,
            payload: &[u8],
        ) -> Result<Vec<u8>, String> {
            context
                .invoke_self(
                    self.target.clone(),
                    "loop",
                    ResourceId::root("worldline.rpc-event"),
                    payload,
                )
                .map_err(|error| error.to_string())
        }
    }

    let a = capability("a");
    let b = capability("b");
    let service_a = Arc::new(RecursiveService { target: b.clone() });
    let service_b = Arc::new(RecursiveService { target: a.clone() });
    struct TwoCapabilityPlugin {
        definition: PluginDefinition,
        a: CapabilityId,
        b: CapabilityId,
        service_a: Arc<dyn CapabilityService>,
        service_b: Arc<dyn CapabilityService>,
    }
    impl Plugin for TwoCapabilityPlugin {
        fn definition(&self) -> &PluginDefinition {
            &self.definition
        }

        fn activate(
            &self,
            context: &mut ActivationContext,
        ) -> Result<Box<dyn PluginRuntime>, PluginError> {
            context.publish_capability(self.a.clone(), Arc::clone(&self.service_a))?;
            context.publish_capability(self.b.clone(), Arc::clone(&self.service_b))?;
            Ok(Box::new(NoopRuntime))
        }
    }
    let mut kernel = Kernel::new();
    let plugin = kernel
        .register(TwoCapabilityPlugin {
            definition: PluginDefinition::new("recursive")
                .provides(a.clone())
                .provides(b.clone()),
            a: a.clone(),
            b: b.clone(),
            service_a,
            service_b,
        })
        .expect("recursive provider must activate");
    let runtime = kernel
        .runtime_id_for_plugin(&plugin)
        .expect("runtime exists");
    let principal = kernel
        .principal_for_runtime(&runtime)
        .expect("runtime principal exists");
    grant(&kernel, &principal, &a.contract(), "loop");
    grant(&kernel, &principal, &b.contract(), "loop");
    let caller = register_principal(&kernel, "recursive-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &a.contract(), "loop");
    let error = kernel
        .capability_for(caller, a)
        .expect("caller handle exists")
        .invoke("loop", b"payload")
        .expect_err("mutual recursion must terminate at depth limit");
    assert!(matches!(error, CapabilityError::InvocationFailed { .. }));
    assert!(kernel.trajectory().into_iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::InvocationDenied {
                reason: worldline_kernel::DenialReason::InvocationDepthExceeded,
                ..
            }
        )
    }));
}

#[test]
fn event_publish_observation_failure_cannot_change_rpc_result() {
    let cap = capability("observation-independent");
    let event = event_contract("observation-independent");
    let mut kernel = Kernel::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let (_, _, provider_principal) = provider(
        &mut kernel,
        "independent-provider",
        cap.clone(),
        Arc::new(EchoService {
            calls: Arc::clone(&calls),
            last_context: Arc::new(Mutex::new(None)),
            event: Some(event.clone()),
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "independent-caller", PrincipalKind::Agent);
    grant(&kernel, &caller, &cap.contract(), "run");
    let subscription_principal =
        register_principal(&kernel, "independent-observer", PrincipalKind::Agent);
    grant(
        &kernel,
        &provider_principal,
        &event.capability_id().contract(),
        "publish",
    );
    grant(
        &kernel,
        &subscription_principal,
        &event.capability_id().contract(),
        "subscribe",
    );
    let _subscription = kernel
        .subscribe(
            subscription_principal,
            event,
            SubscriptionOptions::new(1, OverflowPolicy::RejectForSubscriber),
        )
        .expect("subscription must succeed");
    let result = kernel
        .capability_for(caller, cap)
        .expect("caller handle exists")
        .invoke("run", b"ok")
        .expect("RPC result must not depend on event observer");
    assert_eq!(result, b"ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn trace_context_is_not_authority() {
    let cap = capability("trace-auth");
    let mut kernel = Kernel::new();
    provider(
        &mut kernel,
        "trace-provider",
        cap.clone(),
        Arc::new(EchoService {
            calls: Arc::new(AtomicUsize::new(0)),
            last_context: Arc::new(Mutex::new(None)),
            event: None,
        }),
        ProviderLimits::new(1, 0),
    );
    let caller = register_principal(&kernel, "trace-caller", PrincipalKind::Agent);
    let error = kernel
        .capability_for(caller, cap)
        .expect("handle exists")
        .invoke_with_options(
            "run",
            b"payload",
            RpcCallOptions::new().with_trace_context(TraceContext::new("trusted-looking")),
        )
        .expect_err("trace identity must not authorize an invocation");
    assert!(matches!(
        error,
        CapabilityError::Denied {
            reason: worldline_kernel::DenialReason::NoGrant,
            ..
        }
    ));
}

#[test]
fn event_contract_major_mismatch_is_not_delivered() {
    let contract_v1 = event_contract("versioned");
    let contract_v2 =
        EventContract::new("worldline.test", "versioned", InterfaceVersion::new(2, 0));
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "version-producer", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "version-observer", PrincipalKind::Agent);
    grant(
        &kernel,
        &producer,
        &contract_v1.capability_id().contract(),
        "publish",
    );
    grant(
        &kernel,
        &observer,
        &contract_v2.capability_id().contract(),
        "subscribe",
    );
    let subscription = kernel
        .subscribe(observer, contract_v2, SubscriptionOptions::default())
        .expect("subscription must be authorized");
    let report = kernel
        .publish_event(producer, contract_v1, b"v1", Default::default())
        .expect("publication itself remains valid");
    assert_eq!(report.eligible_subscribers(), 0);
    assert_eq!(subscription.try_recv().expect("receive must work"), None);
}

#[test]
fn event_sequence_continues_across_minor_versions_in_one_major_line() {
    let contract_v1 = event_contract("minor-sequence");
    let contract_v1_1 = EventContract::new(
        "worldline.test",
        "minor-sequence",
        InterfaceVersion::new(1, 1),
    );
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "minor-sequence-producer", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "minor-sequence-observer", PrincipalKind::Agent);
    grant(
        &kernel,
        &producer,
        &contract_v1.capability_id().contract(),
        "publish",
    );
    grant(
        &kernel,
        &producer,
        &contract_v1_1.capability_id().contract(),
        "publish",
    );
    grant(
        &kernel,
        &observer,
        &contract_v1.capability_id().contract(),
        "subscribe",
    );
    let subscription = kernel
        .subscribe(
            observer,
            contract_v1.clone(),
            SubscriptionOptions::default(),
        )
        .expect("subscription must be authorized");
    kernel
        .publish_event(producer.clone(), contract_v1, b"v1.0", Default::default())
        .expect("v1.0 publication must succeed");
    kernel
        .publish_event(producer, contract_v1_1, b"v1.1", Default::default())
        .expect("v1.1 publication must succeed");
    let first = subscription
        .try_recv()
        .expect("first receive")
        .expect("first event");
    let second = subscription
        .try_recv()
        .expect("second receive")
        .expect("second event");
    assert_eq!(first.sequence(), 1);
    assert_eq!(second.sequence(), 2);
}

#[test]
fn event_subscriber_cannot_satisfy_missing_rpc_provider() {
    let event = event_contract("observer-only");
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "observer-only-producer", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "observer-only-observer", PrincipalKind::Agent);
    grant(
        &kernel,
        &producer,
        &event.capability_id().contract(),
        "publish",
    );
    grant(
        &kernel,
        &observer,
        &event.capability_id().contract(),
        "subscribe",
    );
    let _subscription = kernel
        .subscribe(observer, event.clone(), SubscriptionOptions::default())
        .expect("event subscription must be authorized");
    let caller = register_principal(&kernel, "observer-only-caller", PrincipalKind::Agent);
    let missing = CapabilityId::new(
        "worldline.rpc-event",
        "missing-provider",
        InterfaceVersion::new(1, 0),
    );
    grant(&kernel, &caller, &missing.contract(), "run");
    let error = kernel
        .capability_for(caller, missing.clone())
        .expect("handle must exist")
        .invoke("run", b"payload")
        .expect_err("event subscriber cannot become an RPC provider");
    assert!(matches!(
        error,
        CapabilityError::NoCompatibleProvider { capability, .. } if capability == missing
    ));
}

#[test]
fn event_envelope_sequence_is_runtime_local() {
    let contract = event_contract("sequence");
    let cap = contract.capability_id();
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "sequence-producer", PrincipalKind::Agent);
    let observer = register_principal(&kernel, "sequence-observer", PrincipalKind::Agent);
    grant(&kernel, &producer, &cap.contract(), "publish");
    grant(&kernel, &observer, &cap.contract(), "subscribe");
    let subscription = kernel
        .subscribe(observer, contract.clone(), SubscriptionOptions::default())
        .expect("subscription must be authorized");
    kernel
        .publish_event(
            producer.clone(),
            contract.clone(),
            b"one",
            Default::default(),
        )
        .expect("first event");
    kernel
        .publish_event(producer, contract, b"two", Default::default())
        .expect("second event");
    let first = subscription
        .try_recv()
        .expect("receive")
        .expect("first event");
    let second = subscription
        .try_recv()
        .expect("receive")
        .expect("second event");
    assert_eq!(first.sequence() + 1, second.sequence());
}

#[test]
fn cancellation_token_is_idempotent() {
    let token = RpcCancellationToken::new();
    assert!(token.cancel());
    assert!(!token.cancel());
    assert!(token.is_cancelled());
}

#[test]
fn journal_failure_is_explicit() {
    struct FailingJournal;
    impl EventJournal for FailingJournal {
        fn append(
            &self,
            _event: &worldline_kernel::EventEnvelope,
        ) -> Result<(), EventJournalError> {
            Err(EventJournalError::Failure("disk unavailable".to_owned()))
        }

        fn read_from(
            &self,
            _cursor: worldline_kernel::EventCursor,
        ) -> Result<Vec<worldline_kernel::EventEnvelope>, EventJournalError> {
            Ok(Vec::new())
        }
    }

    let contract = event_contract("journal-failure");
    let kernel = Kernel::new();
    let producer = register_principal(&kernel, "journal-failure-producer", PrincipalKind::Agent);
    grant(
        &kernel,
        &producer,
        &contract.capability_id().contract(),
        "publish",
    );
    kernel.set_event_journal(Arc::new(FailingJournal));
    let error = kernel
        .publish_event(
            producer,
            contract,
            b"payload",
            worldline_kernel::EventPublishOptions::default().durable(),
        )
        .expect_err("journal failure must be visible");
    assert!(matches!(
        error,
        worldline_kernel::EventError::EventJournalFailure { .. }
    ));
}
