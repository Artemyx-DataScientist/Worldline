//! Generic artifact / blob storage abstraction for completed download content.
//!
//! Exposes completed downloads as opaque ArtifactRef handles, ensuring
//! host filesystem paths and provider staging directories never leak to consumers.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use worldline_browser_services_contract::ArtifactRef;

/// In-memory content-addressed artifact store.
#[derive(Clone, Debug, Default)]
pub struct ArtifactStore {
    artifacts: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl ArtifactStore {
    pub fn new() -> Self {
        Self {
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Computes deterministic SHA-256-like hex digest for content and stores it.
    pub fn store_bytes(&self, bytes: &[u8], mime_type: Option<String>) -> ArtifactRef {
        let digest = format!("{:x}", simple_hash(bytes));
        let artifact_id = format!("blob-{}", digest);

        let mut store = self.artifacts.lock().unwrap();
        store.insert(artifact_id.clone(), bytes.to_vec());

        ArtifactRef::new(artifact_id, bytes.len() as u64, mime_type, Some(digest))
    }

    /// Retrieves artifact content bytes. Requires independent blob-read authorization.
    pub fn read_bytes(&self, artifact_id: &str) -> Option<Vec<u8>> {
        let store = self.artifacts.lock().unwrap();
        store.get(artifact_id).cloned()
    }

    /// Checks whether an artifact exists.
    pub fn contains(&self, artifact_id: &str) -> bool {
        let store = self.artifacts.lock().unwrap();
        store.contains_key(artifact_id)
    }
}

/// Bounded deterministic 64-bit FNV-1a hash formatted as hex for content addressing.
fn simple_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
