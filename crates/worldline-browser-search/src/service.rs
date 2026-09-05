//! Service implementation for structured search target resolution.

use url::Url;
use worldline_browser_services_contract::{
    MAX_SEARCH_TARGET_URL_LENGTH, OP_RESOLVE_SEARCH, SearchContractError, SearchNavigationTarget,
    SearchResolveRequest,
};
use worldline_kernel::{CapabilityService, RpcOperationContract};

use crate::config::{SearchConfigError, SearchProviderConfig};

/// Replaceable capability service resolving queries to structured URLs.
#[derive(Clone, Debug)]
pub struct SearchProviderService {
    config: SearchProviderConfig,
    base_url: Url,
}

impl SearchProviderService {
    pub fn new(config: SearchProviderConfig) -> Result<Self, SearchConfigError> {
        let base_url = config.validate()?;
        Ok(Self { config, base_url })
    }

    pub fn config(&self) -> &SearchProviderConfig {
        &self.config
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Resolves a user search request into a strictly structured, URL-encoded navigation target.
    pub fn resolve(
        &self,
        req: &SearchResolveRequest,
    ) -> Result<SearchNavigationTarget, SearchContractError> {
        let query = req.query();
        if query.trim().is_empty() {
            return Err(SearchContractError::EmptyQuery);
        }

        let mut target_url = self.base_url.clone();
        {
            let mut pairs = target_url.query_pairs_mut();
            for (key, value) in &self.config.static_parameters {
                pairs.append_pair(key, value);
            }
            pairs.append_pair(&self.config.query_parameter_name, query);
        }

        let url_str = target_url.to_string();
        if url_str.len() > MAX_SEARCH_TARGET_URL_LENGTH {
            return Err(SearchContractError::TargetUrlTooLong {
                length: url_str.len(),
                max: MAX_SEARCH_TARGET_URL_LENGTH,
            });
        }

        SearchNavigationTarget::new(url_str, &self.config.query_parameter_name)
    }
}

impl CapabilityService for SearchProviderService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation != OP_RESOLVE_SEARCH && operation != "resolve" {
            return Err(format!("unsupported search operation '{operation}'"));
        }

        let req: SearchResolveRequest = serde_json::from_slice(payload)
            .map_err(|err| format!("failed to deserialize SearchResolveRequest: {err}"))?;

        let target = self
            .resolve(&req)
            .map_err(|err| format!("search resolution failed: {err}"))?;

        serde_json::to_vec(&target)
            .map_err(|err| format!("failed to serialize SearchNavigationTarget: {err}"))
    }

    fn rpc_operation_contract(
        &self,
        operation: &worldline_kernel::OperationId,
    ) -> RpcOperationContract {
        if operation.as_str() == OP_RESOLVE_SEARCH || operation.as_str() == "resolve" {
            RpcOperationContract::safe(operation.clone())
        } else {
            RpcOperationContract::never_retry(operation.clone())
        }
    }
}
