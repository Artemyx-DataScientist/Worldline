//! Package and plugin identity vocabulary for the external plugin boundary.
//!
//! Identities in this module are distribution and naming vocabulary only:
//!
//! - [`PluginPackageId`] is the distribution/package identity.
//! - [`PluginDefinitionId`] is the logical plugin identity; one package
//!   distribution may provide several plugin definitions.
//! - [`PackageVersion`] is the package release version. It is independent
//!   from capability contract versions: capability contract compatibility is
//!   decided by the kernel's capability contract logic, never by comparing
//!   package versions.
//! - [`AbiVersion`] declares the external baseline (Component Model release,
//!   WASI release, native IPC protocol major) a package was built against.
//!
//! Invariant: package identity is distribution identity only and NEVER
//! grants authority. Knowing, trusting, or validating a package id does not
//! create capability authority; authority is granted exclusively by the
//! kernel's default-deny capability broker.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ProtocolError;

/// Maximum encoded length of a package or plugin definition identifier.
const MAX_ID_LEN: usize = 128;

/// Validates the shared identifier grammar: non-empty, at most
/// [`MAX_ID_LEN`] bytes, only lowercase ASCII letters, digits, `-` and `.`,
/// the first/last character must be a letter or digit (which rules out
/// leading/trailing `.` and `-`), and no dot-separated segment may be empty
/// (which rules out `..` anywhere).
fn validate_identifier(kind: &'static str, value: &str) -> Result<(), ProtocolError> {
    let invalid = |reason: &str| ProtocolError::InvalidPluginManifest {
        reason: format!("invalid {kind} {value:?}: {reason}"),
    };
    if value.is_empty() {
        return Err(invalid("identifier must not be empty"));
    }
    if value.len() > MAX_ID_LEN {
        return Err(invalid("identifier exceeds 128 bytes"));
    }
    let bytes = value.as_bytes();
    let is_boundary = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !is_boundary(bytes[0]) {
        return Err(invalid(
            "identifier must start with a lowercase ASCII letter or digit",
        ));
    }
    if !is_boundary(bytes[bytes.len() - 1]) {
        return Err(invalid(
            "identifier must end with a lowercase ASCII letter or digit",
        ));
    }
    if !bytes
        .iter()
        .all(|&b| is_boundary(b) || b == b'-' || b == b'.')
    {
        return Err(invalid(
            "identifier may contain only lowercase ASCII letters, digits, '-' and '.'",
        ));
    }
    if value.split('.').any(str::is_empty) {
        // Empty dotted segment: ".." anywhere inside the identifier.
        return Err(invalid(
            "identifier must not contain an empty dot-separated segment",
        ));
    }
    Ok(())
}

/// Distribution/package identity of an installed plugin package.
///
/// Invariant: package identity is distribution identity only and NEVER
/// grants authority. Two packages with different ids may implement the same
/// logical contract, and trusting one id does not extend any authority to it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginPackageId(String);

impl PluginPackageId {
    /// Validates and wraps a package identifier.
    pub fn try_new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        validate_identifier("plugin package id", &value)?;
        Ok(Self(value))
    }

    /// The validated identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginPackageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for PluginPackageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PluginPackageId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::try_new(value).map_err(serde::de::Error::custom)
    }
}

/// Logical plugin identity. A package distribution carries one or more plugin
/// definitions, each named by one id of this type.
///
/// Invariant: like package identity, a definition id is naming vocabulary and
/// never grants authority by itself.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginDefinitionId(String);

impl PluginDefinitionId {
    /// Validates and wraps a plugin definition identifier.
    pub fn try_new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        validate_identifier("plugin definition id", &value)?;
        Ok(Self(value))
    }

    /// The validated identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginDefinitionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for PluginDefinitionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PluginDefinitionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::try_new(value).map_err(serde::de::Error::custom)
    }
}

/// Release version of a plugin package (`MAJOR.MINOR.PATCH`).
///
/// Ordering follows numeric major/minor/patch comparison.
///
/// Invariant: `PackageVersion` is independent from capability contract
/// versions. Two packages at different package versions may implement the
/// same capability contract version, and the same package may change
/// implemented contract versions between package versions. Capability
/// contract compatibility is carried by the contract strings themselves and
/// decided by the kernel, never by comparing package versions.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PackageVersion {
    /// Major release number.
    pub major: u32,
    /// Minor release number.
    pub minor: u32,
    /// Patch release number.
    pub patch: u32,
}

impl PackageVersion {
    /// Builds a version from numeric components.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses `MAJOR.MINOR.PATCH`. Each component must be a non-empty,
    /// digit-only ASCII string that fits into a `u32`; signs, whitespace, and
    /// additional components are rejected.
    pub fn parse(value: &str) -> Result<Self, ProtocolError> {
        let invalid = || ProtocolError::InvalidPluginManifest {
            reason: format!("invalid package version {value:?}: expected MAJOR.MINOR.PATCH"),
        };
        let mut components = value.split('.');
        let parse_component = |component: Option<&str>| -> Result<u32, ProtocolError> {
            let component = component.ok_or_else(invalid)?;
            if component.is_empty() || !component.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            component.parse::<u32>().map_err(|_| invalid())
        };
        let major = parse_component(components.next())?;
        let minor = parse_component(components.next())?;
        let patch = parse_component(components.next())?;
        if components.next().is_some() {
            return Err(invalid());
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// External ABI baseline a package requires from the host.
///
/// This is a requirement statement about the environment: which Component
/// Model encoding and WASI release the package targets, and which major
/// version of the native IPC envelope protocol it speaks. It describes what a
/// package was built against; it never grants anything and it never relaxes
/// the kernel's capability checks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbiVersion {
    /// Component Model encoding release, e.g. `"0.3"`.
    pub component_model: String,
    /// WASI release, e.g. `"0.3"` (recorded fallback baseline: `"0.2"`).
    pub wasi: String,
    /// Required major version of the native IPC envelope protocol.
    pub protocol_major: u32,
}

impl AbiVersion {
    /// The v1 external baseline recorded by the architecture decision: WASI
    /// 0.3 on the Component Model, speaking native IPC protocol major 1.
    ///
    /// The recorded fallback (building the reference component for WASI 0.2
    /// with the same host) is a package-level declaration on the manifest, not
    /// a change of this baseline.
    #[must_use]
    pub fn baseline_v1() -> Self {
        Self {
            component_model: "0.3".to_string(),
            wasi: "0.3".to_string(),
            protocol_major: crate::envelope::PROTOCOL_VERSION,
        }
    }

    /// Validates that the textual baseline fields are non-empty.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.component_model.trim().is_empty() {
            return Err(ProtocolError::InvalidPluginManifest {
                reason: "required_abi.component_model must not be empty".to_string(),
            });
        }
        if self.wasi.trim().is_empty() {
            return Err(ProtocolError::InvalidPluginManifest {
                reason: "required_abi.wasi must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_id_accepts_valid_shapes() {
        for value in ["a", "clipboard-bridge", "acme.tools.clipboard-bridge", "v2"] {
            let id = PluginPackageId::try_new(value).expect("valid id");
            assert_eq!(id.as_str(), value);
            assert_eq!(id.to_string(), value);
        }
    }

    #[test]
    fn definition_id_rejects_invalid_shapes() {
        let long = "a".repeat(MAX_ID_LEN + 1);
        for value in [
            "",
            "UPPER",
            "with space",
            "-leading",
            "trailing-",
            "..",
            "a..b",
            ".leading-dot",
            "trailing-dot.",
            "under_score",
            "sla/sh",
            long.as_str(),
        ] {
            let error = PluginDefinitionId::try_new(value).expect_err("must reject");
            assert!(matches!(error, ProtocolError::InvalidPluginManifest { .. }));
        }
    }

    #[test]
    fn ids_serialize_as_plain_strings_and_revalidate_on_deserialize() {
        let id = PluginPackageId::try_new("acme.pkg").expect("valid");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, r#""acme.pkg""#);
        let round: PluginPackageId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round, id);
        assert!(serde_json::from_str::<PluginPackageId>(r#""Bad Id""#).is_err());
    }

    #[test]
    fn package_version_parses_orders_and_displays() {
        let version = PackageVersion::parse("1.2.3").expect("valid");
        assert_eq!(version, PackageVersion::new(1, 2, 3));
        assert_eq!(version.to_string(), "1.2.3");
        assert!(
            PackageVersion::parse("10.0.0").expect("valid")
                > PackageVersion::parse("9.99.99").expect("valid")
        );
        for bad in [
            "", "1", "1.2", "1.2.3.4", "v1.2.3", "1.2.x", "-1.2.3", "1.2.-3",
        ] {
            assert!(PackageVersion::parse(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn abi_version_validates_and_reports_v1_baseline() {
        let baseline = AbiVersion::baseline_v1();
        assert_eq!(baseline.protocol_major, crate::envelope::PROTOCOL_VERSION);
        baseline.validate().expect("baseline is valid");
        let empty = AbiVersion {
            component_model: "  ".to_string(),
            wasi: baseline.wasi.clone(),
            protocol_major: 1,
        };
        assert!(empty.validate().is_err());
    }
}
