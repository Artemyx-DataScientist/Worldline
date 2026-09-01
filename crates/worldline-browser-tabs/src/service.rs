use std::sync::Mutex;

use serde_json::Value;
use worldline_browser_contract::identity::PageId;
use worldline_browser_services_contract::{
    CloseTabRequest, CloseTabResponse, CreateTabRequest, CreateTabResponse, GetTabRequest,
    GetTabResponse, GroupTabsRequest, GroupTabsResponse, ListTabsRequest, ListTabsResponse,
    MoveTabRequest, MoveTabResponse, OP_CLOSE_TAB, OP_CREATE_TAB, OP_GET_TAB, OP_GROUP_TABS,
    OP_LIST_TABS, OP_MOVE_TAB, OP_PIN_TAB, OP_SELECT_TAB, OP_UNGROUP_TABS, PinTabRequest,
    PinTabResponse, SelectTabRequest, SelectTabResponse, TabId, TabState, UngroupTabsRequest,
    UngroupTabsResponse,
};

use crate::state::TabsSnapshot;

/// Tabs service providing transactional tab state management above the engine provider.
pub struct TabsService {
    state: Mutex<TabsSnapshot>,
}

impl Default for TabsService {
    fn default() -> Self {
        Self::new()
    }
}

impl TabsService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(TabsSnapshot::new()),
        }
    }

    pub fn from_snapshot(snapshot: TabsSnapshot) -> Self {
        Self {
            state: Mutex::new(snapshot),
        }
    }

    pub fn export_snapshot(&self) -> TabsSnapshot {
        self.state.lock().unwrap().clone()
    }

    pub fn create_tab(&self, req: CreateTabRequest) -> CreateTabResponse {
        let mut state = self.state.lock().unwrap();
        let tab = state.insert_tab(
            req.page_id,
            req.group_id,
            req.pinned.unwrap_or(false),
            req.select.unwrap_or(true),
        );
        CreateTabResponse { tab }
    }

    pub fn list_tabs(&self, req: ListTabsRequest) -> ListTabsResponse {
        let state = self.state.lock().unwrap();
        let tabs: Vec<TabState> = state
            .tab_order
            .iter()
            .filter_map(|id| state.get_tab(id).cloned())
            .filter(|tab| match &req.group_id {
                Some(gid) => tab.group_id.as_ref() == Some(gid),
                None => true,
            })
            .collect();

        ListTabsResponse {
            tabs,
            selected_tab_id: state.selected_tab_id.clone(),
        }
    }

    pub fn get_tab(&self, req: GetTabRequest) -> Result<GetTabResponse, String> {
        let state = self.state.lock().unwrap();
        state
            .get_tab(&req.tab_id)
            .cloned()
            .map(|tab| GetTabResponse { tab })
            .ok_or_else(|| format!("Tab '{}' not found", req.tab_id))
    }

    pub fn select_tab(&self, req: SelectTabRequest) -> Result<SelectTabResponse, String> {
        let mut state = self.state.lock().unwrap();
        if state.set_selected(&req.tab_id) {
            Ok(SelectTabResponse {
                selected_tab_id: req.tab_id,
            })
        } else {
            Err(format!("Tab '{}' not found to select", req.tab_id))
        }
    }

    pub fn move_tab(&self, req: MoveTabRequest) -> Result<MoveTabResponse, String> {
        let mut state = self.state.lock().unwrap();
        state
            .move_tab(&req.tab_id, req.new_order_index)
            .map(|tab| MoveTabResponse { tab })
            .ok_or_else(|| format!("Tab '{}' not found to move", req.tab_id))
    }

    pub fn pin_tab(&self, req: PinTabRequest) -> Result<PinTabResponse, String> {
        let mut state = self.state.lock().unwrap();
        state
            .pin_tab(&req.tab_id, req.pinned)
            .map(|tab| PinTabResponse { tab })
            .ok_or_else(|| format!("Tab '{}' not found to pin", req.tab_id))
    }

    pub fn group_tabs(&self, req: GroupTabsRequest) -> GroupTabsResponse {
        let mut state = self.state.lock().unwrap();
        let (group, tab_ids) = state.group_tabs(&req.tab_ids, req.group_id, req.title, req.color);
        GroupTabsResponse { group, tab_ids }
    }

    pub fn ungroup_tabs(&self, req: UngroupTabsRequest) -> UngroupTabsResponse {
        let mut state = self.state.lock().unwrap();
        let ungrouped_tab_ids = state.ungroup_tabs(&req.tab_ids);
        UngroupTabsResponse { ungrouped_tab_ids }
    }

    /// Detaches a tab from the tabs service. Does NOT close the underlying PageId.
    pub fn close_tab(&self, req: CloseTabRequest) -> Result<CloseTabResponse, String> {
        let mut state = self.state.lock().unwrap();
        let tab_id = req.tab_id;
        state
            .detach_tab(&tab_id)
            .map(|(detached_page_id, new_selected_tab_id)| CloseTabResponse {
                closed_tab_id: tab_id.clone(),
                detached_page_id,
                new_selected_tab_id,
            })
            .ok_or_else(|| format!("Tab '{}' not found to close", tab_id))
    }

    /// Reconciles state when a page is closed in the engine provider.
    pub fn on_page_closed(&self, page_id: &PageId) -> Vec<TabId> {
        let mut state = self.state.lock().unwrap();
        state.remove_tabs_by_page_id(page_id)
    }

    /// Reconciles references against currently available PageIds on service restart.
    pub fn reconcile_pages(&self, active_pages: &[PageId]) {
        let mut state = self.state.lock().unwrap();
        state.reconcile_surviving_pages(active_pages);
    }

    /// Dispatches RPC operations to the corresponding handler.
    pub fn dispatch(&self, operation: &str, payload: Value) -> Result<Value, String> {
        match operation {
            OP_CREATE_TAB => {
                let req: CreateTabRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.create_tab(req);
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_LIST_TABS => {
                let req: ListTabsRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.list_tabs(req);
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_GET_TAB => {
                let req: GetTabRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.get_tab(req)?;
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_SELECT_TAB => {
                let req: SelectTabRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.select_tab(req)?;
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_MOVE_TAB => {
                let req: MoveTabRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.move_tab(req)?;
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_PIN_TAB => {
                let req: PinTabRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.pin_tab(req)?;
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_GROUP_TABS => {
                let req: GroupTabsRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.group_tabs(req);
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_UNGROUP_TABS => {
                let req: UngroupTabsRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.ungroup_tabs(req);
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            OP_CLOSE_TAB => {
                let req: CloseTabRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let res = self.close_tab(req)?;
                serde_json::to_value(res).map_err(|e| e.to_string())
            }
            unknown => Err(format!("Unsupported tabs operation '{unknown}'")),
        }
    }
}
