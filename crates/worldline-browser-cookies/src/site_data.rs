use worldline_browser_contract::primitives::ClearStorageRequest;
use worldline_browser_services_contract::ClearSiteDataRequest;

/// Validates origin and builds engine ClearStorageRequest.
pub fn validate_and_build_clear_storage(
    req: &ClearSiteDataRequest,
) -> Result<ClearStorageRequest, String> {
    let origin = req.origin.trim();
    if origin.is_empty() {
        return Err("Origin must not be empty".to_string());
    }

    if !origin.starts_with("http://") && !origin.starts_with("https://") {
        return Err("Origin must start with http:// or https://".to_string());
    }

    // Canonicalize origin (strip trailing slash)
    let canonical_origin = origin.trim_end_matches('/').to_string();

    Ok(ClearStorageRequest {
        context_id: req.context_id.clone(),
        origin: canonical_origin,
        storage_type: req.storage_type,
    })
}
