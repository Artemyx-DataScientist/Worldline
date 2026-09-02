//! Experimental, engine-neutral request-policy interception contracts.
//!
//! The contract describes the smallest useful pre-dispatch decision surface.
//! It intentionally contains no engine objects, window, pointer, header,
//! body, cookie, credential, or response representation. Full request URLs
//! are admitted only in the exact context/page-scoped decision DTO; outcome
//! observations do not repeat them.

use serde::{Deserialize, Serialize};

use crate::identity::{BrowserContextId, PageId};

/// Experimental request-policy contract identifier.
pub const CONTRACT_REQUEST_POLICY: &str = "browser.request-policy";
/// Versioned experimental request-policy contract identifier.
pub const CONTRACT_REQUEST_POLICY_V0_1: &str = "browser.request-policy/0.1";
/// Experimental request-policy contract major version.
pub const REQUEST_POLICY_MAJOR_V0_1: u16 = 0;
/// Experimental request-policy contract minor version.
pub const REQUEST_POLICY_MINOR_V0_1: u16 = 1;

/// Request-policy operation names.
pub const OP_REQUEST_POLICY_REGISTER: &str = "register";
pub const OP_REQUEST_POLICY_UNREGISTER: &str = "unregister";
pub const OP_REQUEST_POLICY_DECIDE: &str = "decide";
pub const OP_REQUEST_POLICY_OBSERVE: &str = "observe";

/// Bounded request-policy DTO limits. These are deliberately conservative:
/// the hot path must not turn an engine callback into an allocation sink.
pub const MAX_REQUEST_POLICY_REGISTRATION_ID_BYTES: usize = 128;
pub const MAX_REQUEST_POLICY_PROVIDER_ID_BYTES: usize = 128;
pub const MAX_REQUEST_POLICY_RULE_REFERENCE_BYTES: usize = 128;
pub const MAX_REQUEST_POLICY_URL_BYTES: usize = 8192;
pub const MAX_REQUEST_POLICY_METHOD_BYTES: usize = 32;
pub const MAX_REQUEST_POLICY_ORIGIN_BYTES: usize = 2048;
pub const MAX_REQUEST_POLICY_DEADLINE_MS: u64 = 5_000;

/// The finite default deadline used by the first optional adblock profile.
pub const DEFAULT_REQUEST_POLICY_DEADLINE_MS: u64 = 250;

/// Neutral classification of the resource the engine is about to load.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RequestResourceType {
    MainFrame,
    SubFrame,
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
    Xhr,
    Fetch,
    WebSocket,
    Manifest,
    #[default]
    Other,
}

/// Minimal neutral metadata delivered to an authorized policy evaluator.
///
/// This is decision input, not an observation event. Consumers must not copy
/// `url` into generic telemetry because it may contain sensitive query data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPolicyMetadata {
    pub context_id: BrowserContextId,
    pub page_id: Option<PageId>,
    pub url: String,
    pub method: String,
    pub resource_type: RequestResourceType,
    pub initiator_origin: Option<String>,
    pub top_level_origin: Option<String>,
}

impl RequestPolicyMetadata {
    /// Validates the bounded, neutral request shape before dispatch.
    pub fn validate(&self) -> Result<(), String> {
        validate_id("context_id", self.context_id.as_str(), 256)?;
        if let Some(page_id) = &self.page_id {
            validate_id("page_id", page_id.as_str(), 256)?;
        }
        validate_text("url", &self.url, MAX_REQUEST_POLICY_URL_BYTES, false)?;
        validate_text(
            "method",
            &self.method,
            MAX_REQUEST_POLICY_METHOD_BYTES,
            false,
        )?;
        if let Some(origin) = &self.initiator_origin {
            validate_origin("initiator_origin", origin)?;
        }
        if let Some(origin) = &self.top_level_origin {
            validate_origin("top_level_origin", origin)?;
        }
        Ok(())
    }
}

/// Failure semantics are selected by a registration/profile, not by the
/// generic interception broker. Future security profiles can choose
/// `FailClosed`; the first optional adblock profile explicitly chooses
/// `FailOpen`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPolicyFailureMode {
    FailOpen,
    FailClosed,
}

/// Explicit pre-dispatch action returned by a policy evaluator.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPolicyAction {
    Allow,
    Block,
}

/// Outcome status for a direct request-policy result or safe observation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPolicyOutcome {
    Evaluated,
    FailureFallback,
    DeadlineExceeded,
    Cancelled,
    ScopeDenied,
    Unavailable,
}

/// Host/provider request carrying one exact-scope policy decision input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPolicyRequest {
    pub registration_id: String,
    pub metadata: RequestPolicyMetadata,
    pub deadline_ms: u64,
}

impl RequestPolicyRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_text(
            "registration_id",
            &self.registration_id,
            MAX_REQUEST_POLICY_REGISTRATION_ID_BYTES,
            false,
        )?;
        self.metadata.validate()?;
        if self.deadline_ms == 0 || self.deadline_ms > MAX_REQUEST_POLICY_DEADLINE_MS {
            return Err(format!(
                "deadline_ms must be between 1 and {MAX_REQUEST_POLICY_DEADLINE_MS}"
            ));
        }
        Ok(())
    }
}

/// Direct decision returned to the interception mechanism. The optional
/// provider/rule fields are opaque and are retained only for safe
/// post-outcome observation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPolicyResult {
    pub action: RequestPolicyAction,
    pub outcome: RequestPolicyOutcome,
    pub provider_id: Option<String>,
    pub opaque_rule_ref: Option<String>,
}

impl RequestPolicyResult {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(provider_id) = &self.provider_id {
            validate_text(
                "provider_id",
                provider_id,
                MAX_REQUEST_POLICY_PROVIDER_ID_BYTES,
                false,
            )?;
        }
        if let Some(rule_ref) = &self.opaque_rule_ref {
            validate_text(
                "opaque_rule_ref",
                rule_ref,
                MAX_REQUEST_POLICY_RULE_REFERENCE_BYTES,
                false,
            )?;
        }
        Ok(())
    }
}

/// Exact scope and failure semantics for one registered policy instance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPolicyRegistration {
    pub registration_id: String,
    pub context_id: BrowserContextId,
    pub page_id: Option<PageId>,
    pub failure_mode: RequestPolicyFailureMode,
    pub max_in_flight: u16,
    pub provider_id: String,
}

impl RequestPolicyRegistration {
    pub fn validate(&self) -> Result<(), String> {
        validate_text(
            "registration_id",
            &self.registration_id,
            MAX_REQUEST_POLICY_REGISTRATION_ID_BYTES,
            false,
        )?;
        validate_id("context_id", self.context_id.as_str(), 256)?;
        if let Some(page_id) = &self.page_id {
            validate_id("page_id", page_id.as_str(), 256)?;
        }
        if self.max_in_flight == 0 {
            return Err("max_in_flight must be greater than zero".to_string());
        }
        validate_text(
            "provider_id",
            &self.provider_id,
            MAX_REQUEST_POLICY_PROVIDER_ID_BYTES,
            false,
        )?;
        Ok(())
    }
}

/// Safe post-outcome observation. It deliberately excludes the request URL
/// and all sensitive request content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPolicyObservation {
    pub registration_id: String,
    pub context_id: BrowserContextId,
    pub page_id: Option<PageId>,
    pub action: RequestPolicyAction,
    pub outcome: RequestPolicyOutcome,
    pub provider_id: Option<String>,
    pub opaque_rule_ref: Option<String>,
    pub latency_ms: u64,
}

impl RequestPolicyObservation {
    pub fn validate(&self) -> Result<(), String> {
        validate_text(
            "registration_id",
            &self.registration_id,
            MAX_REQUEST_POLICY_REGISTRATION_ID_BYTES,
            false,
        )?;
        validate_id("context_id", self.context_id.as_str(), 256)?;
        if let Some(page_id) = &self.page_id {
            validate_id("page_id", page_id.as_str(), 256)?;
        }
        RequestPolicyResult {
            action: self.action,
            outcome: self.outcome,
            provider_id: self.provider_id.clone(),
            opaque_rule_ref: self.opaque_rule_ref.clone(),
        }
        .validate()
    }
}

fn validate_id(name: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    validate_text(name, value, max_bytes, false)
}

fn validate_origin(name: &str, value: &str) -> Result<(), String> {
    validate_text(name, value, MAX_REQUEST_POLICY_ORIGIN_BYTES, false)?;
    if !(value.starts_with("http://") || value.starts_with("https://")) {
        return Err(format!("{name} must be an HTTP(S) origin"));
    }
    Ok(())
}

fn validate_text(
    name: &str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if !allow_empty && value.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{name} exceeds {max_bytes} bytes"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{name} contains a control character"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> RequestPolicyMetadata {
        RequestPolicyMetadata {
            context_id: BrowserContextId::new("ctx-a"),
            page_id: Some(PageId::new("page-a")),
            url: "http://127.0.0.1:1234/asset.js?token=redacted".to_string(),
            method: "GET".to_string(),
            resource_type: RequestResourceType::Script,
            initiator_origin: Some("http://127.0.0.1:1234".to_string()),
            top_level_origin: Some("http://127.0.0.1:1234".to_string()),
        }
    }

    #[test]
    fn request_contract_roundtrips_and_is_versioned() {
        let request = RequestPolicyRequest {
            registration_id: "registration-a".to_string(),
            metadata: metadata(),
            deadline_ms: DEFAULT_REQUEST_POLICY_DEADLINE_MS,
        };
        request.validate().expect("valid request");
        let encoded = serde_json::to_vec(&request).expect("encode");
        let decoded: RequestPolicyRequest = serde_json::from_slice(&encoded).expect("decode");
        assert_eq!(decoded, request);
        assert_eq!(CONTRACT_REQUEST_POLICY_V0_1, "browser.request-policy/0.1");
    }

    #[test]
    fn contract_rejects_sensitive_shape_and_unbounded_deadline() {
        let mut value = serde_json::to_value(RequestPolicyRequest {
            registration_id: "registration-a".to_string(),
            metadata: metadata(),
            deadline_ms: DEFAULT_REQUEST_POLICY_DEADLINE_MS,
        })
        .expect("encode");
        value["headers"] = serde_json::json!({"authorization": "secret"});
        assert!(serde_json::from_value::<RequestPolicyRequest>(value).is_err());

        let mut request = RequestPolicyRequest {
            registration_id: "registration-a".to_string(),
            metadata: metadata(),
            deadline_ms: MAX_REQUEST_POLICY_DEADLINE_MS + 1,
        };
        assert!(request.validate().is_err());
        request.deadline_ms = 0;
        assert!(request.validate().is_err());
    }

    #[test]
    fn observation_does_not_carry_request_url() {
        let observation = RequestPolicyObservation {
            registration_id: "registration-a".to_string(),
            context_id: BrowserContextId::new("ctx-a"),
            page_id: Some(PageId::new("page-a")),
            action: RequestPolicyAction::Block,
            outcome: RequestPolicyOutcome::Evaluated,
            provider_id: Some("adblock-v0".to_string()),
            opaque_rule_ref: Some("rule-1".to_string()),
            latency_ms: 3,
        };
        let value = serde_json::to_value(observation).expect("encode");
        assert!(value.get("url").is_none());
        assert!(value.get("headers").is_none());
    }
}
