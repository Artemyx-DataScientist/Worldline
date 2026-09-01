//! In-memory reference implementation of BrowserBackend.
//!
//! Provides deterministic headless behavior for CI, automated testing,
//! and protocol acceptance without requiring external binaries or GPU hardware.

use std::collections::BTreeMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};
use worldline_browser_contract::{
    action::{ActionResult, InteractionKind},
    capture::{
        CaptureArtifactRef, CaptureFormat, CapturePageRequest, CapturePageResponse,
        ReadCaptureArtifactRequest, ReadCaptureArtifactResponse,
    },
    contracts::{
        ActRequest, CloseContextRequest, CloseContextResponse, ClosePageRequest, ClosePageResponse,
        ControlDownloadRequest, CreateContextRequest, CreateContextResponse, CreatePageRequest,
        CreatePageResponse, DownloadAction, DownloadState, DownloadStatusResponse,
        HistoryNavRequest, HistoryNavResponse, ListContextsResponse, ListPagesRequest,
        ListPagesResponse, LoadingState, NavigateRequest, NavigateResponse, ObservePageRequest,
        PageObservation, PageSummary, PermissionDecision, PermissionResponse, QueryDocumentRequest,
        QueryPermissionRequest, ReloadRequest, ReloadResponse, SetPermissionRequest,
        StartDownloadRequest, StopRequest, StopResponse, ViewportInfo,
    },
    error::BrowserError,
    identity::{BrowserContextId, DocumentRevision, DownloadId, NavigationId, PageId},
    primitives::{
        ClearStorageRequest, ClearStorageResponse, Cookie, DeleteCookiesRequest,
        DeleteCookiesResponse, GetCookiesRequest, GetCookiesResponse, SetCookieRequest,
        SetCookieResponse,
    },
    query::{
        AccessibilityNode, AccessibilityRole, AccessibilityTree, DocumentMetadata, DocumentSnapshot,
    },
};

use crate::backend::BrowserBackend;

#[derive(Clone, Debug)]
struct PageState {
    context_id: BrowserContextId,
    url: String,
    title: String,
    revision: DocumentRevision,
    loading_state: LoadingState,
    history: Vec<String>,
    history_idx: usize,
    crashed: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ArtifactData {
    artifact_ref: CaptureArtifactRef,
    raw_bytes: Vec<u8>,
}

type DownloadRecord = (PageId, String, String, DownloadState, u64, u64);

/// In-memory reference backend.
#[allow(clippy::type_complexity)]
pub struct ReferenceBrowserBackend {
    contexts: Mutex<BTreeMap<BrowserContextId, (Option<String>, bool)>>,
    pages: Mutex<BTreeMap<PageId, PageState>>,
    cookies: Mutex<BTreeMap<BrowserContextId, Vec<Cookie>>>,
    storage: Mutex<BTreeMap<(BrowserContextId, String), BTreeMap<String, String>>>,
    permissions: Mutex<BTreeMap<(BrowserContextId, String, String), PermissionDecision>>,
    downloads: Mutex<BTreeMap<DownloadId, DownloadRecord>>,
    artifacts: Mutex<BTreeMap<String, ArtifactData>>,
    next_id: Mutex<u64>,
}

impl Default for ReferenceBrowserBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ReferenceBrowserBackend {
    pub fn new() -> Self {
        Self {
            contexts: Mutex::new(BTreeMap::new()),
            pages: Mutex::new(BTreeMap::new()),
            cookies: Mutex::new(BTreeMap::new()),
            storage: Mutex::new(BTreeMap::new()),
            permissions: Mutex::new(BTreeMap::new()),
            downloads: Mutex::new(BTreeMap::new()),
            artifacts: Mutex::new(BTreeMap::new()),
            next_id: Mutex::new(1),
        }
    }

    fn next_id_str(&self, prefix: &str) -> String {
        let mut guard = self.next_id.lock().unwrap();
        let id = *guard;
        *guard += 1;
        format!("{prefix}-{id}")
    }

    pub fn simulate_renderer_crash(&self, page_id: &PageId) -> Result<(), BrowserError> {
        let mut pages = self.pages.lock().unwrap();
        let page = pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        page.crashed = true;
        Ok(())
    }
}

impl BrowserBackend for ReferenceBrowserBackend {
    fn initialize(&mut self) -> Result<(), BrowserError> {
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), BrowserError> {
        self.pages.lock().unwrap().clear();
        self.contexts.lock().unwrap().clear();
        Ok(())
    }

    fn create_context(
        &mut self,
        req: &CreateContextRequest,
    ) -> Result<CreateContextResponse, BrowserError> {
        let id = BrowserContextId::new(self.next_id_str("ctx"));
        self.contexts
            .lock()
            .unwrap()
            .insert(id.clone(), (req.profile_id.clone(), req.incognito));
        Ok(CreateContextResponse {
            context_id: id,
            profile_id: req.profile_id.clone(),
            incognito: req.incognito,
        })
    }

    fn close_context(
        &mut self,
        req: &CloseContextRequest,
    ) -> Result<CloseContextResponse, BrowserError> {
        let mut contexts = self.contexts.lock().unwrap();
        let existed = contexts.remove(&req.context_id).is_some();
        if !existed {
            return Err(BrowserError::ContextNotFound(req.context_id.clone()));
        }
        let mut pages = self.pages.lock().unwrap();
        pages.retain(|_, p| p.context_id != req.context_id);
        self.cookies.lock().unwrap().remove(&req.context_id);
        Ok(CloseContextResponse {
            context_id: req.context_id.clone(),
            closed: true,
        })
    }

    fn list_contexts(&self) -> Result<ListContextsResponse, BrowserError> {
        let contexts = self.contexts.lock().unwrap();
        Ok(ListContextsResponse {
            contexts: contexts.keys().cloned().collect(),
        })
    }

    fn create_page(&mut self, req: &CreatePageRequest) -> Result<CreatePageResponse, BrowserError> {
        let contexts = self.contexts.lock().unwrap();
        if !contexts.contains_key(&req.context_id) {
            return Err(BrowserError::ContextNotFound(req.context_id.clone()));
        }
        let page_id = PageId::new(self.next_id_str("page"));
        let initial_url = req
            .initial_url
            .clone()
            .unwrap_or_else(|| "about:blank".to_string());
        let page_state = PageState {
            context_id: req.context_id.clone(),
            url: initial_url.clone(),
            title: "Reference Page".to_string(),
            revision: DocumentRevision::initial(),
            loading_state: LoadingState::Complete,
            history: vec![initial_url],
            history_idx: 0,
            crashed: false,
        };
        self.pages
            .lock()
            .unwrap()
            .insert(page_id.clone(), page_state);
        Ok(CreatePageResponse {
            context_id: req.context_id.clone(),
            page_id,
            initial_revision: DocumentRevision::initial(),
        })
    }

    fn close_page(&mut self, req: &ClosePageRequest) -> Result<ClosePageResponse, BrowserError> {
        let mut pages = self.pages.lock().unwrap();
        let existed = pages.remove(&req.page_id).is_some();
        if !existed {
            return Err(BrowserError::PageNotFound(req.page_id.clone()));
        }
        Ok(ClosePageResponse {
            page_id: req.page_id.clone(),
            closed: true,
        })
    }

    fn list_pages(&self, req: &ListPagesRequest) -> Result<ListPagesResponse, BrowserError> {
        let pages = self.pages.lock().unwrap();
        let summaries = pages
            .iter()
            .filter(|(_, p)| p.context_id == req.context_id)
            .map(|(id, p)| PageSummary {
                page_id: id.clone(),
                url: p.url.clone(),
                title: p.title.clone(),
                document_revision: p.revision,
            })
            .collect();
        Ok(ListPagesResponse { pages: summaries })
    }

    fn navigate(&mut self, req: &NavigateRequest) -> Result<NavigateResponse, BrowserError> {
        let mut pages = self.pages.lock().unwrap();
        let page = pages
            .get_mut(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        page.crashed = false; // navigation recovers crashed page
        page.url = req.url.clone();
        page.title = format!("Page: {}", req.url);
        page.revision = page.revision.next();
        page.history.truncate(page.history_idx + 1);
        page.history.push(req.url.clone());
        page.history_idx = page.history.len() - 1;
        page.loading_state = LoadingState::Complete;

        Ok(NavigateResponse {
            page_id: req.page_id.clone(),
            navigation_id: NavigationId::new(self.next_id_str("nav")),
            committed: true,
            document_revision: page.revision,
        })
    }

    fn reload(&mut self, req: &ReloadRequest) -> Result<ReloadResponse, BrowserError> {
        let mut pages = self.pages.lock().unwrap();
        let page = pages
            .get_mut(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        page.crashed = false;
        page.revision = page.revision.next();
        Ok(ReloadResponse {
            page_id: req.page_id.clone(),
            reloaded: true,
            document_revision: page.revision,
        })
    }

    fn stop(&mut self, req: &StopRequest) -> Result<StopResponse, BrowserError> {
        let pages = self.pages.lock().unwrap();
        let _page = pages
            .get(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        Ok(StopResponse {
            page_id: req.page_id.clone(),
            stopped: true,
        })
    }

    fn history_nav(&mut self, req: &HistoryNavRequest) -> Result<HistoryNavResponse, BrowserError> {
        let mut pages = self.pages.lock().unwrap();
        let page = pages
            .get_mut(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        if req.delta > 0 {
            if page.history_idx + 1 < page.history.len() {
                page.history_idx += 1;
                page.url = page.history[page.history_idx].clone();
                page.revision = page.revision.next();
            }
        } else if req.delta < 0 && page.history_idx > 0 {
            page.history_idx -= 1;
            page.url = page.history[page.history_idx].clone();
            page.revision = page.revision.next();
        }
        Ok(HistoryNavResponse {
            page_id: req.page_id.clone(),
            success: true,
            document_revision: page.revision,
        })
    }

    fn observe(&self, req: &ObservePageRequest) -> Result<PageObservation, BrowserError> {
        let pages = self.pages.lock().unwrap();
        let page = pages
            .get(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        if page.crashed {
            return Err(BrowserError::NavigationFailed(
                "Renderer process crashed".to_string(),
            ));
        }
        Ok(PageObservation {
            page_id: req.page_id.clone(),
            url: page.url.clone(),
            title: page.title.clone(),
            loading_state: page.loading_state,
            document_revision: page.revision,
            status_code: 200,
            is_secure: page.url.starts_with("https://"),
            viewport: Some(ViewportInfo {
                width: 1280,
                height: 720,
                device_scale_factor: 1,
            }),
        })
    }

    fn query(&self, req: &QueryDocumentRequest) -> Result<DocumentSnapshot, BrowserError> {
        let pages = self.pages.lock().unwrap();
        let page = pages
            .get(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        if page.crashed {
            return Err(BrowserError::NavigationFailed(
                "Renderer process crashed".to_string(),
            ));
        }

        let root_node = AccessibilityNode::new("root", AccessibilityRole::Root)
            .with_name(&page.title)
            .with_child(
                AccessibilityNode::new("btn-submit", AccessibilityRole::Button)
                    .with_name("Submit")
                    .with_value(""),
            )
            .with_child(
                AccessibilityNode::new("input-query", AccessibilityRole::TextInput)
                    .with_name("Search Query")
                    .with_value("initial value"),
            );

        let tree = AccessibilityTree::new(req.page_id.clone(), page.revision, root_node);
        let tree = match req.bounds.as_ref() {
            Some(bounds) => tree.to_bounded(bounds),
            None => tree,
        };

        let meta = DocumentMetadata {
            page_id: req.page_id.clone(),
            url: page.url.clone(),
            title: page.title.clone(),
            document_revision: page.revision,
            status_code: 200,
        };

        Ok(DocumentSnapshot::new(meta, tree))
    }

    fn act(&mut self, req: &ActRequest) -> Result<ActionResult, BrowserError> {
        let mut pages = self.pages.lock().unwrap();
        let (page_id, kind, message) = match req {
            ActRequest::Click(c) => (
                c.element_ref.page_id().clone(),
                InteractionKind::Click,
                Some("clicked".to_string()),
            ),
            ActRequest::Input(i) => (
                i.element_ref.page_id().clone(),
                InteractionKind::Input,
                Some(i.text.clone()),
            ),
            ActRequest::Focus(f) => (
                f.element_ref.page_id().clone(),
                InteractionKind::Focus,
                None,
            ),
            ActRequest::Submit(s) => (
                s.element_ref.page_id().clone(),
                InteractionKind::Submit,
                None,
            ),
            ActRequest::Scroll(s) => (s.page_id.clone(), InteractionKind::Scroll, None),
        };

        let page = pages
            .get_mut(&page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        if page.crashed {
            return Err(BrowserError::NavigationFailed(
                "Renderer process crashed".to_string(),
            ));
        }

        match kind {
            InteractionKind::Click => {
                page.title = format!("{} (clicked)", page.title);
                page.revision = page.revision.next();
                Ok(ActionResult {
                    page_id,
                    document_revision: page.revision,
                    interaction: InteractionKind::Click,
                    success: true,
                    message,
                })
            }
            InteractionKind::Input => {
                page.revision = page.revision.next();
                Ok(ActionResult {
                    page_id,
                    document_revision: page.revision,
                    interaction: InteractionKind::Input,
                    success: true,
                    message,
                })
            }
            InteractionKind::Focus | InteractionKind::Scroll | InteractionKind::Submit => {
                page.revision = page.revision.next();
                Ok(ActionResult {
                    page_id,
                    document_revision: page.revision,
                    interaction: kind,
                    success: true,
                    message: None,
                })
            }
        }
    }

    fn start_download(
        &mut self,
        req: &StartDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        let download_id = DownloadId::new(self.next_id_str("dl"));
        let dest = req
            .destination_path
            .clone()
            .unwrap_or_else(|| "download.bin".to_string());
        self.downloads.lock().unwrap().insert(
            download_id.clone(),
            (
                req.page_id.clone(),
                req.url.clone(),
                dest.clone(),
                DownloadState::InProgress,
                1024,
                1024,
            ),
        );
        Ok(DownloadStatusResponse {
            download_id,
            page_id: req.page_id.clone(),
            url: req.url.clone(),
            destination_path: dest,
            state: DownloadState::InProgress,
            received_bytes: 1024,
            total_bytes: 1024,
        })
    }

    fn control_download(
        &mut self,
        req: &ControlDownloadRequest,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        let mut dls = self.downloads.lock().unwrap();
        let dl = dls
            .get_mut(&req.download_id)
            .ok_or_else(|| BrowserError::DownloadNotFound(req.download_id.clone()))?;
        match req.action {
            DownloadAction::Cancel => dl.3 = DownloadState::Cancelled,
            DownloadAction::Pause => dl.3 = DownloadState::Paused,
            DownloadAction::Resume => dl.3 = DownloadState::InProgress,
        }
        Ok(DownloadStatusResponse {
            download_id: req.download_id.clone(),
            page_id: dl.0.clone(),
            url: dl.1.clone(),
            destination_path: dl.2.clone(),
            state: dl.3,
            received_bytes: dl.4,
            total_bytes: dl.5,
        })
    }

    fn query_permission(
        &self,
        req: &QueryPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError> {
        let perms = self.permissions.lock().unwrap();
        let key = (
            req.context_id.clone(),
            req.origin.clone(),
            format!("{:?}", req.permission_type),
        );
        let decision = perms
            .get(&key)
            .copied()
            .unwrap_or(PermissionDecision::Prompt);
        Ok(PermissionResponse {
            context_id: req.context_id.clone(),
            permission_type: req.permission_type,
            origin: req.origin.clone(),
            decision,
        })
    }

    fn set_permission(
        &mut self,
        req: &SetPermissionRequest,
    ) -> Result<PermissionResponse, BrowserError> {
        let mut perms = self.permissions.lock().unwrap();
        let key = (
            req.context_id.clone(),
            req.origin.clone(),
            format!("{:?}", req.permission_type),
        );
        perms.insert(key, req.decision);
        Ok(PermissionResponse {
            context_id: req.context_id.clone(),
            permission_type: req.permission_type,
            origin: req.origin.clone(),
            decision: req.decision,
        })
    }

    fn capture(&mut self, req: &CapturePageRequest) -> Result<CapturePageResponse, BrowserError> {
        let pages = self.pages.lock().unwrap();
        let page = pages
            .get(&req.page_id)
            .ok_or_else(|| BrowserError::PageNotFound(req.page_id.clone()))?;
        if page.crashed {
            return Err(BrowserError::NavigationFailed(
                "Renderer process crashed".to_string(),
            ));
        }

        // Generate synthetic image bytes
        let (raw_bytes, mime_type) = match req.format {
            CaptureFormat::Png => (
                vec![
                    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01, 0x02, 0x03,
                ],
                "image/png".to_string(),
            ),
            CaptureFormat::Jpeg => (
                vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
                "image/jpeg".to_string(),
            ),
            CaptureFormat::Webp => (
                vec![0x52, 0x49, 0x46, 0x46, 0x00, 0x00],
                "image/webp".to_string(),
            ),
        };

        let mut hasher = Sha256::new();
        hasher.update(&raw_bytes);
        let blob_id = format!("sha256:{:x}", hasher.finalize());
        let artifact_id = self.next_id_str("artifact");

        let artifact_ref = CaptureArtifactRef {
            artifact_id: artifact_id.clone(),
            page_id: req.page_id.clone(),
            revision: page.revision,
            byte_len: raw_bytes.len(),
            mime_type,
            blob_id,
        };

        self.artifacts.lock().unwrap().insert(
            artifact_id,
            ArtifactData {
                artifact_ref: artifact_ref.clone(),
                raw_bytes,
            },
        );

        Ok(CapturePageResponse {
            artifact: artifact_ref,
        })
    }

    fn read_capture(
        &self,
        req: &ReadCaptureArtifactRequest,
    ) -> Result<ReadCaptureArtifactResponse, BrowserError> {
        let artifacts = self.artifacts.lock().unwrap();
        let artifact =
            artifacts
                .get(&req.artifact_id)
                .ok_or_else(|| BrowserError::ResourceMismatch {
                    expected: "artifact".to_string(),
                    actual: req.artifact_id.clone(),
                })?;

        let total_bytes = artifact.raw_bytes.len();
        let offset = req.offset as usize;
        if offset >= total_bytes {
            return Ok(ReadCaptureArtifactResponse {
                artifact_id: req.artifact_id.clone(),
                data: Vec::new(),
                is_truncated: false,
                total_bytes,
            });
        }

        let end = (offset + req.max_bytes).min(total_bytes);
        let chunk = artifact.raw_bytes[offset..end].to_vec();
        let is_truncated = end < total_bytes;

        Ok(ReadCaptureArtifactResponse {
            artifact_id: req.artifact_id.clone(),
            data: chunk,
            is_truncated,
            total_bytes,
        })
    }

    fn get_cookies(&self, req: &GetCookiesRequest) -> Result<GetCookiesResponse, BrowserError> {
        let cookies_map = self.cookies.lock().unwrap();
        let cookies = cookies_map
            .get(&req.context_id)
            .cloned()
            .unwrap_or_default();
        Ok(GetCookiesResponse { cookies })
    }

    fn set_cookie(&mut self, req: &SetCookieRequest) -> Result<SetCookieResponse, BrowserError> {
        let mut cookies_map = self.cookies.lock().unwrap();
        let list = cookies_map.entry(req.context_id.clone()).or_default();
        list.retain(|c| c.name != req.cookie.name || c.domain != req.cookie.domain);
        list.push(req.cookie.clone());
        Ok(SetCookieResponse { success: true })
    }

    #[allow(clippy::collapsible_if)]
    fn delete_cookies(
        &mut self,
        req: &DeleteCookiesRequest,
    ) -> Result<DeleteCookiesResponse, BrowserError> {
        let mut cookies_map = self.cookies.lock().unwrap();
        let mut count = 0;
        if let Some(list) = cookies_map.get_mut(&req.context_id) {
            let before = list.len();
            list.retain(|c| {
                if let Some(ref name) = req.name {
                    if &c.name == name {
                        return false;
                    }
                }
                if let Some(ref domain) = req.domain {
                    if &c.domain == domain {
                        return false;
                    }
                }
                true
            });
            count = before - list.len();
        }
        Ok(DeleteCookiesResponse {
            deleted_count: count as u32,
        })
    }

    fn clear_storage(
        &mut self,
        req: &ClearStorageRequest,
    ) -> Result<ClearStorageResponse, BrowserError> {
        let mut storage = self.storage.lock().unwrap();
        let key = (req.context_id.clone(), req.origin.clone());
        storage.remove(&key);
        Ok(ClearStorageResponse { cleared: true })
    }
}
