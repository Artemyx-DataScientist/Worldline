#![forbid(unsafe_code)]

mod sqlite;

mod blob;
mod failpoints;

pub use blob::{
    BLOB_READ_CAPABILITY, BlobReadBroker, BlobReadError, BlobReadGrant, FilesystemBlobStore,
};
pub use sqlite::{SqliteEventJournal, SqliteStateBackend};

pub fn trigger_test_failpoint(name: &str) {
    failpoints::hit(name);
}
