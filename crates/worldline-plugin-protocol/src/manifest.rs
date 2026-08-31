//! Plugin package manifest schema (version 1).
//!
//! A manifest is the declarative description a package ships with. It carries
//! package identity and version, the plugin definitions it provides, its
//! execution mode, the external ABI baseline it requires, the capability
//! contracts it provides and requires, the host permissions it requests, its
//! resource limit hints, and the artifact path of its entrypoint relative to
//! the package root.
//!
//! Invariant: a manifest describes requested and declared permissions and
//! capabilities. It NEVER grants them. Loading a manifest never activates
//! plugin authority; authority is only ever granted later by the kernel's
//! default-deny capability broker.
//!
//! Compatibility: exactly one schema version is supported by this build.
//! Unknown schema versions and unknown fields fail closed.

use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;
use crate::identity::{AbiVersion, PackageVersion, PluginDefinitionId, PluginPackageId};

/// Manifest schema version understood by this build. Any other version fails
/// closed with [`ProtocolError::UnsupportedManifestSchema`].
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// How the host executes this package's plugin definitions.
///
/// Execution mode is a property of the package and host policy, not of the
/// logical capability contract: the same contract is served with the same
/// observable semantics in every mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Statically linked Rust code inside the host process (trusted platform
    /// adapter).
    Builtin,
    /// A separate supervised child process behind the versioned IPC envelope
    /// protocol.
    NativeProcess,
    /// A Component Model runtime sandbox with least-authority host imports.
    WasmComponent,
}

/// Classes of host permission a package can request.
///
/// A request in a manifest is not a grant: every class stays default-deny and
/// is only ever activated by an explicit host capability grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionClass {
    /// Scoped filesystem access.
    Filesystem,
    /// Scoped network access.
    Network,
    /// Wall-clock and monotonic time.
    Clock,
    /// Randomness source.
    Random,
    /// Environment variables.
    Environment,
}

/// One requested permission: a class plus the requested scope inside it.
///
/// This is a request record only. Nothing here activates authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedPermission {
    /// The permission class being requested.
    pub class: PermissionClass,
    /// The requested scope, interpreted by the host's permission policy for
    /// the class (for example `"readonly:workspace"`).
    pub scope: String,
}

/// Declared resource limit hints for the runtime hosting this package.
///
/// All fields are optional hints. They are inputs to host policy, not
/// authority: a host may clamp or reject them, and they never bypass the
/// capability broker.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimitHints {
    /// Maximum linear memory in bytes (WASM store limit / process budget).
    pub memory_bytes: Option<u64>,
    /// Maximum component/resource table entries.
    pub table_entries: Option<u32>,
    /// Maximum concurrent host calls.
    pub host_call_concurrency: Option<u32>,
    /// Maximum payload bytes per host call.
    pub host_call_payload_bytes: Option<u64>,
    /// Maximum invocation rate per second.
    pub invocation_rate_per_sec: Option<u32>,
    /// Maximum wall time per invocation in milliseconds.
    pub wall_time_ms: Option<u64>,
}

/// Declarative description of one plugin package.
///
/// Invariant: a manifest describes requested and declared permissions and
/// capabilities. It never grants them, and loading it never activates plugin
/// authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// Manifest schema version. Must equal [`MANIFEST_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Distribution identity of the package. Never grants authority.
    pub package_id: PluginPackageId,
    /// Package release version. Independent from capability contract
    /// versions.
    pub package_version: PackageVersion,
    /// Plugin definitions provided by this package.
    pub plugin_definitions: Vec<PluginDefinitionId>,
    /// How the host executes this package.
    pub execution_mode: ExecutionMode,
    /// External ABI baseline this package requires.
    pub required_abi: AbiVersion,
    /// Capability contracts this package provides to other plugins.
    pub provided_capability_contracts: Vec<String>,
    /// Capability contracts this package requires from the host.
    pub required_capability_contracts: Vec<String>,
    /// Host permissions requested by this package. Requests are not grants.
    pub requested_permissions: Vec<RequestedPermission>,
    /// Declared resource limit hints.
    pub resource_limit_hints: ResourceLimitHints,
    /// Entrypoint/component artifact path, relative to the package root and
    /// confined to it (relative, normal components only).
    pub artifact_path: String,
}

impl PluginManifest {
    /// Parses a manifest from a JSON document.
    ///
    /// Fails closed: unknown fields anywhere in the manifest are rejected by
    /// the parser, an unsupported `schema_version` yields
    /// [`ProtocolError::UnsupportedManifestSchema`], and a non-confining
    /// `artifact_path` yields [`ProtocolError::PackagePathViolation`].
    pub fn from_json(json: &str) -> Result<Self, ProtocolError> {
        let manifest: Self =
            serde_json::from_str(json).map_err(|error| ProtocolError::InvalidPluginManifest {
                reason: error.to_string(),
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Applies the semantic checks that go beyond JSON structure.
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedManifestSchema {
                found: self.schema_version,
            });
        }
        self.required_abi.validate()?;
        validate_package_path(&self.artifact_path)?;
        Ok(())
    }
}

/// Rejects any path that is not a relative path of normal components inside
/// the package root.
///
/// Rejected: empty paths, absolute paths (leading `/`), Windows backslash
/// separators, drive/backdrive forms (`C:`, `C:x`, `C:/x`), UNC paths
/// (`\\server\share`), `.`/`..` components, empty components (doubled
/// separators), embedded NUL, and Windows-reserved device names.
fn validate_package_path(path: &str) -> Result<(), ProtocolError> {
    let violation = || ProtocolError::PackagePathViolation {
        path: path.to_string(),
    };
    if path.is_empty() {
        return Err(violation());
    }
    if path.contains('\\') {
        // Package paths are forward-slash separated; backslashes are the
        // Windows separator and also how UNC paths (`\\server\share`) and
        // drive paths (`C:\x`) are written.
        return Err(violation());
    }
    if path.contains(':') {
        // Drive and backdrive forms ("C:", "C:x", "C:/x").
        return Err(violation());
    }
    if path.contains('\0') {
        return Err(violation());
    }
    if path.starts_with('/') || path.starts_with("//") {
        // Absolute POSIX form; UNC ("//server/share") is covered twice over.
        return Err(violation());
    }
    for component in path.split('/') {
        match component {
            "" => return Err(violation()),   // absolute path or doubled separator
            "." => return Err(violation()),  // non-normal component
            ".." => return Err(violation()), // traversal above the package root
            _ => {}
        }
        if is_windows_reserved_device_name(component) {
            return Err(violation());
        }
    }
    Ok(())
}

/// Windows-first hardening: names that collide with legacy DOS device names
/// are rejected for artifact paths (case-insensitively, on the stem before
/// any extension).
fn is_windows_reserved_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("");
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest_json() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "package_id": "acme.tools.clipboard-bridge",
            "package_version": {"major": 1, "minor": 2, "patch": 3},
            "plugin_definitions": ["clipboard-bridge"],
            "execution_mode": "wasm_component",
            "required_abi": {"component_model": "0.3", "wasi": "0.3", "protocol_major": 1},
            "provided_capability_contracts": ["worldline:clipboard@1"],
            "required_capability_contracts": [],
            "requested_permissions": [
                {"class": "filesystem", "scope": "readonly:workspace"},
                {"class": "clock", "scope": "wall-time"}
            ],
            "resource_limit_hints": {
                "memory_bytes": 33554432,
                "table_entries": 128,
                "host_call_concurrency": 4,
                "host_call_payload_bytes": 1048576,
                "invocation_rate_per_sec": 1000,
                "wall_time_ms": 5000
            },
            "artifact_path": "component/main.wasm"
        })
    }

    #[test]
    fn parses_valid_manifest() {
        let manifest =
            PluginManifest::from_json(&valid_manifest_json().to_string()).expect("valid manifest");
        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.package_id.as_str(), "acme.tools.clipboard-bridge");
        assert_eq!(manifest.package_version, PackageVersion::new(1, 2, 3));
        assert_eq!(manifest.execution_mode, ExecutionMode::WasmComponent);
        assert_eq!(manifest.required_abi, AbiVersion::baseline_v1());
        assert_eq!(manifest.requested_permissions.len(), 2);
        assert_eq!(
            manifest.requested_permissions[0].class,
            PermissionClass::Filesystem
        );
        assert_eq!(manifest.artifact_path, "component/main.wasm");
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let mut json = valid_manifest_json();
        json["schema_version"] = serde_json::json!(2);
        let error =
            PluginManifest::from_json(&json.to_string()).expect_err("schema 2 must fail closed");
        assert_eq!(error, ProtocolError::UnsupportedManifestSchema { found: 2 });
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let mut json = valid_manifest_json();
        json["auto_grant_all_permissions"] = serde_json::json!(true);
        let error = PluginManifest::from_json(&json.to_string())
            .expect_err("unknown field must fail closed");
        assert!(matches!(error, ProtocolError::InvalidPluginManifest { .. }));
    }

    #[test]
    fn rejects_wrong_typed_schema_version() {
        let mut json = valid_manifest_json();
        json["schema_version"] = serde_json::json!("1");
        let error =
            PluginManifest::from_json(&json.to_string()).expect_err("string schema must fail");
        assert!(matches!(error, ProtocolError::InvalidPluginManifest { .. }));
    }

    #[test]
    fn rejects_path_traversal_variants() {
        for path in [
            "../outside.wasm",
            "component/../../outside.wasm",
            ".",
            "./component/main.wasm",
            "/absolute/path.wasm",
            "//server/share/x.wasm",
            r"C:\x\y.wasm",
            r"\\server\share\x.wasm",
            "C:relative.wasm",
            "",
            "component//main.wasm",
            "NUL",
            "con.txt",
        ] {
            let mut json = valid_manifest_json();
            json["artifact_path"] = serde_json::json!(path);
            let error =
                PluginManifest::from_json(&json.to_string()).expect_err("path must be rejected");
            assert_eq!(
                error,
                ProtocolError::PackagePathViolation {
                    path: path.to_string()
                }
            );
        }
    }

    #[test]
    fn accepts_normal_relative_paths() {
        for path in ["main.wasm", "component/main.wasm", "a/b/c/deep.wasm"] {
            let mut json = valid_manifest_json();
            json["artifact_path"] = serde_json::json!(path);
            PluginManifest::from_json(&json.to_string())
                .unwrap_or_else(|error| panic!("{path} must be accepted: {error}"));
        }
    }

    #[test]
    fn rejects_malformed_identity_in_manifest() {
        let mut json = valid_manifest_json();
        json["package_id"] = serde_json::json!("Bad Package Id");
        let error =
            PluginManifest::from_json(&json.to_string()).expect_err("bad id must fail closed");
        assert!(matches!(error, ProtocolError::InvalidPluginManifest { .. }));
    }

    #[test]
    fn rejects_unknown_field_in_nested_structs() {
        let mut json = valid_manifest_json();
        json["resource_limit_hints"]["magic_slots"] = serde_json::json!(7);
        let error =
            PluginManifest::from_json(&json.to_string()).expect_err("nested unknown must fail");
        assert!(matches!(error, ProtocolError::InvalidPluginManifest { .. }));
    }

    #[test]
    fn permission_classes_use_snake_case_names() {
        let classes = [
            (PermissionClass::Filesystem, "filesystem"),
            (PermissionClass::Network, "network"),
            (PermissionClass::Clock, "clock"),
            (PermissionClass::Random, "random"),
            (PermissionClass::Environment, "environment"),
        ];
        for (class, name) in classes {
            let json = serde_json::to_string(&class).expect("serialize");
            assert_eq!(json, format!(r#""{name}""#));
        }
    }

    #[test]
    fn execution_modes_use_snake_case_names() {
        let modes = [
            (ExecutionMode::Builtin, "builtin"),
            (ExecutionMode::NativeProcess, "native_process"),
            (ExecutionMode::WasmComponent, "wasm_component"),
        ];
        for (mode, name) in modes {
            let json = serde_json::to_string(&mode).expect("serialize");
            assert_eq!(json, format!(r#""{name}""#));
        }
    }

    #[test]
    fn empty_limit_hints_deserialize_with_default() {
        let mut json = valid_manifest_json();
        json["resource_limit_hints"] = serde_json::json!({});
        let manifest = PluginManifest::from_json(&json.to_string()).expect("empty hints are valid");
        assert_eq!(manifest.resource_limit_hints, ResourceLimitHints::default());
    }
}
