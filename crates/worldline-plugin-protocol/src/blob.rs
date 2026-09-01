//! Bounded generic native blob host request and response payloads.
//!
//! Provides the transport-neutral structures for native plugin providers
//! to store, retrieve, and verify content-addressed blobs in the host's BlobStore.

use serde::{Deserialize, Serialize};

/// Maximum single blob transfer payload chunk size (4 MB).
pub const MAX_BLOB_CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Action requested by a native provider for host blob storage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BlobAction {
    /// Put raw bytes into content-addressed blob store.
    Put { blob_id: String, bytes: Vec<u8> },
    /// Read bounded slice from content-addressed blob store.
    Get {
        blob_id: String,
        offset: u64,
        max_bytes: usize,
    },
    /// Verify existence and integrity of a blob.
    Verify { blob_id: String },
}

/// Host-bound blob request envelope payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlobRequest {
    pub action: BlobAction,
}

/// Provider-bound blob outcome payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BlobResult {
    PutSuccess {
        blob_id: String,
        byte_len: usize,
    },
    GetSuccess {
        blob_id: String,
        data: Vec<u8>,
        is_truncated: bool,
        total_bytes: usize,
    },
    VerifySuccess {
        blob_id: String,
        exists: bool,
        byte_len: Option<usize>,
    },
    Error {
        reason: String,
    },
}
