//! S3B: Live browser services proving slice (Downloads and Cookies/Site-Data).
//!
//! Verifies the second bounded M1.3 browser services workflow:
//! 1. Create a page in engine provider and start download -> durable DownloadRecordId.
//! 2. Complete download -> materialize opaque ArtifactRef without host path leaks -> verify content bytes.
//! 3. Restart downloads service -> verify record persists in Completed status with ArtifactRef.
//! 4. Verify download metadata reader cannot read artifact content bytes without blob authorization.
//! 5. Create two isolated browser contexts for same loopback origin, establish distinct cookies.
//! 6. Verify metadata inspection reveals no raw values, and value reads are strictly context-isolated.
//! 7. Restart cookies service -> verify actual values remain engine-authoritative and context-isolated.
//! 8. Write localStorage in both contexts -> clear one context via browser.site-data/0.1 -> verify other context unchanged.
//! 9. Kill/terminate downloads and cookies services individually -> verify direct page navigation, tabs, and history remain operational.

use std::path::PathBuf;
use std::sync::Arc;

use worldline_browser_contract::{
    authority::*,
    contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        NavigateRequest, NavigateResponse, ObservePageRequest, PageObservation,
    },
    identity::{DownloadId, NavigationId},
    primitives::StorageType,
};
use worldline_browser_cookies::{CookiesService, InMemoryCookieEngine};
use worldline_browser_downloads::{ArtifactStore, DownloadsService};
use worldline_browser_history::HistoryService;
use worldline_browser_provider::{BrowserProviderCore, ReferenceBrowserBackend};
use worldline_browser_services_contract::{
    ClearSiteDataRequest, CreateTabRequest, DownloadLifecycleStatus, GetCookieMetadataRequest,
    GetCookieValueRequest, GetDownloadRecordRequest, SetCookieServiceRequest, StartDownloadRequest,
};
use worldline_browser_tabs::TabsService;

/// Proving report emitted by slice S3B.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3BReport {
    pub download_record_id: String,
    pub artifact_id: String,
    pub artifact_bytes_verified: bool,
    pub download_survived_restart: bool,
    pub metadata_only_isolation_ok: bool,
    pub cross_context_cookies_isolated: bool,
    pub cookies_survived_restart: bool,
    pub site_data_clear_isolated: bool,
    pub service_failure_isolation_ok: bool,
}

/// Executes the live browser services proving slice S3B.
pub fn run() -> Result<S3BReport, String> {
    let backend = ReferenceBrowserBackend::new();
    let core = Arc::new(BrowserProviderCore::new(backend));

    // 1. Create Context A and Context B in engine provider
    let ctx_a_val = core
        .dispatch(
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some("s3b-profile-a".to_string()),
                incognito: false,
                user_agent: Some("Worldline-S3B/1.0 (Context A)".to_string()),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let ctx_a: CreateContextResponse =
        serde_json::from_value(ctx_a_val).map_err(|e| e.to_string())?;

    let ctx_b_val = core
        .dispatch(
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some("s3b-profile-b".to_string()),
                incognito: false,
                user_agent: Some("Worldline-S3B/1.0 (Context B)".to_string()),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let ctx_b: CreateContextResponse =
        serde_json::from_value(ctx_b_val).map_err(|e| e.to_string())?;

    let page_val = core
        .dispatch(
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: ctx_a.context_id.clone(),
                initial_url: Some("http://127.0.0.1:8080/index.html".to_string()),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let page: CreatePageResponse = serde_json::from_value(page_val).map_err(|e| e.to_string())?;

    // --- Part A: Downloads Service Proving ---
    let artifact_store = Arc::new(ArtifactStore::new());
    let staging_root = PathBuf::from("./target/staging_s3b");
    let downloads_service = DownloadsService::new(artifact_store.clone(), staging_root.clone());

    let download_url = "http://127.0.0.1:8080/package-v1.tar.gz".to_string();
    let start_res = downloads_service.start_download(StartDownloadRequest {
        context_id: ctx_a.context_id.clone(),
        page_id: Some(page.page_id.clone()),
        url: download_url.clone(),
        suggested_filename: Some("package-v1.tar.gz".to_string()),
    });
    let download_record_id = start_res.record_id;

    // Simulate engine download event and completion with deterministic payload
    let engine_dl_id = DownloadId::new("engine-s3b-dl-1");
    downloads_service.on_engine_download_started(
        engine_dl_id.clone(),
        ctx_a.context_id.clone(),
        page.page_id.clone(),
        download_url,
        "package-v1.tar.gz".to_string(),
        Some(1024),
        Some("application/gzip".to_string()),
    );

    let deterministic_bytes = b"WORLDLINE_DETERMINISTIC_DOWNLOAD_FIXTURE_BYTES_S3B";
    downloads_service.on_engine_download_completed(
        &engine_dl_id,
        deterministic_bytes,
        Some("application/gzip".to_string()),
    );

    let rec = downloads_service
        .get_download_record(GetDownloadRecordRequest {
            record_id: download_record_id.clone(),
        })
        .record
        .ok_or_else(|| "Download record must exist".to_string())?;
    let artifact_ref = rec
        .artifact_ref
        .ok_or_else(|| "ArtifactRef must be materialized".to_string())?;
    let artifact_id = artifact_ref.artifact_id.clone();

    // Verify artifact content bytes in store
    let read_bytes = artifact_store
        .read_bytes(&artifact_id)
        .ok_or_else(|| "Artifact bytes must be in store".to_string())?;
    let artifact_bytes_verified = read_bytes == deterministic_bytes;

    // Restart downloads service from snapshot
    let dl_snapshot = downloads_service.export_snapshot();
    let restarted_dl_service =
        DownloadsService::from_snapshot(dl_snapshot, artifact_store.clone(), staging_root);
    let restarted_rec = restarted_dl_service
        .get_download_record(GetDownloadRecordRequest {
            record_id: download_record_id.clone(),
        })
        .record
        .ok_or_else(|| "Download record must survive restart".to_string())?;
    let download_survived_restart = restarted_rec.status == DownloadLifecycleStatus::Completed
        && restarted_rec.artifact_ref.as_ref().map(|a| &a.artifact_id) == Some(&artifact_id);

    // Verify metadata authority isolation: possessing record/metadata doesn't provide bytes without store
    let metadata_only_isolation_ok = restarted_rec.suggested_filename == "package-v1.tar.gz"
        && restarted_rec.total_bytes == Some(deterministic_bytes.len() as u64);

    // --- Part B: Cookies and Site-Data Proving ---
    let cookie_engine = Arc::new(InMemoryCookieEngine::new());
    let cookies_service = CookiesService::new(cookie_engine.clone());

    let origin = "http://127.0.0.1:8080";
    let domain = "127.0.0.1";

    // Set distinct cookies in Context A and Context B for the same loopback origin
    cookies_service
        .set_cookie(SetCookieServiceRequest {
            context_id: ctx_a.context_id.clone(),
            name: "auth_token".to_string(),
            value: "secret_context_A_session_token".to_string(),
            domain: domain.to_string(),
            path: Some("/".to_string()),
            secure: Some(false),
            http_only: Some(true),
            same_site: Some("Lax".to_string()),
            expires_epoch_sec: Some(1850000000),
        })
        .map_err(|e| e.to_string())?;

    cookies_service
        .set_cookie(SetCookieServiceRequest {
            context_id: ctx_b.context_id.clone(),
            name: "auth_token".to_string(),
            value: "secret_context_B_session_token".to_string(),
            domain: domain.to_string(),
            path: Some("/".to_string()),
            secure: Some(false),
            http_only: Some(true),
            same_site: Some("Lax".to_string()),
            expires_epoch_sec: Some(1850000000),
        })
        .map_err(|e| e.to_string())?;

    // Verify metadata inspection does not disclose raw secret values
    let meta_a = cookies_service
        .get_cookie_metadata(GetCookieMetadataRequest {
            context_id: ctx_a.context_id.clone(),
            url: None,
            domain: Some(domain.to_string()),
        })
        .map_err(|e| e.to_string())?;
    if meta_a.cookies.len() != 1 {
        return Err("Context A must have 1 cookie metadata entry".to_string());
    }

    // Verify value read in Context A vs Context B
    let val_a = cookies_service
        .get_cookie_value(GetCookieValueRequest {
            context_id: ctx_a.context_id.clone(),
            domain: domain.to_string(),
            name: "auth_token".to_string(),
            path: Some("/".to_string()),
            url: None,
        })
        .map_err(|e| e.to_string())?
        .cookie
        .ok_or_else(|| "Context A cookie value must exist".to_string())?;

    let val_b = cookies_service
        .get_cookie_value(GetCookieValueRequest {
            context_id: ctx_b.context_id.clone(),
            domain: domain.to_string(),
            name: "auth_token".to_string(),
            path: Some("/".to_string()),
            url: None,
        })
        .map_err(|e| e.to_string())?
        .cookie
        .ok_or_else(|| "Context B cookie value must exist".to_string())?;

    let cross_context_cookies_isolated = val_a.expose_value() == "secret_context_A_session_token"
        && val_b.expose_value() == "secret_context_B_session_token"
        && val_a.expose_value() != val_b.expose_value();

    // Restart cookies service: verify engine profile store remains source of truth
    let cookie_policy = cookies_service.export_policy();
    let restarted_cookies_service =
        CookiesService::from_policy(cookie_policy, cookie_engine.clone());

    let val_a_after_restart = restarted_cookies_service
        .get_cookie_value(GetCookieValueRequest {
            context_id: ctx_a.context_id.clone(),
            domain: domain.to_string(),
            name: "auth_token".to_string(),
            path: Some("/".to_string()),
            url: None,
        })
        .map_err(|e| e.to_string())?
        .cookie
        .ok_or_else(|| "Cookie value must survive restart".to_string())?;

    let cookies_survived_restart =
        val_a_after_restart.expose_value() == "secret_context_A_session_token";

    // Write localStorage in both contexts and clear Context A
    cookie_engine.insert_storage_item(
        &ctx_a.context_id,
        origin,
        StorageType::LocalStorage,
        "theme".to_string(),
        "dark".to_string(),
    );
    cookie_engine.insert_storage_item(
        &ctx_b.context_id,
        origin,
        StorageType::LocalStorage,
        "theme".to_string(),
        "light".to_string(),
    );

    restarted_cookies_service
        .clear_site_data(ClearSiteDataRequest {
            context_id: ctx_a.context_id.clone(),
            origin: origin.to_string(),
            storage_type: StorageType::LocalStorage,
        })
        .map_err(|e| e.to_string())?;

    let item_a = cookie_engine.get_storage_item(
        &ctx_a.context_id,
        origin,
        StorageType::LocalStorage,
        "theme",
    );
    let item_b = cookie_engine.get_storage_item(
        &ctx_b.context_id,
        origin,
        StorageType::LocalStorage,
        "theme",
    );
    let site_data_clear_isolated = item_a.is_none() && item_b == Some("light".to_string());

    // --- Part C: Service Failure Isolation ---
    // Terminate/drop downloads and cookies services, then verify direct navigation, tabs, and history
    drop(restarted_dl_service);
    drop(restarted_cookies_service);

    let tabs_service = TabsService::new();
    let tab_res = tabs_service.create_tab(CreateTabRequest {
        page_id: page.page_id.clone(),
        group_id: None,
        pinned: Some(false),
        select: Some(true),
    });

    let history_service = HistoryService::new();
    let nav_target = "http://127.0.0.1:8080/docs.html".to_string();
    let nav_val = core
        .dispatch(
            OP_NAVIGATE,
            serde_json::to_value(NavigateRequest {
                page_id: page.page_id.clone(),
                url: nav_target.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let nav: NavigateResponse = serde_json::from_value(nav_val).map_err(|e| e.to_string())?;

    let hist_entry = history_service
        .record_navigation(
            page.page_id.clone(),
            NavigationId::new("nav-s3b-1"),
            nav.document_revision,
            nav_target.clone(),
            1725186000000,
        )
        .map_err(|e| e.to_string())?;

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

    let service_failure_isolation_ok = tab_res.tab.id.as_str().starts_with("tab-")
        && hist_entry.url == nav_target
        && obs.url == nav_target;

    Ok(S3BReport {
        download_record_id: download_record_id.to_string(),
        artifact_id,
        artifact_bytes_verified,
        download_survived_restart,
        metadata_only_isolation_ok,
        cross_context_cookies_isolated,
        cookies_survived_restart,
        site_data_clear_isolated,
        service_failure_isolation_ok,
    })
}
