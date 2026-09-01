//! Browser engine provider core implementation and reference backend.

pub mod backend;
pub mod core;
pub mod reference;

pub use backend::BrowserBackend;
pub use core::{BrowserProviderCore, ProviderBudgetLimits};
pub use reference::ReferenceBrowserBackend;
