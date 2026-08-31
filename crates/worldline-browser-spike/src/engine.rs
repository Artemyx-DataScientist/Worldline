use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use worldline_browser_contract::{
    action::{ActionResult, InteractionKind},
    contracts::{
        DownloadAction, DownloadState, DownloadStatusResponse, ElementQueryKind, LoadingState,
        PageObservation, PageSummary, PermissionDecision, PermissionType, ViewportInfo,
    },
    error::BrowserError,
    identity::{BrowserContextId, DocumentRevision, DownloadId, ElementRef, NavigationId, PageId},
    query::{
        AccessibilityNode, AccessibilityRole, AccessibilityTree, DocumentMetadata,
        DocumentSnapshot, QueryBounds, SemanticElement,
    },
};

/// Internal state of an isolated browser context/profile in the reference model.
#[derive(Clone, Debug)]
pub struct ContextState {
    pub id: BrowserContextId,
    pub profile_id: Option<String>,
    pub incognito: bool,
    pub cookies: HashMap<String, String>,
    pub permissions: HashMap<(String, PermissionType), PermissionDecision>,
    pub pages: Vec<PageId>,
}

/// Internal representation of an interactive page form field.
#[derive(Clone, Debug)]
pub struct FormElementState {
    pub key: String,
    pub tag_name: String,
    pub role: AccessibilityRole,
    pub name: String,
    pub value: String,
}

/// Internal state of a browser page surface in the reference model.
#[derive(Clone, Debug)]
pub struct PageState {
    pub page_id: PageId,
    pub context_id: BrowserContextId,
    pub current_url: String,
    pub title: String,
    pub loading_state: LoadingState,
    pub document_revision: DocumentRevision,
    pub status_code: u16,
    pub is_crashed: bool,
    pub form_elements: BTreeMap<String, FormElementState>,
    pub status_text: String,
}

impl PageState {
    pub fn new(page_id: PageId, context_id: BrowserContextId) -> Self {
        Self {
            page_id,
            context_id,
            current_url: "about:blank".to_string(),
            title: "Blank Page".to_string(),
            loading_state: LoadingState::Complete,
            document_revision: DocumentRevision::initial(),
            status_code: 200,
            is_crashed: false,
            form_elements: BTreeMap::new(),
            status_text: "Initialized".to_string(),
        }
    }

    /// Loads the deterministic local test page fixture.
    pub fn load_local_fixture(&mut self, url: &str) {
        self.current_url = url.to_string();
        self.loading_state = LoadingState::Complete;
        self.document_revision = self.document_revision.next();
        self.status_code = 200;
        self.is_crashed = false;

        if url.contains("test-form") || url.contains("local") {
            self.title = "Worldline Local Test Form".to_string();
            self.status_text = "Ready".to_string();

            let mut elements = BTreeMap::new();
            elements.insert(
                "query-input".to_string(),
                FormElementState {
                    key: "query-input".to_string(),
                    tag_name: "input".to_string(),
                    role: AccessibilityRole::TextInput,
                    name: "Search Query".to_string(),
                    value: "initial text".to_string(),
                },
            );
            elements.insert(
                "submit-btn".to_string(),
                FormElementState {
                    key: "submit-btn".to_string(),
                    tag_name: "button".to_string(),
                    role: AccessibilityRole::Button,
                    name: "Submit Query".to_string(),
                    value: "".to_string(),
                },
            );
            self.form_elements = elements;
        } else {
            self.title = format!("Page: {url}");
            self.status_text = "Loaded".to_string();
            self.form_elements.clear();
        }
    }

    pub fn build_accessibility_tree(&self) -> AccessibilityTree {
        let mut root =
            AccessibilityNode::new("root-1", AccessibilityRole::Root).with_name(self.title.clone());

        let mut form_group = AccessibilityNode::new("form-group-1", AccessibilityRole::Form)
            .with_name("Interactive Form");

        for elem in self.form_elements.values() {
            let elem_ref = ElementRef::new(
                self.page_id.clone(),
                self.document_revision,
                elem.key.clone(),
            );
            let child = AccessibilityNode::new(elem.key.clone(), elem.role)
                .with_name(elem.name.clone())
                .with_value(elem.value.clone())
                .with_element_ref(elem_ref);
            form_group = form_group.with_child(child);
        }

        let status_node = AccessibilityNode::new("status-1", AccessibilityRole::StaticText)
            .with_name(format!("Status: {}", self.status_text));
        form_group = form_group.with_child(status_node);

        root = root.with_child(form_group);

        AccessibilityTree::new(self.page_id.clone(), self.document_revision, root)
    }
}

/// In-memory reference browser supervisor managing isolated contexts, pages, downloads and permissions.
#[derive(Clone, Debug, Default)]
pub struct ReferenceBrowserSupervisor {
    inner: Arc<Mutex<SupervisorInner>>,
}

#[derive(Debug, Default)]
struct SupervisorInner {
    contexts: HashMap<BrowserContextId, ContextState>,
    pages: HashMap<PageId, PageState>,
    downloads: HashMap<DownloadId, DownloadStatusResponse>,
    next_context_id: u64,
    next_page_id: u64,
    next_nav_id: u64,
    next_download_id: u64,
}

impl ReferenceBrowserSupervisor {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SupervisorInner::default())),
        }
    }

    pub fn create_context(
        &self,
        requested_id: Option<BrowserContextId>,
        profile_id: Option<String>,
        incognito: bool,
    ) -> Result<BrowserContextId, BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let context_id = match requested_id {
            Some(id) => id,
            None => {
                inner.next_context_id += 1;
                BrowserContextId::new(format!("ctx-spike-{}", inner.next_context_id))
            }
        };

        inner.contexts.insert(
            context_id.clone(),
            ContextState {
                id: context_id.clone(),
                profile_id,
                incognito,
                cookies: HashMap::new(),
                permissions: HashMap::new(),
                pages: Vec::new(),
            },
        );
        Ok(context_id)
    }

    pub fn close_context(&self, context_id: &BrowserContextId) -> Result<(), BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let ctx = inner
            .contexts
            .remove(context_id)
            .ok_or_else(|| BrowserError::ContextNotFound(context_id.clone()))?;
        for page_id in ctx.pages {
            inner.pages.remove(&page_id);
        }
        Ok(())
    }

    pub fn list_contexts(&self) -> Vec<BrowserContextId> {
        if let Ok(inner) = self.inner.lock() {
            inner.contexts.keys().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn set_cookie(
        &self,
        context_id: &BrowserContextId,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let ctx = inner
            .contexts
            .get_mut(context_id)
            .ok_or_else(|| BrowserError::ContextNotFound(context_id.clone()))?;
        ctx.cookies.insert(key.into(), value.into());
        Ok(())
    }

    pub fn get_cookie(
        &self,
        context_id: &BrowserContextId,
        key: &str,
    ) -> Result<Option<String>, BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let ctx = inner
            .contexts
            .get(context_id)
            .ok_or_else(|| BrowserError::ContextNotFound(context_id.clone()))?;
        Ok(ctx.cookies.get(key).cloned())
    }

    pub fn create_page(
        &self,
        context_id: &BrowserContextId,
        requested_page_id: Option<PageId>,
        initial_url: Option<String>,
    ) -> Result<(PageId, DocumentRevision), BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        if !inner.contexts.contains_key(context_id) {
            return Err(BrowserError::ContextNotFound(context_id.clone()));
        }

        let page_id = match requested_page_id {
            Some(id) => id,
            None => {
                inner.next_page_id += 1;
                PageId::new(format!("page-spike-{}", inner.next_page_id))
            }
        };

        if let Some(ctx) = inner.contexts.get_mut(context_id) {
            ctx.pages.push(page_id.clone());
        }

        let mut page = PageState::new(page_id.clone(), context_id.clone());
        if let Some(url) = initial_url {
            page.load_local_fixture(&url);
        }
        let rev = page.document_revision;
        inner.pages.insert(page_id.clone(), page);
        Ok((page_id, rev))
    }

    pub fn close_page(&self, page_id: &PageId) -> Result<(), BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .remove(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        if let Some(ctx) = inner.contexts.get_mut(&page.context_id) {
            ctx.pages.retain(|p| p != page_id);
        }
        Ok(())
    }

    pub fn list_pages(
        &self,
        context_id: &BrowserContextId,
    ) -> Result<Vec<PageSummary>, BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let ctx = inner
            .contexts
            .get(context_id)
            .ok_or_else(|| BrowserError::ContextNotFound(context_id.clone()))?;
        let mut summaries = Vec::new();
        for page_id in &ctx.pages {
            if let Some(p) = inner.pages.get(page_id) {
                summaries.push(PageSummary {
                    page_id: p.page_id.clone(),
                    url: p.current_url.clone(),
                    title: p.title.clone(),
                    document_revision: p.document_revision,
                });
            }
        }
        Ok(summaries)
    }

    pub fn navigate(
        &self,
        page_id: &PageId,
        url: &str,
    ) -> Result<(NavigationId, DocumentRevision), BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        inner.next_nav_id += 1;
        let nav_id = NavigationId::new(format!("nav-{}", inner.next_nav_id));

        let page = inner
            .pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }

        page.load_local_fixture(url);
        Ok((nav_id, page.document_revision))
    }

    pub fn reload(&self, page_id: &PageId) -> Result<DocumentRevision, BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }
        page.document_revision = page.document_revision.next();
        Ok(page.document_revision)
    }

    pub fn stop(&self, page_id: &PageId) -> Result<(), BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }
        Ok(())
    }

    pub fn history_nav(
        &self,
        page_id: &PageId,
        _delta: i32,
    ) -> Result<DocumentRevision, BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }
        page.document_revision = page.document_revision.next();
        Ok(page.document_revision)
    }

    pub fn observe(&self, page_id: &PageId) -> Result<PageObservation, BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }

        Ok(PageObservation {
            page_id: page.page_id.clone(),
            url: page.current_url.clone(),
            title: page.title.clone(),
            loading_state: page.loading_state,
            document_revision: page.document_revision,
            status_code: page.status_code,
            is_secure: page.current_url.starts_with("https"),
            viewport: Some(ViewportInfo {
                width: 1280,
                height: 800,
                device_scale_factor: 1,
            }),
        })
    }

    pub fn query_document(
        &self,
        page_id: &PageId,
        bounds: Option<&QueryBounds>,
    ) -> Result<DocumentSnapshot, BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }

        let raw_tree = page.build_accessibility_tree();
        let default_bounds = QueryBounds::default();
        let active_bounds = bounds.unwrap_or(&default_bounds);
        let bounded_tree = raw_tree.to_bounded(active_bounds);

        Ok(DocumentSnapshot::new(
            DocumentMetadata {
                page_id: page.page_id.clone(),
                url: page.current_url.clone(),
                title: page.title.clone(),
                document_revision: page.document_revision,
                status_code: page.status_code,
            },
            bounded_tree,
        ))
    }

    pub fn extract_text(
        &self,
        page_id: &PageId,
        target_element: Option<&ElementRef>,
    ) -> Result<(String, DocumentRevision), BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }

        if let Some(elem_ref) = target_element {
            if elem_ref.document_revision() != page.document_revision {
                return Err(BrowserError::StaleElementReference {
                    expected_revision: elem_ref.document_revision(),
                    actual_revision: page.document_revision,
                });
            }
            let elem = page
                .form_elements
                .get(elem_ref.node_key())
                .ok_or_else(|| BrowserError::ElementNotFound(elem_ref.node_key().to_string()))?;
            Ok((elem.value.clone(), page.document_revision))
        } else {
            let tree = page.build_accessibility_tree();
            Ok((tree.root.collect_text(), page.document_revision))
        }
    }

    pub fn find_elements(
        &self,
        page_id: &PageId,
        query: &str,
        _kind: ElementQueryKind,
    ) -> Result<(Vec<SemanticElement>, DocumentRevision), BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }

        let mut elements = Vec::new();
        for (key, f) in &page.form_elements {
            if key.contains(query) || f.name.contains(query) || f.value.contains(query) {
                let elem_ref =
                    ElementRef::new(page.page_id.clone(), page.document_revision, key.clone());
                elements.push(SemanticElement {
                    element_ref: elem_ref,
                    tag_name: f.tag_name.clone(),
                    attributes: BTreeMap::new(),
                    text_content: f.value.clone(),
                });
            }
        }
        Ok((elements, page.document_revision))
    }

    pub fn execute_action(
        &self,
        element_ref: &ElementRef,
        kind: InteractionKind,
        text_payload: Option<&str>,
    ) -> Result<ActionResult, BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get_mut(element_ref.page_id())
            .ok_or_else(|| BrowserError::PageNotFound(element_ref.page_id().clone()))?;

        if page.is_crashed {
            return Err(BrowserError::EngineCrashed(
                "page renderer process has terminated".to_string(),
            ));
        }

        if element_ref.document_revision() != page.document_revision {
            return Err(BrowserError::StaleElementReference {
                expected_revision: element_ref.document_revision(),
                actual_revision: page.document_revision,
            });
        }

        let elem = page
            .form_elements
            .get_mut(element_ref.node_key())
            .ok_or_else(|| BrowserError::ElementNotFound(element_ref.node_key().to_string()))?;

        match kind {
            InteractionKind::Input => {
                if let Some(text) = text_payload {
                    elem.value = text.to_string();
                }
                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some(format!("input set to '{}'", elem.value)),
                })
            }
            InteractionKind::Click | InteractionKind::Submit => {
                let query_val = page
                    .form_elements
                    .get("query-input")
                    .map(|f| f.value.clone())
                    .unwrap_or_default();
                page.title = format!("Results for {query_val}");
                page.status_text = format!("Submitted: {query_val}");
                page.document_revision = page.document_revision.next();

                Ok(ActionResult {
                    page_id: page.page_id.clone(),
                    document_revision: page.document_revision,
                    interaction: kind,
                    success: true,
                    message: Some(format!("form submitted with '{query_val}'")),
                })
            }
            InteractionKind::Focus => Ok(ActionResult {
                page_id: page.page_id.clone(),
                document_revision: page.document_revision,
                interaction: kind,
                success: true,
                message: Some("focused".to_string()),
            }),
            InteractionKind::Scroll => Ok(ActionResult {
                page_id: page.page_id.clone(),
                document_revision: page.document_revision,
                interaction: kind,
                success: true,
                message: Some("scrolled".to_string()),
            }),
        }
    }

    pub fn start_download(
        &self,
        page_id: &PageId,
        url: &str,
        destination_path: Option<String>,
    ) -> Result<DownloadId, BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        if !inner.pages.contains_key(page_id) {
            return Err(BrowserError::PageNotFound(page_id.clone()));
        }
        inner.next_download_id += 1;
        let dl_id = DownloadId::new(format!("dl-{}", inner.next_download_id));
        let dest = destination_path.unwrap_or_else(|| format!("/downloads/{dl_id}"));
        inner.downloads.insert(
            dl_id.clone(),
            DownloadStatusResponse {
                download_id: dl_id.clone(),
                page_id: page_id.clone(),
                url: url.to_string(),
                destination_path: dest,
                total_bytes: 1024,
                received_bytes: 512,
                state: DownloadState::InProgress,
            },
        );
        Ok(dl_id)
    }

    pub fn control_download(
        &self,
        download_id: &DownloadId,
        action: DownloadAction,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let dl = inner
            .downloads
            .get_mut(download_id)
            .ok_or_else(|| BrowserError::DownloadNotFound(download_id.clone()))?;
        match action {
            DownloadAction::Pause => dl.state = DownloadState::Paused,
            DownloadAction::Resume => dl.state = DownloadState::InProgress,
            DownloadAction::Cancel => dl.state = DownloadState::Cancelled,
        }
        Ok(dl.clone())
    }

    pub fn download_status(
        &self,
        download_id: &DownloadId,
    ) -> Result<DownloadStatusResponse, BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        inner
            .downloads
            .get(download_id)
            .cloned()
            .ok_or_else(|| BrowserError::DownloadNotFound(download_id.clone()))
    }

    pub fn query_permission(
        &self,
        context_id: &BrowserContextId,
        origin: &str,
        perm_type: PermissionType,
    ) -> Result<PermissionDecision, BrowserError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let ctx = inner
            .contexts
            .get(context_id)
            .ok_or_else(|| BrowserError::ContextNotFound(context_id.clone()))?;
        Ok(ctx
            .permissions
            .get(&(origin.to_string(), perm_type))
            .copied()
            .unwrap_or(PermissionDecision::Prompt))
    }

    pub fn set_permission(
        &self,
        context_id: &BrowserContextId,
        origin: &str,
        perm_type: PermissionType,
        decision: PermissionDecision,
    ) -> Result<PermissionDecision, BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let ctx = inner
            .contexts
            .get_mut(context_id)
            .ok_or_else(|| BrowserError::ContextNotFound(context_id.clone()))?;
        ctx.permissions
            .insert((origin.to_string(), perm_type), decision);
        Ok(decision)
    }

    /// Simulates deliberate crash / termination of an engine process.
    pub fn crash_page_process(&self, page_id: &PageId) -> Result<(), BrowserError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| BrowserError::EngineHung("lock poisoned".to_string()))?;
        let page = inner
            .pages
            .get_mut(page_id)
            .ok_or_else(|| BrowserError::PageNotFound(page_id.clone()))?;
        page.is_crashed = true;
        page.loading_state = LoadingState::Failed;
        Ok(())
    }
}
