use std::{
    cell::RefCell,
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    CapabilityError, CapabilityId, CapabilityTarget, CausationRef, CorrelationId, EventContract,
    EventError, EventPublishOptions, InvocationId, InvocationRequest, OperationId, PrincipalId,
    PublishReport, ResourceId, RpcCallOptions, RpcCancellationToken, RpcOperationContract,
    RpcOutcomeClass, RpcRequestId, RpcRetryClass, TraceContext,
    capability::CapabilityRegistry,
    error::panic_message,
    events::EventTransport,
    events::InvocationCompletedMetadata,
    rpc::{FlowFailure, ProviderFlowControl, ProviderLimits},
    runtime::RuntimeId,
    security::{AuthoritySet, AuthoritySource, SecurityStore},
    trajectory::{Trajectory, TrajectoryEventKind},
};

/// Maximum number of nested invocation edges admitted on one causal call
/// stack. The root invocation has depth zero, so it may be followed by this
/// many nested invocations.
pub const MAX_NESTED_INVOCATION_DEPTH: usize = 32;

#[derive(Clone)]
struct InvocationFrame {
    runtime_id: RuntimeId,
    principal: PrincipalId,
}

thread_local! {
    static INVOCATION_FRAMES: RefCell<Vec<InvocationFrame>> = const { RefCell::new(Vec::new()) };
}

struct InvocationFrameGuard;

impl InvocationFrameGuard {
    fn current_depth() -> usize {
        INVOCATION_FRAMES.with(|frames| frames.borrow().len())
    }

    fn current_provider() -> Option<InvocationFrame> {
        INVOCATION_FRAMES.with(|frames| frames.borrow().last().cloned())
    }

    fn contains_runtime(runtime_id: RuntimeId) -> bool {
        INVOCATION_FRAMES.with(|frames| {
            frames
                .borrow()
                .iter()
                .any(|frame| frame.runtime_id == runtime_id)
        })
    }

    fn enter(runtime_id: RuntimeId, principal: PrincipalId) -> Self {
        INVOCATION_FRAMES.with(|frames| {
            frames.borrow_mut().push(InvocationFrame {
                runtime_id,
                principal,
            })
        });
        Self
    }
}

impl Drop for InvocationFrameGuard {
    fn drop(&mut self) {
        INVOCATION_FRAMES.with(|frames| {
            let _ = frames.borrow_mut().pop();
        });
    }
}

/// Consumer-facing proxy for a capability. The proxy carries a kernel-owned
/// caller identity and broker reference, never a provider service reference.
#[derive(Clone)]
pub struct CapabilityHandle {
    required: CapabilityId,
    caller: PrincipalId,
    target: CapabilityTarget,
    broker: Arc<InvocationBroker>,
}

impl CapabilityHandle {
    pub(crate) fn new(
        required: CapabilityId,
        caller: PrincipalId,
        broker: Arc<InvocationBroker>,
    ) -> Self {
        Self::targeted(required, caller, CapabilityTarget::AnyCompatible, broker)
    }

    pub(crate) fn targeted(
        required: CapabilityId,
        caller: PrincipalId,
        target: CapabilityTarget,
        broker: Arc<InvocationBroker>,
    ) -> Self {
        Self {
            required,
            caller,
            target,
            broker,
        }
    }

    pub fn id(&self) -> &CapabilityId {
        &self.required
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub fn target(&self) -> &CapabilityTarget {
        &self.target
    }

    pub fn is_available(&self) -> bool {
        self.broker
            .registry
            .has_targeted_provider(&self.required, &self.target)
    }

    /// Invokes against the capability's namespace root resource.
    ///
    /// Authorization is checked at admission time, before provider resolution
    /// and execution. Revoking a grant later does not cancel an invocation
    /// that has already been admitted and is still in flight.
    pub fn invoke(
        &self,
        operation: impl Into<OperationId>,
        payload: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        self.invoke_with_resource(
            operation,
            ResourceId::root(self.required.namespace()),
            payload,
        )
    }

    pub fn invoke_with_resource(
        &self,
        operation: impl Into<OperationId>,
        resource: impl Into<ResourceId>,
        payload: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        self.invoke_with_resource_options(operation, resource, payload, RpcCallOptions::default())
    }

    pub fn invoke_with_options(
        &self,
        operation: impl Into<OperationId>,
        payload: &[u8],
        options: RpcCallOptions,
    ) -> Result<Vec<u8>, CapabilityError> {
        self.invoke_with_resource_options(
            operation,
            ResourceId::root(self.required.namespace()),
            payload,
            options,
        )
    }

    pub fn invoke_with_resource_options(
        &self,
        operation: impl Into<OperationId>,
        resource: impl Into<ResourceId>,
        payload: &[u8],
        options: RpcCallOptions,
    ) -> Result<Vec<u8>, CapabilityError> {
        let request = request_with_options(
            self.caller.clone(),
            self.required.clone(),
            self.target.clone(),
            operation,
            resource,
            payload,
            options,
        );
        self.broker.invoke(request)
    }

    /// Sends a request through this handle while replacing its caller and
    /// capability fields with the handle's trusted identity.
    pub fn invoke_request(&self, request: InvocationRequest) -> Result<Vec<u8>, CapabilityError> {
        self.broker.invoke(request.with_handle_identity(
            self.caller.clone(),
            self.required.clone(),
            self.target.clone(),
        ))
    }

    pub fn invoke_with_authority(
        &self,
        operation: impl Into<OperationId>,
        resource: impl Into<ResourceId>,
        authority: AuthoritySet,
        payload: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        let request = InvocationRequest::new(
            self.caller.clone(),
            self.required.clone(),
            operation,
            resource,
            payload,
        )
        .with_target(self.target.clone())
        .with_authority(AuthoritySource::Delegated(authority));
        self.broker.invoke(request)
    }
}

fn request_with_options(
    caller: PrincipalId,
    capability: CapabilityId,
    target: CapabilityTarget,
    operation: impl Into<OperationId>,
    resource: impl Into<ResourceId>,
    payload: &[u8],
    options: RpcCallOptions,
) -> InvocationRequest {
    let (request_id, deadline, cancellation, retry_class, retry, idempotency_key, trace_context) =
        options.into_parts();
    let mut request = InvocationRequest::new(caller, capability, operation, resource, payload)
        .with_target(target)
        .with_cancellation(cancellation)
        .with_retry_classification(retry_class);
    if let Some(request_id) = request_id {
        request = request.with_rpc_request_id(request_id);
    }
    if let Some(deadline) = deadline {
        request = request.with_deadline(deadline);
    } else {
        request = request.with_no_deadline();
    }
    if retry {
        request = request.with_retry();
    }
    if let Some(idempotency_key) = idempotency_key {
        request = request.with_idempotency_key(idempotency_key);
    }
    if let Some(trace_context) = trace_context {
        request = request.with_trace_context(trace_context);
    }
    request
}

impl std::fmt::Debug for CapabilityHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapabilityHandle")
            .field("required", &self.required)
            .field("caller", &self.caller)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Trusted context supplied by the invocation broker to a provider.
pub struct InvocationContext {
    rpc_request_id: RpcRequestId,
    invocation_id: InvocationId,
    caller: PrincipalId,
    provider: PrincipalId,
    provider_runtime_id: RuntimeId,
    capability: CapabilityId,
    operation: OperationId,
    resource: ResourceId,
    authority: AuthoritySet,
    causal_parent: Option<InvocationId>,
    nested_depth: usize,
    deadline: Option<Duration>,
    cancellation: RpcCancellationToken,
    retry_class: RpcRetryClass,
    idempotency_key: Option<String>,
    correlation_id: CorrelationId,
    broker: Arc<InvocationBroker>,
}

impl InvocationContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        rpc_request_id: RpcRequestId,
        invocation_id: InvocationId,
        caller: PrincipalId,
        provider: PrincipalId,
        provider_runtime_id: RuntimeId,
        capability: CapabilityId,
        operation: OperationId,
        resource: ResourceId,
        authority: AuthoritySet,
        causal_parent: Option<InvocationId>,
        nested_depth: usize,
        deadline: Option<Duration>,
        cancellation: RpcCancellationToken,
        retry_class: RpcRetryClass,
        idempotency_key: Option<String>,
        correlation_id: CorrelationId,
        broker: Arc<InvocationBroker>,
    ) -> Self {
        Self {
            rpc_request_id,
            invocation_id,
            caller,
            provider,
            provider_runtime_id,
            capability,
            operation,
            resource,
            authority,
            causal_parent,
            nested_depth,
            deadline,
            cancellation,
            retry_class,
            idempotency_key,
            correlation_id,
            broker,
        }
    }

    pub fn rpc_request_id(&self) -> &RpcRequestId {
        &self.rpc_request_id
    }

    pub fn request_id(&self) -> &RpcRequestId {
        self.rpc_request_id()
    }

    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub fn provider(&self) -> &PrincipalId {
        &self.provider
    }

    pub const fn provider_runtime_id(&self) -> RuntimeId {
        self.provider_runtime_id
    }

    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    pub fn operation(&self) -> &OperationId {
        &self.operation
    }

    pub fn resource(&self) -> &ResourceId {
        &self.resource
    }

    pub fn authority(&self) -> &AuthoritySet {
        &self.authority
    }

    pub fn causal_parent(&self) -> Option<&InvocationId> {
        self.causal_parent.as_ref()
    }

    pub const fn nested_depth(&self) -> usize {
        self.nested_depth
    }

    pub const fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    pub fn cancellation(&self) -> RpcCancellationToken {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub const fn retry_classification(&self) -> RpcRetryClass {
        self.retry_class
    }

    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// Provider-side trace always points at this concrete invocation attempt.
    pub fn trace_context(&self) -> TraceContext {
        TraceContext::new(self.correlation_id.clone())
            .with_causation(CausationRef::Invocation(self.invocation_id.clone()))
    }

    /// Publishes a separately authorized observation.  The provider identity
    /// and runtime are supplied by the kernel context, not by event payload.
    pub fn publish_event(
        &self,
        contract: EventContract,
        payload: &[u8],
        options: EventPublishOptions,
    ) -> Result<PublishReport, EventError> {
        self.broker.events.publish(
            self.provider.clone(),
            Some(self.provider_runtime_id),
            contract,
            payload,
            options.with_trace_context(self.trace_context()),
            false,
        )
    }

    /// Invoke another capability using only authority delegated into this
    /// invocation. No provider self-authority fallback is attempted.
    pub fn invoke_delegated(
        &self,
        capability: impl Into<CapabilityId>,
        operation: impl Into<OperationId>,
        resource: impl Into<ResourceId>,
        payload: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        let request = InvocationRequest::new(
            self.caller.clone(),
            capability,
            operation,
            resource,
            payload,
        )
        .with_authority(AuthoritySource::Delegated(self.authority.clone()))
        .with_trace_context(self.trace_context())
        .with_causal_parent(self.invocation_id.clone())
        .with_provider_runtime(self.provider_runtime_id)
        .with_nested_depth(self.nested_depth.saturating_add(1));
        self.broker.invoke(request)
    }

    /// Invoke another capability using only grants belonging to this
    /// provider's principal. The current invocation remains the causal parent.
    pub fn invoke_self(
        &self,
        capability: impl Into<CapabilityId>,
        operation: impl Into<OperationId>,
        resource: impl Into<ResourceId>,
        payload: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        let request = InvocationRequest::new(
            self.provider.clone(),
            capability,
            operation,
            resource,
            payload,
        )
        .with_authority(AuthoritySource::ProviderSelf(self.provider.clone()))
        .with_trace_context(self.trace_context())
        .with_causal_parent(self.invocation_id.clone())
        .with_provider_runtime(self.provider_runtime_id)
        .with_nested_depth(self.nested_depth.saturating_add(1));
        self.broker.invoke(request)
    }
}

/// Kernel-owned invocation boundary.
pub(crate) struct InvocationBroker {
    pub(crate) registry: Arc<CapabilityRegistry>,
    pub(crate) security: Arc<SecurityStore>,
    pub(crate) trajectory: Trajectory,
    pub(crate) flow: Arc<ProviderFlowControl>,
    pub(crate) events: EventTransport,
}

impl InvocationBroker {
    pub(crate) fn new(
        registry: Arc<CapabilityRegistry>,
        security: Arc<SecurityStore>,
        trajectory: Trajectory,
        events: EventTransport,
    ) -> Self {
        Self {
            registry,
            security,
            trajectory,
            flow: Arc::new(ProviderFlowControl::default()),
            events,
        }
    }

    pub(crate) fn register_provider(
        &self,
        runtime_id: RuntimeId,
        capability: CapabilityId,
        limits: ProviderLimits,
    ) {
        self.flow.register(runtime_id, capability, limits);
    }

    pub(crate) fn unregister_provider(&self, runtime_id: RuntimeId) {
        self.flow.unregister(runtime_id);
    }

    /// Authorization is admission-time: once this boundary admits a request,
    /// later grant revocation does not cancel the provider call already in
    /// flight. Revocation applies to subsequent admissions.
    pub(crate) fn invoke(
        self: &Arc<Self>,
        request: InvocationRequest,
    ) -> Result<Vec<u8>, CapabilityError> {
        let (
            caller,
            capability,
            operation,
            resource,
            authority_source,
            payload,
            causal_parent,
            requested_depth,
            request_provider_runtime,
            supplied_request_id,
            deadline,
            cancellation,
            requested_retry,
            retry,
            idempotency_key,
            supplied_trace,
            target,
        ) = request.into_parts();
        let invocation = self.security.allocate_invocation();
        let request_id = supplied_request_id
            .clone()
            .unwrap_or_else(|| self.security.allocate_rpc_request());
        let correlation_id = supplied_trace
            .as_ref()
            .map(|trace| trace.correlation_id().clone())
            .unwrap_or_else(|| self.security.allocate_correlation());
        let trace_context = match (supplied_trace, causal_parent.clone()) {
            (Some(trace), Some(parent)) if trace.causation().is_none() => {
                TraceContext::new(trace.correlation_id().clone())
                    .with_causation(CausationRef::Invocation(parent))
            }
            (Some(trace), _) => trace,
            (None, Some(parent)) => TraceContext::new(correlation_id.clone())
                .with_causation(CausationRef::Invocation(parent)),
            (None, None) => TraceContext::new(correlation_id.clone()),
        };
        let contract = capability.contract();
        self.trajectory
            .push_security(TrajectoryEventKind::RpcRequestCreated {
                request_id: request_id.clone(),
                invocation: invocation.clone(),
                caller: caller.clone(),
                capability: contract.clone(),
                operation: operation.clone(),
                correlation_id: trace_context.correlation_id().clone(),
                causation: trace_context.causation().cloned(),
                retry_class: requested_retry,
            });
        let started_at = Instant::now();
        let deadline_at = deadline.map(|duration| started_at + duration);

        if retry && supplied_request_id.is_none() {
            return Err(CapabilityError::RpcInvalidRetry {
                request_id,
                invocation,
                requested: requested_retry,
                declared: RpcRetryClass::NeverRetry,
            });
        }
        if cancellation.is_cancelled() {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCancelled {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            return Err(CapabilityError::RpcCancelled {
                request_id,
                invocation,
            });
        }
        if deadline_at.is_some_and(|limit| Instant::now() >= limit) {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcDeadlineExceeded {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            return Err(CapabilityError::RpcDeadlineExceeded {
                request_id,
                invocation,
            });
        }

        let enclosing_provider = InvocationFrameGuard::current_provider();
        let nested_depth = InvocationFrameGuard::current_depth().max(requested_depth);
        if nested_depth > MAX_NESTED_INVOCATION_DEPTH {
            let reason = crate::DenialReason::InvocationDepthExceeded;
            self.trajectory
                .push_security(TrajectoryEventKind::InvocationDenied {
                    invocation: invocation.clone(),
                    caller: caller.clone(),
                    capability: contract,
                    operation: operation.clone(),
                    resource: resource.clone(),
                    reason,
                    causal_parent,
                });
            return Err(CapabilityError::Denied {
                invocation,
                caller,
                capability: Box::new(capability),
                operation,
                resource: Box::new(resource),
                reason,
            });
        }

        let authority = match self.security.authorize(
            &caller,
            &capability,
            &operation,
            &resource,
            &authority_source,
            enclosing_provider.as_ref().map(|frame| &frame.principal),
            enclosing_provider.as_ref().map(|frame| &frame.runtime_id),
            request_provider_runtime,
        ) {
            Ok(authority) => authority,
            Err(reason) => {
                self.trajectory
                    .push_security(TrajectoryEventKind::InvocationDenied {
                        invocation: invocation.clone(),
                        caller: caller.clone(),
                        capability: contract.clone(),
                        operation: operation.clone(),
                        resource: resource.clone(),
                        reason,
                        causal_parent: causal_parent.clone(),
                    });
                return Err(CapabilityError::Denied {
                    invocation,
                    caller,
                    capability: Box::new(capability),
                    operation,
                    resource: Box::new(resource),
                    reason,
                });
            }
        };
        if cancellation.is_cancelled() {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCancelled {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            return Err(CapabilityError::RpcCancelled {
                request_id,
                invocation,
            });
        }
        if deadline_at.is_some_and(|limit| Instant::now() >= limit) {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcDeadlineExceeded {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            return Err(CapabilityError::RpcDeadlineExceeded {
                request_id,
                invocation,
            });
        }
        self.trajectory
            .push_security(TrajectoryEventKind::InvocationAuthorized {
                invocation: invocation.clone(),
                caller: caller.clone(),
                capability: contract.clone(),
                operation: operation.clone(),
                resource: resource.clone(),
                authority: authority.clone(),
                causal_parent: causal_parent.clone(),
            });

        let (resolved, selection_diag) =
            self.registry
                .selection_target(&capability, &target, &BTreeSet::new());
        self.trajectory
            .push_security(TrajectoryEventKind::CapabilityProviderSelected {
                request_id: request_id.clone(),
                invocation: invocation.clone(),
                capability: contract.clone(),
                target_installation: target.target_installation().cloned(),
                selected_installation: resolved
                    .as_ref()
                    .map(|p| p.descriptor.installation_id().clone()),
                selected_runtime: resolved.as_ref().map(|p| p.descriptor.runtime_id()),
                candidate_count: selection_diag.compatible_candidate_count(),
                policy: selection_diag.policy().to_owned(),
                outcome: if resolved.is_some() {
                    "Selected".to_owned()
                } else {
                    "Unavailable".to_owned()
                },
            });

        let Some(resolved) = resolved else {
            self.trajectory
                .push_security(TrajectoryEventKind::InvocationFailed {
                    invocation: invocation.clone(),
                    causal_parent: causal_parent.clone(),
                });
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCompleted {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id: None,
                    outcome: RpcOutcomeClass::NoCompatibleProvider,
                });
            return Err(match target {
                CapabilityTarget::AnyCompatible => CapabilityError::NoCompatibleProvider {
                    request_id,
                    invocation,
                    capability,
                },
                CapabilityTarget::Installation(target_installation) => {
                    CapabilityError::TargetUnavailable {
                        request_id,
                        invocation,
                        capability: Box::new(capability),
                        target: target_installation,
                    }
                }
            });
        };
        let declared_contract: RpcOperationContract =
            resolved.service.rpc_operation_contract(&operation);
        if requested_retry.rank() > declared_contract.retry_class().rank()
            || (retry && matches!(declared_contract.retry_class(), RpcRetryClass::NeverRetry))
        {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCompleted {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id: Some(resolved.descriptor.runtime_id()),
                    outcome: RpcOutcomeClass::InvalidRetryClassification,
                });
            return Err(CapabilityError::RpcInvalidRetry {
                request_id,
                invocation,
                requested: requested_retry,
                declared: declared_contract.retry_class(),
            });
        }
        if retry
            && (matches!(declared_contract.retry_class(), RpcRetryClass::Idempotent)
                || matches!(requested_retry, RpcRetryClass::Idempotent)
                || declared_contract.idempotency_key_required())
            && idempotency_key.as_deref().is_none_or(str::is_empty)
        {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCompleted {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id: Some(resolved.descriptor.runtime_id()),
                    outcome: RpcOutcomeClass::InvalidIdempotencyKey,
                });
            return Err(CapabilityError::RpcMissingIdempotencyKey {
                request_id,
                invocation,
            });
        }
        let effective_retry = declared_contract.retry_class();
        if deadline_at.is_some_and(|limit| Instant::now() >= limit) {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcDeadlineExceeded {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            return Err(CapabilityError::RpcDeadlineExceeded {
                request_id,
                invocation,
            });
        }

        let runtime_id = resolved.descriptor.runtime_id();
        let provider_capability = resolved.descriptor.capability().clone();
        // A synchronous causal cycle can revisit a runtime several edges
        // below its original frame (A -> B -> A). Treat that nested edge as
        // reentrant so flow control cannot deadlock before the independent
        // admission-depth guard terminates the cycle.
        let reentrant = InvocationFrameGuard::contains_runtime(runtime_id);
        if !reentrant && self.flow.is_saturated(runtime_id, &provider_capability) {
            self.trajectory
                .push_security(TrajectoryEventKind::RpcQueued {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id,
                    queue_depth: self.flow.queue_depth(runtime_id, &provider_capability),
                });
        }
        let permit = match self.flow.acquire(
            runtime_id,
            &provider_capability,
            deadline_at,
            &cancellation,
            reentrant,
        ) {
            Ok(permit) => permit,
            Err(failure) => {
                let (outcome, error) =
                    flow_failure(failure, request_id.clone(), invocation.clone(), runtime_id);
                if matches!(failure, FlowFailure::ProviderBusy | FlowFailure::QueueFull) {
                    self.trajectory
                        .push_security(TrajectoryEventKind::RpcBackpressured {
                            request_id: request_id.clone(),
                            invocation: invocation.clone(),
                            runtime_id,
                            outcome,
                        });
                } else if matches!(failure, FlowFailure::Cancelled) {
                    self.trajectory
                        .push_security(TrajectoryEventKind::RpcCancelled {
                            request_id: request_id.clone(),
                            invocation: invocation.clone(),
                        });
                } else if matches!(failure, FlowFailure::DeadlineExceeded) {
                    self.trajectory
                        .push_security(TrajectoryEventKind::RpcDeadlineExceeded {
                            request_id: request_id.clone(),
                            invocation: invocation.clone(),
                        });
                }
                self.trajectory
                    .push_security(TrajectoryEventKind::RpcCompleted {
                        request_id: request_id.clone(),
                        invocation: invocation.clone(),
                        runtime_id: Some(runtime_id),
                        outcome,
                    });
                return Err(error);
            }
        };
        if cancellation.is_cancelled() {
            drop(permit);
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCancelled {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCompleted {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id: Some(runtime_id),
                    outcome: RpcOutcomeClass::Cancelled,
                });
            return Err(CapabilityError::RpcCancelled {
                request_id,
                invocation,
            });
        }
        if deadline_at.is_some_and(|limit| Instant::now() >= limit) {
            drop(permit);
            self.trajectory
                .push_security(TrajectoryEventKind::RpcDeadlineExceeded {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCompleted {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id: Some(runtime_id),
                    outcome: RpcOutcomeClass::DeadlineExceeded,
                });
            return Err(CapabilityError::RpcDeadlineExceeded {
                request_id,
                invocation,
            });
        }

        self.trajectory
            .push_security(TrajectoryEventKind::RpcDispatched {
                request_id: request_id.clone(),
                invocation: invocation.clone(),
                runtime_id,
            });
        let context = InvocationContext::new(
            request_id.clone(),
            invocation.clone(),
            caller.clone(),
            resolved.descriptor.principal().clone(),
            runtime_id,
            capability.clone(),
            operation.clone(),
            resource.clone(),
            authority,
            causal_parent.clone(),
            nested_depth,
            deadline,
            cancellation.clone(),
            effective_retry,
            idempotency_key,
            trace_context.correlation_id().clone(),
            Arc::clone(self),
        );
        self.trajectory
            .push_security(TrajectoryEventKind::InvocationStarted {
                invocation: invocation.clone(),
                caller,
                provider: resolved.descriptor.principal().clone(),
                capability: contract,
                operation,
                resource,
                payload_size: payload.len(),
                causal_parent,
            });

        let _frame =
            InvocationFrameGuard::enter(runtime_id, resolved.descriptor.principal().clone());
        let provider_result = catch_unwind(AssertUnwindSafe(|| {
            resolved.service.invoke_with_context(&context, &payload)
        }));
        let late_cancellation = cancellation.is_cancelled();
        let late_deadline = deadline_at.is_some_and(|limit| Instant::now() >= limit);
        let provider = resolved.descriptor.principal().clone();
        let result = if late_cancellation {
            self.trajectory
                .push_security(TrajectoryEventKind::InvocationFailed {
                    invocation: invocation.clone(),
                    causal_parent: context.causal_parent.clone(),
                });
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCancelled {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCompleted {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id: Some(runtime_id),
                    outcome: RpcOutcomeClass::Cancelled,
                });
            self.publish_invocation_completed(
                &provider,
                runtime_id,
                &context,
                RpcOutcomeClass::Cancelled,
            );
            Err(CapabilityError::RpcCancelled {
                request_id: request_id.clone(),
                invocation: invocation.clone(),
            })
        } else if late_deadline {
            self.trajectory
                .push_security(TrajectoryEventKind::InvocationFailed {
                    invocation: invocation.clone(),
                    causal_parent: context.causal_parent.clone(),
                });
            self.trajectory
                .push_security(TrajectoryEventKind::RpcDeadlineExceeded {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                });
            self.trajectory
                .push_security(TrajectoryEventKind::RpcCompleted {
                    request_id: request_id.clone(),
                    invocation: invocation.clone(),
                    runtime_id: Some(runtime_id),
                    outcome: RpcOutcomeClass::DeadlineExceeded,
                });
            self.publish_invocation_completed(
                &provider,
                runtime_id,
                &context,
                RpcOutcomeClass::DeadlineExceeded,
            );
            Err(CapabilityError::RpcDeadlineExceeded {
                request_id: request_id.clone(),
                invocation: invocation.clone(),
            })
        } else {
            match provider_result {
                Ok(Ok(response)) => {
                    self.trajectory
                        .push_security(TrajectoryEventKind::InvocationCompleted {
                            invocation: invocation.clone(),
                            causal_parent: context.causal_parent.clone(),
                        });
                    self.trajectory
                        .push_security(TrajectoryEventKind::RpcCompleted {
                            request_id: request_id.clone(),
                            invocation: invocation.clone(),
                            runtime_id: Some(runtime_id),
                            outcome: RpcOutcomeClass::Success,
                        });
                    self.publish_invocation_completed(
                        &provider,
                        runtime_id,
                        &context,
                        RpcOutcomeClass::Success,
                    );
                    Ok(response)
                }
                Ok(Err(message)) => {
                    self.trajectory
                        .push_security(TrajectoryEventKind::InvocationFailed {
                            invocation: invocation.clone(),
                            causal_parent: context.causal_parent.clone(),
                        });
                    self.trajectory
                        .push_security(TrajectoryEventKind::RpcCompleted {
                            request_id: request_id.clone(),
                            invocation: invocation.clone(),
                            runtime_id: Some(runtime_id),
                            outcome: RpcOutcomeClass::ProviderReturnedError,
                        });
                    self.publish_invocation_completed(
                        &provider,
                        runtime_id,
                        &context,
                        RpcOutcomeClass::ProviderReturnedError,
                    );
                    Err(CapabilityError::InvocationFailed {
                        capability,
                        message,
                    })
                }
                Err(payload) => {
                    self.trajectory
                        .push_security(TrajectoryEventKind::InvocationFailed {
                            invocation: invocation.clone(),
                            causal_parent: context.causal_parent.clone(),
                        });
                    self.trajectory
                        .push_security(TrajectoryEventKind::RpcCompleted {
                            request_id: request_id.clone(),
                            invocation: invocation.clone(),
                            runtime_id: Some(runtime_id),
                            outcome: RpcOutcomeClass::ProviderCrashed,
                        });
                    self.publish_invocation_completed(
                        &provider,
                        runtime_id,
                        &context,
                        RpcOutcomeClass::ProviderCrashed,
                    );
                    Err(CapabilityError::InvocationFailed {
                        capability,
                        message: format!(
                            "provider invocation panicked: {}",
                            panic_message(payload.as_ref())
                        ),
                    })
                }
            }
        };
        drop(permit);
        result
    }

    fn publish_invocation_completed(
        &self,
        provider: &PrincipalId,
        runtime_id: RuntimeId,
        context: &InvocationContext,
        outcome: RpcOutcomeClass,
    ) {
        let options = EventPublishOptions::default().with_trace_context(context.trace_context());
        let metadata = InvocationCompletedMetadata::new(
            context.rpc_request_id().clone(),
            context.invocation_id().clone(),
            context.caller().clone(),
            runtime_id,
            context.capability().contract(),
            context.operation().clone(),
            outcome,
        );
        let _ = self.events.publish_invocation_completed(
            provider.clone(),
            runtime_id,
            metadata,
            options,
        );
    }
}

fn flow_failure(
    failure: FlowFailure,
    request_id: RpcRequestId,
    invocation: InvocationId,
    runtime: RuntimeId,
) -> (RpcOutcomeClass, CapabilityError) {
    match failure {
        FlowFailure::ProviderBusy => (
            RpcOutcomeClass::ProviderBusy,
            CapabilityError::RpcProviderBusy {
                request_id,
                invocation,
                runtime,
            },
        ),
        FlowFailure::QueueFull => (
            RpcOutcomeClass::QueueFull,
            CapabilityError::RpcQueueFull {
                request_id,
                invocation,
                runtime,
            },
        ),
        FlowFailure::DeadlineExceeded => (
            RpcOutcomeClass::DeadlineExceeded,
            CapabilityError::RpcDeadlineExceeded {
                request_id,
                invocation,
            },
        ),
        FlowFailure::Cancelled => (
            RpcOutcomeClass::Cancelled,
            CapabilityError::RpcCancelled {
                request_id,
                invocation,
            },
        ),
        FlowFailure::RuntimeUnavailable => (
            RpcOutcomeClass::RuntimeUnavailable,
            CapabilityError::RpcRuntimeUnavailable {
                request_id,
                invocation,
                runtime,
            },
        ),
    }
}
