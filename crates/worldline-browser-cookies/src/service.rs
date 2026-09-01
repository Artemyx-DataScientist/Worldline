use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use worldline_browser_contract::identity::BrowserContextId;
use worldline_browser_contract::primitives::{
    ClearStorageRequest, ClearStorageResponse, Cookie, DeleteCookiesRequest, DeleteCookiesResponse,
    GetCookiesRequest, GetCookiesResponse, SetCookieRequest, SetCookieResponse, StorageType,
};
use worldline_browser_services_contract::{
    ClearSiteDataRequest, ClearSiteDataResponse, CookieMetadata, CookieValue,
    DeleteCookieServiceRequest, DeleteCookieServiceResponse, GetCookieMetadataRequest,
    GetCookieMetadataResponse, GetCookieValueRequest, GetCookieValueResponse,
    SetCookieServiceRequest, SetCookieServiceResponse,
};

use crate::policy::CookiePolicySnapshot;
use crate::site_data::validate_and_build_clear_storage;

/// Interface for low-level engine cookie and storage primitives.
pub trait CookieEngineBackend: Send + Sync {
    fn get_cookies(&self, req: GetCookiesRequest) -> Result<GetCookiesResponse, String>;
    fn set_cookie(&self, req: SetCookieRequest) -> Result<SetCookieResponse, String>;
    fn delete_cookies(&self, req: DeleteCookiesRequest) -> Result<DeleteCookiesResponse, String>;
    fn clear_storage(&self, req: ClearStorageRequest) -> Result<ClearStorageResponse, String>;
}

/// In-memory implementation of engine cookie and storage primitives for testing and validation.
#[derive(Default)]
pub struct InMemoryCookieEngine {
    /// Cookies strictly partitioned by context ID.
    cookies: Mutex<BTreeMap<BrowserContextId, Vec<Cookie>>>,
    /// Storage records strictly partitioned by (context ID, origin, storage type).
    storage: Mutex<BTreeMap<(BrowserContextId, String, StorageType), BTreeMap<String, String>>>,
}

impl InMemoryCookieEngine {
    pub fn new() -> Self {
        Self {
            cookies: Mutex::new(BTreeMap::new()),
            storage: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn insert_storage_item(
        &self,
        context_id: &BrowserContextId,
        origin: &str,
        storage_type: StorageType,
        key: String,
        value: String,
    ) {
        let mut storage = self.storage.lock().unwrap();
        let canonical_origin = origin.trim_end_matches('/').to_string();
        storage
            .entry((context_id.clone(), canonical_origin, storage_type))
            .or_default()
            .insert(key, value);
    }

    pub fn get_storage_item(
        &self,
        context_id: &BrowserContextId,
        origin: &str,
        storage_type: StorageType,
        key: &str,
    ) -> Option<String> {
        let storage = self.storage.lock().unwrap();
        let canonical_origin = origin.trim_end_matches('/').to_string();
        storage
            .get(&(context_id.clone(), canonical_origin, storage_type))
            .and_then(|map| map.get(key).cloned())
    }
}

impl CookieEngineBackend for InMemoryCookieEngine {
    fn get_cookies(&self, req: GetCookiesRequest) -> Result<GetCookiesResponse, String> {
        let cookies_guard = self.cookies.lock().unwrap();
        let context_cookies = cookies_guard
            .get(&req.context_id)
            .cloned()
            .unwrap_or_default();

        let filtered = context_cookies
            .into_iter()
            .filter(|c| {
                if let Some(domain) = &req.domain {
                    if !c.domain.ends_with(domain) && domain != &c.domain {
                        return false;
                    }
                }
                true
            })
            .collect();

        Ok(GetCookiesResponse { cookies: filtered })
    }

    fn set_cookie(&self, req: SetCookieRequest) -> Result<SetCookieResponse, String> {
        let mut cookies_guard = self.cookies.lock().unwrap();
        let context_cookies = cookies_guard.entry(req.context_id).or_default();

        // Overwrite existing cookie with same name, domain, path
        context_cookies.retain(|c| {
            !(c.name == req.cookie.name
                && c.domain == req.cookie.domain
                && c.path == req.cookie.path)
        });
        context_cookies.push(req.cookie);

        Ok(SetCookieResponse { success: true })
    }

    fn delete_cookies(&self, req: DeleteCookiesRequest) -> Result<DeleteCookiesResponse, String> {
        let mut cookies_guard = self.cookies.lock().unwrap();
        let context_cookies = cookies_guard.entry(req.context_id).or_default();

        let before = context_cookies.len();
        context_cookies.retain(|c| {
            if let Some(name) = &req.name {
                if &c.name != name {
                    return true;
                }
            }
            if let Some(domain) = &req.domain {
                if &c.domain != domain && !c.domain.ends_with(domain) {
                    return true;
                }
            }
            false
        });
        let deleted_count = (before - context_cookies.len()) as u32;

        Ok(DeleteCookiesResponse { deleted_count })
    }

    fn clear_storage(&self, req: ClearStorageRequest) -> Result<ClearStorageResponse, String> {
        let mut storage = self.storage.lock().unwrap();
        let canonical_origin = req.origin.trim_end_matches('/').to_string();

        let keys_to_remove: Vec<_> = storage
            .keys()
            .filter(|(cid, origin, st)| {
                cid == &req.context_id
                    && origin == &canonical_origin
                    && (req.storage_type == StorageType::All || st == &req.storage_type)
            })
            .cloned()
            .collect();

        let had_items = !keys_to_remove.is_empty();
        for key in keys_to_remove {
            storage.remove(&key);
        }

        Ok(ClearStorageResponse {
            cleared: had_items || true,
        })
    }
}

/// Cookies and site-data service providing metadata inspection, secret redaction,
/// and scope isolation above engine primitives.
pub struct CookiesService {
    backend: Arc<dyn CookieEngineBackend>,
    policy: Mutex<CookiePolicySnapshot>,
}

impl CookiesService {
    pub fn new(backend: Arc<dyn CookieEngineBackend>) -> Self {
        Self {
            backend,
            policy: Mutex::new(CookiePolicySnapshot::new()),
        }
    }

    pub fn from_policy(
        policy: CookiePolicySnapshot,
        backend: Arc<dyn CookieEngineBackend>,
    ) -> Self {
        Self {
            backend,
            policy: Mutex::new(policy),
        }
    }

    pub fn export_policy(&self) -> CookiePolicySnapshot {
        self.policy.lock().unwrap().clone()
    }

    pub fn set_metadata_only(&self, context_id: BrowserContextId, metadata_only: bool) {
        let mut policy = self.policy.lock().unwrap();
        policy.set_metadata_only(context_id, metadata_only);
    }

    // --- Capability RPC Handlers ---

    /// Inspects cookie metadata without disclosing raw cookie values.
    pub fn get_cookie_metadata(
        &self,
        req: GetCookieMetadataRequest,
    ) -> Result<GetCookieMetadataResponse, String> {
        let engine_res = self.backend.get_cookies(GetCookiesRequest {
            context_id: req.context_id,
            url: req.url,
            domain: req.domain,
        })?;

        let metadata_list = engine_res
            .cookies
            .into_iter()
            .map(|c| CookieMetadata {
                name: c.name,
                domain: c.domain,
                path: c.path,
                secure: c.secure,
                http_only: c.http_only,
                same_site: c.same_site,
                expires_epoch_sec: c.expires_epoch_sec,
            })
            .collect();

        Ok(GetCookieMetadataResponse {
            cookies: metadata_list,
        })
    }

    /// Discloses a single secret cookie value to authorized caller.
    pub fn get_cookie_value(
        &self,
        req: GetCookieValueRequest,
    ) -> Result<GetCookieValueResponse, String> {
        {
            let policy = self.policy.lock().unwrap();
            if policy.is_metadata_only(&req.context_id) {
                return Err("Cookie value access is denied by policy".to_string());
            }
        }

        let engine_res = self.backend.get_cookies(GetCookiesRequest {
            context_id: req.context_id,
            url: req.url,
            domain: Some(req.domain.clone()),
        })?;

        let target_path = req.path.as_deref().unwrap_or("/");
        let cookie_match = engine_res.cookies.into_iter().find(|c| {
            c.name == req.name
                && c.domain == req.domain
                && (c.path == target_path || target_path.starts_with(&c.path))
        });

        let value_container =
            cookie_match.map(|c| CookieValue::new(c.name, c.domain, c.path, c.value));

        Ok(GetCookieValueResponse {
            cookie: value_container,
        })
    }

    /// Mutates a cookie in the engine profile store.
    pub fn set_cookie(
        &self,
        req: SetCookieServiceRequest,
    ) -> Result<SetCookieServiceResponse, String> {
        let engine_cookie = Cookie {
            name: req.name,
            value: req.value,
            domain: req.domain,
            path: req.path.unwrap_or_else(|| "/".to_string()),
            secure: req.secure.unwrap_or(false),
            http_only: req.http_only.unwrap_or(false),
            same_site: req.same_site,
            expires_epoch_sec: req.expires_epoch_sec,
        };

        let res = self.backend.set_cookie(SetCookieRequest {
            context_id: req.context_id,
            cookie: engine_cookie,
        })?;

        Ok(SetCookieServiceResponse {
            success: res.success,
        })
    }

    /// Deletes cookies from the engine profile store.
    pub fn delete_cookie(
        &self,
        req: DeleteCookieServiceRequest,
    ) -> Result<DeleteCookieServiceResponse, String> {
        let res = self.backend.delete_cookies(DeleteCookiesRequest {
            context_id: req.context_id,
            url: req.url,
            name: Some(req.name),
            domain: Some(req.domain),
        })?;

        Ok(DeleteCookieServiceResponse {
            deleted_count: res.deleted_count,
        })
    }

    /// Clears origin-scoped site data in the target context.
    pub fn clear_site_data(
        &self,
        req: ClearSiteDataRequest,
    ) -> Result<ClearSiteDataResponse, String> {
        let engine_req = validate_and_build_clear_storage(&req)?;
        let res = self.backend.clear_storage(engine_req)?;
        Ok(ClearSiteDataResponse {
            cleared: res.cleared,
        })
    }
}
