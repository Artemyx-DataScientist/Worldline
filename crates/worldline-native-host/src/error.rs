//! Typed failures of the native-process transport. Every failure maps into
//! a deterministic classification; malformed external bytes never panic
//! the host.

use std::fmt;

/// Typed native transport failure classes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeHostError {
    /// The child answered with a protocol version the host does not speak.
    UnsupportedProtocolVersion { found: u32 },
    /// The handshake failed or the child's declared identity did not match
    /// the identity the host expected.
    HandshakeFailed { reason: String },
    /// The child violated the framed envelope protocol.
    ProtocolViolation { reason: String },
    /// The transport closed before a correlated reply arrived.
    TransportClosed,
    /// The child process crashed abnormally.
    ProcessCrashed { status: String },
    /// The child process exited with the given code.
    ProcessExited { code: i32 },
    /// More than the configured number of requests are already in flight.
    InvocationLimitExceeded { limit: usize },
    /// A frame exceeded the configured bound; rejected before allocation.
    PayloadTooLarge { limit: usize, actual: usize },
    /// The correlated reply did not arrive within the deadline.
    DeadlineExceeded { deadline_ms: u64 },
    /// The child returned an explicit guest-style error payload.
    GuestReturnedError { message: String },
    /// The child executable could not be spawned.
    SpawnFailed { reason: String },
    /// Graceful shutdown did not complete in time and the child was killed.
    ShutdownTimeout { deadline_ms: u64 },
}

impl fmt::Display for NativeHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProtocolVersion { found } => write!(
                formatter,
                "native child speaks unsupported protocol version {found}"
            ),
            Self::HandshakeFailed { reason } => {
                write!(formatter, "native handshake failed: {reason}")
            }
            Self::ProtocolViolation { reason } => {
                write!(formatter, "native protocol violation: {reason}")
            }
            Self::TransportClosed => formatter.write_str("native transport closed unexpectedly"),
            Self::ProcessCrashed { status } => {
                write!(formatter, "native child process crashed: {status}")
            }
            Self::ProcessExited { code } => {
                write!(formatter, "native child process exited with code {code}")
            }
            Self::InvocationLimitExceeded { limit } => write!(
                formatter,
                "native transport in-flight limit of {limit} requests exceeded"
            ),
            Self::PayloadTooLarge { limit, actual } => write!(
                formatter,
                "native frame of {actual} bytes exceeds the {limit} byte limit"
            ),
            Self::DeadlineExceeded { deadline_ms } => write!(
                formatter,
                "native call did not complete within {deadline_ms} ms"
            ),
            Self::GuestReturnedError { message } => {
                write!(formatter, "native provider returned an error: {message}")
            }
            Self::SpawnFailed { reason } => {
                write!(formatter, "native child spawn failed: {reason}")
            }
            Self::ShutdownTimeout { deadline_ms } => write!(
                formatter,
                "graceful native shutdown exceeded {deadline_ms} ms and the child was killed"
            ),
        }
    }
}

impl std::error::Error for NativeHostError {}
