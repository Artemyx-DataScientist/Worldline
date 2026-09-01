//! Experimental browser.tabs v0.1 contract definitions.

use std::fmt;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::PageId;

pub const CONTRACT_BROWSER_TABS: &str = "browser.tabs";
pub const CONTRACT_BROWSER_TABS_VERSION: &str = "0.1";

pub const OP_CREATE_TAB: &str = "browser.tabs.create";
pub const OP_LIST_TABS: &str = "browser.tabs.list";
pub const OP_GET_TAB: &str = "browser.tabs.get";
pub const OP_SELECT_TAB: &str = "browser.tabs.select";
pub const OP_MOVE_TAB: &str = "browser.tabs.move";
pub const OP_PIN_TAB: &str = "browser.tabs.pin";
pub const OP_GROUP_TABS: &str = "browser.tabs.group";
pub const OP_UNGROUP_TABS: &str = "browser.tabs.ungroup";
pub const OP_CLOSE_TAB: &str = "browser.tabs.close";

pub const AUTH_TABS_READ: &str = "browser.tabs.read";
pub const AUTH_TABS_MUTATE: &str = "browser.tabs.mutate";

/// Opaque identity of a user-level tab.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct TabId(String);

impl TabId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TabId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for TabId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for TabId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque identity of a tab group.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct TabGroupId(String);

impl TabGroupId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TabGroupId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for TabGroupId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for TabGroupId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Metadata describing a tab group.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: TabGroupId,
    pub title: Option<String>,
    pub color: Option<String>,
}

/// State representation of a tab within the tabs service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    pub id: TabId,
    pub page_id: PageId,
    pub group_id: Option<TabGroupId>,
    pub pinned: bool,
    pub selected: bool,
    pub order_index: usize,
    pub title: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateTabRequest {
    pub page_id: PageId,
    pub group_id: Option<TabGroupId>,
    pub pinned: Option<bool>,
    pub select: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateTabResponse {
    pub tab: TabState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListTabsRequest {
    pub group_id: Option<TabGroupId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListTabsResponse {
    pub tabs: Vec<TabState>,
    pub selected_tab_id: Option<TabId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetTabRequest {
    pub tab_id: TabId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetTabResponse {
    pub tab: TabState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SelectTabRequest {
    pub tab_id: TabId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SelectTabResponse {
    pub selected_tab_id: TabId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MoveTabRequest {
    pub tab_id: TabId,
    pub new_order_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MoveTabResponse {
    pub tab: TabState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PinTabRequest {
    pub tab_id: TabId,
    pub pinned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PinTabResponse {
    pub tab: TabState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GroupTabsRequest {
    pub tab_ids: Vec<TabId>,
    pub group_id: Option<TabGroupId>,
    pub title: Option<String>,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GroupTabsResponse {
    pub group: TabGroup,
    pub tab_ids: Vec<TabId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UngroupTabsRequest {
    pub tab_ids: Vec<TabId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UngroupTabsResponse {
    pub ungrouped_tab_ids: Vec<TabId>,
}

/// Closes (detaches) a tab from the tabs service.
/// IMPORTANT: Closing a tab does NOT close the underlying PageId.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloseTabRequest {
    pub tab_id: TabId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloseTabResponse {
    pub closed_tab_id: TabId,
    pub detached_page_id: PageId,
    pub new_selected_tab_id: Option<TabId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_id_roundtrip() {
        let tab_id = TabId::new("tab-123");
        assert_eq!(tab_id.as_str(), "tab-123");
        assert_eq!(tab_id.to_string(), "tab-123");

        let json = serde_json::to_string(&tab_id).unwrap();
        let parsed: TabId = serde_json::from_str(&json).unwrap();
        assert_eq!(tab_id, parsed);
    }
}
