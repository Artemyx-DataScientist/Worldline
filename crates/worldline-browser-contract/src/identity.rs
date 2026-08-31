use serde::{Deserialize, Serialize};
use std::fmt;

/// Opaque identity of an isolated browser context or profile.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BrowserContextId(String);

impl BrowserContextId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for BrowserContextId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for BrowserContextId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for BrowserContextId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque identity of a browser page surface.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PageId(String);

impl PageId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PageId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PageId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for PageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Logical navigation attempt or outcome identity.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct NavigationId(String);

impl NavigationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NavigationId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for NavigationId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for NavigationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Monotonic revision counter of a page's document state.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct DocumentRevision(u64);

impl DocumentRevision {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn initial() -> Self {
        Self(1)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for DocumentRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "rev:{}", self.0)
    }
}

/// Opaque identity of a download operation.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DownloadId(String);

impl DownloadId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DownloadId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DownloadId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for DownloadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Stable element reference within a specific page and document revision.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ElementRef {
    page_id: PageId,
    document_revision: DocumentRevision,
    node_key: String,
}

impl ElementRef {
    pub fn new(
        page_id: impl Into<PageId>,
        document_revision: DocumentRevision,
        node_key: impl Into<String>,
    ) -> Self {
        Self {
            page_id: page_id.into(),
            document_revision,
            node_key: node_key.into(),
        }
    }

    pub fn page_id(&self) -> &PageId {
        &self.page_id
    }

    pub const fn document_revision(&self) -> DocumentRevision {
        self.document_revision
    }

    pub fn node_key(&self) -> &str {
        &self.node_key
    }

    pub fn is_valid_for(&self, page_id: &PageId, current_revision: DocumentRevision) -> bool {
        &self.page_id == page_id && self.document_revision == current_revision
    }
}

impl fmt::Display for ElementRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "elem:{}:{}:{}",
            self.page_id, self.document_revision.0, self.node_key
        )
    }
}

/// Constructs standard resource scope URI strings matching Worldline security model.
pub fn context_resource(context_id: &BrowserContextId) -> String {
    format!("browser-context/{context_id}")
}

pub fn page_resource(page_id: &PageId) -> String {
    format!("browser-page/{page_id}")
}

pub fn download_resource(download_id: &DownloadId) -> String {
    format!("browser-download/{download_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_ref_staleness_check() {
        let page = PageId::new("page-1");
        let rev1 = DocumentRevision::new(1);
        let rev2 = DocumentRevision::new(2);
        let elem = ElementRef::new(page.clone(), rev1, "btn-submit");

        assert!(elem.is_valid_for(&page, rev1));
        assert!(!elem.is_valid_for(&page, rev2));
        assert!(!elem.is_valid_for(&PageId::new("page-2"), rev1));
    }

    #[test]
    fn resource_path_formatting() {
        assert_eq!(
            context_resource(&BrowserContextId::new("ctx-1")),
            "browser-context/ctx-1"
        );
        assert_eq!(
            page_resource(&PageId::new("page-10")),
            "browser-page/page-10"
        );
        assert_eq!(
            download_resource(&DownloadId::new("dl-99")),
            "browser-download/dl-99"
        );
    }
}
