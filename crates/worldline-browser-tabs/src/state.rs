use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::PageId;
use worldline_browser_services_contract::{TabGroup, TabGroupId, TabId, TabState};

/// Persistent snapshot of tabs service state.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TabsSnapshot {
    pub tabs: BTreeMap<TabId, TabState>,
    pub tab_order: Vec<TabId>,
    pub groups: BTreeMap<TabGroupId, TabGroup>,
    pub selected_tab_id: Option<TabId>,
    pub next_tab_index: u64,
    pub next_group_index: u64,
}

impl TabsSnapshot {
    pub fn new() -> Self {
        Self {
            tabs: BTreeMap::new(),
            tab_order: Vec::new(),
            groups: BTreeMap::new(),
            selected_tab_id: None,
            next_tab_index: 1,
            next_group_index: 1,
        }
    }

    pub fn generate_tab_id(&mut self) -> TabId {
        let id = TabId::new(format!("tab-{}", self.next_tab_index));
        self.next_tab_index += 1;
        id
    }

    pub fn generate_group_id(&mut self) -> TabGroupId {
        let id = TabGroupId::new(format!("group-{}", self.next_group_index));
        self.next_group_index += 1;
        id
    }

    pub fn insert_tab(
        &mut self,
        page_id: PageId,
        group_id: Option<TabGroupId>,
        pinned: bool,
        select: bool,
    ) -> TabState {
        let tab_id = self.generate_tab_id();
        let order_index = self.tab_order.len();

        let tab = TabState {
            id: tab_id.clone(),
            page_id,
            group_id,
            pinned,
            selected: false,
            order_index,
            title: None,
            url: None,
        };

        self.tabs.insert(tab_id.clone(), tab.clone());
        self.tab_order.push(tab_id.clone());

        if select || self.selected_tab_id.is_none() {
            self.set_selected(&tab_id);
        }

        self.get_tab(&tab_id).cloned().unwrap_or(tab)
    }

    pub fn get_tab(&self, tab_id: &TabId) -> Option<&TabState> {
        self.tabs.get(tab_id)
    }

    pub fn get_tab_mut(&mut self, tab_id: &TabId) -> Option<&mut TabState> {
        self.tabs.get_mut(tab_id)
    }

    pub fn set_selected(&mut self, tab_id: &TabId) -> bool {
        if !self.tabs.contains_key(tab_id) {
            return false;
        }

        for (id, tab) in &mut self.tabs {
            tab.selected = id == tab_id;
        }
        self.selected_tab_id = Some(tab_id.clone());
        true
    }

    pub fn move_tab(&mut self, tab_id: &TabId, new_index: usize) -> Option<TabState> {
        let current_pos = self.tab_order.iter().position(|id| id == tab_id)?;
        self.tab_order.remove(current_pos);

        let target_index = new_index.min(self.tab_order.len());
        self.tab_order.insert(target_index, tab_id.clone());

        self.recompute_indices();
        self.get_tab(tab_id).cloned()
    }

    pub fn pin_tab(&mut self, tab_id: &TabId, pinned: bool) -> Option<TabState> {
        let tab = self.tabs.get_mut(tab_id)?;
        tab.pinned = pinned;

        // Ensure pinned tabs stay at the beginning
        let id_clone = tab_id.clone();
        if pinned {
            let current_pos = self.tab_order.iter().position(|id| id == &id_clone)?;
            self.tab_order.remove(current_pos);
            // Insert after the last pinned tab
            let insert_pos = self
                .tab_order
                .iter()
                .position(|id| !self.tabs.get(id).is_some_and(|t| t.pinned))
                .unwrap_or(self.tab_order.len());
            self.tab_order.insert(insert_pos, id_clone);
        }

        self.recompute_indices();
        self.get_tab(tab_id).cloned()
    }

    pub fn group_tabs(
        &mut self,
        tab_ids: &[TabId],
        group_id: Option<TabGroupId>,
        title: Option<String>,
        color: Option<String>,
    ) -> (TabGroup, Vec<TabId>) {
        let gid = group_id.unwrap_or_else(|| self.generate_group_id());
        let group = TabGroup {
            id: gid.clone(),
            title,
            color,
        };
        self.groups.insert(gid.clone(), group.clone());

        let mut affected = Vec::new();
        for id in tab_ids {
            if let Some(tab) = self.tabs.get_mut(id) {
                tab.group_id = Some(gid.clone());
                affected.push(id.clone());
            }
        }

        (group, affected)
    }

    pub fn ungroup_tabs(&mut self, tab_ids: &[TabId]) -> Vec<TabId> {
        let mut ungrouped = Vec::new();
        for id in tab_ids {
            if let Some(tab) = self.tabs.get_mut(id) {
                tab.group_id = None;
                ungrouped.push(id.clone());
            }
        }
        ungrouped
    }

    /// Detaches a tab without affecting the underlying PageId.
    pub fn detach_tab(&mut self, tab_id: &TabId) -> Option<(PageId, Option<TabId>)> {
        let tab = self.tabs.remove(tab_id)?;
        let detached_page_id = tab.page_id;

        if let Some(pos) = self.tab_order.iter().position(|id| id == tab_id) {
            self.tab_order.remove(pos);
        }

        self.recompute_indices();

        // If the closed tab was selected, pick an adjacent tab
        if self.selected_tab_id.as_ref() == Some(tab_id) {
            self.selected_tab_id = None;
            if let Some(first_tab) = self.tab_order.first().cloned() {
                self.set_selected(&first_tab);
            }
        }

        Some((detached_page_id, self.selected_tab_id.clone()))
    }

    /// Reconciles when a page was closed externally.
    pub fn remove_tabs_by_page_id(&mut self, page_id: &PageId) -> Vec<TabId> {
        let matching_tabs: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|(_, tab)| &tab.page_id == page_id)
            .map(|(id, _)| id.clone())
            .collect();

        for id in &matching_tabs {
            self.detach_tab(id);
        }

        matching_tabs
    }

    /// Reconciles surviving tabs against available pages on restart.
    pub fn reconcile_surviving_pages(&mut self, available_pages: &[PageId]) {
        let orphaned_tabs: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|(_, tab)| !available_pages.contains(&tab.page_id))
            .map(|(id, _)| id.clone())
            .collect();

        for id in orphaned_tabs {
            self.detach_tab(&id);
        }
    }

    fn recompute_indices(&mut self) {
        for (index, id) in self.tab_order.iter().enumerate() {
            if let Some(tab) = self.tabs.get_mut(id) {
                tab.order_index = index;
            }
        }
    }
}
