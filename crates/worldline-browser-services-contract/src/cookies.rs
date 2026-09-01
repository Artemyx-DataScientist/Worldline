//! Experimental browser.cookies v0.1 contract definitions.

use std::fmt;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::BrowserContextId;

pub const CONTRACT_BROWSER_COOKIES: &str = "browser.cookies";
pub const CONTRACT_BROWSER_COOKIES_VERSION: &str = "0.1";

pub const OP_GET_COOKIE_METADATA: &str = "browser.cookies.get_metadata";
pub const OP_GET_COOKIE_VALUE: &str = "browser.cookies.get_value";
pub const OP_SET_COOKIE: &str = "browser.cookies.set";
pub const OP_DELETE_COOKIE: &str = "browser.cookies.delete";

pub const AUTH_COOKIES_METADATA_READ: &str = "browser.cookies.metadata_read";
pub const AUTH_COOKIES_VALUE_READ: &str = "browser.cookies.value_read";
pub const AUTH_COOKIES_MUTATE: &str = "browser.cookies.mutate";
pub const AUTH_COOKIES_ADMIN: &str = "browser.cookies.admin";

/// Cookie metadata structure containing NO secret values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CookieMetadata {
    pub name: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires_epoch_sec: Option<u64>,
}

/// Secret cookie value container with redacted diagnostic representation.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CookieValue {
    pub name: String,
    pub domain: String,
    pub path: String,
    pub value: String,
}

impl CookieValue {
    pub fn new(
        name: impl Into<String>,
        domain: impl Into<String>,
        path: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            domain: domain.into(),
            path: path.into(),
            value: value.into(),
        }
    }

    /// Explicit accessor for authorized consumers.
    pub fn expose_value(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for CookieValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CookieValue")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCookieMetadataRequest {
    pub context_id: BrowserContextId,
    pub url: Option<String>,
    pub domain: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCookieMetadataResponse {
    pub cookies: Vec<CookieMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCookieValueRequest {
    pub context_id: BrowserContextId,
    pub domain: String,
    pub name: String,
    pub path: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCookieValueResponse {
    pub cookie: Option<CookieValue>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetCookieServiceRequest {
    pub context_id: BrowserContextId,
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: Option<String>,
    pub secure: Option<bool>,
    pub http_only: Option<bool>,
    pub same_site: Option<String>,
    pub expires_epoch_sec: Option<u64>,
}

impl SetCookieServiceRequest {
    pub fn expose_value(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for SetCookieServiceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetCookieServiceRequest")
            .field("context_id", &self.context_id)
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .field("expires_epoch_sec", &self.expires_epoch_sec)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetCookieServiceResponse {
    pub success: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteCookieServiceRequest {
    pub context_id: BrowserContextId,
    pub domain: String,
    pub name: String,
    pub path: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteCookieServiceResponse {
    pub deleted_count: u32,
}
