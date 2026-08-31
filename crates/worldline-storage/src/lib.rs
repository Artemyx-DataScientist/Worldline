#![forbid(unsafe_code)]

mod sqlite;

mod blob;
mod failpoints;

pub use blob::FilesystemBlobStore;
pub use sqlite::{SqliteEventJournal, SqliteStateBackend};

pub fn trigger_test_failpoint(name: &str) {
    failpoints::hit(name);
}
