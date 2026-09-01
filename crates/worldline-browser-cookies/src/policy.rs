use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use worldline_browser_contract::identity::BrowserContextId;

/// Policy rules governing service-level cookie access permissions per context.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CookiePolicySnapshot {
    /// Contexts that have value disclosure disabled (metadata-only).
    pub metadata_only_contexts: BTreeMap<BrowserContextId, bool>,
}

impl CookiePolicySnapshot {
    pub fn new() -> Self {
        Self {
            metadata_only_contexts: BTreeMap::new(),
        }
    }

    pub fn set_metadata_only(&mut self, context_id: BrowserContextId, metadata_only: bool) {
        self.metadata_only_contexts
            .insert(context_id, metadata_only);
    }

    pub fn is_metadata_only(&self, context_id: &BrowserContextId) -> bool {
        self.metadata_only_contexts
            .get(context_id)
            .copied()
            .unwrap_or(false)
    }
}
