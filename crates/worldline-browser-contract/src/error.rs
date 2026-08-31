use serde::{Deserialize, Serialize};
use std::fmt;

use crate::identity::{BrowserContextId, DocumentRevision, DownloadId, PageId};

/// Engine-neutral error taxonomy for all browser operations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BrowserError {
    ContextNotFound(BrowserContextId),
    PageNotFound(PageId),
    DownloadNotFound(DownloadId),
    NavigationFailed(String),
    StaleElementReference {
        expected_revision: DocumentRevision,
        actual_revision: DocumentRevision,
    },
    ElementNotFound(String),
    DocumentRevisionMismatch {
        expected_revision: DocumentRevision,
        actual_revision: DocumentRevision,
    },
    PermissionDenied(String),
    EngineCrashed(String),
    EngineHung(String),
    Timeout(String),
    NetworkError(String),
    InvalidRequest(String),
    UnsupportedOperation(String),
}

impl BrowserError {
    pub fn is_stale_element(&self) -> bool {
        matches!(self, Self::StaleElementReference { .. })
    }

    pub fn is_crashed(&self) -> bool {
        matches!(self, Self::EngineCrashed(_))
    }

    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self)
            .unwrap_or_else(|_| format!("{{\"error\":\"{}\"}}", self).into_bytes())
    }

    pub fn from_json_slice(slice: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(slice).map_err(|e| format!("failed to parse BrowserError: {e}"))
    }
}

impl fmt::Display for BrowserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContextNotFound(id) => write!(formatter, "browser context not found: {id}"),
            Self::PageNotFound(id) => write!(formatter, "browser page not found: {id}"),
            Self::DownloadNotFound(id) => write!(formatter, "download not found: {id}"),
            Self::NavigationFailed(msg) => write!(formatter, "navigation failed: {msg}"),
            Self::StaleElementReference {
                expected_revision,
                actual_revision,
            } => {
                write!(
                    formatter,
                    "element reference is stale: expected {expected_revision}, current page is {actual_revision}"
                )
            }
            Self::ElementNotFound(selector) => write!(formatter, "element not found: {selector}"),
            Self::DocumentRevisionMismatch {
                expected_revision,
                actual_revision,
            } => {
                write!(
                    formatter,
                    "document revision mismatch: expected {expected_revision}, actual {actual_revision}"
                )
            }
            Self::PermissionDenied(msg) => write!(formatter, "browser permission denied: {msg}"),
            Self::EngineCrashed(msg) => write!(formatter, "browser engine crashed: {msg}"),
            Self::EngineHung(msg) => write!(formatter, "browser engine hung/unresponsive: {msg}"),
            Self::Timeout(msg) => write!(formatter, "browser operation timed out: {msg}"),
            Self::NetworkError(msg) => write!(formatter, "browser network error: {msg}"),
            Self::InvalidRequest(msg) => write!(formatter, "invalid browser request: {msg}"),
            Self::UnsupportedOperation(msg) => {
                write!(formatter, "unsupported browser operation: {msg}")
            }
        }
    }
}

impl std::error::Error for BrowserError {}
