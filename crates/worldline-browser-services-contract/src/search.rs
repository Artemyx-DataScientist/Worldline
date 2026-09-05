//! Experimental browser.search v0.1 contract definitions.
//!
//! Exposes an engine-neutral, data-only search target resolution capability
//! without page navigation or browser mutation authority.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const CONTRACT_BROWSER_SEARCH: &str = "browser.search";
pub const CONTRACT_BROWSER_SEARCH_VERSION: &str = "0.1";

pub const OP_RESOLVE_SEARCH: &str = "browser.search.resolve";

pub const AUTH_SEARCH_RESOLVE: &str = "browser.search.resolve";

pub const MAX_SEARCH_QUERY_LENGTH: usize = 1024;
pub const MAX_SEARCH_TARGET_URL_LENGTH: usize = 4096;

/// Errors returned by search contract operations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchContractError {
    EmptyQuery,
    QueryTooLong { length: usize, max: usize },
    TargetUrlTooLong { length: usize, max: usize },
    InvalidQuery(String),
    TargetConstructionFailed(String),
    ConfigurationError(String),
}

impl fmt::Display for SearchContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyQuery => write!(f, "search query cannot be empty"),
            Self::QueryTooLong { length, max } => {
                write!(
                    f,
                    "search query length {length} exceeds maximum limit {max}"
                )
            }
            Self::TargetUrlTooLong { length, max } => {
                write!(
                    f,
                    "search target URL length {length} exceeds maximum limit {max}"
                )
            }
            Self::InvalidQuery(reason) => write!(f, "invalid search query: {reason}"),
            Self::TargetConstructionFailed(reason) => {
                write!(f, "failed to construct search target URL: {reason}")
            }
            Self::ConfigurationError(reason) => {
                write!(f, "search provider configuration error: {reason}")
            }
        }
    }
}

impl Error for SearchContractError {}

/// Request to resolve a classified search query into a navigation target.
///
/// Note: [`fmt::Debug`] and [`fmt::Display`] are explicitly implemented to redact
/// raw user query text to preserve user privacy and prevent sensitive queries
/// from leaking into generic diagnostics, metrics, panic messages, or trace logs.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResolveRequest {
    pub query: String,
}

impl SearchResolveRequest {
    pub fn new(query: impl Into<String>) -> Result<Self, SearchContractError> {
        let query = query.into();
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(SearchContractError::EmptyQuery);
        }
        if query.len() > MAX_SEARCH_QUERY_LENGTH {
            return Err(SearchContractError::QueryTooLong {
                length: query.len(),
                max: MAX_SEARCH_QUERY_LENGTH,
            });
        }
        Ok(Self { query })
    }

    /// Access the raw query string for authorized provider resolution.
    pub fn query(&self) -> &str {
        &self.query
    }
}

impl fmt::Debug for SearchResolveRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchResolveRequest")
            .field("query_len", &self.query.len())
            .field("query", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Display for SearchResolveRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SearchResolveRequest([REDACTED: {} bytes])",
            self.query.len()
        )
    }
}

/// The bounded navigation target result produced by search resolution.
///
/// This is pure data suitable for a subsequent, separately authorized
/// `browser.navigate` capability call. It does not carry or grant any
/// page mutation or navigation authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchNavigationTarget {
    pub url: String,
    pub query_parameter_name: String,
}

impl SearchNavigationTarget {
    pub fn new(
        url: impl Into<String>,
        query_parameter_name: impl Into<String>,
    ) -> Result<Self, SearchContractError> {
        let url = url.into();
        if url.len() > MAX_SEARCH_TARGET_URL_LENGTH {
            return Err(SearchContractError::TargetUrlTooLong {
                length: url.len(),
                max: MAX_SEARCH_TARGET_URL_LENGTH,
            });
        }
        Ok(Self {
            url,
            query_parameter_name: query_parameter_name.into(),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn query_parameter_name(&self) -> &str {
        &self.query_parameter_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_request_construction() {
        let req = SearchResolveRequest::new("worldline operating system").expect("valid request");
        assert_eq!(req.query(), "worldline operating system");
    }

    #[test]
    fn empty_and_whitespace_query_rejected() {
        assert_eq!(
            SearchResolveRequest::new(""),
            Err(SearchContractError::EmptyQuery)
        );
        assert_eq!(
            SearchResolveRequest::new("   \t\n  "),
            Err(SearchContractError::EmptyQuery)
        );
    }

    #[test]
    fn oversized_query_rejected() {
        let long_query = "a".repeat(MAX_SEARCH_QUERY_LENGTH + 1);
        match SearchResolveRequest::new(long_query) {
            Err(SearchContractError::QueryTooLong { length, max }) => {
                assert_eq!(length, MAX_SEARCH_QUERY_LENGTH + 1);
                assert_eq!(max, MAX_SEARCH_QUERY_LENGTH);
            }
            other => panic!("expected QueryTooLong, got {other:?}"),
        }
    }

    #[test]
    fn privacy_redacted_debug_and_display() {
        let secret = "sensitive medical diagnosis query";
        let req = SearchResolveRequest::new(secret).expect("valid request");
        let debug_str = format!("{req:?}");
        let display_str = format!("{req}");

        assert!(!debug_str.contains(secret), "Debug leaked secret query!");
        assert!(
            !display_str.contains(secret),
            "Display leaked secret query!"
        );
        assert!(debug_str.contains("[REDACTED]"));
        assert!(display_str.contains("[REDACTED"));
    }

    #[test]
    fn valid_navigation_target_construction() {
        let target = SearchNavigationTarget::new("https://duckduckgo.com/html/?q=worldline", "q")
            .expect("valid target");
        assert_eq!(target.url(), "https://duckduckgo.com/html/?q=worldline");
        assert_eq!(target.query_parameter_name(), "q");
    }

    #[test]
    fn oversized_target_url_rejected() {
        let long_url = format!(
            "https://example.com/?q={}",
            "x".repeat(MAX_SEARCH_TARGET_URL_LENGTH)
        );
        match SearchNavigationTarget::new(long_url, "q") {
            Err(SearchContractError::TargetUrlTooLong { length, max }) => {
                assert!(length > max);
                assert_eq!(max, MAX_SEARCH_TARGET_URL_LENGTH);
            }
            other => panic!("expected TargetUrlTooLong, got {other:?}"),
        }
    }

    #[test]
    fn serde_roundtrip() {
        let req = SearchResolveRequest::new("hello search").expect("valid");
        let json = serde_json::to_string(&req).expect("serialize");
        let deserialized: SearchResolveRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(req, deserialized);

        let target =
            SearchNavigationTarget::new("https://example.com/?q=test", "q").expect("valid");
        let json_target = serde_json::to_string(&target).expect("serialize target");
        let deserialized_target: SearchNavigationTarget =
            serde_json::from_str(&json_target).expect("deserialize target");
        assert_eq!(target, deserialized_target);
    }
}
