//! Acceptance tests for the external plugin boundary protocol vocabulary
//! (task T-002 of C-KERNEL-STABLE-IPC-WASM-COMPONENT-BOUNDARY-20260831).
//!
//! These tests exercise the fail-closed behavior required at the boundary:
//! unknown schema/protocol versions, unknown fields, path traversal, and
//! oversized frames must never be accepted or crash the host.

use serde_json::json;
use worldline_plugin_protocol::{
    AbiVersion, Envelope, ExecutionMode, MessageKind, PROTOCOL_VERSION, PackageVersion,
    PluginDefinitionId, PluginManifest, PluginPackageId, ProtocolError,
};

fn valid_manifest_json() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "package_id": "acme.tools.clipboard-bridge",
        "package_version": {"major": 1, "minor": 2, "patch": 3},
        "plugin_definitions": ["clipboard-bridge"],
        "execution_mode": "native_process",
        "required_abi": {"component_model": "0.3", "wasi": "0.3", "protocol_major": 1},
        "provided_capability_contracts": ["worldline:clipboard@1"],
        "required_capability_contracts": ["worldline:clock@1"],
        "requested_permissions": [
            {"class": "filesystem", "scope": "readonly:workspace"}
        ],
        "resource_limit_hints": {
            "memory_bytes": 67108864,
            "host_call_payload_bytes": 1048576
        },
        "artifact_path": "bin/bridge.exe"
    })
}

#[test]
fn manifest_with_unknown_schema_version_fails_closed() {
    let mut manifest = valid_manifest_json();
    manifest["schema_version"] = json!(999);
    let error = PluginManifest::from_json(&manifest.to_string()).expect_err("must fail closed");
    assert_eq!(
        error,
        ProtocolError::UnsupportedManifestSchema { found: 999 }
    );
    // A negative or fractional version cannot even be represented as u32, so
    // it surfaces as a structural manifest error, also failing closed.
    let mut manifest = valid_manifest_json();
    manifest["schema_version"] = json!(-1);
    let error = PluginManifest::from_json(&manifest.to_string()).expect_err("must fail closed");
    assert!(matches!(error, ProtocolError::InvalidPluginManifest { .. }));
}

#[test]
fn manifest_with_unknown_top_level_field_fails_closed() {
    let mut manifest = valid_manifest_json();
    manifest["silent_extra_permissions"] = json!(["everything"]);
    let error = PluginManifest::from_json(&manifest.to_string()).expect_err("must fail closed");
    assert!(matches!(error, ProtocolError::InvalidPluginManifest { .. }));
}

#[test]
fn path_traversal_variants_are_rejected() {
    let traversal_paths = [
        "../outside.wasm",
        "bin/../../outside.wasm",
        "/absolute/path.wasm",
        r"C:\x\y.wasm",
        r"\\server\share\y.wasm",
        "C:relative.wasm",
        "C:/x/y.wasm",
        "//server/share/y.wasm",
        "",
        ".",
        "./bin/bridge.exe",
        "bin//bridge.exe",
    ];
    for path in traversal_paths {
        let mut manifest = valid_manifest_json();
        manifest["artifact_path"] = json!(path);
        let error =
            PluginManifest::from_json(&manifest.to_string()).expect_err("path must be rejected");
        assert_eq!(
            error,
            ProtocolError::PackagePathViolation {
                path: path.to_string()
            },
            "path {path:?} must map to PackagePathViolation"
        );
    }
}

#[test]
fn valid_manifest_parses_with_all_fields() {
    let manifest =
        PluginManifest::from_json(&valid_manifest_json().to_string()).expect("valid manifest");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.package_id.as_str(), "acme.tools.clipboard-bridge");
    assert_eq!(manifest.package_version, PackageVersion::new(1, 2, 3));
    assert_eq!(
        manifest
            .plugin_definitions
            .iter()
            .map(PluginDefinitionId::as_str)
            .collect::<Vec<_>>(),
        vec!["clipboard-bridge"]
    );
    assert_eq!(manifest.execution_mode, ExecutionMode::NativeProcess);
    assert_eq!(manifest.required_abi.protocol_major, PROTOCOL_VERSION);
    assert_eq!(
        manifest.provided_capability_contracts,
        vec!["worldline:clipboard@1"]
    );
    assert_eq!(
        manifest.required_capability_contracts,
        vec!["worldline:clock@1"]
    );
    assert_eq!(manifest.requested_permissions.len(), 1);
    assert_eq!(manifest.resource_limit_hints.memory_bytes, Some(67_108_864));
    assert_eq!(manifest.resource_limit_hints.table_entries, None);
    assert_eq!(manifest.artifact_path, "bin/bridge.exe");

    // A manifest describes requests; it has no way to represent grants.
    assert_eq!(AbiVersion::baseline_v1().protocol_major, 1);
}

#[test]
fn envelope_roundtrips_encode_decode() {
    for kind in [
        MessageKind::LifecycleRequest,
        MessageKind::LifecycleResult,
        MessageKind::CapabilityRequest,
        MessageKind::CapabilityResult,
        MessageKind::RequestPolicyRequest,
        MessageKind::RequestPolicyResult,
        MessageKind::Cancellation,
        MessageKind::EventPublishRequest,
        MessageKind::StateRequest,
        MessageKind::StateResult,
        MessageKind::BlobRequest,
        MessageKind::BlobResult,
        MessageKind::ProtocolError,
    ] {
        let envelope = Envelope::new(kind, 7, json!({"note": "payload is opaque"}));
        let bytes = envelope.encode().expect("encode");
        let decoded = Envelope::decode(&bytes, 4096).expect("decode");
        assert_eq!(decoded, envelope, "kind {kind:?} must roundtrip");
    }
}

#[test]
fn oversized_frame_is_rejected_before_parsing() {
    // 64 KiB of garbage: not valid JSON at all. If the size gate ran after
    // parsing this would be a MalformedEnvelope or a panic, never this.
    let garbage = vec![b'\0'; 64 * 1024];
    let error = Envelope::decode(&garbage, 1024).expect_err("must be rejected by size gate");
    assert_eq!(
        error,
        ProtocolError::PayloadTooLarge {
            limit: 1024,
            actual: 64 * 1024
        }
    );

    // A syntactically plausible but oversized frame is also size-gated first:
    // it is rejected without ever being parsed.
    let oversized = vec![b' '; 16 * 1024];
    let error = Envelope::decode(&oversized, 1024).expect_err("must be rejected by size gate");
    assert!(matches!(error, ProtocolError::PayloadTooLarge { .. }));
}

#[test]
fn unsupported_protocol_version_is_deterministic() {
    let decode_with = |version: u32| {
        let frame = json!({
            "protocol_version": version,
            "message_kind": "lifecycle_request",
            "correlation_id": 1,
            "payload": {}
        });
        Envelope::decode(frame.to_string().as_bytes(), 4096)
    };
    let error = decode_with(2).expect_err("must fail closed");
    assert_eq!(
        error,
        ProtocolError::UnsupportedProtocolVersion { found: 2 }
    );
    // Same input, same variant: classification does not depend on parse
    // ordering or string content.
    assert_eq!(decode_with(2).expect_err("deterministic"), error);
    // Different unknown versions are told apart.
    assert_ne!(
        decode_with(3).expect_err("distinct version"),
        ProtocolError::UnsupportedProtocolVersion { found: 2 }
    );
    assert_eq!(
        decode_with(PROTOCOL_VERSION)
            .expect("current version decodes")
            .protocol_version,
        PROTOCOL_VERSION
    );
}

#[test]
fn package_version_parse_order_and_independence() {
    let version = PackageVersion::parse("1.2.3").expect("valid");
    assert_eq!(version, PackageVersion::new(1, 2, 3));
    assert_eq!(version.to_string(), "1.2.3");

    let v1_0_0 = PackageVersion::parse("1.0.0").expect("valid");
    let v1_0_1 = PackageVersion::parse("1.0.1").expect("valid");
    let v1_1_0 = PackageVersion::parse("1.1.0").expect("valid");
    let v2_0_0 = PackageVersion::parse("2.0.0").expect("valid");
    assert!(v1_0_0 < v1_0_1);
    assert!(v1_0_1 < v1_1_0);
    assert!(v1_1_0 < v2_0_0);

    // Independence from capability contract versions: the same contract
    // string is served by packages at different package versions, and
    // package version ordering says nothing about contract compatibility.
    let contract = "worldline:clipboard@1";
    let older = PackageVersion::parse("0.9.0").expect("valid");
    let newer = PackageVersion::parse("9.4.1").expect("valid");
    assert!(older < newer);
    assert_eq!(contract, "worldline:clipboard@1");
    assert_ne!(older, newer);

    for bad in [
        "1", "1.2", "1.2.3.4", "v1.2.3", "1.2.x", "-1.2.3", "01.02..3",
    ] {
        assert!(
            PackageVersion::parse(bad).is_err(),
            "package version {bad:?} must be rejected"
        );
    }
}

#[test]
fn identifier_validation_rejects_bad_shapes() {
    let long = "a".repeat(129);
    let bad_ids = [
        "",
        "UPPERCASE",
        "has space",
        "-leading-dash",
        "trailing-dash-",
        "..",
        ".hidden",
        "under_score",
        "semi;colon",
        "sla/sh",
        long.as_str(),
    ];
    for bad in bad_ids {
        assert!(
            PluginPackageId::try_new(bad).is_err(),
            "package id {bad:?} must be rejected"
        );
        assert!(
            PluginDefinitionId::try_new(bad).is_err(),
            "definition id {bad:?} must be rejected"
        );
    }

    let package = PluginPackageId::try_new("acme.tools.clipboard-bridge").expect("valid");
    let definition = PluginDefinitionId::try_new("clipboard-bridge").expect("valid");
    // Distinct types: a package id is not interchangeable with a definition
    // id even when their text looks similar.
    assert_ne!(package.as_str(), definition.as_str());
    assert_eq!(package.as_str(), "acme.tools.clipboard-bridge");
}
