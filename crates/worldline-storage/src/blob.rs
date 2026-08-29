use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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
