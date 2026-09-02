//! Small deterministic request-policy profile for Worldline.
//!
//! This crate intentionally implements only the bounded matcher needed by the
//! first proving slice. It is not a uBlock/AdGuard syntax implementation and
//! has no engine, kernel, UI, subscription, DOM, or response-filtering
//! dependency.

use std::fmt;

use url::Url;
use worldline_browser_contract::identity::{BrowserContextId, PageId};
use worldline_browser_contract::request_policy::{
    MAX_REQUEST_POLICY_RULE_REFERENCE_BYTES, RequestPolicyAction, RequestPolicyFailureMode,
    RequestPolicyMetadata, RequestPolicyOutcome, RequestPolicyRegistration, RequestPolicyRequest,
    RequestPolicyResult, RequestResourceType,
};
use worldline_browser_provider::{
    RequestPolicyCancellation, RequestPolicyEvaluator, RequestPolicyEvaluatorError,
};

/// Stable identity of this optional profile within the experimental slice.
pub const AD_BLOCK_PROVIDER_ID: &str = "worldline.browser.adblock.v0";
/// Explicit profile registration name used by the first proving fixture.
pub const AD_BLOCK_PROFILE_ID: &str = "worldline.browser.adblock.profile.v0";
/// Maximum number of ordered rules in one policy instance.
pub const MAX_RULES: usize = 1_024;
/// Maximum encoded rule-list size admitted by the deterministic parser.
pub const MAX_RULE_LIST_BYTES: usize = 256 * 1024;
/// Maximum bytes in one matcher field.
pub const MAX_MATCHER_BYTES: usize = 512;

/// Minimal action understood by this profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdblockAction {
    Allow,
    Block,
}

/// One bounded ordered host/URL matcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdblockRule {
    pub rule_id: String,
    pub action: AdblockAction,
    /// Exact host or a suffix matcher. `*.example.test` also matches the
    /// apex host only when written as `example.test`; subdomains are matched
    /// at a DNS label boundary.
    pub host: Option<String>,
    /// Literal substring matched against the full request URL.
    pub url_contains: Option<String>,
    /// Optional exact neutral resource-type match.
    pub resource_type: Option<RequestResourceType>,
}

impl AdblockRule {
    pub fn validate(&self) -> Result<(), AdblockError> {
        validate_text(
            "rule_id",
            &self.rule_id,
            MAX_REQUEST_POLICY_RULE_REFERENCE_BYTES,
        )?;
        if self.host.is_none() && self.url_contains.is_none() {
            return Err(AdblockError::InvalidRule(
                "a rule needs host or url matcher".to_string(),
            ));
        }
        if let Some(host) = &self.host {
            validate_text("host", host, MAX_MATCHER_BYTES)?;
            validate_host_pattern(host)?;
        }
        if let Some(url) = &self.url_contains {
            validate_text("url", url, MAX_MATCHER_BYTES)?;
        }
        Ok(())
    }

    fn matches(&self, metadata: &RequestPolicyMetadata) -> bool {
        if let Some(resource_type) = self.resource_type
            && resource_type != metadata.resource_type
        {
            return false;
        }
        if let Some(host) = &self.host {
            let Some(request_host) = Url::parse(&metadata.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            else {
                return false;
            };
            if !host_matches(host, &request_host) {
                return false;
            }
        }
        self.url_contains
            .as_ref()
            .is_none_or(|needle| metadata.url.contains(needle))
    }
}

/// Bounded parser/evaluator failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdblockError {
    InvalidRule(String),
    InvalidRuleList(String),
    TooManyRules { limit: usize },
    RuleListTooLarge { limit: usize, actual: usize },
}

impl fmt::Display for AdblockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRule(reason) => write!(formatter, "invalid adblock rule: {reason}"),
            Self::InvalidRuleList(reason) => {
                write!(formatter, "invalid adblock rule list: {reason}")
            }
            Self::TooManyRules { limit } => {
                write!(formatter, "adblock rule limit of {limit} exceeded")
            }
            Self::RuleListTooLarge { limit, actual } => write!(
                formatter,
                "adblock rule list of {actual} bytes exceeds the {limit} byte limit"
            ),
        }
    }
}

impl std::error::Error for AdblockError {}

/// Replaceable deterministic policy instance. Rules are evaluated in input
/// order; the first matching rule wins and the default is Allow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdblockPolicy {
    rules: Vec<AdblockRule>,
}

impl AdblockPolicy {
    pub fn from_rules(rules: Vec<AdblockRule>) -> Result<Self, AdblockError> {
        if rules.len() > MAX_RULES {
            return Err(AdblockError::TooManyRules { limit: MAX_RULES });
        }
        for rule in &rules {
            rule.validate()?;
        }
        Ok(Self { rules })
    }

    /// Parses only the intentionally documented mini-format:
    ///
    /// `block|allow rule-id host=example.test url=/track resource=script`
    ///
    /// Fields after the action/id are optional, unordered, and must be
    /// unique. Quoting, regular expressions, subscriptions, and other
    /// filter-list dialects are deliberately not accepted.
    pub fn from_text(text: &str) -> Result<Self, AdblockError> {
        if text.len() > MAX_RULE_LIST_BYTES {
            return Err(AdblockError::RuleListTooLarge {
                limit: MAX_RULE_LIST_BYTES,
                actual: text.len(),
            });
        }
        let mut rules = Vec::new();
        for (line_index, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if rules.len() >= MAX_RULES {
                return Err(AdblockError::TooManyRules { limit: MAX_RULES });
            }
            let mut fields = line.split_whitespace();
            let action = match fields.next() {
                Some("allow") => AdblockAction::Allow,
                Some("block") => AdblockAction::Block,
                Some(other) => {
                    return Err(AdblockError::InvalidRuleList(format!(
                        "line {} has unknown action '{other}'",
                        line_index + 1
                    )));
                }
                None => continue,
            };
            let rule_id = fields.next().ok_or_else(|| {
                AdblockError::InvalidRuleList(format!("line {} has no rule id", line_index + 1))
            })?;
            let mut host = None;
            let mut url_contains = None;
            let mut resource_type = None;
            for field in fields {
                let (key, value) = field.split_once('=').ok_or_else(|| {
                    AdblockError::InvalidRuleList(format!(
                        "line {} has malformed field '{field}'",
                        line_index + 1
                    ))
                })?;
                if value.is_empty() {
                    return Err(AdblockError::InvalidRuleList(format!(
                        "line {} has empty {key} field",
                        line_index + 1
                    )));
                }
                match key {
                    "host" if host.is_none() => host = Some(value.to_string()),
                    "url" if url_contains.is_none() => url_contains = Some(value.to_string()),
                    "resource" if resource_type.is_none() => {
                        resource_type = Some(parse_resource_type(value).ok_or_else(|| {
                            AdblockError::InvalidRuleList(format!(
                                "line {} has unknown resource type '{value}'",
                                line_index + 1
                            ))
                        })?);
                    }
                    "host" | "url" | "resource" => {
                        return Err(AdblockError::InvalidRuleList(format!(
                            "line {} repeats {key}",
                            line_index + 1
                        )));
                    }
                    other => {
                        return Err(AdblockError::InvalidRuleList(format!(
                            "line {} has unknown field '{other}'",
                            line_index + 1
                        )));
                    }
                }
            }
            rules.push(AdblockRule {
                rule_id: rule_id.to_string(),
                action,
                host,
                url_contains,
                resource_type,
            });
        }
        Self::from_rules(rules)
    }

    pub fn rules(&self) -> &[AdblockRule] {
        &self.rules
    }

    /// Evaluates the neutral request contract without requiring a broker.
    /// Broker integrations should use the `RequestPolicyEvaluator` impl.
    pub fn evaluate(
        &self,
        request: &RequestPolicyRequest,
    ) -> Result<RequestPolicyResult, AdblockError> {
        request.validate().map_err(AdblockError::InvalidRuleList)?;
        let matching_rule = self
            .rules
            .iter()
            .find(|rule| rule.matches(&request.metadata));
        Ok(RequestPolicyResult {
            action: matching_rule
                .map(|rule| rule.action)
                .map(adblock_action)
                .unwrap_or(RequestPolicyAction::Allow),
            outcome: RequestPolicyOutcome::Evaluated,
            provider_id: Some(AD_BLOCK_PROVIDER_ID.to_string()),
            opaque_rule_ref: matching_rule.map(|rule| rule.rule_id.clone()),
        })
    }

    /// Builds the explicit profile registration used by this optional plugin.
    /// FailOpen is a property of this registration/profile, not of the
    /// generic request-policy broker.
    pub fn fail_open_registration(
        context_id: BrowserContextId,
        page_id: Option<PageId>,
        max_in_flight: u16,
    ) -> RequestPolicyRegistration {
        RequestPolicyRegistration {
            registration_id: AD_BLOCK_PROFILE_ID.to_string(),
            context_id,
            page_id,
            failure_mode: RequestPolicyFailureMode::FailOpen,
            max_in_flight,
            provider_id: AD_BLOCK_PROVIDER_ID.to_string(),
        }
    }
}

impl RequestPolicyEvaluator for AdblockPolicy {
    fn provider_id(&self) -> &str {
        AD_BLOCK_PROVIDER_ID
    }

    fn decide(
        &self,
        request: &RequestPolicyRequest,
        cancellation: &RequestPolicyCancellation,
    ) -> Result<RequestPolicyResult, RequestPolicyEvaluatorError> {
        if cancellation.is_cancelled() {
            return Err(RequestPolicyEvaluatorError::Cancelled);
        }
        self.evaluate(request)
            .map_err(|error| RequestPolicyEvaluatorError::Rejected(error.to_string()))
    }
}

fn adblock_action(action: AdblockAction) -> RequestPolicyAction {
    match action {
        AdblockAction::Allow => RequestPolicyAction::Allow,
        AdblockAction::Block => RequestPolicyAction::Block,
    }
}

fn validate_text(name: &str, value: &str, limit: usize) -> Result<(), AdblockError> {
    if value.is_empty() {
        return Err(AdblockError::InvalidRule(format!(
            "{name} must not be empty"
        )));
    }
    if value.len() > limit {
        return Err(AdblockError::InvalidRule(format!(
            "{name} exceeds {limit} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(AdblockError::InvalidRule(format!(
            "{name} contains a control character"
        )));
    }
    Ok(())
}

fn validate_host_pattern(host: &str) -> Result<(), AdblockError> {
    let candidate = host.strip_prefix("*.").unwrap_or(host);
    if candidate.is_empty()
        || candidate.contains('/')
        || candidate.contains(':')
        || candidate.contains('*')
        || candidate.chars().any(|character| character.is_whitespace())
    {
        return Err(AdblockError::InvalidRule(
            "host must be a DNS name with an optional '*.' prefix".to_string(),
        ));
    }
    Ok(())
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.strip_prefix("*.").unwrap_or(pattern);
    host == pattern || host.ends_with(&format!(".{pattern}"))
}

fn parse_resource_type(value: &str) -> Option<RequestResourceType> {
    Some(match value {
        "main_frame" => RequestResourceType::MainFrame,
        "sub_frame" => RequestResourceType::SubFrame,
        "stylesheet" => RequestResourceType::Stylesheet,
        "script" => RequestResourceType::Script,
        "image" => RequestResourceType::Image,
        "font" => RequestResourceType::Font,
        "media" => RequestResourceType::Media,
        "xhr" => RequestResourceType::Xhr,
        "fetch" => RequestResourceType::Fetch,
        "websocket" => RequestResourceType::WebSocket,
        "manifest" => RequestResourceType::Manifest,
        "other" => RequestResourceType::Other,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(url: &str, resource_type: RequestResourceType) -> RequestPolicyRequest {
        RequestPolicyRequest {
            registration_id: AD_BLOCK_PROFILE_ID.to_string(),
            metadata: RequestPolicyMetadata {
                context_id: BrowserContextId::new("ctx-a"),
                page_id: Some(PageId::new("page-a")),
                url: url.to_string(),
                method: "GET".to_string(),
                resource_type,
                initiator_origin: Some("http://site.test".to_string()),
                top_level_origin: Some("http://site.test".to_string()),
            },
            deadline_ms: 250,
        }
    }

    #[test]
    fn ordered_host_rule_blocks_and_allow_exception_is_deterministic() {
        let policy = AdblockPolicy::from_text(
            "allow exception host=ads.example.test url=/safe.js\nblock tracker host=example.test resource=script",
        )
        .expect("mini rule list must parse");
        let allowed = policy
            .evaluate(&request(
                "http://ads.example.test/safe.js",
                RequestResourceType::Script,
            ))
            .expect("request must evaluate");
        let blocked = policy
            .evaluate(&request(
                "http://ads.example.test/tracker.js",
                RequestResourceType::Script,
            ))
            .expect("request must evaluate");
        assert_eq!(allowed.action, RequestPolicyAction::Allow);
        assert_eq!(allowed.opaque_rule_ref.as_deref(), Some("exception"));
        assert_eq!(blocked.action, RequestPolicyAction::Block);
        assert_eq!(blocked.opaque_rule_ref.as_deref(), Some("tracker"));
    }

    #[test]
    fn malformed_and_oversized_rule_lists_are_rejected() {
        assert!(AdblockPolicy::from_text("block rule host=ads.test unknown=x").is_err());
        assert!(AdblockPolicy::from_text("block rule host=").is_err());
        assert!(AdblockPolicy::from_text(&"#".repeat(MAX_RULE_LIST_BYTES + 1)).is_err());
        let rules = (0..=MAX_RULES)
            .map(|index| AdblockRule {
                rule_id: format!("rule-{index}"),
                action: AdblockAction::Block,
                host: Some("ads.test".to_string()),
                url_contains: None,
                resource_type: None,
            })
            .collect();
        assert!(matches!(
            AdblockPolicy::from_rules(rules),
            Err(AdblockError::TooManyRules { .. })
        ));
    }

    #[test]
    fn profile_registration_declares_fail_open_without_changing_generic_broker() {
        let registration = AdblockPolicy::fail_open_registration(
            BrowserContextId::new("ctx-a"),
            Some(PageId::new("page-a")),
            8,
        );
        assert_eq!(
            registration.failure_mode,
            RequestPolicyFailureMode::FailOpen
        );
        assert_eq!(registration.provider_id, AD_BLOCK_PROVIDER_ID);
        assert!(registration.validate().is_ok());
        assert!(
            AD_BLOCK_PROVIDER_ID.len()
                <= worldline_browser_contract::request_policy::MAX_REQUEST_POLICY_PROVIDER_ID_BYTES
        );
    }
}
