use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::identity::{DocumentRevision, ElementRef, PageId};

/// Standard semantic roles for accessibility query nodes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum AccessibilityRole {
    Root,
    Heading,
    Button,
    Link,
    TextInput,
    StaticText,
    Checkbox,
    Radio,
    Form,
    Group,
    Dialog,
    List,
    ListItem,
    Image,
    Generic,
}

/// A node in the bounded accessibility semantic tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityNode {
    pub node_id: String,
    pub role: AccessibilityRole,
    pub name: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub element_ref: Option<ElementRef>,
    pub children: Vec<AccessibilityNode>,
}

impl AccessibilityNode {
    pub fn new(node_id: impl Into<String>, role: AccessibilityRole) -> Self {
        Self {
            node_id: node_id.into(),
            role,
            name: None,
            value: None,
            description: None,
            element_ref: None,
            children: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_element_ref(mut self, element_ref: ElementRef) -> Self {
        self.element_ref = Some(element_ref);
        self
    }

    pub fn with_child(mut self, child: AccessibilityNode) -> Self {
        self.children.push(child);
        self
    }

    pub fn collect_text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(name) = &self.name {
            parts.push(name.clone());
        }
        if let Some(val) = &self.value {
            parts.push(val.clone());
        }
        for child in &self.children {
            let child_text = child.collect_text();
            if !child_text.is_empty() {
                parts.push(child_text);
            }
        }
        parts.join(" ")
    }
}

/// Structured accessibility tree snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityTree {
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub root: AccessibilityNode,
}

/// Page document metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub page_id: PageId,
    pub url: String,
    pub title: String,
    pub document_revision: DocumentRevision,
    pub status_code: u16,
}

/// Bounded document snapshot returned through query capability.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentSnapshot {
    pub metadata: DocumentMetadata,
    pub accessibility_tree: AccessibilityTree,
}

/// Structured semantic element representation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SemanticElement {
    pub element_ref: ElementRef,
    pub tag_name: String,
    pub attributes: BTreeMap<String, String>,
    pub text_content: String,
}
