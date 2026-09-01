//! Browser durable history service plugin for Worldline.
//!
//! Owns transactional user history records, idempotent deduplication of committed
//! navigation facts, title enrichment, and query/deletion operations without
//! granting NavigatePage authority or modifying browser engine state.

pub mod service;
pub mod store;

pub use service::HistoryService;
pub use store::{ConsistencyError, HistoryStoreSnapshot};
