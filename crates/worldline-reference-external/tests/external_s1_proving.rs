//! M0.6 external S1 proving path: proves that capability RPC, state commit,
//! independent event observation, restart continuity, and runtime authority
//! discontinuity hold identically across external execution modes (native process
//! and sandboxed WASM component), as well as across provider mode replacement.

mod support;

use std::sync::Arc;

use worldline_kernel::{
    GrantLifetime, InMemoryStateBackend, Kernel, PrincipalId, PrincipalKind, ResourceScope,
    RpcCallOptions, StateBackend, SubscriptionOptions, TraceContext,
};
use worldline_reference_external::{
    NativeEchoOptions, NativeEchoPlugin, OPERATION_PUBLISH_OBSERVATION,
    OPERATION_STATEFUL_INCREMENT, WasmEchoPlugin, echo_capability, format_increment_result,
    format_observation_result, observation_contract,
};

#[test]
fn native_provider_proves_s1_state_and_event_restart_continuity() {
    let backend: Arc<dyn StateBackend> = Arc::new(InMemoryStateBackend::new());

    let (installation_id, old_runtime_id) = {
        let mut kernel = Kernel::with_state_backend(backend.clone()).expect("kernel init");
        let options = NativeEchoOptions::new(support::native_provider_program());
        let plugin = NativeEchoPlugin::new("reference.echo.s1.native", options);
        let plugin_id = kernel.register(plugin).expect("register native plugin");
        kernel.reconcile();

        let installation = kernel
            .installation_id_for_plugin(&plugin_id)
            .expect("installation id");
        let runtime_id = kernel
            .runtime_id_for_plugin(&plugin_id)
            .expect("runtime id");
        let runtime_principal = kernel
            .principal_for_plugin(&plugin_id)
            .expect("runtime principal");

        let caller = PrincipalId::new("s1-caller");
        let observer = PrincipalId::new("s1-observer");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .expect("reg caller");
        kernel
            .register_principal_id(observer.clone(), PrincipalKind::Agent)
            .expect("reg observer");

        let cap = echo_capability();
        let event = observation_contract();
        for op in [OPERATION_STATEFUL_INCREMENT, OPERATION_PUBLISH_OBSERVATION] {
            kernel
                .create_root_grant(
                    caller.clone(),
                    cap.contract(),
                    [op],
                    ResourceScope::Any,
                    false,
                    GrantLifetime::Persistent,
                )
                .expect("grant caller");
        }
        kernel
            .create_root_grant(
                runtime_principal,
                event.capability_id(),
                ["publish"],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant provider publish");
        kernel
            .create_root_grant(
                observer.clone(),
                event.capability_id(),
                ["subscribe"],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant observer subscribe");

        let sub = kernel
            .subscribe(observer.clone(), event, SubscriptionOptions::default())
            .expect("subscribe");

        // 1. Invoke stateful increment
        let res = kernel
            .capability_for(caller.clone(), cap.clone())
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_STATEFUL_INCREMENT,
                b"step1",
                RpcCallOptions::new()
                    .with_request_id("s1-req-1")
                    .with_trace_context(TraceContext::new("s1-trace")),
            )
            .expect("invoke increment");
        assert_eq!(res, format_increment_result(1, b"step1"));

        // 2. Invoke publish observation
        let res = kernel
            .capability_for(caller.clone(), cap)
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_PUBLISH_OBSERVATION,
                b"obs1",
                RpcCallOptions::new()
                    .with_request_id("s1-req-2")
                    .with_trace_context(TraceContext::new("s1-trace")),
            )
            .expect("invoke obs");
        assert_eq!(res, format_observation_result(b"obs1"));

        let envelope = sub.try_recv().expect("recv").expect("envelope exists");
        assert_eq!(envelope.payload(), b"obs1");
        assert_eq!(envelope.producer_runtime_id(), Some(runtime_id));

        (installation, runtime_id)
    };

    // Restart kernel over the same StateBackend
    {
        let mut kernel = Kernel::with_state_backend(backend).expect("kernel restart");
        let options = NativeEchoOptions::new(support::native_provider_program());
        let plugin = NativeEchoPlugin::new("reference.echo.s1.native", options);
        let plugin_id = kernel.register(plugin).expect("register restarted plugin");
        kernel.reconcile();

        let new_installation = kernel
            .installation_id_for_plugin(&plugin_id)
            .expect("installation id");
        let new_runtime_id = kernel
            .runtime_id_for_plugin(&plugin_id)
            .expect("new runtime id");
        let new_runtime_principal = kernel
            .principal_for_plugin(&plugin_id)
            .expect("new runtime principal");

        assert_eq!(
            new_installation, installation_id,
            "installation state identity must persist across restart"
        );
        assert_ne!(
            new_runtime_id, old_runtime_id,
            "runtime identity must be distinct after restart"
        );

        let caller = PrincipalId::new("s1-caller");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .expect("reg caller");
        let cap = echo_capability();
        kernel
            .create_root_grant(
                caller.clone(),
                cap.contract(),
                [OPERATION_STATEFUL_INCREMENT],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant caller");
        kernel
            .create_root_grant(
                new_runtime_principal,
                observation_contract().capability_id(),
                ["publish"],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant new provider");

        // 3. Stateful increment continues counting from previous state (2)
        let res = kernel
            .capability_for(caller, cap)
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_STATEFUL_INCREMENT,
                b"step2",
                RpcCallOptions::new()
                    .with_request_id("s1-req-3")
                    .with_trace_context(TraceContext::new("s1-trace")),
            )
            .expect("invoke increment 2");
        assert_eq!(res, format_increment_result(2, b"step2"));
    }
}

#[test]
fn wasm_provider_proves_s1_state_and_event_restart_continuity() {
    let backend: Arc<dyn StateBackend> = Arc::new(InMemoryStateBackend::new());
    let component_bytes = support::test_components().benign_echo.clone();

    let (installation_id, old_runtime_id) = {
        let mut kernel = Kernel::with_state_backend(backend.clone()).expect("kernel init");
        let plugin =
            WasmEchoPlugin::from_component("reference.echo.s1.wasm", component_bytes.clone());
        let plugin_id = kernel.register(plugin).expect("register wasm plugin");
        kernel.reconcile();

        let installation = kernel
            .installation_id_for_plugin(&plugin_id)
            .expect("installation id");
        let runtime_id = kernel
            .runtime_id_for_plugin(&plugin_id)
            .expect("runtime id");
        let runtime_principal = kernel
            .principal_for_plugin(&plugin_id)
            .expect("runtime principal");

        let caller = PrincipalId::new("wasm-caller");
        let observer = PrincipalId::new("wasm-observer");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .expect("reg caller");
        kernel
            .register_principal_id(observer.clone(), PrincipalKind::Agent)
            .expect("reg observer");

        let cap = echo_capability();
        let event = observation_contract();
        for op in [OPERATION_STATEFUL_INCREMENT, OPERATION_PUBLISH_OBSERVATION] {
            kernel
                .create_root_grant(
                    caller.clone(),
                    cap.contract(),
                    [op],
                    ResourceScope::Any,
                    false,
                    GrantLifetime::Persistent,
                )
                .expect("grant caller");
        }
        kernel
            .create_root_grant(
                runtime_principal,
                event.capability_id(),
                ["publish"],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant provider publish");
        kernel
            .create_root_grant(
                observer.clone(),
                event.capability_id(),
                ["subscribe"],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant observer subscribe");

        let sub = kernel
            .subscribe(observer.clone(), event, SubscriptionOptions::default())
            .expect("subscribe");

        let res = kernel
            .capability_for(caller.clone(), cap.clone())
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_STATEFUL_INCREMENT,
                b"w1",
                RpcCallOptions::new()
                    .with_request_id("w-req-1")
                    .with_trace_context(TraceContext::new("w-trace")),
            )
            .expect("invoke increment");
        assert_eq!(res, format_increment_result(1, b"w1"));

        let res = kernel
            .capability_for(caller.clone(), cap)
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_PUBLISH_OBSERVATION,
                b"w-obs",
                RpcCallOptions::new()
                    .with_request_id("w-req-2")
                    .with_trace_context(TraceContext::new("w-trace")),
            )
            .expect("invoke obs");
        assert_eq!(res, format_observation_result(b"w-obs"));

        let envelope = sub.try_recv().expect("recv").expect("envelope");
        assert_eq!(envelope.payload(), b"w-obs");
        assert_eq!(envelope.producer_runtime_id(), Some(runtime_id));

        (installation, runtime_id)
    };

    // Restart kernel with WASM provider
    {
        let mut kernel = Kernel::with_state_backend(backend).expect("kernel restart");
        let plugin = WasmEchoPlugin::from_component("reference.echo.s1.wasm", component_bytes);
        let plugin_id = kernel.register(plugin).expect("register restarted wasm");
        kernel.reconcile();

        let new_installation = kernel
            .installation_id_for_plugin(&plugin_id)
            .expect("installation id");
        let new_runtime_id = kernel
            .runtime_id_for_plugin(&plugin_id)
            .expect("new runtime id");
        let new_runtime_principal = kernel
            .principal_for_plugin(&plugin_id)
            .expect("new runtime principal");

        assert_eq!(new_installation, installation_id);
        assert_ne!(new_runtime_id, old_runtime_id);

        let caller = PrincipalId::new("wasm-caller");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .expect("reg caller");
        let cap = echo_capability();
        kernel
            .create_root_grant(
                caller.clone(),
                cap.contract(),
                [OPERATION_STATEFUL_INCREMENT],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant caller");
        kernel
            .create_root_grant(
                new_runtime_principal,
                observation_contract().capability_id(),
                ["publish"],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant new provider");

        let res = kernel
            .capability_for(caller, cap)
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_STATEFUL_INCREMENT,
                b"w2",
                RpcCallOptions::new()
                    .with_request_id("w-req-3")
                    .with_trace_context(TraceContext::new("w-trace")),
            )
            .expect("invoke increment 2");
        assert_eq!(res, format_increment_result(2, b"w2"));
    }
}

#[test]
fn provider_replacement_across_modes_retains_installation_state() {
    let backend: Arc<dyn StateBackend> = Arc::new(InMemoryStateBackend::new());

    // 1. First run with Native provider
    let installation_id = {
        let mut kernel = Kernel::with_state_backend(backend.clone()).expect("kernel init");
        let options = NativeEchoOptions::new(support::native_provider_program());
        let plugin = NativeEchoPlugin::new("reference.echo.replace", options);
        let plugin_id = kernel.register(plugin).expect("register native plugin");
        kernel.reconcile();

        let installation = kernel
            .installation_id_for_plugin(&plugin_id)
            .expect("installation id");
        let caller = PrincipalId::new("replace-caller");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .expect("reg caller");
        let cap = echo_capability();
        kernel
            .create_root_grant(
                caller.clone(),
                cap.contract(),
                [OPERATION_STATEFUL_INCREMENT],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant caller");

        let res = kernel
            .capability_for(caller, cap)
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_STATEFUL_INCREMENT,
                b"from-native",
                RpcCallOptions::new().with_request_id("repl-1"),
            )
            .expect("native increment");
        assert_eq!(res, format_increment_result(1, b"from-native"));

        installation
    };

    // 2. Second run replacing with WASM component over the same installation
    {
        let mut kernel = Kernel::with_state_backend(backend).expect("kernel restart");
        let component = support::test_components().benign_echo.clone();
        let plugin = WasmEchoPlugin::from_component("reference.echo.replace", component);
        let plugin_id = kernel.register(plugin).expect("register wasm replacement");
        kernel.reconcile();

        let new_installation = kernel
            .installation_id_for_plugin(&plugin_id)
            .expect("installation id");
        assert_eq!(new_installation, installation_id);

        let caller = PrincipalId::new("replace-caller");
        kernel
            .register_principal_id(caller.clone(), PrincipalKind::Agent)
            .expect("reg caller");
        let cap = echo_capability();
        kernel
            .create_root_grant(
                caller.clone(),
                cap.contract(),
                [OPERATION_STATEFUL_INCREMENT],
                ResourceScope::Any,
                false,
                GrantLifetime::Persistent,
            )
            .expect("grant caller");

        let res = kernel
            .capability_for(caller, cap)
            .expect("cap handle")
            .invoke_with_options(
                OPERATION_STATEFUL_INCREMENT,
                b"from-wasm",
                RpcCallOptions::new().with_request_id("repl-2"),
            )
            .expect("wasm increment");
        assert_eq!(res, format_increment_result(2, b"from-wasm"));
    }
}
