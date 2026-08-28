use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

use worldline_kernel::{
    ActivationContext, CapabilityError, CapabilityId, CapabilityService, DenialReason,
    GrantLifetime, InterfaceVersion, Kernel, NoopRuntime, Plugin, PluginDefinition, PluginError,
    PluginId, PluginRuntime, Principal, PrincipalId, PrincipalKind, ResourceId, ResourceScope,
    RuntimeState, TrajectoryEventKind,
};

fn document_capability() -> CapabilityId {
    CapabilityId::new(
        "worldline.security",
        "document",
        InterfaceVersion::new(1, 0),
    )
}

fn storage_capability() -> CapabilityId {
    CapabilityId::new("worldline.security", "storage", InterfaceVersion::new(1, 0))
}

fn resource(namespace: &str, segments: &[&str]) -> ResourceId {
    ResourceId::new(namespace, segments.iter().copied())
}

struct CountingService {
    calls: Arc<AtomicUsize>,
}

impl CapabilityService for CountingService {
    fn invoke(&self, operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(operation.as_bytes().to_vec())
    }
}

struct ProviderPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<dyn CapabilityService>,
}

struct DependencyConsumer {
    definition: PluginDefinition,
    capability: CapabilityId,
    handle: Arc<Mutex<Option<worldline_kernel::CapabilityHandle>>>,
}

impl Plugin for DependencyConsumer {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        let handle = context
            .capability(&self.capability)
            .map_err(|error| PluginError::new(error.to_string()))?;
        *self.handle.lock().expect("handle lock is not poisoned") = Some(handle);
        Ok(Box::new(NoopRuntime))
    }
}

impl Plugin for ProviderPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        context.publish_capability(self.capability.clone(), Arc::clone(&self.service))?;
        Ok(Box::new(NoopRuntime))
    }
}

fn register_provider(
    kernel: &mut Kernel,
    name: &'static str,
    capability: &CapabilityId,
    service: Arc<dyn CapabilityService>,
) -> PluginId {
    kernel
        .register(ProviderPlugin {
            definition: PluginDefinition::new(name).provides(capability.clone()),
            capability: capability.clone(),
            service,
        })
        .expect("provider registration must succeed")
}

fn register_principal(kernel: &Kernel, id: &str, kind: PrincipalKind) -> PrincipalId {
    let principal = PrincipalId::new(id);
    kernel
        .register_principal(Principal::new(principal.clone(), kind))
        .expect("principal registration must succeed");
    principal
}

#[test]
fn resolved_dependency_does_not_create_authority() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let handle = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();
    let consumer = kernel
        .register(DependencyConsumer {
            definition: PluginDefinition::new("consumer").requires(capability.clone()),
            capability: capability.clone(),
            handle: Arc::clone(&handle),
        })
        .expect("consumer registration must succeed");
    let provider = register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CountingService {
            calls: Arc::clone(&calls),
        }),
    );
    assert_eq!(kernel.plugin_state(&consumer), Some(RuntimeState::Active));
    assert_eq!(kernel.plugin_state(&provider), Some(RuntimeState::Active));
    let error = handle
        .lock()
        .expect("handle lock is not poisoned")
        .as_ref()
        .expect("resolved dependency must produce a handle")
        .invoke("read", b"payload")
        .expect_err("dependency resolution must not create a grant");
    assert_eq!(error.denial_reason(), Some(DenialReason::NoGrant));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

fn root_grant(
    kernel: &Kernel,
    subject: &PrincipalId,
    capability: &CapabilityId,
    operations: &[&str],
    scope: ResourceScope,
    delegable: bool,
) -> worldline_kernel::GrantId {
    kernel
        .create_root_grant(
            subject.clone(),
            capability.contract(),
            operations.iter().copied(),
            scope,
            delegable,
            GrantLifetime::Persistent,
        )
        .expect("root grant must be created")
}

#[test]
fn available_but_unauthorized_is_denied_before_provider() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CountingService {
            calls: Arc::clone(&calls),
        }),
    );

    let handle = kernel
        .capability_for(agent, capability.clone())
        .expect("registered principal must receive a handle");
    assert!(handle.is_available());
    let error = handle
        .invoke_with_resource("read", resource("workspace", &["project"]), b"secret")
        .expect_err("availability must not imply authorization");
    assert!(matches!(
        error,
        CapabilityError::Denied {
            reason: DenialReason::NoGrant,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn grants_limit_operations_and_resources() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CountingService {
            calls: Arc::clone(&calls),
        }),
    );
    root_grant(
        &kernel,
        &agent,
        &capability,
        &["read"],
        ResourceScope::Subtree(resource("workspace", &["project"])),
        false,
    );
    let handle = kernel
        .capability_for(agent, capability)
        .expect("registered principal must receive a handle");

    handle
        .invoke_with_resource(
            "read",
            resource("workspace", &["project", "file"]),
            b"payload",
        )
        .expect("read in subtree must be allowed");
    let write = handle
        .invoke_with_resource(
            "write",
            resource("workspace", &["project", "file"]),
            b"payload",
        )
        .expect_err("read grant must not authorize write");
    assert!(matches!(
        write,
        CapabilityError::Denied {
            reason: DenialReason::OperationNotAllowed,
            ..
        }
    ));
    let sibling = handle
        .invoke_with_resource(
            "read",
            resource("workspace", &["other", "file"]),
            b"payload",
        )
        .expect_err("sibling subtree must be denied");
    assert!(matches!(
        sibling,
        CapabilityError::Denied {
            reason: DenialReason::ResourceOutOfScope,
            ..
        }
    ));
    let parent = handle
        .invoke_with_resource("read", resource("workspace", &[]), b"payload")
        .expect_err("parent resource must be denied");
    assert!(matches!(
        parent,
        CapabilityError::Denied {
            reason: DenialReason::ResourceOutOfScope,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn exact_resource_scope_is_structural_and_does_not_match_descendants() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CountingService {
            calls: Arc::clone(&calls),
        }),
    );
    root_grant(
        &kernel,
        &agent,
        &capability,
        &["read"],
        ResourceScope::Exact(resource("workspace", &["project"])),
        false,
    );
    let handle = kernel
        .capability_for(agent, capability)
        .expect("registered principal must receive a handle");
    handle
        .invoke_with_resource("read", resource("workspace", &["project"]), b"payload")
        .expect("exact resource must be allowed");
    for candidate in [
        resource("workspace", &["project", "file"]),
        resource("workspace", &["projectile"]),
        resource("workspace", &["other"]),
    ] {
        let error = handle
            .invoke_with_resource("read", candidate, b"payload")
            .expect_err("non-exact resource must be denied");
        assert_eq!(
            error.denial_reason(),
            Some(DenialReason::ResourceOutOfScope)
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn grant_is_bound_to_interface_major_not_minor() {
    let capability_major_one = document_capability();
    let capability_major_two = CapabilityId::new(
        "worldline.security",
        "document",
        InterfaceVersion::new(2, 0),
    );
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "document-provider",
        &capability_major_one,
        Arc::new(CountingService {
            calls: Arc::new(AtomicUsize::new(0)),
        }),
    );
    root_grant(
        &kernel,
        &agent,
        &capability_major_one,
        &["read"],
        ResourceScope::Any,
        false,
    );
    let handle = kernel
        .capability_for(agent, capability_major_two)
        .expect("registered principal must receive a handle");
    let error = handle
        .invoke("read", b"payload")
        .expect_err("different major must not use the grant");
    assert_eq!(
        error.denial_reason(),
        Some(DenialReason::CapabilityVersionMismatch)
    );
}

#[test]
fn delegation_is_attenuated_and_revocation_is_transitive() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let alice = register_principal(&kernel, "alice", PrincipalKind::User);
    let bob = register_principal(&kernel, "bob", PrincipalKind::Agent);
    let carol = register_principal(&kernel, "carol", PrincipalKind::PluginRuntime);
    register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CountingService {
            calls: Arc::clone(&calls),
        }),
    );

    let root = root_grant(
        &kernel,
        &alice,
        &capability,
        &["read", "write"],
        ResourceScope::Subtree(resource("workspace", &["project"])),
        true,
    );
    let child = kernel
        .delegate_grant(
            root.clone(),
            bob.clone(),
            ["read"],
            ResourceScope::Exact(resource("workspace", &["project", "file"])),
            GrantLifetime::Persistent,
        )
        .expect("attenuated child must be created");
    let grandchild = kernel
        .delegate_grant(
            child.clone(),
            carol.clone(),
            ["read"],
            ResourceScope::Exact(resource("workspace", &["project", "file"])),
            GrantLifetime::Persistent,
        )
        .expect("attenuated grandchild must be created");

    let widened_operation = kernel.delegate_grant(
        root.clone(),
        bob.clone(),
        ["read", "write"],
        ResourceScope::Any,
        GrantLifetime::Persistent,
    );
    assert!(matches!(
        widened_operation,
        Err(worldline_kernel::SecurityError::Denied {
            reason: DenialReason::DelegationWouldWidenAuthority
        })
    ));
    let non_delegable = root_grant(
        &kernel,
        &alice,
        &capability,
        &["read"],
        ResourceScope::Any,
        false,
    );
    assert!(matches!(
        kernel.delegate_grant(
            non_delegable,
            bob.clone(),
            ["read"],
            ResourceScope::Any,
            GrantLifetime::Persistent,
        ),
        Err(worldline_kernel::SecurityError::Denied {
            reason: DenialReason::DelegationNotAllowed
        })
    ));

    let handle = kernel
        .capability_for(carol, capability.clone())
        .expect("registered principal must receive a handle");
    handle
        .invoke_with_authority(
            "read",
            resource("workspace", &["project", "file"]),
            worldline_kernel::AuthoritySet::from_grant(grandchild.clone()),
            b"payload",
        )
        .expect("grandchild authority must work before revocation");
    kernel
        .revoke_grant(&root)
        .expect("root revocation must succeed");
    assert!(!kernel.is_grant_active(&root));
    assert!(!kernel.is_grant_active(&child));
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::GrantAutoRevoked { grant } if grant == &grandchild
        )
    }));
    let error = handle
        .invoke_with_authority(
            "read",
            resource("workspace", &["project", "file"]),
            worldline_kernel::AuthoritySet::from_grant(grandchild),
            b"payload",
        )
        .expect_err("descendant authority must be revoked");
    assert!(matches!(
        error,
        CapabilityError::Denied {
            reason: DenialReason::GrantRevoked,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn lifecycle_scoped_grant_is_revoked_on_teardown() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    let provider = register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CountingService {
            calls: Arc::clone(&calls),
        }),
    );
    let scope = kernel
        .lifecycle_scope_for(&provider)
        .expect("active provider must own a lifecycle scope");
    kernel
        .create_grant(
            worldline_kernel::GrantRequest::new(
                kernel.system_principal(),
                agent.clone(),
                capability.contract(),
            )
            .allow_operation("read")
            .with_lifetime(GrantLifetime::Lifecycle(scope)),
        )
        .expect("scoped grant must be created");
    let handle = kernel
        .capability_for(agent, capability)
        .expect("registered principal must receive a handle");
    handle
        .invoke("read", b"payload")
        .expect("scoped grant must work while scope is active");

    kernel.stop();
    let error = handle
        .invoke("read", b"payload")
        .expect_err("scope cleanup must revoke the grant");
    assert!(matches!(
        error,
        CapabilityError::Denied {
            reason: DenialReason::GrantRevoked,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn compatible_provider_replacement_preserves_contract_grant_without_expansion() {
    let capability_v1 = document_capability();
    let capability_v1_minor = CapabilityId::new(
        "worldline.security",
        "document",
        InterfaceVersion::new(1, 1),
    );
    let calls_a = Arc::new(AtomicUsize::new(0));
    let calls_b = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    root_grant(
        &kernel,
        &agent,
        &capability_v1,
        &["read"],
        ResourceScope::Any,
        false,
    );
    let provider_a = register_provider(
        &mut kernel,
        "provider-a",
        &capability_v1_minor,
        Arc::new(CountingService {
            calls: Arc::clone(&calls_a),
        }),
    );
    let handle = kernel
        .capability_for(agent, capability_v1_minor.clone())
        .expect("registered principal must receive a handle");
    handle
        .invoke("read", b"payload")
        .expect("provider A must answer");

    register_provider(
        &mut kernel,
        "provider-b",
        &capability_v1_minor,
        Arc::new(CountingService {
            calls: Arc::clone(&calls_b),
        }),
    );
    kernel
        .unregister(&provider_a)
        .expect("provider A removal must succeed");
    handle
        .invoke("read", b"payload")
        .expect("compatible provider B must answer");
    assert_eq!(calls_a.load(Ordering::SeqCst), 1);
    assert_eq!(calls_b.load(Ordering::SeqCst), 1);
    let write = handle
        .invoke("write", b"payload")
        .expect_err("replacement must not expand the grant");
    assert!(matches!(
        write,
        CapabilityError::Denied {
            reason: DenialReason::OperationNotAllowed,
            ..
        }
    ));
    assert_eq!(calls_b.load(Ordering::SeqCst), 1);
}

struct DeputyService {
    document: CapabilityId,
    storage: CapabilityId,
    nested: Arc<Mutex<Vec<String>>>,
}

impl CapabilityService for DeputyService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(b"document".to_vec())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        _payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        if context.capability() != &self.document {
            return Ok(b"storage".to_vec());
        }
        let result = match context.operation().as_str() {
            "delegated" => context
                .invoke_delegated(
                    self.storage.clone(),
                    "write",
                    ResourceId::root("storage"),
                    b"nested",
                )
                .map(|_| "delegated-allowed".to_owned())
                .unwrap_or_else(|error| format!("delegated-denied:{:?}", error.denial_reason())),
            "self" => context
                .invoke_self(
                    self.storage.clone(),
                    "write",
                    ResourceId::root("storage"),
                    b"nested",
                )
                .map(|_| "self-allowed".to_owned())
                .unwrap_or_else(|error| format!("self-denied:{error:?}")),
            _ => "no-nested-call".to_owned(),
        };
        self.nested
            .lock()
            .expect("nested log lock is not poisoned")
            .push(result.clone());
        Ok(result.into_bytes())
    }
}

#[test]
fn confused_deputy_has_no_delegated_to_self_fallback() {
    let document = document_capability();
    let storage = storage_capability();
    let storage_calls = Arc::new(AtomicUsize::new(0));
    let nested = Arc::new(Mutex::new(Vec::new()));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    root_grant(
        &kernel,
        &agent,
        &document,
        &["delegated", "self"],
        ResourceScope::Any,
        false,
    );
    let storage_provider = register_provider(
        &mut kernel,
        "storage-provider",
        &storage,
        Arc::new(CountingService {
            calls: Arc::clone(&storage_calls),
        }),
    );
    let document_provider = kernel
        .register(ProviderPlugin {
            definition: PluginDefinition::new("document-provider")
                .provides(document.clone())
                .requires(storage.clone()),
            capability: document.clone(),
            service: Arc::new(DeputyService {
                document: document.clone(),
                storage: storage.clone(),
                nested: Arc::clone(&nested),
            }),
        })
        .expect("document provider registration must succeed");
    let provider_principal = kernel
        .principal_for_plugin(&document_provider)
        .expect("provider principal must be registered");
    root_grant(
        &kernel,
        &provider_principal,
        &storage,
        &["write"],
        ResourceScope::Any,
        false,
    );
    let handle = kernel
        .capability_for(agent, document)
        .expect("registered principal must receive a handle");

    let delegated_result = String::from_utf8(
        handle
            .invoke("delegated", b"payload")
            .expect("outer read must still execute"),
    )
    .expect("result must be UTF-8");
    assert!(delegated_result.starts_with("delegated-denied:"));
    assert_eq!(storage_calls.load(Ordering::SeqCst), 0);

    let self_result = String::from_utf8(
        handle
            .invoke("self", b"payload")
            .expect("outer self test must execute"),
    )
    .expect("result must be UTF-8");
    assert_eq!(self_result, "self-allowed");
    assert_eq!(storage_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        nested
            .lock()
            .expect("nested log lock is not poisoned")
            .as_slice(),
        ["delegated-denied:Some(NoGrant)", "self-allowed"]
    );
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::InvocationDenied {
                reason: DenialReason::NoGrant,
                causal_parent: Some(_),
                ..
            }
        )
    }));
    assert!(
        kernel
            .plugin_state(&storage_provider)
            .is_some_and(|state| state == RuntimeState::Active)
    );
}

struct CallerRecordingService {
    caller: Arc<Mutex<Option<PrincipalId>>>,
}

impl CapabilityService for CallerRecordingService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        _payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        *self.caller.lock().expect("caller lock is not poisoned") = Some(context.caller().clone());
        Ok(Vec::new())
    }
}

#[test]
fn payload_cannot_spoof_caller_and_trajectory_omits_payload() {
    let capability = document_capability();
    let caller_seen = Arc::new(Mutex::new(None));
    let mut kernel = Kernel::new();
    let agent_a = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    let agent_b = register_principal(&kernel, "agent-b", PrincipalKind::Agent);
    root_grant(
        &kernel,
        &agent_a,
        &capability,
        &["read"],
        ResourceScope::Any,
        false,
    );
    register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CallerRecordingService {
            caller: Arc::clone(&caller_seen),
        }),
    );
    let handle = kernel
        .capability_for(agent_a.clone(), capability)
        .expect("registered principal must receive a handle");
    let marker = b"super-secret-marker:agent-b";
    handle
        .invoke("read", marker)
        .expect("authorized call must work");
    assert_eq!(
        caller_seen
            .lock()
            .expect("caller lock is not poisoned")
            .as_ref(),
        Some(&agent_a)
    );
    assert!(!format!("{:?}", kernel.trajectory()).contains("super-secret-marker"));
    let _ = agent_b;
}

fn security_trajectory_scenario() -> Vec<TrajectoryEventKind> {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "agent-a", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "document-provider",
        &capability,
        Arc::new(CountingService { calls }),
    );
    let handle = kernel
        .capability_for(agent.clone(), capability.clone())
        .expect("registered principal must receive a handle");
    let _ = handle.invoke("read", b"denied");
    let grant = root_grant(
        &kernel,
        &agent,
        &capability,
        &["read"],
        ResourceScope::Any,
        false,
    );
    handle
        .invoke("read", b"allowed")
        .expect("grant must allow read");
    kernel
        .revoke_grant(&grant)
        .expect("grant revocation must succeed");
    let _ = handle.invoke("read", b"revoked");
    kernel
        .trajectory()
        .into_iter()
        .map(|event| event.kind().clone())
        .collect()
}

#[test]
fn identical_security_scenarios_have_identical_event_kinds() {
    assert_eq!(
        security_trajectory_scenario(),
        security_trajectory_scenario()
    );
}

struct MutualRecursionService {
    other: CapabilityId,
    stopped: Arc<AtomicUsize>,
}

impl CapabilityService for MutualRecursionService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        _payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        match context.invoke_self(
            self.other.clone(),
            "loop",
            ResourceId::root(self.other.namespace()),
            b"recursive",
        ) {
            Ok(_) => Ok(Vec::new()),
            Err(error) if error.denial_reason() == Some(DenialReason::InvocationDepthExceeded) => {
                self.stopped.fetch_add(1, Ordering::SeqCst);
                Ok(Vec::new())
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

#[test]
fn mutual_recursion_reaches_the_admission_depth_limit_safely() {
    let capability_a = CapabilityId::new("worldline.recursion", "a", InterfaceVersion::new(1, 0));
    let capability_b = CapabilityId::new("worldline.recursion", "b", InterfaceVersion::new(1, 0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "recursion-agent", PrincipalKind::Agent);
    let stopped = Arc::new(AtomicUsize::new(0));
    let provider_a = register_provider(
        &mut kernel,
        "recursion-provider-a",
        &capability_a,
        Arc::new(MutualRecursionService {
            other: capability_b.clone(),
            stopped: Arc::clone(&stopped),
        }),
    );
    let provider_b = register_provider(
        &mut kernel,
        "recursion-provider-b",
        &capability_b,
        Arc::new(MutualRecursionService {
            other: capability_a.clone(),
            stopped: Arc::clone(&stopped),
        }),
    );
    let principal_a = kernel
        .principal_for_plugin(&provider_a)
        .expect("provider A principal must be registered");
    let principal_b = kernel
        .principal_for_plugin(&provider_b)
        .expect("provider B principal must be registered");
    root_grant(
        &kernel,
        &principal_a,
        &capability_b,
        &["loop"],
        ResourceScope::Any,
        false,
    );
    root_grant(
        &kernel,
        &principal_b,
        &capability_a,
        &["loop"],
        ResourceScope::Any,
        false,
    );
    root_grant(
        &kernel,
        &agent,
        &capability_a,
        &["loop"],
        ResourceScope::Any,
        false,
    );

    let handle = kernel
        .capability_for(agent, capability_a)
        .expect("registered principal must receive a handle");
    handle
        .invoke("loop", b"recursive")
        .expect("depth limit must stop recursion without failing the outer call");

    assert_eq!(stopped.load(Ordering::SeqCst), 1);
    let events = kernel.trajectory();
    let started = events
        .iter()
        .filter(|event| matches!(event.kind(), TrajectoryEventKind::InvocationStarted { .. }))
        .count();
    assert_eq!(
        started,
        worldline_kernel::MAX_NESTED_INVOCATION_DEPTH + 1,
        "root invocation plus the configured nested depth must be admitted"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind(),
                    TrajectoryEventKind::InvocationDenied {
                        reason: DenialReason::InvocationDepthExceeded,
                        ..
                    }
                )
            })
            .count(),
        1
    );
}

#[test]
fn repeated_revoke_emits_one_state_change_event() {
    let capability = document_capability();
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "revoke-agent", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "revoke-provider",
        &capability,
        Arc::new(CountingService {
            calls: Arc::new(AtomicUsize::new(0)),
        }),
    );
    let grant = root_grant(
        &kernel,
        &agent,
        &capability,
        &["read"],
        ResourceScope::Any,
        false,
    );

    kernel
        .revoke_grant(&grant)
        .expect("first revoke must succeed");
    let event_count_after_first = kernel.trajectory().len();
    kernel
        .revoke_grant(&grant)
        .expect("repeating an idempotent revoke must still succeed");
    let events = kernel.trajectory();

    assert_eq!(events.len(), event_count_after_first);
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind(),
                    TrajectoryEventKind::GrantRevoked { grant: event_grant }
                        if event_grant == &grant
                )
            })
            .count(),
        1
    );
    assert!(!kernel.is_grant_active(&grant));
}

#[test]
fn unregister_revokes_runtime_subject_grants_and_descendants() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "runtime-child", PrincipalKind::Agent);
    let provider = register_provider(
        &mut kernel,
        "runtime-owner",
        &capability,
        Arc::new(CountingService {
            calls: Arc::clone(&calls),
        }),
    );
    let provider_principal = kernel
        .principal_for_plugin(&provider)
        .expect("runtime principal must be registered");
    let runtime_grant = root_grant(
        &kernel,
        &provider_principal,
        &capability,
        &["read"],
        ResourceScope::Any,
        true,
    );
    let child_grant = kernel
        .delegate_grant(
            runtime_grant.clone(),
            agent.clone(),
            ["read"],
            ResourceScope::Any,
            GrantLifetime::Persistent,
        )
        .expect("runtime-owned grant must be delegable");
    let handle = kernel
        .capability_for(agent, capability.clone())
        .expect("registered principal must receive a handle");
    handle
        .invoke_with_authority(
            "read",
            ResourceId::root("worldline.security"),
            worldline_kernel::AuthoritySet::from_grant(child_grant.clone()),
            b"before-unregister",
        )
        .expect("descendant grant must work before owner unregisters");

    kernel
        .unregister(&provider)
        .expect("runtime owner unregister must succeed");

    assert!(!kernel.is_grant_active(&runtime_grant));
    assert!(!kernel.is_grant_active(&child_grant));
    let events = kernel.trajectory();
    assert!(events.iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::GrantRevoked { grant } if grant == &runtime_grant
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::GrantAutoRevoked { grant } if grant == &child_grant
        )
    }));
    let error = handle
        .invoke_with_authority(
            "read",
            ResourceId::root("worldline.security"),
            worldline_kernel::AuthoritySet::from_grant(child_grant),
            b"after-unregister",
        )
        .expect_err("descendant authority must be revoked before provider resolution");
    assert_eq!(error.denial_reason(), Some(DenialReason::GrantRevoked));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct SelfProbeService {
    trigger: CapabilityId,
    target: CapabilityId,
}

impl CapabilityService for SelfProbeService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(b"not-contextual".to_vec())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        _payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        if context.capability() != &self.trigger {
            return Ok(b"target".to_vec());
        }
        match context.invoke_self(
            self.target.clone(),
            "write",
            ResourceId::root(self.target.namespace()),
            b"provider-self",
        ) {
            Ok(_) => Ok(b"self-allowed".to_vec()),
            Err(error) => Ok(format!("self-denied:{:?}", error.denial_reason()).into_bytes()),
        }
    }
}

#[test]
fn re_registering_same_plugin_id_does_not_inherit_runtime_authority() {
    let document = document_capability();
    let storage = storage_capability();
    let storage_calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "reregister-agent", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "reregister-storage",
        &storage,
        Arc::new(CountingService {
            calls: Arc::clone(&storage_calls),
        }),
    );
    let provider = register_provider(
        &mut kernel,
        "reregister-runtime",
        &document,
        Arc::new(SelfProbeService {
            trigger: document.clone(),
            target: storage.clone(),
        }),
    );
    let provider_principal = kernel
        .principal_for_plugin(&provider)
        .expect("runtime principal must be registered");
    let runtime_grant = root_grant(
        &kernel,
        &provider_principal,
        &storage,
        &["write"],
        ResourceScope::Any,
        false,
    );
    root_grant(
        &kernel,
        &agent,
        &document,
        &["trigger"],
        ResourceScope::Any,
        false,
    );
    let handle = kernel
        .capability_for(agent, document.clone())
        .expect("registered principal must receive a handle");
    assert_eq!(
        String::from_utf8(handle.invoke("trigger", b"before").unwrap())
            .expect("provider response must be UTF-8"),
        "self-allowed"
    );
    assert_eq!(storage_calls.load(Ordering::SeqCst), 1);

    kernel
        .unregister(&provider)
        .expect("runtime unregister must succeed");
    let replacement = register_provider(
        &mut kernel,
        "reregister-runtime",
        &document,
        Arc::new(SelfProbeService {
            trigger: document.clone(),
            target: storage,
        }),
    );
    assert_eq!(replacement, provider);
    assert_eq!(
        kernel
            .grant(&runtime_grant)
            .expect("revoked grant must remain auditable")
            .status(),
        worldline_kernel::GrantStatus::Revoked
    );
    assert_eq!(
        String::from_utf8(handle.invoke("trigger", b"after").unwrap())
            .expect("provider response must be UTF-8"),
        "self-denied:Some(GrantRevoked)"
    );
    assert_eq!(storage_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn provider_self_succeeds_using_provider_grants() {
    let document = document_capability();
    let storage = storage_capability();
    let storage_calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "self-agent", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "self-storage",
        &storage,
        Arc::new(CountingService {
            calls: Arc::clone(&storage_calls),
        }),
    );
    let document_provider = register_provider(
        &mut kernel,
        "self-document",
        &document,
        Arc::new(SelfProbeService {
            trigger: document.clone(),
            target: storage.clone(),
        }),
    );
    let provider_principal = kernel
        .principal_for_plugin(&document_provider)
        .expect("provider principal must be registered");
    root_grant(
        &kernel,
        &provider_principal,
        &storage,
        &["write"],
        ResourceScope::Any,
        false,
    );
    root_grant(
        &kernel,
        &agent,
        &document,
        &["trigger"],
        ResourceScope::Any,
        false,
    );
    let handle = kernel
        .capability_for(agent, document)
        .expect("registered principal must receive a handle");

    assert_eq!(
        String::from_utf8(handle.invoke("trigger", b"payload").unwrap())
            .expect("provider response must be UTF-8"),
        "self-allowed"
    );
    assert_eq!(storage_calls.load(Ordering::SeqCst), 1);
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::InvocationStarted {
                caller,
                capability,
                causal_parent: Some(_),
                ..
            } if caller == &provider_principal && capability == &storage.contract()
        )
    }));
}

#[test]
fn provider_self_authority_remains_unavailable_to_caller() {
    let storage = storage_capability();
    let storage_calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "self-unavailable-agent", PrincipalKind::Agent);
    let provider = register_provider(
        &mut kernel,
        "self-unavailable-provider",
        &storage,
        Arc::new(CountingService {
            calls: Arc::clone(&storage_calls),
        }),
    );
    let provider_principal = kernel
        .principal_for_plugin(&provider)
        .expect("provider principal must be registered");
    root_grant(
        &kernel,
        &provider_principal,
        &storage,
        &["write"],
        ResourceScope::Any,
        false,
    );

    let caller_handle = kernel
        .capability_for(agent.clone(), storage.clone())
        .expect("registered caller must receive a handle");
    let caller_error = caller_handle
        .invoke("write", b"caller")
        .expect_err("provider self grant must not become caller authority");
    assert_eq!(caller_error.denial_reason(), Some(DenialReason::NoGrant));

    let forged_request = worldline_kernel::InvocationRequest::new(
        agent,
        storage.clone(),
        "write",
        ResourceId::root(storage.namespace()),
        b"caller",
    )
    .with_authority(worldline_kernel::AuthoritySource::ProviderSelf(
        provider_principal,
    ));
    let forged_error = kernel
        .invoke(forged_request)
        .expect_err("ProviderSelf must require an active provider frame");
    assert_eq!(
        forged_error.denial_reason(),
        Some(DenialReason::InvalidAuthoritySource)
    );
    assert_eq!(storage_calls.load(Ordering::SeqCst), 0);
}

struct DelegatedOnlyService {
    trigger: CapabilityId,
    target: CapabilityId,
}

impl CapabilityService for DelegatedOnlyService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        _payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        if context.capability() != &self.trigger {
            return Ok(Vec::new());
        }
        match context.invoke_delegated(
            self.target.clone(),
            "write",
            ResourceId::root(self.target.namespace()),
            b"delegated",
        ) {
            Ok(_) => Ok(b"unexpected-allowed".to_vec()),
            Err(error) => Ok(format!("delegated-denied:{:?}", error.denial_reason()).into_bytes()),
        }
    }
}

#[test]
fn delegated_failure_never_falls_back_to_provider_self() {
    let document = document_capability();
    let storage = storage_capability();
    let storage_calls = Arc::new(AtomicUsize::new(0));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "delegated-agent", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "delegated-storage",
        &storage,
        Arc::new(CountingService {
            calls: Arc::clone(&storage_calls),
        }),
    );
    let document_provider = register_provider(
        &mut kernel,
        "delegated-document",
        &document,
        Arc::new(DelegatedOnlyService {
            trigger: document.clone(),
            target: storage.clone(),
        }),
    );
    let provider_principal = kernel
        .principal_for_plugin(&document_provider)
        .expect("provider principal must be registered");
    root_grant(
        &kernel,
        &provider_principal,
        &storage,
        &["write"],
        ResourceScope::Any,
        false,
    );
    root_grant(
        &kernel,
        &agent,
        &document,
        &["trigger"],
        ResourceScope::Any,
        false,
    );
    let handle = kernel
        .capability_for(agent, document)
        .expect("registered caller must receive a handle");

    let result = String::from_utf8(handle.invoke("trigger", b"payload").unwrap())
        .expect("provider response must be UTF-8");
    assert_eq!(result, "delegated-denied:Some(NoGrant)");
    assert_eq!(storage_calls.load(Ordering::SeqCst), 0);

    let events = kernel.trajectory();
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind(),
                    TrajectoryEventKind::InvocationDenied {
                        capability,
                        reason: DenialReason::NoGrant,
                        causal_parent: Some(_),
                        ..
                    } if capability == &storage.contract()
                )
            })
            .count(),
        1
    );
    assert!(!events.iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::InvocationAuthorized {
                capability,
                causal_parent: Some(_),
                ..
            } if capability == &storage.contract()
        )
    }));
}

struct BlockingService {
    entered: mpsc::Sender<()>,
    release: Arc<Barrier>,
    calls: Arc<AtomicUsize>,
}

impl CapabilityService for BlockingService {
    fn invoke(&self, _operation: &str, _payload: &[u8]) -> Result<Vec<u8>, String> {
        self.entered
            .send(())
            .map_err(|error| format!("failed to signal admission: {error}"))?;
        self.release.wait();
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(b"completed".to_vec())
    }
}

#[test]
fn revocation_does_not_cancel_an_admitted_in_flight_invocation() {
    let capability = document_capability();
    let calls = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let mut kernel = Kernel::new();
    let agent = register_principal(&kernel, "in-flight-agent", PrincipalKind::Agent);
    register_provider(
        &mut kernel,
        "in-flight-provider",
        &capability,
        Arc::new(BlockingService {
            entered: entered_tx,
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        }),
    );
    let grant = root_grant(
        &kernel,
        &agent,
        &capability,
        &["read"],
        ResourceScope::Any,
        false,
    );
    let handle = kernel
        .capability_for(agent, capability)
        .expect("registered principal must receive a handle");
    let worker_handle = handle.clone();
    let worker = std::thread::spawn(move || worker_handle.invoke("read", b"in-flight"));

    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("provider must be entered after authorization");
    kernel
        .revoke_grant(&grant)
        .expect("revocation during an in-flight call must succeed");
    release.wait();
    worker
        .join()
        .expect("invocation worker must not panic")
        .expect("admitted invocation must complete despite revocation");

    let error = handle
        .invoke("read", b"after-revoke")
        .expect_err("subsequent admission must observe revocation");
    assert_eq!(error.denial_reason(), Some(DenialReason::GrantRevoked));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
