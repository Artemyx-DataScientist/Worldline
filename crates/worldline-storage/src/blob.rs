use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
};

use sha2::{Digest, Sha256};
use worldline_kernel::{BlobId, BlobStore, PersistenceError};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);
const BLOB_DIRECTORY: &str = "blobs";

/// Filesystem-backed immutable content-addressed blob storage.
///
/// The default retention policy is conservative: blobs are never
/// automatically deleted because cross-domain ownership/reference rules are
/// not yet part of the kernel contract.
pub struct FilesystemBlobStore {
    profile_root: PathBuf,
    blob_root: PathBuf,
}

impl FilesystemBlobStore {
    pub fn open(profile_root: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let requested_root = profile_root.as_ref();
        fs::create_dir_all(requested_root).map_err(|error| PersistenceError::BlobWriteFailed {
            message: format!(
                "cannot create profile root '{}': {error}",
                requested_root.display()
            ),
        })?;
        let profile_root = fs::canonicalize(requested_root).map_err(|error| {
            PersistenceError::BlobWriteFailed {
                message: format!(
                    "cannot canonicalize profile root '{}': {error}",
                    requested_root.display()
                ),
            }
        })?;
        let blob_root = profile_root.join(BLOB_DIRECTORY);
        fs::create_dir_all(&blob_root).map_err(|error| PersistenceError::BlobWriteFailed {
            message: format!("cannot create blob root '{}': {error}", blob_root.display()),
        })?;
        let blob_root =
            fs::canonicalize(&blob_root).map_err(|error| PersistenceError::BlobWriteFailed {
                message: format!(
                    "cannot canonicalize blob root '{}': {error}",
                    blob_root.display()
                ),
            })?;
        Ok(Self {
            profile_root,
            blob_root,
        })
    }

    pub fn profile_root(&self) -> &Path {
        &self.profile_root
    }

    pub fn blob_root(&self) -> &Path {
        &self.blob_root
    }

    /// Reads a blob only after the generic host broker admits the exact
    /// principal/capability/blob tuple.
    pub fn get_with_authority(
        &self,
        id: &BlobId,
        grant: &BlobReadGrant,
    ) -> Result<Vec<u8>, BlobReadError> {
        if grant.blob_id() != id.as_str()
            || grant.capability() != BLOB_READ_CAPABILITY
            || !grant.is_active()
        {
            return Err(BlobReadError::AccessDenied {
                principal_id: grant.principal_id().to_owned(),
                blob_id: id.as_str().to_owned(),
            });
        }
        self.get(id).map_err(BlobReadError::Storage)
    }

    fn path_for(&self, id: &BlobId) -> Result<PathBuf, PersistenceError> {
        let path = self.blob_root.join(id.as_str());
        if !path.starts_with(&self.blob_root) {
            return Err(PersistenceError::BlobCorrupt { id: id.clone() });
        }
        if path.exists() {
            let canonical = fs::canonicalize(&path)
                .map_err(|_| PersistenceError::BlobCorrupt { id: id.clone() })?;
            if !canonical.starts_with(&self.blob_root) || !canonical.is_file() {
                return Err(PersistenceError::BlobCorrupt { id: id.clone() });
            }
            Ok(canonical)
        } else {
            Ok(path)
        }
    }

    fn digest_id(bytes: &[u8]) -> Result<BlobId, PersistenceError> {
        let digest = Sha256::digest(bytes);
        let mut encoded = String::with_capacity("sha256-v1-".len() + digest.len() * 2);
        encoded.push_str("sha256-v1-");
        for byte in digest {
            use std::fmt::Write as _;
            write!(encoded, "{byte:02x}").map_err(|_| PersistenceError::BlobWriteFailed {
                message: "cannot format blob digest".to_owned(),
            })?;
        }
        BlobId::new(encoded)
    }

    fn read_verified(&self, id: &BlobId) -> Result<Vec<u8>, PersistenceError> {
        let path = self.path_for(id)?;
        let bytes = fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                PersistenceError::BlobNotFound { id: id.clone() }
            } else {
                PersistenceError::BlobWriteFailed {
                    message: format!("read '{}': {error}", path.display()),
                }
            }
        })?;
        let actual = Self::digest_id(&bytes)?;
        if actual != *id {
            return Err(PersistenceError::BlobCorrupt { id: id.clone() });
        }
        Ok(bytes)
    }
}

impl BlobStore for FilesystemBlobStore {
    fn put(&self, bytes: &[u8]) -> Result<BlobId, PersistenceError> {
        let id = Self::digest_id(bytes)?;
        let final_path = self.blob_root.join(id.as_str());
        if final_path.exists() {
            self.read_verified(&id)?;
            return Ok(id);
        }

        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary_path = self.blob_root.join(format!(
            ".{}.{}.{}.tmp",
            id.as_str(),
            std::process::id(),
            sequence
        ));
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)
                .map_err(|error| PersistenceError::BlobWriteFailed {
                    message: format!(
                        "create temporary blob '{}': {error}",
                        temporary_path.display()
                    ),
                })?;
            file.write_all(bytes)
                .map_err(|error| PersistenceError::BlobWriteFailed {
                    message: format!(
                        "write temporary blob '{}': {error}",
                        temporary_path.display()
                    ),
                })?;
            file.sync_all()
                .map_err(|error| PersistenceError::BlobWriteFailed {
                    message: format!(
                        "sync temporary blob '{}': {error}",
                        temporary_path.display()
                    ),
                })?;
            drop(file);
            crate::failpoints::hit("during-blob-temporary-write");
            if final_path.exists() {
                self.read_verified(&id)?;
                return Ok(());
            }
            fs::rename(&temporary_path, &final_path).map_err(|error| {
                PersistenceError::BlobWriteFailed {
                    message: format!(
                        "publish blob '{}' as '{}': {error}",
                        temporary_path.display(),
                        final_path.display()
                    ),
                }
            })?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result.map(|()| id)
    }

    fn get(&self, id: &BlobId) -> Result<Vec<u8>, PersistenceError> {
        self.read_verified(id)
    }

    fn exists(&self, id: &BlobId) -> Result<bool, PersistenceError> {
        let path = self.blob_root.join(id.as_str());
        if !path.exists() {
            return Ok(false);
        }
        self.path_for(id).map(|_| true)
    }

    fn verify(&self, id: &BlobId) -> Result<(), PersistenceError> {
        self.read_verified(id).map(|_| ())
    }
}

/// Generic host capability required before blob bytes may be read.
///
/// This authority is intentionally owned by the generic storage boundary,
/// not by a browser service. Browser services may reference a blob, but they
/// cannot reinterpret their own metadata capability as a byte-read grant.
pub const BLOB_READ_CAPABILITY: &str = "blob.read";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlobReadError {
    AccessDenied {
        principal_id: String,
        blob_id: String,
    },
    Storage(PersistenceError),
}

impl std::fmt::Display for BlobReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessDenied {
                principal_id,
                blob_id,
            } => write!(
                formatter,
                "principal '{principal_id}' lacks blob-read authority for '{blob_id}'"
            ),
            Self::Storage(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BlobReadError {}

/// Host-side admission broker for artifact/blob reads.
///
/// Grants are exact `(principal, capability, blob)` tuples. The broker never
/// accepts a browser-specific capability and the storage adapter checks the
/// active tuple again at the byte-read boundary.
#[derive(Clone, Debug, Default)]
pub struct BlobReadBroker {
    grants: Arc<Mutex<BTreeSet<(String, String, String)>>>,
}

impl BlobReadBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(
        &self,
        principal_id: impl Into<String>,
        capability: &str,
        blob_id: impl Into<String>,
    ) -> Result<BlobReadGrant, String> {
        if capability != BLOB_READ_CAPABILITY {
            return Err(format!(
                "capability '{capability}' cannot authorize generic blob reads"
            ));
        }
        let principal_id = principal_id.into();
        let blob_id = BlobId::new(blob_id.into())
            .map_err(|error| format!("invalid blob scope for read grant: {error}"))?;
        if principal_id.is_empty() {
            return Err("blob read grant requires a principal".to_string());
        }
        let blob_id = blob_id.as_str().to_owned();
        let key = (principal_id.clone(), capability.to_owned(), blob_id.clone());
        self.grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key);
        Ok(BlobReadGrant {
            principal_id,
            capability: capability.to_owned(),
            blob_id,
            broker: Arc::clone(&self.grants),
        })
    }
}

/// Non-forgeable-by-blob-id, exact-scope generic read grant.
#[derive(Clone, Debug)]
pub struct BlobReadGrant {
    principal_id: String,
    capability: String,
    blob_id: String,
    broker: Arc<Mutex<BTreeSet<(String, String, String)>>>,
}

impl BlobReadGrant {
    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }

    pub fn blob_id(&self) -> &str {
        &self.blob_id
    }

    pub fn is_active(&self) -> bool {
        self.broker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&(
                self.principal_id.clone(),
                self.capability.clone(),
                self.blob_id.clone(),
            ))
    }
}
