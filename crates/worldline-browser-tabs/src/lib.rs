//! Browser tabs service plugin for Worldline.
//!
//! Provides user-level tab state management, ordering, grouping, pinning,
//! selection, and deterministic reconciliation without conferring PageId authority
//! or issuing implicit page-close commands.

pub mod service;
pub mod state;

pub use service::TabsService;
pub use state::TabsSnapshot;
