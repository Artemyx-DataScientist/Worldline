//! Experimental browser.site-data v0.1 contract definitions.

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::BrowserContextId;
pub use worldline_browser_contract::primitives::StorageType;

pub const CONTRACT_BROWSER_SITE_DATA: &str = "browser.site-data";
pub const CONTRACT_BROWSER_SITE_DATA_VERSION: &str = "0.1";

pub const OP_CLEAR_SITE_DATA: &str = "browser.site_data.clear";
pub const AUTH_SITE_DATA_CLEAR: &str = "browser.site_data.clear";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearSiteDataRequest {
    pub context_id: BrowserContextId,
    pub origin: String,
    pub storage_type: StorageType,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearSiteDataResponse {
    pub cleared: bool,
}
