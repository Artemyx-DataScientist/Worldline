use std::{
    cell::RefCell,
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use crate::{
    CapabilityError, CapabilityId, InvocationId, InvocationRequest, OperationId, PrincipalId,
    ResourceId,
    capability::CapabilityRegistry,
    error::panic_message,
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
    broker: Arc<InvocationBroker>,
}

impl CapabilityHandle {
    pub(crate) fn new(
        required: CapabilityId,
        caller: PrincipalId,
        broker: Arc<InvocationBroker>,
    ) -> Self {
        Self {
            required,
            caller,
            broker,
        }
    }

    pub fn id(&self) -> &CapabilityId {
        &self.required
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub fn is_available(&self) -> bool {
        self.broker.registry.has_provider(&self.required)
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
        let request = InvocationRequest::new(
            self.caller.clone(),
            self.required.clone(),
            operation,
            resource,
            payload,
        );
        self.broker.invoke(request)
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
        .with_authority(AuthoritySource::Delegated(authority));
        self.broker.invoke(request)
    }
}

impl std::fmt::Debug for CapabilityHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapabilityHandle")
            .field("required", &self.required)
            .field("caller", &self.caller)
            .finish_non_exhaustive()
    }
}

/// Trusted context supplied by the invocation broker to a provider.
pub struct InvocationContext {
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
    broker: Arc<InvocationBroker>,
}

impl InvocationContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
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
        broker: Arc<InvocationBroker>,
    ) -> Self {
        Self {
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
            broker,
        }
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
}

impl InvocationBroker {
    pub(crate) fn new(
        registry: Arc<CapabilityRegistry>,
        security: Arc<SecurityStore>,
        trajectory: Trajectory,
    ) -> Self {
        Self {
            registry,
            security,
            trajectory,
        }
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
        ) = request.into_parts();
        let invocation = self.security.allocate_invocation();
        let contract = capability.contract();
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

        let Some(resolved) = self.registry.resolve(&capability, &BTreeSet::new()) else {
            self.trajectory
                .push_security(TrajectoryEventKind::InvocationFailed {
                    invocation,
                    causal_parent,
                });
            return Err(CapabilityError::Unavailable { capability });
        };
        let context = InvocationContext::new(
            invocation.clone(),
            caller.clone(),
            resolved.descriptor.principal().clone(),
            resolved.descriptor.runtime_id(),
            capability.clone(),
            operation.clone(),
            resource.clone(),
            authority,
            causal_parent.clone(),
            nested_depth,
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

        let _frame = InvocationFrameGuard::enter(
            resolved.descriptor.runtime_id(),
            resolved.descriptor.principal().clone(),
        );
        let result = catch_unwind(AssertUnwindSafe(|| {
            resolved.service.invoke_with_context(&context, &payload)
        }));
        match result {
            Ok(Ok(response)) => {
                self.trajectory
                    .push_security(TrajectoryEventKind::InvocationCompleted {
                        invocation,
                        causal_parent: context.causal_parent.clone(),
                    });
                Ok(response)
            }
            Ok(Err(message)) => {
                self.trajectory
                    .push_security(TrajectoryEventKind::InvocationFailed {
                        invocation,
                        causal_parent: context.causal_parent.clone(),
                    });
                Err(CapabilityError::InvocationFailed {
                    capability,
                    message,
                })
            }
            Err(payload) => {
                self.trajectory
                    .push_security(TrajectoryEventKind::InvocationFailed {
                        invocation,
                        causal_parent: context.causal_parent.clone(),
                    });
                Err(CapabilityError::InvocationFailed {
                    capability,
                    message: format!(
                        "provider invocation panicked: {}",
                        panic_message(payload.as_ref())
                    ),
                })
            }
        }
    }
}
