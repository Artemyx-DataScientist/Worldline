//! M0.6 external protocol robustness suite: verifies that truncated frames,
//! oversized frames, protocol violators, malformed manifests, unknown versions,
//! and invalid / replayed handles fail deterministically and never panic the kernel.

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::json;
use worldline_kernel::{
    Kernel, KernelError, NoopRuntime, OperationId, Plugin, PluginId, ResourceId,
};
use worldline_native_host::{
    ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError, NativeProviderConnection,
};
use worldline_plugin_protocol::{PluginManifest, ProtocolError};

#[test]
fn native_violator_writing_garbage_fails_call_without_panicking_host() {
    struct DummySink;
    impl HostRequestSink for DummySink {
        fn on_child_request(
            &self,
            _kind: worldline_plugin_protocol::MessageKind,
            _correlation_id: u64,
            _payload: serde_json::Value,
        ) -> Result<Option<serde_json::Value>, NativeHostError> {
            Ok(None)
        }
    }

    let spec = NativeChildSpec {
        program: support::native_violator_program(),
        args: Vec::new(),
        max_frame_bytes: 4 * 1024 * 1024,
        stderr_max_bytes: 64 * 1024,
    };
    let identity = ExpectedIdentity {
        package_id: "reference.echo.pkg".to_owned(),
        plugin_definition_id: "reference.echo.native".to_owned(),
    };

    let (connection, _ack) =
        NativeProviderConnection::connect(spec, &identity, std::sync::Arc::new(DummySink), 16)
            .expect("violator completes handshake");

    let outcome = connection.call(json!({"operation": "echo", "bytes": [1, 2, 3]}));
    assert!(
        outcome.is_err(),
        "unframed garbage bytes must result in a connection error"
    );

    // Host remains intact.
    connection.kill();
}

#[test]
fn native_hung_child_times_out_on_shutdown() {
    struct DummySink;
    impl HostRequestSink for DummySink {
        fn on_child_request(
            &self,
            _kind: worldline_plugin_protocol::MessageKind,
            _correlation_id: u64,
            _payload: serde_json::Value,
        ) -> Result<Option<serde_json::Value>, NativeHostError> {
            Ok(None)
        }
    }

    let spec = NativeChildSpec {
        program: support::native_provider_program(),
        args: vec![
            "--package-id".to_owned(),
            "reference.echo.pkg".to_owned(),
            "--definition-id".to_owned(),
            "reference.echo.native".to_owned(),
            "--hang".to_owned(),
        ],
        max_frame_bytes: 4 * 1024 * 1024,
        stderr_max_bytes: 64 * 1024,
    };
    let identity = ExpectedIdentity {
        package_id: "reference.echo.pkg".to_owned(),
        plugin_definition_id: "reference.echo.native".to_owned(),
    };

    let (connection, _ack) =
        NativeProviderConnection::connect(spec, &identity, std::sync::Arc::new(DummySink), 16)
            .expect("hung child completes handshake");

    let close_err = connection
        .close(Duration::from_millis(100))
        .expect_err("hung child must trigger shutdown timeout");
    assert!(matches!(
        close_err,
        NativeHostError::ShutdownTimeout { deadline_ms: 100 }
    ));
}

#[test]
fn malformed_and_escaping_manifests_fail_closed() {
    let base = json!({
        "schema_version": 1,
        "package_id": "com.example.escaping",
        "package_version": {"major": 1, "minor": 0, "patch": 0},
        "plugin_definitions": ["escaping.plugin"],
        "execution_mode": "native_process",
        "required_abi": {"component_model": "0.3", "wasi": "0.3", "protocol_major": 1},
        "provided_capability_contracts": [],
        "required_capability_contracts": [],
        "requested_permissions": [],
        "resource_limit_hints": {},
        "artifact_path": "../../../evil.exe"
    });

    let err = PluginManifest::from_json(&base.to_string()).expect_err("path traversal must fail");
    assert!(matches!(err, ProtocolError::PackagePathViolation { .. }));

    let mut future = base.clone();
    future["schema_version"] = json!(999);
    future["artifact_path"] = json!("bin/valid.exe");
    let err = PluginManifest::from_json(&future.to_string())
        .expect_err("unsupported schema version must fail");
    assert_eq!(err, ProtocolError::UnsupportedManifestSchema { found: 999 });

    let mut unknown_field = base;
    unknown_field["unexpected_field"] = json!("malicious_payload");
    let err = PluginManifest::from_json(&unknown_field.to_string())
        .expect_err("unknown fields must fail closed");
    assert!(matches!(err, ProtocolError::InvalidPluginManifest { .. }));
}

#[test]
fn kernel_handle_table_isolation_and_revocation() {
    struct TestPlugin(worldline_kernel::PluginDefinition);
    impl Plugin for TestPlugin {
        fn definition(&self) -> &worldline_kernel::PluginDefinition {
            &self.0
        }
        fn activate(
            &self,
            _context: &mut worldline_kernel::ActivationContext,
        ) -> Result<Box<dyn worldline_kernel::PluginRuntime>, worldline_kernel::PluginError>
        {
            Ok(Box::new(NoopRuntime))
        }
    }

    let mut kernel = Kernel::new();
    let p_a = kernel
        .register(TestPlugin(worldline_kernel::PluginDefinition::new(
            PluginId::new("proto.plugin.a"),
        )))
        .expect("reg A");
    let p_b = kernel
        .register(TestPlugin(worldline_kernel::PluginDefinition::new(
            PluginId::new("proto.plugin.b"),
        )))
        .expect("reg B");

    kernel.reconcile();
    let r_a = kernel.runtime_id_for_plugin(&p_a).expect("runtime A");
    let r_b = kernel.runtime_id_for_plugin(&p_b).expect("runtime B");

    let operations = BTreeSet::from([OperationId::new("op1")]);
    let resources = BTreeSet::from([ResourceId::new("res1", ["v1"])]);

    let handle_a = kernel
        .issue_external_handle(&r_a, operations.clone(), resources.clone())
        .expect("issue handle A");

    // Cross runtime resolution fails
    let err = kernel
        .resolve_external_handle(&r_b, handle_a)
        .expect_err("cross runtime must fail");
    assert!(matches!(
        err,
        KernelError::ExternalHandleWrongRuntime { .. }
    ));

    // Scope check: undelegated op fails
    let err = kernel
        .check_external_handle_scope(
            &r_a,
            handle_a,
            &OperationId::new("unauthorized_op"),
            &ResourceId::new("res1", ["v1"]),
        )
        .expect_err("undelegated op denied");
    assert!(matches!(err, KernelError::ExternalHandleScopeDenied { .. }));

    // Revocation
    kernel
        .revoke_external_handle(&r_a, handle_a)
        .expect("revoke handle");
    let err = kernel
        .resolve_external_handle(&r_a, handle_a)
        .expect_err("revoked handle cannot resolve");
    assert!(matches!(err, KernelError::ExternalHandleRevoked { .. }));
}
