//! S3A: Live browser services proving slice (Tabs and Durable History).
//!
//! Verifies the first bounded M1.3 browser services workflow:
//! 1. Create a real PageId in the browser provider.
//! 2. Attach the PageId to a TabId in the tabs service.
//! 3. Navigate the page to a loopback/test URL.
//! 4. Durably record the committed navigation in the history service.
//! 5. Enrich the title via page readiness.
//! 6. Restart the history service from snapshot and verify record survival.
//! 7. Remove/detach the tab in the tabs service.
//! 8. Verify the referenced PageId remains alive and operational in the engine provider.

use std::sync::Arc;

use worldline_browser_contract::{
    authority::*,
    contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        NavigateRequest, NavigateResponse, ObservePageRequest, PageObservation,
    },
    identity::NavigationId,
};
use worldline_browser_history::{HistoryService, HistoryStoreSnapshot};
use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};
use worldline_browser_services_contract::{
    CloseTabRequest, CreateTabRequest, GetHistoryEntryRequest, HistoryEntryId, TabId,
};
use worldline_browser_tabs::TabsService;

/// Proving report emitted by slice S3A.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3AReport {
    pub page_id: String,
    pub tab_id: String,
    pub history_entry_id: String,
    pub initial_url: String,
    pub navigated_url: String,
    pub history_survived_restart: bool,
    pub page_survived_tab_removal: bool,
    pub post_removal_navigation_ok: bool,
}

/// Executes the live browser services proving slice S3A.
pub fn run() -> Result<S3AReport, String> {
    let backend = ReferenceBrowserBackend::new();
    let core = Arc::new(BrowserProviderCore::new(backend));

    // 1. Create browser context and page in engine provider
    let ctx_val = core
        .dispatch(
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some("s3a-profile".to_string()),
                incognito: false,
                user_agent: Some("Worldline-S3A/1.0".to_string()),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let ctx: CreateContextResponse = serde_json::from_value(ctx_val).map_err(|e| e.to_string())?;

    let initial_url = "https://worldline.test/s3a-initial".to_string();
    let page_val = core
        .dispatch(
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx.context_id.clone(),
                initial_url: Some(initial_url.clone()),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let page: CreatePageResponse = serde_json::from_value(page_val).map_err(|e| e.to_string())?;

    // 2. Attach PageId to a TabId in the tabs service
    let tabs_service = TabsService::new();
    let tab_res = tabs_service.create_tab(CreateTabRequest {
        page_id: page.page_id.clone(),
        group_id: None,
        pinned: Some(false),
        select: Some(true),
    });
    let tab_id: TabId = tab_res.tab.id;

    // 3. Navigate page
    let target_url = "https://worldline.test/s3a-navigated".to_string();
    let nav_val = core
        .dispatch(
            OP_NAVIGATE,
            serde_json::to_value(NavigateRequest {
                page_id: page.page_id.clone(),
                url: target_url.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let nav: NavigateResponse = serde_json::from_value(nav_val).map_err(|e| e.to_string())?;

    // 4. Durably record committed navigation in history service
    let history_service = HistoryService::new();
    let nav_id = NavigationId::new(format!("nav-s3a-{}", nav.document_revision.value()));
    let hist_entry = history_service
        .record_navigation(
            page.page_id.clone(),
            nav_id,
            nav.document_revision,
            target_url.clone(),
            1725185000000,
        )
        .map_err(|e| e.to_string())?;
    let history_entry_id: HistoryEntryId = hist_entry.entry_id.clone();

    // 5. Enrich title via simulated page ready fact
    history_service.enrich_title(
        &page.page_id,
        nav.document_revision,
        "S3A Proving Page".to_string(),
    );

    // 6. Restart history service from snapshot and verify survival
    let history_snapshot = history_service.export_snapshot();
    let history_snapshot_json =
        serde_json::to_string(&history_snapshot).map_err(|e| e.to_string())?;
    let restored_snapshot: HistoryStoreSnapshot =
        serde_json::from_str(&history_snapshot_json).map_err(|e| e.to_string())?;
    let restarted_history = HistoryService::from_snapshot(restored_snapshot);

    let get_entry = restarted_history
        .get_history_entry(GetHistoryEntryRequest {
            entry_id: history_entry_id.clone(),
        })
        .map_err(|e| e.to_string())?;
    let history_survived_restart = get_entry.entry.url == target_url
        && get_entry.entry.title == Some("S3A Proving Page".to_string());

    // 7. Remove/detach tab in tabs service
    let close_res = tabs_service
        .close_tab(CloseTabRequest {
            tab_id: tab_id.clone(),
        })
        .map_err(|e| e.to_string())?;
    let page_survived_tab_removal = close_res.detached_page_id == page.page_id;

    // 8. Verify the referenced PageId remains alive and operational in the engine provider
    let post_removal_url = "https://worldline.test/s3a-post-removal".to_string();
    let post_nav_val = core
        .dispatch(
            OP_NAVIGATE,
            serde_json::to_value(NavigateRequest {
                page_id: page.page_id.clone(),
                url: post_removal_url.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let _post_nav: NavigateResponse =
        serde_json::from_value(post_nav_val).map_err(|e| e.to_string())?;

    let obs_val = core
        .dispatch(
            OP_OBSERVE,
            serde_json::to_value(ObservePageRequest {
                page_id: page.page_id.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let obs: PageObservation = serde_json::from_value(obs_val).map_err(|e| e.to_string())?;
    let post_removal_navigation_ok = obs.url == post_removal_url;

    Ok(S3AReport {
        page_id: page.page_id.to_string(),
        tab_id: tab_id.to_string(),
        history_entry_id: history_entry_id.to_string(),
        initial_url,
        navigated_url: target_url,
        history_survived_restart,
        page_survived_tab_removal,
        post_removal_navigation_ok,
    })
}
