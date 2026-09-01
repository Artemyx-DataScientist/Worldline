//! Experimental 0.1 browser capture capability contract.
//!
//! Provides screen and page capture returning provider-scoped artifact references
//! backed by the host's content-addressed `BlobStore`.

use serde::{Deserialize, Serialize};

use crate::identity::{DocumentRevision, ElementRef, PageId};

pub const CONTRACT_CAPTURE: &str = "capture";
pub const CAPTURE_MAJOR_V0_1: u16 = 0;
pub const CAPTURE_MINOR_V0_1: u16 = 1;

/// Format for page or viewport capture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
}

/// Target surface for capture.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureTarget {
    #[default]
    PageViewport,
    FullPage,
    Element {
        element_ref: ElementRef,
    },
}

/// Request to capture an image of the page viewport or element.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapturePageRequest {
    pub page_id: PageId,
    pub target: CaptureTarget,
    pub format: CaptureFormat,
    pub quality: Option<u8>,
    pub max_bytes: Option<usize>,
}

/// Provider-scoped artifact reference for captured image bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaptureArtifactRef {
    pub artifact_id: String,
    pub page_id: PageId,
    pub revision: DocumentRevision,
    pub byte_len: usize,
    pub mime_type: String,
    pub blob_id: String,
}

/// Response containing the provider-scoped artifact reference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapturePageResponse {
    pub artifact: CaptureArtifactRef,
}

/// Request to read bounded artifact chunks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadCaptureArtifactRequest {
    pub artifact_id: String,
    pub offset: u64,
    pub max_bytes: usize,
}

/// Bounded response containing artifact bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadCaptureArtifactResponse {
    pub artifact_id: String,
    pub data: Vec<u8>,
    pub is_truncated: bool,
    pub total_bytes: usize,
}
