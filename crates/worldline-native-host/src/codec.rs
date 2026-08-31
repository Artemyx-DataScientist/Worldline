//! Framed transport primitives: a frame is a 4-byte big-endian byte length
//! followed by that many bytes. Frame length is checked against the
//! configured bound **before** any payload allocation.

use std::io::{Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

use worldline_plugin_protocol::{Envelope, ProtocolError};

use crate::error::NativeHostError;

/// Writes one envelope as a framed record.
pub fn write_frame<W: Write>(writer: &mut W, envelope: &Envelope) -> Result<(), NativeHostError> {
    let bytes = envelope
        .encode()
        .map_err(|error| NativeHostError::ProtocolViolation {
            reason: format!("outgoing envelope failed to encode: {error}"),
        })?;
    write_json_bytes(writer, &bytes)
}

/// Reads one envelope as a framed record. The length gate runs before any
/// payload allocation.
pub fn read_frame<R: Read>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<Envelope, NativeHostError> {
    let bytes = read_json_bytes(reader, max_frame_bytes)?;
    Envelope::decode(&bytes, max_frame_bytes).map_err(|error| {
        // The decoder re-checks the size gate; map its typed errors.
        match error {
            ProtocolError::UnsupportedProtocolVersion { found } => {
                NativeHostError::UnsupportedProtocolVersion { found }
            }
            other => NativeHostError::ProtocolViolation {
                reason: other.to_string(),
            },
        }
    })
}

/// Writes one length-prefixed JSON record (used by the handshake).
pub fn write_json_frame<W: Write, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), NativeHostError> {
    let bytes = serde_json::to_vec(value).map_err(|error| NativeHostError::ProtocolViolation {
        reason: format!("handshake message failed to encode: {error}"),
    })?;
    write_json_bytes(writer, &bytes)
}

/// Reads one length-prefixed JSON record (used by the handshake). The
/// length gate runs before allocation.
pub fn read_json_frame<R: Read, T: DeserializeOwned>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<T, NativeHostError> {
    let bytes = read_json_bytes(reader, max_frame_bytes)?;
    serde_json::from_slice(&bytes).map_err(|error| NativeHostError::ProtocolViolation {
        reason: format!("handshake message is malformed: {error}"),
    })
}

fn write_json_bytes<W: Write>(writer: &mut W, bytes: &[u8]) -> Result<(), NativeHostError> {
    let length = u32::try_from(bytes.len()).map_err(|_| NativeHostError::PayloadTooLarge {
        limit: u32::MAX as usize,
        actual: bytes.len(),
    })?;
    writer
        .write_all(&length.to_be_bytes())
        .and_then(|()| writer.write_all(bytes))
        .and_then(|()| writer.flush())
        .map_err(|_| NativeHostError::TransportClosed)
}

fn read_json_bytes<R: Read>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<Vec<u8>, NativeHostError> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(NativeHostError::TransportClosed);
        }
        Err(_) => return Err(NativeHostError::TransportClosed),
    }
    let length = u32::from_be_bytes(header) as usize;
    if length > max_frame_bytes {
        // Gate before allocation: the oversized body is never buffered.
        return Err(NativeHostError::PayloadTooLarge {
            limit: max_frame_bytes,
            actual: length,
        });
    }
    let mut body = vec![0u8; length];
    match reader.read_exact(&mut body) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(NativeHostError::TransportClosed);
        }
        Err(_) => return Err(NativeHostError::TransportClosed),
    }
    Ok(body)
}
