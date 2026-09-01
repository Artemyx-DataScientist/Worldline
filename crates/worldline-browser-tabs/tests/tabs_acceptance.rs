use worldline_browser_contract::identity::PageId;
use worldline_browser_services_contract::{
    CloseTabRequest, CreateTabRequest, GroupTabsRequest, ListTabsRequest, MoveTabRequest,
    OP_CREATE_TAB, OP_GET_TAB, OP_LIST_TABS, PinTabRequest, SelectTabRequest, TabGroupId,
    UngroupTabsRequest,
};
use worldline_browser_tabs::{TabsService, TabsSnapshot};

#[test]
fn tab_lifecycle_and_detach_semantics() {
    let service = TabsService::new();
    let page1 = PageId::new("page-1");
    let page2 = PageId::new("page-2");

    // 1. Create tab 1 referencing page1
    let t1_res = service.create_tab(CreateTabRequest {
        page_id: page1.clone(),
        group_id: None,
        pinned: Some(false),
        select: Some(true),
    });
    let t1 = t1_res.tab;
    assert_eq!(t1.page_id, page1);
    assert!(t1.selected);
    assert_eq!(t1.order_index, 0);

    // 2. Create tab 2 referencing page2
    let t2_res = service.create_tab(CreateTabRequest {
        page_id: page2.clone(),
        group_id: None,
        pinned: Some(false),
        select: Some(false),
    });
    let t2 = t2_res.tab;
    assert_eq!(t2.page_id, page2);
    assert!(!t2.selected);
    assert_eq!(t2.order_index, 1);

    // 3. List tabs
    let list = service.list_tabs(ListTabsRequest::default());
    assert_eq!(list.tabs.len(), 2);
    assert_eq!(list.selected_tab_id, Some(t1.id.clone()));

    // 4. Select tab 2
    let sel_res = service
        .select_tab(SelectTabRequest {
            tab_id: t2.id.clone(),
        })
        .unwrap();
    assert_eq!(sel_res.selected_tab_id, t2.id);

    let list_after_sel = service.list_tabs(ListTabsRequest::default());
    assert_eq!(list_after_sel.selected_tab_id, Some(t2.id.clone()));

    // 5. Close / Detach Tab 1
    // DETACH ONLY: The underlying PageId remains valid and alive
    let close_res = service
        .close_tab(CloseTabRequest {
            tab_id: t1.id.clone(),
        })
        .unwrap();
    assert_eq!(close_res.closed_tab_id, t1.id);
    assert_eq!(close_res.detached_page_id, page1);
    // After closing tab 1, tab 2 is still the selected tab
    assert_eq!(close_res.new_selected_tab_id, Some(t2.id.clone()));

    let list_after_close = service.list_tabs(ListTabsRequest::default());
    assert_eq!(list_after_close.tabs.len(), 1);
    assert_eq!(list_after_close.tabs[0].id, t2.id);
}

#[test]
fn tab_pinning_moving_and_grouping() {
    let service = TabsService::new();
    let page1 = PageId::new("page-1");
    let page2 = PageId::new("page-2");
    let page3 = PageId::new("page-3");

    let t1 = service
        .create_tab(CreateTabRequest {
            page_id: page1,
            group_id: None,
            pinned: Some(false),
            select: Some(true),
        })
        .tab;

    let t2 = service
        .create_tab(CreateTabRequest {
            page_id: page2,
            group_id: None,
            pinned: Some(false),
            select: Some(false),
        })
        .tab;

    let t3 = service
        .create_tab(CreateTabRequest {
            page_id: page3,
            group_id: None,
            pinned: Some(false),
            select: Some(false),
        })
        .tab;

    // Pin tab 3 -> should move to index 0
    let pin_res = service
        .pin_tab(PinTabRequest {
            tab_id: t3.id.clone(),
            pinned: true,
        })
        .unwrap();
    assert!(pin_res.tab.pinned);
    assert_eq!(pin_res.tab.order_index, 0);

    // Move tab 1 to the end
    let move_res = service
        .move_tab(MoveTabRequest {
            tab_id: t1.id.clone(),
            new_order_index: 2,
        })
        .unwrap();
    assert_eq!(move_res.tab.order_index, 2);

    // Group tab 1 and tab 2
    let grp_res = service.group_tabs(GroupTabsRequest {
        tab_ids: vec![t1.id.clone(), t2.id.clone()],
        group_id: Some(TabGroupId::new("work-group")),
        title: Some("Work".to_string()),
        color: Some("blue".to_string()),
    });
    assert_eq!(grp_res.group.id.as_str(), "work-group");
    assert_eq!(grp_res.tab_ids.len(), 2);

    // Filter list by group
    let grp_list = service.list_tabs(ListTabsRequest {
        group_id: Some(TabGroupId::new("work-group")),
    });
    assert_eq!(grp_list.tabs.len(), 2);

    // Ungroup tab 1
    let ungrp_res = service.ungroup_tabs(UngroupTabsRequest {
        tab_ids: vec![t1.id.clone()],
    });
    assert_eq!(ungrp_res.ungrouped_tab_ids, vec![t1.id]);
}

#[test]
fn tab_external_event_and_restart_reconciliation() {
    let service = TabsService::new();
    let page1 = PageId::new("page-1");
    let page2 = PageId::new("page-2");
    let page3 = PageId::new("page-3");

    let t1 = service
        .create_tab(CreateTabRequest {
            page_id: page1.clone(),
            group_id: None,
            pinned: Some(false),
            select: Some(true),
        })
        .tab;

    let t2 = service
        .create_tab(CreateTabRequest {
            page_id: page2.clone(),
            group_id: None,
            pinned: Some(false),
            select: Some(false),
        })
        .tab;

    let _t3 = service
        .create_tab(CreateTabRequest {
            page_id: page3.clone(),
            group_id: None,
            pinned: Some(false),
            select: Some(false),
        })
        .tab;

    // Simulate external page closure for page2
    let removed_tabs = service.on_page_closed(&page2);
    assert_eq!(removed_tabs, vec![t2.id]);

    let list = service.list_tabs(ListTabsRequest::default());
    assert_eq!(list.tabs.len(), 2);

    // Export persistent state snapshot (simulate persistence)
    let snapshot = service.export_snapshot();
    let snapshot_json = serde_json::to_string(&snapshot).unwrap();
    let restored_snapshot: TabsSnapshot = serde_json::from_str(&snapshot_json).unwrap();

    // Restart service from snapshot
    let restarted_service = TabsService::from_snapshot(restored_snapshot);

    // Reconcile with available active pages (say page3 crashed/disappeared)
    restarted_service.reconcile_pages(std::slice::from_ref(&page1));

    let final_list = restarted_service.list_tabs(ListTabsRequest::default());
    assert_eq!(final_list.tabs.len(), 1);
    assert_eq!(final_list.tabs[0].id, t1.id);
    assert_eq!(final_list.tabs[0].page_id, page1);
}

#[test]
fn tab_rpc_dispatcher() {
    let service = TabsService::new();
    let create_payload = serde_json::json!({
        "page_id": "page-99",
        "pinned": false,
        "select": true
    });

    let res = service.dispatch(OP_CREATE_TAB, create_payload).unwrap();
    assert_eq!(res["tab"]["page_id"], "page-99");
    let tab_id = res["tab"]["id"].as_str().unwrap().to_string();

    let get_payload = serde_json::json!({
        "tab_id": tab_id
    });
    let get_res = service.dispatch(OP_GET_TAB, get_payload).unwrap();
    assert_eq!(get_res["tab"]["id"], tab_id);

    let list_res = service
        .dispatch(OP_LIST_TABS, serde_json::json!({}))
        .unwrap();
    assert_eq!(list_res["tabs"].as_array().unwrap().len(), 1);
}
