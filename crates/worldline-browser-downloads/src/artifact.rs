//! Generic durable artifact/blob access for completed download content.
//!
//! Download metadata and blob-read authority are deliberately separate. The
//! artifact reference is safe metadata; bytes are available only through a
//! broker-issued, artifact-scoped `BlobReadGrant`.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use worldline_browser_services_contract::ArtifactRef;
use worldline_kernel::BlobStore;
use worldline_storage::{BLOB_READ_CAPABILITY, BlobReadError, BlobReadGrant, FilesystemBlobStore};

/// Capability name required to issue an artifact-scoped blob read grant.
pub const AUTH_BLOB_READ: &str = BLOB_READ_CAPABILITY;

#[derive(Clone)]
pub struct ArtifactStore {
    blobs: Arc<FilesystemBlobStore>,
}

impl fmt::Debug for ArtifactStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactStore")
            .field("profile_root", &self.blobs.profile_root())
            .finish()
    }
}

impl ArtifactStore {
    /// Opens the generic durable host blob store at an explicit profile root.
    pub fn open(profile_root: impl AsRef<Path>) -> Result<Self, String> {
        let blobs = FilesystemBlobStore::open(profile_root).map_err(|error| error.to_string())?;
        Ok(Self {
            blobs: Arc::new(blobs),
        })
    }

    /// Opens the default local development profile. Hosted integrations must
    /// use [`Self::open`] with the host-selected persistence root.
    pub fn new() -> Self {
        let root = std::env::var_os("WORLDLINE_DOWNLOAD_BLOB_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join("target")
                    .join("worldline-download-blobs")
            });
        Self::open(root).expect("default download blob store must be openable")
    }

    /// Stores bytes through the generic content-addressed host blob store.
    pub fn store_bytes(
        &self,
        bytes: &[u8],
        mime_type: Option<String>,
    ) -> Result<ArtifactRef, String> {
        let blob_id = self
            .blobs
            .put(bytes)
            .map_err(|error| format!("store download artifact: {error}"))?;
        let sha256_hash = blob_id
            .as_str()
            .strip_prefix("sha256-v1-")
            .map(str::to_owned)
            .ok_or_else(|| "generic blob store returned a non-SHA-256 identity".to_string())?;
        Ok(ArtifactRef::new(
            blob_id.as_str(),
            bytes.len() as u64,
            mime_type,
            Some(sha256_hash),
        ))
    }

    /// Handles a generic host blob `Put` request and verifies that the
    /// provider-supplied content identity is the SHA-256 identity of bytes.
    pub fn put_blob(&self, blob_id: &str, bytes: &[u8]) -> Result<(), String> {
        let artifact = self.store_bytes(bytes, None)?;
        if artifact.artifact_id != blob_id {
            return Err(format!(
                "blob identity mismatch: provider supplied '{blob_id}', host computed '{}'",
                artifact.artifact_id
            ));
        }
        Ok(())
    }

    /// Reads bytes only when the grant was issued for this exact artifact by
    /// the independent blob-read broker.
    pub fn read_bytes_with_authority(
        &self,
        artifact_id: &str,
        grant: &BlobReadGrant,
    ) -> Result<Vec<u8>, ArtifactReadError> {
        let blob_id = worldline_kernel::BlobId::new(artifact_id).map_err(|error| {
            ArtifactReadError::NotFound {
                artifact_id: artifact_id.to_string(),
                reason: error.to_string(),
            }
        })?;
        self.blobs
            .get_with_authority(&blob_id, grant)
            .map_err(|error| match error {
                BlobReadError::AccessDenied { principal_id, .. } => {
                    ArtifactReadError::AccessDenied {
                        principal_id,
                        artifact_id: artifact_id.to_string(),
                    }
                }
                BlobReadError::Storage(error) => ArtifactReadError::NotFound {
                    artifact_id: artifact_id.to_string(),
                    reason: error.to_string(),
                },
            })
    }

    /// Checks existence without disclosing bytes. Metadata inspection does
    /// not require the blob-read grant.
    pub fn contains(&self, artifact_id: &str) -> bool {
        let Ok(blob_id) = worldline_kernel::BlobId::new(artifact_id) else {
            return false;
        };
        self.blobs.exists(&blob_id).unwrap_or(false)
    }

    pub fn profile_root(&self) -> &Path {
        self.blobs.profile_root()
    }
}

impl Default for ArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactReadError {
    AccessDenied {
        principal_id: String,
        artifact_id: String,
    },
    NotFound {
        artifact_id: String,
        reason: String,
    },
}

impl fmt::Display for ArtifactReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessDenied {
                principal_id,
                artifact_id,
            } => write!(
                formatter,
                "principal '{principal_id}' lacks blob-read authority for artifact '{artifact_id}'"
            ),
            Self::NotFound {
                artifact_id,
                reason,
            } => write!(
                formatter,
                "artifact '{artifact_id}' is unavailable: {reason}"
            ),
        }
    }
}

impl std::error::Error for ArtifactReadError {}
