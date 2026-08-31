use serde::{Deserialize, Serialize};

use crate::{
    error::BrowserError,
    identity::{DocumentRevision, ElementRef, PageId},
};

/// Specific interaction kind requested on a page element.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InteractionKind {
    Click,
    Input,
    Focus,
    Submit,
    Scroll,
}

/// Request to click an element.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClickActionRequest {
    pub element_ref: ElementRef,
}

/// Request to input text into an editable element.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InputActionRequest {
    pub element_ref: ElementRef,
    pub text: String,
    pub clear_first: bool,
}

/// Request to focus an element.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FocusActionRequest {
    pub element_ref: ElementRef,
}

/// Request to submit a form element.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SubmitActionRequest {
    pub element_ref: ElementRef,
}

/// Request to scroll a page viewport.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScrollActionRequest {
    pub page_id: PageId,
    pub delta_x: i32,
    pub delta_y: i32,
}

/// Result of executing an authorized action on a page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub interaction: InteractionKind,
    pub success: bool,
    pub message: Option<String>,
}

/// Helper function to validate an element reference before action dispatch.
pub fn validate_element_reference(
    element_ref: &ElementRef,
    page_id: &PageId,
    current_revision: DocumentRevision,
) -> Result<(), BrowserError> {
    if element_ref.page_id() != page_id {
        return Err(BrowserError::ResourceMismatch {
            expected: page_id.to_string(),
            actual: element_ref.page_id().to_string(),
        });
    }
    if element_ref.document_revision() != current_revision {
        return Err(BrowserError::StaleElementReference {
            expected_revision: element_ref.document_revision(),
            actual_revision: current_revision,
        });
    }
    Ok(())
}
