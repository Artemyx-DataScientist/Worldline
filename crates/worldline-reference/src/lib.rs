//! Small reference plugin families used to prove that Worldline's kernel
//! remains product-agnostic.
//!
//! The implementations in this crate are architectural probes, not browser,
//! agent, or UI runtimes. They use only the public generic kernel contracts.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use worldline_kernel::{ActivationContext, CapabilityHandle, PluginError};

pub mod agent_like;
pub mod browser_like;
pub mod observation;
#[cfg(windows)]
pub(crate) mod real_cef_lock;
pub mod request_policy;
pub mod request_policy_feasibility;
pub mod s0;
pub mod s1;
pub mod s2;
pub mod s3a;
pub mod s3b;
pub mod s3c;
pub mod ui_like;

pub use observation::{
    Observation, ObservationBus, ObservationDelivery, ObservationDraft, ObservationFailure,
    ObservationId, SubscriberId,
};

pub type CapabilitySlot = Arc<Mutex<Option<CapabilityHandle>>>;

pub(crate) fn capture_capability(
    context: &ActivationContext,
    capability: &worldline_kernel::CapabilityId,
    slot: &CapabilitySlot,
) -> Result<(), PluginError> {
    let handle = context
        .capability(capability)
        .map_err(|error| PluginError::new(error.to_string()))?;
    *slot
        .lock()
        .map_err(|_| PluginError::new("capability slot lock is poisoned"))? = Some(handle);
    Ok(())
}

pub(crate) fn increment_activation_count(context: &ActivationContext) -> Result<u64, PluginError> {
    let count = context
        .state()
        .get("activation-count")
        .map_err(|error| PluginError::new(error.to_string()))?
        .and_then(|value| String::from_utf8(value).ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| PluginError::new("activation count exhausted"))?;
    let mut transaction = context
        .state()
        .transaction()
        .map_err(|error| PluginError::new(error.to_string()))?;
    transaction
        .put("activation-count", count.to_string().as_bytes())
        .map_err(|error| PluginError::new(error.to_string()))?;
    transaction
        .commit()
        .map_err(|error| PluginError::new(error.to_string()))?;
    Ok(count)
}
