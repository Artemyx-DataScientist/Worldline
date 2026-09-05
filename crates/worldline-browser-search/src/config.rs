//! Structured configuration for replaceable search provider plugins.

use std::collections::BTreeSet;
use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use url::Url;

/// Errors arising from invalid search provider configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchConfigError {
    InvalidUrl(String),
    InsecureScheme { scheme: String },
    ForbiddenScheme { scheme: String },
    UserInfoNotAllowed,
    MissingHost,
    EmptyQueryParameterName,
    QueryParameterNameTooLong { length: usize, max: usize },
    TooManyStaticParameters { count: usize, max: usize },
    DuplicateStaticParameterKey(String),
    StaticParameterConflictsWithQueryName(String),
    StaticParameterKeyTooLong { length: usize, max: usize },
    StaticParameterValueTooLong { length: usize, max: usize },
}

impl fmt::Display for SearchConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(err) => write!(f, "invalid base URL: {err}"),
            Self::InsecureScheme { scheme } => {
                write!(
                    f,
                    "insecure scheme '{scheme}': production search providers must use HTTPS; HTTP is allowed only for loopback test fixtures"
                )
            }
            Self::ForbiddenScheme { scheme } => {
                write!(
                    f,
                    "forbidden scheme '{scheme}': only HTTPS (or loopback HTTP) is permitted"
                )
            }
            Self::UserInfoNotAllowed => {
                write!(f, "URL userinfo/credentials are strictly forbidden")
            }
            Self::MissingHost => {
                write!(f, "search base URL must contain a valid non-empty hostname")
            }
            Self::EmptyQueryParameterName => write!(f, "query parameter name cannot be empty"),
            Self::QueryParameterNameTooLong { length, max } => {
                write!(
                    f,
                    "query parameter name length {length} exceeds maximum {max}"
                )
            }
            Self::TooManyStaticParameters { count, max } => {
                write!(f, "static parameter count {count} exceeds maximum {max}")
            }
            Self::DuplicateStaticParameterKey(key) => {
                write!(f, "duplicate static parameter key: '{key}'")
            }
            Self::StaticParameterConflictsWithQueryName(key) => {
                write!(
                    f,
                    "static parameter '{key}' conflicts with the designated query parameter name"
                )
            }
            Self::StaticParameterKeyTooLong { length, max } => {
                write!(
                    f,
                    "static parameter key length {length} exceeds maximum {max}"
                )
            }
            Self::StaticParameterValueTooLong { length, max } => {
                write!(
                    f,
                    "static parameter value length {length} exceeds maximum {max}"
                )
            }
        }
    }
}

impl std::error::Error for SearchConfigError {}

pub const MAX_QUERY_PARAM_NAME_LENGTH: usize = 64;
pub const MAX_STATIC_PARAMETERS: usize = 32;
pub const MAX_STATIC_PARAM_KEY_LENGTH: usize = 128;
pub const MAX_STATIC_PARAM_VALUE_LENGTH: usize = 512;

/// Installation-owned structured search provider configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchProviderConfig {
    pub name: String,
    pub base_url: String,
    pub query_parameter_name: String,
    pub static_parameters: Vec<(String, String)>,
    pub allow_loopback_http: bool,
}

impl SearchProviderConfig {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        query_parameter_name: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            query_parameter_name: query_parameter_name.into(),
            static_parameters: Vec::new(),
            allow_loopback_http: false,
        }
    }

    pub fn with_static_parameter(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.static_parameters.push((key.into(), value.into()));
        self
    }

    pub fn with_loopback_http(mut self, allow: bool) -> Self {
        self.allow_loopback_http = allow;
        self
    }

    /// Validates the configuration against all security, structural, and privacy invariants.
    pub fn validate(&self) -> Result<Url, SearchConfigError> {
        let parsed_url =
            Url::parse(&self.base_url).map_err(|e| SearchConfigError::InvalidUrl(e.to_string()))?;

        // Credentials / userinfo check
        if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
            return Err(SearchConfigError::UserInfoNotAllowed);
        }

        // Host check
        let host_str = match parsed_url.host_str() {
            Some(h) if !h.trim().is_empty() => h,
            _ => return Err(SearchConfigError::MissingHost),
        };

        // Scheme check: production requires HTTPS; HTTP is allowed ONLY for loopback test fixtures
        let scheme = parsed_url.scheme();
        if scheme == "https" {
            // HTTPS is always permitted
        } else if scheme == "http" {
            if !self.allow_loopback_http || !is_loopback_host(host_str) {
                return Err(SearchConfigError::InsecureScheme {
                    scheme: scheme.to_string(),
                });
            }
        } else {
            return Err(SearchConfigError::ForbiddenScheme {
                scheme: scheme.to_string(),
            });
        }

        // Query parameter name check
        let trimmed_name = self.query_parameter_name.trim();
        if trimmed_name.is_empty() {
            return Err(SearchConfigError::EmptyQueryParameterName);
        }
        if self.query_parameter_name.len() > MAX_QUERY_PARAM_NAME_LENGTH {
            return Err(SearchConfigError::QueryParameterNameTooLong {
                length: self.query_parameter_name.len(),
                max: MAX_QUERY_PARAM_NAME_LENGTH,
            });
        }

        // Static parameters check
        if self.static_parameters.len() > MAX_STATIC_PARAMETERS {
            return Err(SearchConfigError::TooManyStaticParameters {
                count: self.static_parameters.len(),
                max: MAX_STATIC_PARAMETERS,
            });
        }

        let mut seen_keys = BTreeSet::new();
        for (key, val) in &self.static_parameters {
            if key.len() > MAX_STATIC_PARAM_KEY_LENGTH {
                return Err(SearchConfigError::StaticParameterKeyTooLong {
                    length: key.len(),
                    max: MAX_STATIC_PARAM_KEY_LENGTH,
                });
            }
            if val.len() > MAX_STATIC_PARAM_VALUE_LENGTH {
                return Err(SearchConfigError::StaticParameterValueTooLong {
                    length: val.len(),
                    max: MAX_STATIC_PARAM_VALUE_LENGTH,
                });
            }
            if key == &self.query_parameter_name {
                return Err(SearchConfigError::StaticParameterConflictsWithQueryName(
                    key.clone(),
                ));
            }
            if !seen_keys.insert(key.clone()) {
                return Err(SearchConfigError::DuplicateStaticParameterKey(key.clone()));
            }
        }

        Ok(parsed_url)
    }
}

/// Helper function to determine if a host string is a loopback address.
pub fn is_loopback_host(host: &str) -> bool {
    let clean = host.trim_start_matches('[').trim_end_matches(']');
    if clean.eq_ignore_ascii_case("localhost") || clean == "127.0.0.1" || clean == "::1" {
        return true;
    }
    if let Ok(ip) = clean.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    false
}
