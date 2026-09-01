use worldline_browser_contract::{
    authority::{OP_CLICK, OP_NAVIGATE, OP_OBSERVE, OP_QUERY_DOCUMENT},
    identity::{DocumentRevision, NavigationId, PageId},
};
use worldline_browser_services_contract::{
    AUTH_HISTORY_DELETE, AUTH_HISTORY_READ, AUTH_TABS_MUTATE, AUTH_TABS_READ,
    CONTRACT_BROWSER_HISTORY, CONTRACT_BROWSER_HISTORY_VERSION, CONTRACT_BROWSER_TABS,
    CONTRACT_BROWSER_TABS_VERSION, ClearHistoryRequest, CloseTabRequest, CloseTabResponse,
    CreateTabRequest, HistoryEntry, HistoryEntryId, QueryHistoryRequest, TabGroupId, TabId,
    TabState,
};

#[test]
fn tabs_contract_wire_fixtures_stability() {
    assert_eq!(CONTRACT_BROWSER_TABS, "browser.tabs");
    assert_eq!(CONTRACT_BROWSER_TABS_VERSION, "0.1");

    let create_tab_json = r#"{
        "page_id": "page-123",
        "group_id": "grp-1",
        "pinned": true,
        "select": true
    }"#;
    let create_req: CreateTabRequest = serde_json::from_str(create_tab_json).unwrap();
    assert_eq!(create_req.page_id.as_str(), "page-123");
    assert_eq!(
        create_req.group_id.as_ref().map(|g| g.as_str()),
        Some("grp-1")
    );
    assert_eq!(create_req.pinned, Some(true));
    assert_eq!(create_req.select, Some(true));

    let tab_state = TabState {
        id: TabId::new("tab-1"),
        page_id: PageId::new("page-123"),
        group_id: Some(TabGroupId::new("grp-1")),
        pinned: true,
        selected: true,
        order_index: 0,
        title: Some("Example Tab".to_string()),
        url: Some("https://worldline.test/page".to_string()),
    };
    let tab_json = serde_json::to_string(&tab_state).unwrap();
    let tab_res: TabState = serde_json::from_str(&tab_json).unwrap();
    assert_eq!(tab_state, tab_res);

    let close_req = CloseTabRequest {
        tab_id: TabId::new("tab-1"),
    };
    let close_json = serde_json::to_string(&close_req).unwrap();
    let close_parsed: CloseTabRequest = serde_json::from_str(&close_json).unwrap();
    assert_eq!(close_req, close_parsed);

    let close_res = CloseTabResponse {
        closed_tab_id: TabId::new("tab-1"),
        detached_page_id: PageId::new("page-123"),
        new_selected_tab_id: None,
    };
    let close_res_json = serde_json::to_string(&close_res).unwrap();
    let close_res_parsed: CloseTabResponse = serde_json::from_str(&close_res_json).unwrap();
    assert_eq!(close_res, close_res_parsed);
}

#[test]
fn history_contract_wire_fixtures_stability() {
    assert_eq!(CONTRACT_BROWSER_HISTORY, "browser.history");
    assert_eq!(CONTRACT_BROWSER_HISTORY_VERSION, "0.1");

    let entry = HistoryEntry {
        entry_id: HistoryEntryId::new("hist-1"),
        page_id: PageId::new("page-123"),
        navigation_id: NavigationId::new("nav-55"),
        document_revision: DocumentRevision::new(2),
        url: "https://worldline.test/docs".to_string(),
        title: Some("Worldline Docs".to_string()),
        committed_at_unix_ms: 1725180000000,
        visit_count: 1,
    };

    let entry_json = serde_json::to_string(&entry).unwrap();
    let parsed_entry: HistoryEntry = serde_json::from_str(&entry_json).unwrap();
    assert_eq!(entry, parsed_entry);

    let query_req = QueryHistoryRequest {
        query: Some("Docs".to_string()),
        max_results: Some(10),
        start_time_unix_ms: Some(1725000000000),
        end_time_unix_ms: None,
    };
    let query_json = serde_json::to_string(&query_req).unwrap();
    let parsed_query: QueryHistoryRequest = serde_json::from_str(&query_json).unwrap();
    assert_eq!(query_req, parsed_query);

    let clear_req = ClearHistoryRequest {
        start_time_unix_ms: None,
        end_time_unix_ms: None,
    };
    let clear_json = serde_json::to_string(&clear_req).unwrap();
    let parsed_clear: ClearHistoryRequest = serde_json::from_str(&clear_json).unwrap();
    assert_eq!(clear_req, parsed_clear);
}

#[test]
fn authority_separation_invariants() {
    // Tabs authorities are distinct from engine authorities
    let tabs_authorities = [AUTH_TABS_READ, AUTH_TABS_MUTATE];
    let history_authorities = [AUTH_HISTORY_READ, AUTH_HISTORY_DELETE];
    let engine_mutation_authorities = [OP_NAVIGATE, OP_CLICK];
    let engine_observation_authorities = [OP_OBSERVE, OP_QUERY_DOCUMENT];

    for tabs_auth in &tabs_authorities {
        for eng_mut in &engine_mutation_authorities {
            assert_ne!(
                *tabs_auth, *eng_mut,
                "Tabs authority must not grant engine mutation"
            );
        }
        for eng_obs in &engine_observation_authorities {
            assert_ne!(
                *tabs_auth, *eng_obs,
                "Tabs authority must not grant engine observation"
            );
        }
    }

    for hist_auth in &history_authorities {
        for eng_mut in &engine_mutation_authorities {
            assert_ne!(
                *hist_auth, *eng_mut,
                "History authority must not grant engine mutation"
            );
        }
    }
}
