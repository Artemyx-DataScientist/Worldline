use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use url::Url;
use worldline_browser_contract::identity::BrowserContextId;
use worldline_browser_contract::primitives::{
    ClearStorageRequest, ClearStorageResponse, Cookie, CookieV0_2, DeleteCookiesRequest,
    DeleteCookiesResponse, GetCookiesRequest, GetCookiesResponse, GetCookiesResponseV0_2,
    SetCookieRequest, SetCookieRequestV0_2, SetCookieResponse, StorageType,
};
use worldline_browser_services_contract::{
    ClearSiteDataRequest, ClearSiteDataResponse, CookieMetadata, CookieMetadataV0_2, CookieValue,
    DeleteCookieServiceRequest, DeleteCookieServiceResponse, GetCookieMetadataRequest,
    GetCookieMetadataResponse, GetCookieMetadataResponseV0_2, GetCookieValueRequest,
    GetCookieValueResponse, SetCookieServiceRequest, SetCookieServiceRequestV0_2,
    SetCookieServiceResponse,
};

use crate::policy::CookiePolicySnapshot;
use crate::site_data::validate_and_build_clear_storage;

/// Interface for low-level engine cookie and storage primitives.
pub trait CookieEngineBackend: Send + Sync {
    fn get_cookies(&self, req: GetCookiesRequest) -> Result<GetCookiesResponse, String>;
    fn set_cookie(&self, req: SetCookieRequest) -> Result<SetCookieResponse, String>;
    fn delete_cookies(&self, req: DeleteCookiesRequest) -> Result<DeleteCookiesResponse, String>;
    fn clear_storage(&self, req: ClearStorageRequest) -> Result<ClearStorageResponse, String>;

    /// Additive engine.cookies/0.2 operations. Implementations that only
    /// understand 0.1 retain compatibility with a conservative host-only
    /// projection; native implementations override these methods.
    fn get_cookies_v0_2(&self, req: GetCookiesRequest) -> Result<GetCookiesResponseV0_2, String> {
        let response = self.get_cookies(req)?;
        Ok(GetCookiesResponseV0_2 {
            cookies: response.cookies.into_iter().map(Into::into).collect(),
        })
    }

    fn set_cookie_v0_2(&self, req: SetCookieRequestV0_2) -> Result<SetCookieResponse, String> {
        self.set_cookie(SetCookieRequest {
            context_id: req.context_id,
            cookie: req.cookie.into(),
        })
    }
}

/// Canonicalizes a host input before it participates in cookie matching.
///
/// Cookie authorization uses DNS-label boundaries, never a raw string suffix.
/// `Url` supplies IDNA processing for Unicode host names; the remaining
/// validation rejects credentials, paths, empty labels, and malformed labels.
pub(crate) fn canonical_host(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\', '@', '?', '#'])
    {
        return Err("cookie host contains invalid characters".to_string());
    }
    if value.starts_with("..") {
        return Err("cookie host has more than one leading dot".to_string());
    }
    if value.ends_with("..") {
        return Err("cookie host has more than one trailing dot".to_string());
    }
    let value = value.trim_start_matches('.');
    if value.is_empty() || value.starts_with('.') {
        return Err("cookie host has an invalid leading label".to_string());
    }
    let url_value = if value.contains(':') && !value.starts_with('[') {
        format!("https://[{value}]/")
    } else {
        format!("https://{value}/")
    };
    let parsed = Url::parse(&url_value).map_err(|error| format!("invalid cookie host: {error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "cookie host is missing".to_string())?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err("cookie host is empty".to_string());
    }
    if !host.contains(':') {
        for label in host.split('.') {
            if label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err("cookie host contains an invalid DNS label".to_string());
            }
        }
    }
    Ok(host)
}

pub(crate) fn canonical_host_from_url(value: &str) -> Result<String, String> {
    let parsed = Url::parse(value).map_err(|error| format!("invalid cookie URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("cookie URL must use HTTP or HTTPS".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "cookie URL has no host".to_string())?;
    canonical_host(host)
}

pub(crate) fn canonical_origin(value: &str) -> Result<String, String> {
    let parsed = Url::parse(value).map_err(|error| format!("invalid storage origin: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("storage origin must use HTTP or HTTPS".to_string());
    }
    let host = canonical_host(
        parsed
            .host_str()
            .ok_or_else(|| "storage origin has no host".to_string())?,
    )?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(format!(
        "{}://{host}{port}",
        parsed.scheme().to_ascii_lowercase()
    ))
}

pub(crate) fn cookie_domain_matches(
    request_host: &str,
    cookie_domain: &str,
    host_only: bool,
) -> Result<bool, String> {
    let request_host = canonical_host(request_host)?;
    let cookie_domain = canonical_host(cookie_domain)?;
    Ok(if host_only {
        request_host == cookie_domain
    } else {
        request_host == cookie_domain || request_host.ends_with(&format!(".{cookie_domain}"))
    })
}

pub(crate) fn domain_selector_matches(
    selected_domain: &str,
    cookie_domain: &str,
) -> Result<bool, String> {
    let selected_domain = canonical_host(selected_domain)?;
    let cookie_domain = canonical_host(cookie_domain)?;
    Ok(cookie_domain == selected_domain || cookie_domain.ends_with(&format!(".{selected_domain}")))
}

pub(crate) fn cookie_path_matches(cookie_path: &str, request_path: &str) -> bool {
    let cookie_path = if cookie_path.is_empty() {
        "/"
    } else {
        cookie_path
    };
    let request_path = if request_path.is_empty() {
        "/"
    } else {
        request_path
    };
    request_path == cookie_path
        || request_path.starts_with(cookie_path.strip_suffix('/').unwrap_or(cookie_path))
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/'))
}

/// In-memory implementation of engine cookie and storage primitives for testing and validation.
type StorageKey = (BrowserContextId, String, StorageType);
type StorageValues = BTreeMap<String, String>;

#[derive(Default)]
pub struct InMemoryCookieEngine {
    /// Cookies strictly partitioned by context ID.
    cookies: Mutex<BTreeMap<BrowserContextId, Vec<CookieV0_2>>>,
    /// Storage records strictly partitioned by (context ID, origin, storage type).
    storage: Mutex<BTreeMap<StorageKey, StorageValues>>,
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
        let canonical_origin = canonical_origin(origin).expect("storage test origin must be valid");
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
        let canonical_origin = canonical_origin(origin).expect("storage test origin must be valid");
        storage
            .get(&(context_id.clone(), canonical_origin, storage_type))
            .and_then(|map| map.get(key).cloned())
    }
}

impl CookieEngineBackend for InMemoryCookieEngine {
    fn get_cookies(&self, req: GetCookiesRequest) -> Result<GetCookiesResponse, String> {
        Ok(GetCookiesResponse {
            cookies: self
                .get_cookies_v0_2(req)?
                .cookies
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    fn set_cookie(&self, req: SetCookieRequest) -> Result<SetCookieResponse, String> {
        self.set_cookie_v0_2(SetCookieRequestV0_2 {
            context_id: req.context_id,
            cookie: req.cookie.into(),
        })
    }

    fn get_cookies_v0_2(&self, req: GetCookiesRequest) -> Result<GetCookiesResponseV0_2, String> {
        let cookies_guard = self.cookies.lock().unwrap();
        let context_cookies = cookies_guard
            .get(&req.context_id)
            .cloned()
            .unwrap_or_default();

        let request_host = req
            .url
            .as_deref()
            .map(canonical_host_from_url)
            .transpose()?;
        let selected_domain = req.domain.as_deref().map(canonical_host).transpose()?;

        let filtered = context_cookies
            .into_iter()
            .filter(|c| {
                let host_matches = request_host.as_deref().is_none_or(|host| {
                    cookie_domain_matches(host, &c.domain, c.host_only).unwrap_or(false)
                });
                let selector_matches = selected_domain.as_deref().is_none_or(|domain| {
                    domain_selector_matches(domain, &c.domain).unwrap_or(false)
                });
                host_matches && selector_matches
            })
            .collect();

        Ok(GetCookiesResponseV0_2 { cookies: filtered })
    }

    fn set_cookie_v0_2(&self, req: SetCookieRequestV0_2) -> Result<SetCookieResponse, String> {
        let mut cookie = req.cookie;
        let raw_domain = cookie.domain.clone();
        cookie.domain = canonical_host(&raw_domain)?;
        if raw_domain.trim_start().starts_with('.') {
            cookie.host_only = false;
        }
        if cookie.path.is_empty() || !cookie.path.starts_with('/') {
            return Err("cookie path must be an absolute path".to_string());
        }
        let mut cookies_guard = self.cookies.lock().unwrap();
        let context_cookies = cookies_guard.entry(req.context_id).or_default();

        // Overwrite existing cookie with same name, domain, path
        context_cookies.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        context_cookies.push(cookie);

        Ok(SetCookieResponse { success: true })
    }

    fn delete_cookies(&self, req: DeleteCookiesRequest) -> Result<DeleteCookiesResponse, String> {
        let selected_host = req
            .url
            .as_deref()
            .map(canonical_host_from_url)
            .transpose()?;
        let selected_domain = req.domain.as_deref().map(canonical_host).transpose()?;
        let mut cookies_guard = self.cookies.lock().unwrap();
        let context_cookies = cookies_guard.entry(req.context_id).or_default();

        let before = context_cookies.len();
        context_cookies.retain(|c| {
            if let Some(name) = &req.name
                && &c.name != name
            {
                return true;
            }
            let host_matches = selected_host.as_deref().is_none_or(|host| {
                cookie_domain_matches(host, &c.domain, c.host_only).unwrap_or(false)
            });
            let domain_matches = selected_domain
                .as_deref()
                .is_none_or(|domain| domain_selector_matches(domain, &c.domain).unwrap_or(false));
            !(host_matches && domain_matches)
        });
        let deleted_count = (before - context_cookies.len()) as u32;

        Ok(DeleteCookiesResponse { deleted_count })
    }

    fn clear_storage(&self, req: ClearStorageRequest) -> Result<ClearStorageResponse, String> {
        let mut storage = self.storage.lock().unwrap();
        let canonical_origin = canonical_origin(&req.origin)?;

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

        Ok(ClearStorageResponse { cleared: had_items })
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

    /// Versioned metadata inspection that preserves host-only versus
    /// domain-cookie scope from the engine.cookies/0.2 primitive.
    pub fn get_cookie_metadata_v0_2(
        &self,
        req: GetCookieMetadataRequest,
    ) -> Result<GetCookieMetadataResponseV0_2, String> {
        let engine_res = self.backend.get_cookies_v0_2(GetCookiesRequest {
            context_id: req.context_id,
            url: req.url,
            domain: req.domain,
        })?;

        Ok(GetCookieMetadataResponseV0_2 {
            cookies: engine_res
                .cookies
                .into_iter()
                .map(|c| CookieMetadataV0_2 {
                    name: c.name,
                    domain: c.domain,
                    path: c.path,
                    secure: c.secure,
                    http_only: c.http_only,
                    same_site: c.same_site,
                    expires_epoch_sec: c.expires_epoch_sec,
                    host_only: c.host_only,
                })
                .collect(),
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

        let requested_domain = canonical_host(&req.domain)?;
        let engine_res = self.backend.get_cookies(GetCookiesRequest {
            context_id: req.context_id,
            url: req.url,
            domain: Some(requested_domain.clone()),
        })?;

        let target_path = req.path.as_deref().unwrap_or("/");
        let cookie_match = engine_res.cookies.into_iter().find(|c| {
            c.name == req.name
                && canonical_host(&c.domain)
                    .map(|domain| domain == requested_domain)
                    .unwrap_or(false)
                && cookie_path_matches(&c.path, target_path)
        });

        let value_container =
            cookie_match.map(|c| CookieValue::new(c.name, c.domain, c.path, c.value));

        Ok(GetCookieValueResponse {
            cookie: value_container,
        })
    }

    /// Versioned secret-value lookup using the explicit engine.cookies/0.2
    /// scope semantics. The value remains redacted by `CookieValue`'s Debug
    /// implementation and is returned only through this value-authorized API.
    pub fn get_cookie_value_v0_2(
        &self,
        req: GetCookieValueRequest,
    ) -> Result<GetCookieValueResponse, String> {
        {
            let policy = self.policy.lock().unwrap();
            if policy.is_metadata_only(&req.context_id) {
                return Err("Cookie value access is denied by policy".to_string());
            }
        }

        let requested_domain = canonical_host(&req.domain)?;
        let engine_res = self.backend.get_cookies_v0_2(GetCookiesRequest {
            context_id: req.context_id,
            url: req.url,
            domain: Some(requested_domain.clone()),
        })?;

        let target_path = req.path.as_deref().unwrap_or("/");
        let cookie_match = engine_res.cookies.into_iter().find(|c| {
            c.name == req.name
                && canonical_host(&c.domain)
                    .map(|domain| domain == requested_domain)
                    .unwrap_or(false)
                && cookie_path_matches(&c.path, target_path)
        });

        Ok(GetCookieValueResponse {
            cookie: cookie_match.map(|c| CookieValue::new(c.name, c.domain, c.path, c.value)),
        })
    }

    /// Mutates a cookie in the engine profile store.
    pub fn set_cookie(
        &self,
        req: SetCookieServiceRequest,
    ) -> Result<SetCookieServiceResponse, String> {
        let domain = canonical_host(&req.domain)?;
        let engine_cookie = Cookie {
            name: req.name,
            value: req.value,
            domain,
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

    /// Versioned cookie mutation carrying explicit host-only/domain-cookie
    /// scope. This is the only service path used by the real native S3B proof.
    pub fn set_cookie_v0_2(
        &self,
        req: SetCookieServiceRequestV0_2,
    ) -> Result<SetCookieServiceResponse, String> {
        let domain = canonical_host(&req.domain)?;
        let res = self.backend.set_cookie_v0_2(SetCookieRequestV0_2 {
            context_id: req.context_id,
            cookie: CookieV0_2 {
                name: req.name,
                value: req.value,
                domain,
                path: req.path.unwrap_or_else(|| "/".to_string()),
                secure: req.secure.unwrap_or(false),
                http_only: req.http_only.unwrap_or(false),
                same_site: req.same_site,
                expires_epoch_sec: req.expires_epoch_sec,
                host_only: req.host_only,
            },
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
        let domain = canonical_host(&req.domain)?;
        let res = self.backend.delete_cookies(DeleteCookiesRequest {
            context_id: req.context_id,
            url: req.url,
            name: Some(req.name),
            domain: Some(domain),
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
