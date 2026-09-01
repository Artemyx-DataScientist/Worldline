//! Engine-neutral experimental capability contracts for Worldline browser services.
//!
//! Exposes experimental v0.1 surfaces for:
//! - `browser.tabs`: tab lifecycle, ordering, grouping, pinning, and selection.
//! - `browser.history`: durable navigation history records, query, and independent clear.

pub mod history;
pub mod tabs;

pub use history::{
    AUTH_HISTORY_DELETE, AUTH_HISTORY_READ, CONTRACT_BROWSER_HISTORY,
    CONTRACT_BROWSER_HISTORY_VERSION, ClearHistoryRequest, ClearHistoryResponse,
    DeleteHistoryEntryRequest, DeleteHistoryEntryResponse, GetHistoryEntryRequest,
    GetHistoryEntryResponse, HistoryEntry, HistoryEntryId, OP_CLEAR_HISTORY,
    OP_DELETE_HISTORY_ENTRY, OP_GET_HISTORY_ENTRY, OP_QUERY_HISTORY, QueryHistoryRequest,
    QueryHistoryResponse,
};
pub use tabs::{
    AUTH_TABS_MUTATE, AUTH_TABS_READ, CONTRACT_BROWSER_TABS, CONTRACT_BROWSER_TABS_VERSION,
    CloseTabRequest, CloseTabResponse, CreateTabRequest, CreateTabResponse, GetTabRequest,
    GetTabResponse, GroupTabsRequest, GroupTabsResponse, ListTabsRequest, ListTabsResponse,
    MoveTabRequest, MoveTabResponse, OP_CLOSE_TAB, OP_CREATE_TAB, OP_GET_TAB, OP_GROUP_TABS,
    OP_LIST_TABS, OP_MOVE_TAB, OP_PIN_TAB, OP_SELECT_TAB, OP_UNGROUP_TABS, PinTabRequest,
    PinTabResponse, SelectTabRequest, SelectTabResponse, TabGroup, TabGroupId, TabId, TabState,
    UngroupTabsRequest, UngroupTabsResponse,
};
