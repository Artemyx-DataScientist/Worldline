//! M0.6 external boundary acceptance: opaque runtime-scoped authority
//! handles must be host-created, attenuated, revocable, and impossible to
//! use across runtimes or after runtime death.

use std::collections::BTreeSet;

use worldline_kernel::{
    CapabilityId, InterfaceVersion, Kernel, KernelError, NoopRuntime, OperationId, Plugin,
    PluginId, ResourceId, RuntimeId, RuntimeState, TrajectoryEventKind,
};

fn echo_capability() -> CapabilityId {
    CapabilityId::new("worldline.test", "echo", InterfaceVersion::new(1, 0))
}

fn register_active_runtime(kernel: &mut Kernel, name: &str) -> (PluginId, RuntimeId) {
    let definition = worldline_kernel::PluginDefinition::new(PluginId::new(name));
    let plugin_id = kernel
        .register(ExternalUserPlugin {
            definition,
            capability: echo_capability(),
        })
        .expect("registration must succeed");
    kernel.reconcile();
    let runtime = kernel
        .runtime_id_for_plugin(&plugin_id)
        .expect("reconcile must activate the runtime");
    assert_eq!(
        kernel.plugin_state(&plugin_id),
        Some(RuntimeState::Active),
        "runtime must be active for handle tests"
    );
    (plugin_id, runtime)
}

struct ExternalUserPlugin {
    definition: worldline_kernel::PluginDefinition,
    #[allow(dead_code)]
    capability: CapabilityId,
}

impl Plugin for ExternalUserPlugin {
    fn definition(&self) -> &worldline_kernel::PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        _context: &mut worldline_kernel::ActivationContext,
    ) -> Result<Box<dyn worldline_kernel::PluginRuntime>, worldline_kernel::PluginError> {
        Ok(Box::new(NoopRuntime))
    }
}

fn operations() -> BTreeSet<OperationId> {
    BTreeSet::from([OperationId::new("echo")])
}

fn resources() -> BTreeSet<ResourceId> {
    BTreeSet::from([ResourceId::new("reference.echo", ["v1"])])
}

fn denied(error: &KernelError) -> String {
    error.to_string()
}

#[test]
fn handle_resolves_only_for_its_owning_runtime() {
    let mut kernel = Kernel::new();
    let (_, runtime_a) = register_active_runtime(&mut kernel, "plugin.external-a");
    let (_, runtime_b) = register_active_runtime(&mut kernel, "plugin.external-b");

    let handle = kernel
        .issue_external_handle(&runtime_a, operations(), resources())
        .expect("active runtime must receive a handle");

    let view = kernel
        .resolve_external_handle(&runtime_a, handle)
        .expect("owner must resolve its own handle");
    assert_eq!(view.runtime(), &runtime_a);
    assert!(view.operations().contains(&OperationId::new("echo")));

    let KernelError::ExternalHandleWrongRuntime { .. } = kernel
        .resolve_external_handle(&runtime_b, handle)
        .expect_err("a handle owned by runtime A must never resolve for runtime B")
    else {
        panic!("cross-runtime resolution must fail with ExternalHandleWrongRuntime");
    };
}

#[test]
fn handle_scope_attenuation_cannot_be_widened() {
    let mut kernel = Kernel::new();
    let (_, runtime) = register_active_runtime(&mut kernel, "plugin.external-att");
    let handle = kernel
        .issue_external_handle(&runtime, operations(), resources())
        .expect("handle issuance must succeed");

    kernel
        .check_external_handle_scope(
            &runtime,
            handle,
            &OperationId::new("echo"),
            &ResourceId::new("reference.echo", ["v1"]),
        )
        .expect("delegated operation over delegated resource must pass");

    for (operation, resource, why) in [
        (
            OperationId::new("admin"),
            ResourceId::new("reference.echo", ["v1"]),
            "undelegated operation",
        ),
        (
            OperationId::new("echo"),
            ResourceId::new("other.installation", ["state"]),
            "undelegated resource",
        ),
    ] {
        let KernelError::ExternalHandleScopeDenied { .. } = kernel
            .check_external_handle_scope(&runtime, handle, &operation, &resource)
            .expect_err(why)
        else {
            panic!("attenuation widening must fail with ExternalHandleScopeDenied");
        };
    }
}

#[test]
fn unknown_revoked_and_denied_handles_are_deterministic() {
    let mut kernel = Kernel::new();
    let (_, runtime) = register_active_runtime(&mut kernel, "plugin.external-det");
    let handle = kernel
        .issue_external_handle(&runtime, operations(), resources())
        .expect("handle issuance must succeed");

    assert!(matches!(
        kernel.resolve_external_handle(&runtime, 9_999),
        Err(KernelError::InvalidExternalHandle { handle: 9_999 })
    ));

    kernel
        .revoke_external_handle(&runtime, handle)
        .expect("owner revocation must succeed");
    assert!(matches!(
        kernel.resolve_external_handle(&runtime, handle),
        Err(KernelError::ExternalHandleRevoked { .. })
    ));
    assert!(matches!(
        kernel.revoke_external_handle(&runtime, handle),
        Err(KernelError::ExternalHandleRevoked { .. })
    ));
}

#[test]
fn inactive_runtime_cannot_receive_handles() {
    let mut kernel = Kernel::new();
    let (_, runtime) = register_active_runtime(&mut kernel, "plugin.external-inactive");
    kernel.stop();
    assert!(matches!(
        kernel.issue_external_handle(&runtime, operations(), resources()),
        Err(KernelError::ExternalRuntimeNotActive { .. })
    ));
}

#[test]
fn terminal_paths_revoke_every_handle_and_restarts_get_fresh_values() {
    let mut kernel = Kernel::new();
    let (plugin_id, first_runtime) = register_active_runtime(&mut kernel, "plugin.external-term");
    let first_handle = kernel
        .issue_external_handle(&first_runtime, operations(), resources())
        .expect("handle issuance must succeed");
    let second_handle = kernel
        .issue_external_handle(&first_runtime, operations(), resources())
        .expect("handle issuance must succeed");
    assert_eq!(kernel.live_external_handles(&first_runtime), 2);

    kernel
        .unregister(&plugin_id)
        .expect("unregister must succeed");

    for handle in [first_handle, second_handle] {
        assert!(matches!(
            kernel.resolve_external_handle(&first_runtime, handle),
            Err(KernelError::ExternalHandleRevoked { .. })
        ));
    }
    assert_eq!(kernel.live_external_handles(&first_runtime), 0);

    let (_, restarted_runtime) = register_active_runtime(&mut kernel, "plugin.external-term");
    assert_ne!(
        first_runtime, restarted_runtime,
        "a restarted runtime must receive a fresh identity"
    );
    let restarted_handle = kernel
        .issue_external_handle(&restarted_runtime, operations(), resources())
        .expect("restarted runtime must receive handles");
    assert_ne!(
        restarted_handle, first_handle,
        "fresh values must never alias revoked ones"
    );
    assert!(matches!(
        kernel.resolve_external_handle(&first_runtime, first_handle),
        Err(KernelError::ExternalHandleRevoked { .. })
    ));
}

#[test]
fn empty_attenuation_delegates_nothing_and_denials_leave_trajectory_facts() {
    let mut kernel = Kernel::new();
    let (_, runtime) = register_active_runtime(&mut kernel, "plugin.external-empty");
    let handle = kernel
        .issue_external_handle(&runtime, BTreeSet::new(), BTreeSet::new())
        .expect("an empty handle is valid but delegates nothing");

    assert!(matches!(
        kernel.check_external_handle_scope(
            &runtime,
            handle,
            &OperationId::new("echo"),
            &ResourceId::new("reference.echo", ["v1"]),
        ),
        Err(KernelError::ExternalHandleScopeDenied { .. })
    ));

    let denial_facts = kernel
        .trajectory()
        .into_iter()
        .filter(|event| {
            matches!(
                event.kind(),
                TrajectoryEventKind::ExternalHandleDenied { .. }
            )
        })
        .count();
    assert!(denial_facts > 0, "denials must leave metadata-only facts");

    let unknown = kernel
        .resolve_external_handle(&runtime, 123_456)
        .unwrap_err();
    assert!(
        denied(&unknown).contains("does not exist"),
        "denials must be deterministic: {unknown}"
    );
}
