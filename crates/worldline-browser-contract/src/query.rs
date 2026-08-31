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

/// Explicit budget bounds for document and accessibility queries to prevent unbounded IPC allocations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryBounds {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_text_len: usize,
    pub max_total_text_bytes: usize,
}

impl Default for QueryBounds {
    fn default() -> Self {
        Self {
            max_depth: 16,
            max_nodes: 256,
            max_text_len: 512,
            max_total_text_bytes: 65_536,
        }
    }
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

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
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

    pub fn count_nodes(&self) -> usize {
        1 + self.children.iter().map(|c| c.count_nodes()).sum::<usize>()
    }

    /// Prunes the node and its descendants according to QueryBounds.
    pub fn prune_bounded(
        &self,
        current_depth: usize,
        bounds: &QueryBounds,
        nodes_budget: &mut usize,
        text_bytes_budget: &mut usize,
        truncated: &mut bool,
    ) -> Option<AccessibilityNode> {
        if *nodes_budget == 0 {
            *truncated = true;
            return None;
        }
        *nodes_budget -= 1;

        let mut truncate_text = |s: &Option<String>, text_budget: &mut usize| -> Option<String> {
            s.as_ref().map(|text| {
                let max_len = bounds.max_text_len.min(*text_budget);
                if text.len() > max_len {
                    *truncated = true;
                    let slice = text.chars().take(max_len).collect::<String>();
                    *text_budget = text_budget.saturating_sub(slice.len());
                    slice
                } else {
                    *text_budget = text_budget.saturating_sub(text.len());
                    text.clone()
                }
            })
        };

        let bounded_name = truncate_text(&self.name, text_bytes_budget);
        let bounded_value = truncate_text(&self.value, text_bytes_budget);
        let bounded_desc = truncate_text(&self.description, text_bytes_budget);

        let mut bounded_children = Vec::new();
        if current_depth < bounds.max_depth {
            for child in &self.children {
                if let Some(pruned_child) = child.prune_bounded(
                    current_depth + 1,
                    bounds,
                    nodes_budget,
                    text_bytes_budget,
                    truncated,
                ) {
                    bounded_children.push(pruned_child);
                }
            }
        } else if !self.children.is_empty() {
            *truncated = true;
        }

        Some(AccessibilityNode {
            node_id: self.node_id.clone(),
            role: self.role,
            name: bounded_name,
            value: bounded_value,
            description: bounded_desc,
            element_ref: self.element_ref.clone(),
            children: bounded_children,
        })
    }
}

/// Structured accessibility tree snapshot with explicit bounding and truncation reporting.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityTree {
    pub page_id: PageId,
    pub document_revision: DocumentRevision,
    pub root: AccessibilityNode,
    pub is_truncated: bool,
    pub total_node_count: usize,
    pub truncated_node_count: usize,
}

impl AccessibilityTree {
    pub fn new(
        page_id: PageId,
        document_revision: DocumentRevision,
        root: AccessibilityNode,
    ) -> Self {
        let total_node_count = root.count_nodes();
        Self {
            page_id,
            document_revision,
            root,
            is_truncated: false,
            total_node_count,
            truncated_node_count: 0,
        }
    }

    pub fn to_bounded(&self, bounds: &QueryBounds) -> Self {
        let mut nodes_budget = bounds.max_nodes;
        let mut text_bytes_budget = bounds.max_total_text_bytes;
        let mut truncated = false;

        let root = self
            .root
            .prune_bounded(
                1,
                bounds,
                &mut nodes_budget,
                &mut text_bytes_budget,
                &mut truncated,
            )
            .unwrap_or_else(|| AccessibilityNode::new("truncated-root", AccessibilityRole::Root));

        let retained_count = root.count_nodes();
        let truncated_count = self.total_node_count.saturating_sub(retained_count);

        Self {
            page_id: self.page_id.clone(),
            document_revision: self.document_revision,
            root,
            is_truncated: truncated || truncated_count > 0,
            total_node_count: self.total_node_count,
            truncated_node_count: truncated_count,
        }
    }
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
    pub is_truncated: bool,
}

impl DocumentSnapshot {
    pub fn new(metadata: DocumentMetadata, accessibility_tree: AccessibilityTree) -> Self {
        let is_truncated = accessibility_tree.is_truncated;
        Self {
            metadata,
            accessibility_tree,
            is_truncated,
        }
    }

    pub fn to_bounded(&self, bounds: &QueryBounds) -> Self {
        let bounded_ax = self.accessibility_tree.to_bounded(bounds);
        let is_truncated = bounded_ax.is_truncated;
        Self {
            metadata: self.metadata.clone(),
            accessibility_tree: bounded_ax,
            is_truncated,
        }
    }
}

/// Structured semantic element representation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SemanticElement {
    pub element_ref: ElementRef,
    pub tag_name: String,
    pub attributes: BTreeMap<String, String>,
    pub text_content: String,
}
