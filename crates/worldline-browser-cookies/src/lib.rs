//! Browser cookies and site-data service plugin for Worldline.
//!
//! Exposes cookie metadata inspection separately from secret-value access and mutation,
//! with redacted diagnostic formatting, cross-context isolation, and bounded site-data
//! clearing over engine storage primitives.

pub mod policy;
pub mod service;
pub mod site_data;

pub use policy::CookiePolicySnapshot;
pub use service::{CookieEngineBackend, CookiesService, InMemoryCookieEngine};
pub use site_data::validate_and_build_clear_storage;
