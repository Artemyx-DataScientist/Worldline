//! Contract stability and machine-checkable compatibility matrix model.
//!
//! Stability model rules (see `docs/adr/ADR-OPERABILITY-COMPATIBILITY-UPGRADE-V1.md`):
//! - `Stable`: follows semantic compatibility rules across supported major lines.
//!   Minor evolution allows backward-compatible optional extensions. Breaking changes require a new major.
//!   Supported SDK/kernel baselines maintain an N / N-1 / N-2 compatibility promise.
//! - `Experimental`: exact supported range only, no N-2 promise; incompatibility is detected before activation.
//!
//! Invariants:
//! - Stability class is metadata of a contract version line, not of a provider implementation.
//! - A provider cannot self-declare an incompatible implementation as `Stable` to force acceptance.
//! - Compatibility classification is machine-checkable and NEVER confers capability authority.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::identity::AbiVersion;

/// Stability lifecycle class of a capability or plugin contract.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStability {
    /// Follows semantic versioning and N/N-1/N-2 negotiation.
    Stable,
    /// Exact supported range only; breaking changes permitted between minor releases.
    Experimental,
}

impl fmt::Display for ContractStability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stable => formatter.write_str("stable"),
            Self::Experimental => formatter.write_str("experimental"),
        }
    }
}

/// Parsed or declared capability contract specification.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ContractSpec {
    /// Capability namespace (e.g. "reference.echo").
    pub namespace: String,
    /// Capability name (e.g. "echo").
    pub name: String,
    /// Major interface version.
    pub major: u32,
    /// Minor interface version.
    pub minor: u32,
    /// Stability lifecycle class.
    pub stability: ContractStability,
}

impl ContractSpec {
    /// Constructs a contract specification.
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        major: u32,
        minor: u32,
        stability: ContractStability,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            major,
            minor,
            stability,
        }
    }

    /// Parses a contract spec string like `"reference.echo/echo@1.2"` or `"experimental:reference.echo/echo@0.1"`.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let (stability, rest) = if let Some(stripped) = raw.strip_prefix("experimental:") {
            (ContractStability::Experimental, stripped)
        } else if let Some(stripped) = raw.strip_prefix("stable:") {
            (ContractStability::Stable, stripped)
        } else {
            (ContractStability::Stable, raw)
        };

        let parts: Vec<&str> = rest.split('@').collect();
        if parts.len() != 2 {
            return Err(format!(
                "invalid contract format {raw:?}, expected <namespace>/<name>@<major>.<minor>"
            ));
        }

        let path_parts: Vec<&str> = parts[0].split('/').collect();
        if path_parts.len() != 2 {
            return Err(format!(
                "invalid contract path in {raw:?}, expected <namespace>/<name>"
            ));
        }

        let namespace = path_parts[0].trim().to_string();
        let name = path_parts[1].trim().to_string();

        let ver_parts: Vec<&str> = parts[1].split('.').collect();
        if ver_parts.is_empty() || ver_parts.len() > 3 {
            return Err(format!("invalid contract version in {raw:?}"));
        }

        let major = ver_parts[0]
            .parse::<u32>()
            .map_err(|e| format!("invalid major in {raw:?}: {e}"))?;
        let minor = if ver_parts.len() > 1 {
            ver_parts[1]
                .parse::<u32>()
                .map_err(|e| format!("invalid minor in {raw:?}: {e}"))?
        } else {
            0
        };

        Ok(Self {
            namespace,
            name,
            major,
            minor,
            stability,
        })
    }

    /// Evaluates compatibility when `self` is the provided contract and `required` is the consumer demand.
    #[must_use]
    pub fn evaluate_compatibility(&self, required: &ContractSpec) -> ContractCompatibilityOutcome {
        if self.namespace != required.namespace || self.name != required.name {
            return ContractCompatibilityOutcome::NameMismatch;
        }

        if self.stability != required.stability {
            return ContractCompatibilityOutcome::StabilityMismatch {
                required: required.stability,
                provided: self.stability,
            };
        }

        match self.stability {
            ContractStability::Stable => {
                if self.major != required.major {
                    ContractCompatibilityOutcome::IncompatibleMajor {
                        required_major: required.major,
                        provided_major: self.major,
                    }
                } else if self.minor < required.minor {
                    ContractCompatibilityOutcome::IncompatibleMinor {
                        required_minor: required.minor,
                        provided_minor: self.minor,
                    }
                } else {
                    ContractCompatibilityOutcome::Compatible {
                        negotiated_major: required.major,
                        negotiated_minor: required.minor,
                    }
                }
            }
            ContractStability::Experimental => {
                if self.major == required.major && self.minor == required.minor {
                    ContractCompatibilityOutcome::Compatible {
                        negotiated_major: required.major,
                        negotiated_minor: required.minor,
                    }
                } else {
                    ContractCompatibilityOutcome::IncompatibleExperimental {
                        required: format!("{}.{}", required.major, required.minor),
                        provided: format!("{}.{}", self.major, self.minor),
                        reason: "experimental contracts require exact major.minor match"
                            .to_string(),
                    }
                }
            }
        }
    }
}

impl fmt::Display for ContractSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.stability == ContractStability::Experimental {
            write!(
                formatter,
                "experimental:{}/{}@{}.{}",
                self.namespace, self.name, self.major, self.minor
            )
        } else {
            write!(
                formatter,
                "{}/{}@{}.{}",
                self.namespace, self.name, self.major, self.minor
            )
        }
    }
}

/// Outcome of contract compatibility resolution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ContractCompatibilityOutcome {
    /// Contracts are compatible with negotiated version.
    Compatible {
        negotiated_major: u32,
        negotiated_minor: u32,
    },
    /// Major version mismatch on a stable contract without an adapter.
    IncompatibleMajor {
        required_major: u32,
        provided_major: u32,
    },
    /// Provided provider lacks required minor features.
    IncompatibleMinor {
        required_minor: u32,
        provided_minor: u32,
    },
    /// Incompatible version on an experimental contract line.
    IncompatibleExperimental {
        required: String,
        provided: String,
        reason: String,
    },
    /// Stability class mismatch (e.g. required Stable but provided Experimental).
    StabilityMismatch {
        required: ContractStability,
        provided: ContractStability,
    },
    /// Capability name or namespace mismatch.
    NameMismatch,
}

impl ContractCompatibilityOutcome {
    /// Returns true if the outcome represents successful compatibility.
    #[must_use]
    pub const fn is_compatible(&self) -> bool {
        matches!(self, Self::Compatible { .. })
    }
}

/// Supported SDK baselines for N, N-1, and N-2 matrix checks.
pub const SUPPORTED_SDK_VERSIONS: &[&str] = &["0.7", "0.6", "0.5"];

/// Checks whether an external ABI baseline is supported by this kernel build.
pub fn evaluate_abi_compatibility(required_abi: &AbiVersion) -> Result<(), String> {
    required_abi.validate().map_err(|e| e.to_string())?;

    let baseline = AbiVersion::baseline_v1();
    if required_abi.protocol_major != baseline.protocol_major {
        return Err(format!(
            "unsupported native IPC protocol major {}: expected {}",
            required_abi.protocol_major, baseline.protocol_major
        ));
    }

    // Component model baseline check (supports 0.3, fallback 0.2)
    if required_abi.component_model != "0.3" && required_abi.component_model != "0.2" {
        return Err(format!(
            "unsupported component model version {:?}: expected 0.3 or 0.2",
            required_abi.component_model
        ));
    }

    // WASI baseline check (supports 0.3, fallback 0.2)
    if required_abi.wasi != "0.3" && required_abi.wasi != "0.2" {
        return Err(format!(
            "unsupported WASI version {:?}: expected 0.3 or 0.2",
            required_abi.wasi
        ));
    }

    Ok(())
}

/// Checks if an SDK version string is within the supported N/N-1/N-2 matrix.
#[must_use]
pub fn is_supported_sdk_version(sdk_version: &str) -> bool {
    SUPPORTED_SDK_VERSIONS.contains(&sdk_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_contract_allows_higher_minor_provider() {
        let req = ContractSpec::new("test", "echo", 1, 0, ContractStability::Stable);
        let prov = ContractSpec::new("test", "echo", 1, 2, ContractStability::Stable);
        let outcome = prov.evaluate_compatibility(&req);
        assert_eq!(
            outcome,
            ContractCompatibilityOutcome::Compatible {
                negotiated_major: 1,
                negotiated_minor: 0,
            }
        );
        assert!(outcome.is_compatible());
    }

    #[test]
    fn stable_contract_rejects_lower_minor_provider() {
        let req = ContractSpec::new("test", "echo", 1, 2, ContractStability::Stable);
        let prov = ContractSpec::new("test", "echo", 1, 0, ContractStability::Stable);
        let outcome = prov.evaluate_compatibility(&req);
        assert_eq!(
            outcome,
            ContractCompatibilityOutcome::IncompatibleMinor {
                required_minor: 2,
                provided_minor: 0,
            }
        );
        assert!(!outcome.is_compatible());
    }

    #[test]
    fn stable_contract_rejects_different_major() {
        let req = ContractSpec::new("test", "echo", 1, 0, ContractStability::Stable);
        let prov = ContractSpec::new("test", "echo", 2, 0, ContractStability::Stable);
        let outcome = prov.evaluate_compatibility(&req);
        assert_eq!(
            outcome,
            ContractCompatibilityOutcome::IncompatibleMajor {
                required_major: 1,
                provided_major: 2,
            }
        );
        assert!(!outcome.is_compatible());
    }

    #[test]
    fn experimental_contract_requires_exact_minor() {
        let req = ContractSpec::new("test", "echo", 0, 1, ContractStability::Experimental);
        let prov_match = ContractSpec::new("test", "echo", 0, 1, ContractStability::Experimental);
        let prov_higher = ContractSpec::new("test", "echo", 0, 2, ContractStability::Experimental);

        assert!(prov_match.evaluate_compatibility(&req).is_compatible());
        let outcome_higher = prov_higher.evaluate_compatibility(&req);
        assert!(!outcome_higher.is_compatible());
        assert!(matches!(
            outcome_higher,
            ContractCompatibilityOutcome::IncompatibleExperimental { .. }
        ));
    }

    #[test]
    fn parsing_contract_specs_roundtrips() {
        let spec_stable = ContractSpec::parse("reference.echo/echo@1.2").expect("valid");
        assert_eq!(spec_stable.stability, ContractStability::Stable);
        assert_eq!(spec_stable.namespace, "reference.echo");
        assert_eq!(spec_stable.name, "echo");
        assert_eq!(spec_stable.major, 1);
        assert_eq!(spec_stable.minor, 2);

        let spec_exp = ContractSpec::parse("experimental:reference.echo/echo@0.1").expect("valid");
        assert_eq!(spec_exp.stability, ContractStability::Experimental);
        assert_eq!(spec_exp.major, 0);
        assert_eq!(spec_exp.minor, 1);
    }

    #[test]
    fn abi_and_sdk_matrix_validation() {
        let valid_abi = AbiVersion::baseline_v1();
        assert!(evaluate_abi_compatibility(&valid_abi).is_ok());

        let unsupported_protocol = AbiVersion {
            component_model: "0.3".to_string(),
            wasi: "0.3".to_string(),
            protocol_major: 99,
        };
        assert!(evaluate_abi_compatibility(&unsupported_protocol).is_err());

        assert!(is_supported_sdk_version("0.7"));
        assert!(is_supported_sdk_version("0.6"));
        assert!(is_supported_sdk_version("0.5"));
        assert!(!is_supported_sdk_version("0.1"));
    }
}
