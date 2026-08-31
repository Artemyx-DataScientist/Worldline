//! Typed failures of the external plugin boundary protocol.
//!
//! Every failure is deterministic: the same violating input always maps to the
//! same variant, so a host can classify a protocol violation and decide on
//! termination or quarantine of the offending runtime without parsing error
//! strings. None of these failures crash the host by themselves.

use std::fmt;

/// Typed protocol failures for the Worldline external plugin boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    /// A wire envelope declared a native IPC protocol version this build does
    /// not support. Unknown protocol versions fail closed.
    UnsupportedProtocolVersion {
        /// The protocol version found on the wire.
        found: u32,
    },
    /// A manifest declared a schema version this build does not support.
    /// Unknown manifest schemas fail closed.
    UnsupportedManifestSchema {
        /// The manifest schema version declared by the document.
        found: u32,
    },
    /// The manifest is structurally invalid or violates a documented rule:
    /// malformed identity, malformed package version, unsupported ABI
    /// description, or an unknown field rejected by the fail-closed parser.
    InvalidPluginManifest {
        /// Explanation of the violation.
        reason: String,
    },
    /// A manifest path escapes the package root or is not a relative path of
    /// normal components (absolute path, `..` traversal, drive/backdrive or
    /// UNC form, empty, or a Windows-reserved device name).
    PackagePathViolation {
        /// The offending path exactly as it appeared in the source document.
        path: String,
    },
    /// A frame exceeded the declared boundary size limit. This is checked
    /// before any parsing or allocation happens.
    PayloadTooLarge {
        /// The configured maximum frame size in bytes.
        limit: usize,
        /// The actual frame size in bytes.
        actual: usize,
    },
    /// The frame is not a well-formed envelope of the supported protocol:
    /// invalid JSON, missing wire fields, unknown wire fields, or wrong field
    /// types.
    MalformedEnvelope {
        /// Explanation of the violation.
        reason: String,
    },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProtocolVersion { found } => {
                write!(formatter, "unsupported envelope protocol version: {found}")
            }
            Self::UnsupportedManifestSchema { found } => {
                write!(formatter, "unsupported manifest schema version: {found}")
            }
            Self::InvalidPluginManifest { reason } => {
                write!(formatter, "invalid plugin manifest: {reason}")
            }
            Self::PackagePathViolation { path } => write!(
                formatter,
                "package path is not a relative normal path inside the package root: {path:?}"
            ),
            Self::PayloadTooLarge { limit, actual } => {
                write!(
                    formatter,
                    "boundary frame too large: {actual} bytes exceeds the {limit} byte limit"
                )
            }
            Self::MalformedEnvelope { reason } => {
                write!(formatter, "malformed envelope: {reason}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}
