use std::collections::BTreeSet;

// Operation names for browser capability contracts
pub const OP_CREATE_CONTEXT: &str = "create";
pub const OP_CLOSE_CONTEXT: &str = "close";
pub const OP_LIST_CONTEXTS: &str = "list";

pub const OP_CREATE_PAGE: &str = "create";
pub const OP_CLOSE_PAGE: &str = "close";
pub const OP_LIST_PAGES: &str = "list";

pub const OP_NAVIGATE: &str = "navigate";
pub const OP_RELOAD: &str = "reload";
pub const OP_STOP: &str = "stop";
pub const OP_BACK: &str = "back";
pub const OP_FORWARD: &str = "forward";

pub const OP_OBSERVE: &str = "observe";
pub const OP_GET_TITLE: &str = "get_title";
pub const OP_GET_URL: &str = "get_url";

pub const OP_QUERY_DOCUMENT: &str = "query_document";
pub const OP_QUERY_ACCESSIBILITY: &str = "query_accessibility";
pub const OP_FIND_ELEMENTS: &str = "find_elements";
pub const OP_EXTRACT_TEXT: &str = "extract_text";

pub const OP_CLICK: &str = "click";
pub const OP_INPUT: &str = "input";
pub const OP_FOCUS: &str = "focus";
pub const OP_SUBMIT: &str = "submit";
pub const OP_SCROLL: &str = "scroll";

pub const OP_DOWNLOAD_START: &str = "start";
pub const OP_DOWNLOAD_CONTROL: &str = "control";
pub const OP_DOWNLOAD_STATUS: &str = "status";

pub const OP_PERMISSION_QUERY: &str = "query";
pub const OP_PERMISSION_SET: &str = "set";

pub const OP_CAPTURE: &str = "capture";
pub const OP_READ_CAPTURE: &str = "read_capture";

pub const OP_COOKIE_GET: &str = "cookie_get";
pub const OP_COOKIE_SET: &str = "cookie_set";
pub const OP_COOKIE_DELETE: &str = "cookie_delete";
pub const OP_COOKIE_GET_V0_2: &str = "cookie_get_v0_2";
pub const OP_COOKIE_SET_V0_2: &str = "cookie_set_v0_2";

pub const OP_STORAGE_CLEAR: &str = "storage_clear";
pub const OP_STORAGE_SET_V0_2: &str = "storage_set_v0_2";
pub const OP_STORAGE_GET_V0_2: &str = "storage_get_v0_2";

pub const OP_DOWNLOAD_HOOK: &str = "download_hook";

/// Logical authority classifications representing security rights.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BrowserAuthority {
    ObservePage,
    QueryDocument,
    NavigatePage,
    ActOnPage,
    ControlDownload,
    ManagePermission,
    CapturePage,
    ManageCookies,
    ManageStorage,
    ControlDownloadHook,
}

impl BrowserAuthority {
    /// Returns the capability contract and operations covered by this authority.
    pub fn allowed_operations(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::ObservePage => ("browser.observe", &[OP_OBSERVE, OP_GET_TITLE, OP_GET_URL]),
            Self::QueryDocument => (
                "browser.query",
                &[
                    OP_QUERY_DOCUMENT,
                    OP_QUERY_ACCESSIBILITY,
                    OP_FIND_ELEMENTS,
                    OP_EXTRACT_TEXT,
                ],
            ),
            Self::NavigatePage => (
                "browser.navigate",
                &[OP_NAVIGATE, OP_RELOAD, OP_STOP, OP_BACK, OP_FORWARD],
            ),
            Self::ActOnPage => (
                "browser.act",
                &[OP_CLICK, OP_INPUT, OP_FOCUS, OP_SUBMIT, OP_SCROLL],
            ),
            Self::ControlDownload => (
                "browser.download",
                &[OP_DOWNLOAD_START, OP_DOWNLOAD_CONTROL, OP_DOWNLOAD_STATUS],
            ),
            Self::ManagePermission => (
                "browser.permission",
                &[OP_PERMISSION_QUERY, OP_PERMISSION_SET],
            ),
            Self::CapturePage => ("browser.capture", &[OP_CAPTURE, OP_READ_CAPTURE]),
            Self::ManageCookies => (
                "browser.engine.cookies",
                &[
                    OP_COOKIE_GET,
                    OP_COOKIE_SET,
                    OP_COOKIE_DELETE,
                    OP_COOKIE_GET_V0_2,
                    OP_COOKIE_SET_V0_2,
                ],
            ),
            Self::ManageStorage => (
                "browser.engine.storage",
                &[OP_STORAGE_CLEAR, OP_STORAGE_SET_V0_2, OP_STORAGE_GET_V0_2],
            ),
            Self::ControlDownloadHook => ("browser.engine.download_hook", &[OP_DOWNLOAD_HOOK]),
        }
    }

    /// Checks whether this authority permits the requested operation on a contract.
    pub fn permits(self, contract_name: &str, operation: &str) -> bool {
        let (expected_contract, allowed_ops) = self.allowed_operations();
        if contract_name != expected_contract {
            return false;
        }
        allowed_ops.contains(&operation)
    }
}

/// Set of granted browser authorities.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BrowserAuthoritySet {
    authorities: BTreeSet<BrowserAuthority>,
}

impl BrowserAuthoritySet {
    pub fn new() -> Self {
        Self {
            authorities: BTreeSet::new(),
        }
    }

    pub fn with(mut self, authority: BrowserAuthority) -> Self {
        self.authorities.insert(authority);
        self
    }

    pub fn grant(&mut self, authority: BrowserAuthority) {
        self.authorities.insert(authority);
    }

    pub fn contains(&self, authority: BrowserAuthority) -> bool {
        self.authorities.contains(&authority)
    }

    pub fn permits(&self, contract_name: &str, operation: &str) -> bool {
        self.authorities
            .iter()
            .any(|auth| auth.permits(contract_name, operation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_does_not_permit_action_or_navigation() {
        let query_auth = BrowserAuthority::QueryDocument;
        assert!(query_auth.permits("browser.query", OP_QUERY_DOCUMENT));
        assert!(query_auth.permits("browser.query", OP_FIND_ELEMENTS));
        assert!(!query_auth.permits("browser.act", OP_CLICK));
        assert!(!query_auth.permits("browser.act", OP_INPUT));
        assert!(!query_auth.permits("browser.navigate", OP_NAVIGATE));
    }

    #[test]
    fn observe_does_not_permit_navigation_or_action() {
        let observe_auth = BrowserAuthority::ObservePage;
        assert!(observe_auth.permits("browser.observe", OP_OBSERVE));
        assert!(!observe_auth.permits("browser.navigate", OP_NAVIGATE));
        assert!(!observe_auth.permits("browser.act", OP_CLICK));
    }

    #[test]
    fn authority_set_checking() {
        let set = BrowserAuthoritySet::new()
            .with(BrowserAuthority::ObservePage)
            .with(BrowserAuthority::QueryDocument);

        assert!(set.permits("browser.observe", OP_OBSERVE));
        assert!(set.permits("browser.query", OP_FIND_ELEMENTS));
        assert!(!set.permits("browser.act", OP_CLICK));
        assert!(!set.permits("browser.navigate", OP_NAVIGATE));
    }

    #[test]
    fn versioned_engine_primitives_keep_their_existing_authority() {
        let authorities = BrowserAuthoritySet::new()
            .with(BrowserAuthority::ManageCookies)
            .with(BrowserAuthority::ManageStorage);

        assert!(authorities.permits("browser.engine.cookies", OP_COOKIE_GET_V0_2));
        assert!(authorities.permits("browser.engine.cookies", OP_COOKIE_SET_V0_2));
        assert!(authorities.permits("browser.engine.storage", OP_STORAGE_SET_V0_2));
        assert!(authorities.permits("browser.engine.storage", OP_STORAGE_GET_V0_2));
        assert!(!authorities.permits("browser.engine.cookies", OP_STORAGE_GET_V0_2));
    }
}
