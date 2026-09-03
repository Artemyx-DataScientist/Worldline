//! S3C request-policy proving slice.
//!
//! S3C is deliberately separate from S3B. The reference path proves broker
//! scope/lifecycle semantics deterministically; the hosted path below proves
//! that the real CEF provider sends the neutral request contract and applies
//! the replaceable adblock profile before the origin sees a blocked resource.

use std::sync::Arc;
use std::time::Duration;

use worldline_browser_adblock::{AD_BLOCK_PROVIDER_ID, AdblockPolicy};
use worldline_browser_contract::authority::{BrowserAuthority, BrowserAuthoritySet};
use worldline_browser_contract::identity::{BrowserContextId, PageId};
use worldline_browser_contract::request_policy::{
    RequestPolicyAction, RequestPolicyFailureMode, RequestPolicyMetadata, RequestPolicyObservation,
    RequestPolicyOutcome, RequestPolicyRegistration, RequestPolicyRequest, RequestPolicyResult,
    RequestResourceType,
};
use worldline_browser_provider::{
    RequestPolicyBroker, RequestPolicyBrokerLimits, RequestPolicyCaller, RequestPolicyCancellation,
    RequestPolicyEvaluator, RequestPolicyEvaluatorError,
};

/// Report for the S3C deterministic or hosted proving slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3CReport {
    pub topology: String,
    pub blocked_origin_hits: usize,
    pub allowed_origin_hits: usize,
    pub page_usable: bool,
    pub exact_scope_isolated: bool,
    pub replacement_isolated: bool,
    pub lifecycle_cleanup: bool,
    pub fail_open_timeout: bool,
    pub fail_open_unavailable: bool,
    pub safe_observations: bool,
    pub accepted: bool,
}

fn caller(
    context_id: &str,
    page_id: Option<&str>,
    decide: bool,
    observe: bool,
) -> RequestPolicyCaller {
    let mut authorities = BrowserAuthoritySet::new();
    if decide {
        authorities.grant(BrowserAuthority::DecideRequestPolicy);
    }
    if observe {
        authorities.grant(BrowserAuthority::ObserveRequestPolicy);
    }
    RequestPolicyCaller::new(
        authorities,
        BrowserContextId::new(context_id),
        page_id.map(PageId::new),
    )
}

fn request(
    registration_id: &str,
    context_id: &str,
    page_id: &str,
    url: &str,
) -> RequestPolicyRequest {
    RequestPolicyRequest {
        registration_id: registration_id.to_string(),
        metadata: RequestPolicyMetadata {
            context_id: BrowserContextId::new(context_id),
            page_id: Some(PageId::new(page_id)),
            url: url.to_string(),
            method: "GET".to_string(),
            resource_type: RequestResourceType::Script,
            initiator_origin: Some("http://127.0.0.1".to_string()),
            top_level_origin: Some("http://127.0.0.1".to_string()),
        },
        deadline_ms: 250,
    }
}

struct UnavailableAdblock;

impl RequestPolicyEvaluator for UnavailableAdblock {
    fn provider_id(&self) -> &str {
        AD_BLOCK_PROVIDER_ID
    }

    fn decide(
        &self,
        _request: &RequestPolicyRequest,
        _cancellation: &RequestPolicyCancellation,
    ) -> Result<RequestPolicyResult, RequestPolicyEvaluatorError> {
        Err(RequestPolicyEvaluatorError::Unavailable(
            "fixture profile unloaded".to_string(),
        ))
    }
}

struct SlowAdblock;

impl RequestPolicyEvaluator for SlowAdblock {
    fn provider_id(&self) -> &str {
        AD_BLOCK_PROVIDER_ID
    }

    fn decide(
        &self,
        _request: &RequestPolicyRequest,
        cancellation: &RequestPolicyCancellation,
    ) -> Result<RequestPolicyResult, RequestPolicyEvaluatorError> {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_millis(40) {
            if cancellation.is_cancelled() {
                return Err(RequestPolicyEvaluatorError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(RequestPolicyResult {
            action: RequestPolicyAction::Block,
            outcome: RequestPolicyOutcome::Evaluated,
            provider_id: None,
            opaque_rule_ref: None,
        })
    }
}

/// Runs S3C's deterministic broker/evaluator fixture. It is not real CEF
/// evidence and is intentionally reported as a separate topology.
pub fn run_reference() -> Result<S3CReport, String> {
    let broker = RequestPolicyBroker::with_limits(RequestPolicyBrokerLimits {
        max_registrations: 8,
        max_total_in_flight: 8,
        max_observations: 32,
    });
    let owner = caller("s3c-context-a", Some("s3c-page-a"), true, false);
    let observer = caller("s3c-context-a", Some("s3c-page-a"), false, true);
    let policy =
        AdblockPolicy::from_text("block tracker host=127.0.0.1 url=/blocked.js resource=script")
            .map_err(|error| error.to_string())?;
    let registration = AdblockPolicy::fail_open_registration(
        BrowserContextId::new("s3c-context-a"),
        Some(PageId::new("s3c-page-a")),
        8,
    );
    broker
        .register(registration.clone(), Arc::new(policy.clone()), &owner)
        .map_err(|error| error.to_string())?;
    let blocked = broker
        .decide(
            &owner,
            request(
                &registration.registration_id,
                "s3c-context-a",
                "s3c-page-a",
                "http://127.0.0.1/blocked.js",
            ),
        )
        .map_err(|error| error.to_string())?;
    let allowed = broker
        .decide(
            &owner,
            request(
                &registration.registration_id,
                "s3c-context-a",
                "s3c-page-a",
                "http://127.0.0.1/allowed.js",
            ),
        )
        .map_err(|error| error.to_string())?;
    let blocked_origin_hits = usize::from(blocked.action == RequestPolicyAction::Allow);
    let allowed_origin_hits = usize::from(allowed.action == RequestPolicyAction::Allow);
    let observations: Vec<RequestPolicyObservation> = broker
        .drain_observations(&observer)
        .map_err(|error| error.to_string())?;
    let safe_observations = observations.iter().all(|observation| {
        let value = serde_json::to_value(observation).unwrap_or_default();
        value.get("url").is_none() && value.get("headers").is_none()
    });

    let foreign = broker.decide(
        &caller("s3c-context-b", Some("s3c-page-b"), true, false),
        request(
            &registration.registration_id,
            "s3c-context-a",
            "s3c-page-a",
            "http://127.0.0.1/blocked.js",
        ),
    );
    let exact_scope_isolated = foreign.is_err();

    broker
        .unregister(&registration.registration_id, &owner)
        .map_err(|error| error.to_string())?;
    let replacement = AdblockPolicy::from_rules(Vec::new()).map_err(|error| error.to_string())?;
    broker
        .register(registration.clone(), Arc::new(replacement), &owner)
        .map_err(|error| error.to_string())?;
    let replacement_result = broker
        .decide(
            &owner,
            request(
                &registration.registration_id,
                "s3c-context-a",
                "s3c-page-a",
                "http://127.0.0.1/blocked.js",
            ),
        )
        .map_err(|error| error.to_string())?;
    let replacement_isolated = replacement_result.action == RequestPolicyAction::Allow;

    broker.invalidate_page(&PageId::new("s3c-page-a"));
    let lifecycle_cleanup = broker
        .decide(
            &owner,
            request(
                &registration.registration_id,
                "s3c-context-a",
                "s3c-page-a",
                "http://127.0.0.1/allowed.js",
            ),
        )
        .is_err();

    let unavailable_registration = RequestPolicyRegistration {
        registration_id: "s3c-unavailable".to_string(),
        context_id: BrowserContextId::new("s3c-context-a"),
        page_id: Some(PageId::new("s3c-page-a")),
        failure_mode: RequestPolicyFailureMode::FailOpen,
        max_in_flight: 1,
        provider_id: AD_BLOCK_PROVIDER_ID.to_string(),
    };
    let timeout_registration = RequestPolicyRegistration {
        registration_id: "s3c-timeout".to_string(),
        context_id: BrowserContextId::new("s3c-context-a"),
        page_id: Some(PageId::new("s3c-page-a")),
        failure_mode: RequestPolicyFailureMode::FailOpen,
        max_in_flight: 1,
        provider_id: AD_BLOCK_PROVIDER_ID.to_string(),
    };
    // These registrations use a fresh broker because page invalidation above
    // intentionally proves stale registration rejection.
    let degraded_broker = RequestPolicyBroker::new();
    degraded_broker
        .register(
            unavailable_registration.clone(),
            Arc::new(UnavailableAdblock),
            &owner,
        )
        .map_err(|error| error.to_string())?;
    degraded_broker
        .register(timeout_registration.clone(), Arc::new(SlowAdblock), &owner)
        .map_err(|error| error.to_string())?;
    let unavailable = degraded_broker
        .decide(
            &owner,
            request(
                &unavailable_registration.registration_id,
                "s3c-context-a",
                "s3c-page-a",
                "http://127.0.0.1/unavailable.js",
            ),
        )
        .map_err(|error| error.to_string())?;
    let mut timeout_request = request(
        &timeout_registration.registration_id,
        "s3c-context-a",
        "s3c-page-a",
        "http://127.0.0.1/slow.js",
    );
    timeout_request.deadline_ms = 5;
    let timeout = degraded_broker
        .decide(&owner, timeout_request)
        .map_err(|error| error.to_string())?;

    let report = S3CReport {
        topology: "deterministic loopback contract/broker/evaluator; no CEF".to_string(),
        blocked_origin_hits,
        allowed_origin_hits,
        page_usable: allowed.action == RequestPolicyAction::Allow,
        exact_scope_isolated,
        replacement_isolated,
        lifecycle_cleanup,
        fail_open_timeout: timeout.action == RequestPolicyAction::Allow
            && timeout.outcome == RequestPolicyOutcome::DeadlineExceeded,
        fail_open_unavailable: unavailable.action == RequestPolicyAction::Allow
            && unavailable.outcome == RequestPolicyOutcome::FailureFallback,
        safe_observations,
        accepted: blocked.action == RequestPolicyAction::Block
            && blocked_origin_hits == 0
            && allowed_origin_hits == 1
            && allowed.action == RequestPolicyAction::Allow
            && exact_scope_isolated
            && replacement_isolated
            && lifecycle_cleanup
            && timeout.action == RequestPolicyAction::Allow
            && timeout.outcome == RequestPolicyOutcome::DeadlineExceeded
            && unavailable.action == RequestPolicyAction::Allow
            && unavailable.outcome == RequestPolicyOutcome::FailureFallback
            && safe_observations,
    };
    Ok(report)
}

#[cfg(windows)]
pub fn run() -> Result<S3CReport, String> {
    real::run()
}

#[cfg(not(windows))]
pub fn run() -> Result<S3CReport, String> {
    Err("S3C-real requires the hosted Windows CEF runtime".to_string())
}

#[cfg(windows)]
mod real {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use worldline_browser_adblock::{AD_BLOCK_PROFILE_ID, AdblockPolicy};
    use worldline_browser_contract::authority::{OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_OBSERVE};
    use worldline_browser_contract::contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        LoadingState, ObservePageRequest, PageObservation,
    };
    use worldline_browser_contract::identity::PageId;
    use worldline_browser_contract::request_policy::{
        RequestPolicyAction, RequestPolicyObservation, RequestPolicyRequest, RequestPolicyResult,
    };
    use worldline_native_host::{
        ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError,
        NativeProviderConnection,
    };
    use worldline_plugin_protocol::{MessageKind, REQUEST_POLICY_INTERFACE};

    use crate::real_cef_lock::RealCefRunGuard;

    use super::S3CReport;

    const SCRIPT_COUNT: usize = 4;
    const SLOW_REQUEST_DELAY: Duration = Duration::from_millis(400);
    const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
    const MAX_POLICY_IN_FLIGHT: usize = 8;

    struct LoopbackServer {
        base_url: String,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
        hits: Arc<Mutex<Vec<String>>>,
    }

    impl LoopbackServer {
        fn start(nonce: &str) -> Result<Self, String> {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .map_err(|error| format!("bind S3C loopback server: {error}"))?;
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("configure S3C loopback server: {error}"))?;
            let address = listener
                .local_addr()
                .map_err(|error| format!("read S3C loopback address: {error}"))?;
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let connections = Arc::new(Mutex::new(Vec::new()));
            let worker_connections = Arc::clone(&connections);
            let hits = Arc::new(Mutex::new(Vec::new()));
            let worker_hits = Arc::clone(&hits);
            let index_body = Arc::new(index_body(nonce));
            let worker_index_body = Arc::clone(&index_body);
            let worker = thread::Builder::new()
                .name("worldline-s3c-loopback".to_string())
                .spawn(move || {
                    while !worker_stop.load(Ordering::SeqCst) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let body = Arc::clone(&worker_index_body);
                                let hits = Arc::clone(&worker_hits);
                                let connection = thread::Builder::new()
                                    .name("worldline-s3c-http".to_string())
                                    .spawn(move || serve_connection(stream, body, hits));
                                if let Ok(connection) = connection {
                                    worker_connections
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                                        .push(connection);
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(error)
                                if error.kind() == std::io::ErrorKind::ConnectionAborted
                                    || error.kind() == std::io::ErrorKind::ConnectionReset =>
                            {
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(error) => {
                                eprintln!("S3C loopback server accept error: {error:?}");
                                break;
                            }
                        }
                    }
                })
                .map_err(|error| format!("start S3C loopback server: {error}"))?;

            Ok(Self {
                base_url: format!("http://{address}"),
                stop,
                worker: Some(worker),
                connections,
                hits,
            })
        }

        fn clear_hits(&self) {
            self.hits
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
        }

        fn snapshot_hits(&self) -> Vec<String> {
            self.hits
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    impl Drop for LoopbackServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let connections = std::mem::take(
                &mut *self
                    .connections
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
            for connection in connections {
                let _ = connection.join();
            }
        }
    }

    fn index_body(nonce: &str) -> String {
        let mut body = String::from(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Worldline S3C</title></head><body><h1>Worldline S3C</h1>",
        );
        for index in 0..SCRIPT_COUNT {
            body.push_str(&format!(
                "<script src=\"/active-{index}.js?run={nonce}\"></script>"
            ));
        }
        body.push_str(&format!(
            "<script src=\"/allowed.js?run={nonce}\"></script><script src=\"/blocked.js?run={nonce}\"></script><script src=\"/slow.js?run={nonce}\"></script><script src=\"/unavailable.js?run={nonce}\"></script></body></html>"
        ));
        body
    }

    fn serve_connection(
        mut stream: TcpStream,
        index_body: Arc<String>,
        hits: Arc<Mutex<Vec<String>>>,
    ) {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut request = Vec::with_capacity(4096);
        let mut chunk = [0_u8; 1024];
        while request.len() < 32 * 1024 {
            let Ok(read) = stream.read(&mut chunk) else {
                return;
            };
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        if request.is_empty() {
            return;
        }
        let request = String::from_utf8_lossy(&request);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|target| target.split('?').next())
            .unwrap_or("/")
            .to_string();
        hits.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(path.clone());
        let (status, content_type, body): (&str, &str, &[u8]) = if path == "/index.html" {
            ("200 OK", "text/html; charset=utf-8", index_body.as_bytes())
        } else if path.starts_with("/active-")
            && path.ends_with(".js")
            && path.len() > "/active-.js".len()
        {
            (
                "200 OK",
                "text/javascript; charset=utf-8",
                b"window.__worldline_s3c_active = true;",
            )
        } else if matches!(
            path.as_str(),
            "/allowed.js" | "/blocked.js" | "/slow.js" | "/unavailable.js"
        ) {
            (
                "200 OK",
                "text/javascript; charset=utf-8",
                b"window.__worldline_s3c_resource = true;",
            )
        } else {
            ("404 Not Found", "text/plain", b"not found")
        };
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(headers.as_bytes());
        let _ = stream.write_all(body);
    }

    struct PolicyMetrics {
        request_count: AtomicUsize,
        block_results: AtomicUsize,
        allow_results: AtomicUsize,
        timeout_candidates: AtomicUsize,
        unavailable_failures: AtomicUsize,
        current: AtomicUsize,
        max_in_flight: AtomicUsize,
        latencies_us: Mutex<Vec<u64>>,
        scopes: Mutex<Vec<(String, String, String, String)>>,
        observations: Mutex<Vec<RequestPolicyObservation>>,
    }

    impl PolicyMetrics {
        fn new() -> Self {
            Self {
                request_count: AtomicUsize::new(0),
                block_results: AtomicUsize::new(0),
                allow_results: AtomicUsize::new(0),
                timeout_candidates: AtomicUsize::new(0),
                unavailable_failures: AtomicUsize::new(0),
                current: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                latencies_us: Mutex::new(Vec::new()),
                scopes: Mutex::new(Vec::new()),
                observations: Mutex::new(Vec::new()),
            }
        }
    }

    struct S3CHostSink {
        policy: Option<Arc<AdblockPolicy>>,
        metrics: Arc<PolicyMetrics>,
    }

    impl HostRequestSink for S3CHostSink {
        fn on_child_request(
            &self,
            kind: MessageKind,
            _correlation_id: u64,
            payload: Value,
        ) -> Result<Option<Value>, NativeHostError> {
            match kind {
                MessageKind::RequestPolicyRequest => {
                    let request: RequestPolicyRequest =
                        serde_json::from_value(payload).map_err(|error| {
                            NativeHostError::ProtocolViolation {
                                reason: format!("invalid S3C policy request: {error}"),
                            }
                        })?;
                    request
                        .validate()
                        .map_err(|reason| NativeHostError::ProtocolViolation {
                            reason: format!("invalid S3C policy request: {reason}"),
                        })?;
                    let started = Instant::now();
                    self.metrics.request_count.fetch_add(1, Ordering::SeqCst);
                    let current = self.metrics.current.fetch_add(1, Ordering::SeqCst) + 1;
                    self.metrics
                        .max_in_flight
                        .fetch_max(current, Ordering::SeqCst);
                    let page_id = request
                        .metadata
                        .page_id
                        .as_ref()
                        .map(|page| page.as_str().to_string())
                        .unwrap_or_default();
                    self.metrics
                        .scopes
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push((
                            request.registration_id.clone(),
                            request.metadata.context_id.as_str().to_string(),
                            page_id,
                            request.metadata.url.clone(),
                        ));

                    let result = if request.metadata.url.contains("/slow.js") {
                        self.metrics
                            .timeout_candidates
                            .fetch_add(1, Ordering::SeqCst);
                        thread::sleep(SLOW_REQUEST_DELAY);
                        self.evaluate(&request)
                    } else if request.metadata.url.contains("/unavailable.js") {
                        self.metrics
                            .unavailable_failures
                            .fetch_add(1, Ordering::SeqCst);
                        Err(NativeHostError::ProtocolViolation {
                            reason: "S3C adblock profile is unavailable for this request"
                                .to_string(),
                        })
                    } else {
                        self.evaluate(&request)
                    };
                    self.metrics.current.fetch_sub(1, Ordering::SeqCst);
                    self.metrics
                        .latencies_us
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(started.elapsed().as_micros() as u64);

                    match result {
                        Ok(result) => {
                            match result.action {
                                RequestPolicyAction::Allow => {
                                    self.metrics.allow_results.fetch_add(1, Ordering::SeqCst);
                                }
                                RequestPolicyAction::Block => {
                                    self.metrics.block_results.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                            let observation = RequestPolicyObservation {
                                registration_id: request.registration_id,
                                context_id: request.metadata.context_id,
                                page_id: request.metadata.page_id,
                                action: result.action,
                                outcome: result.outcome,
                                provider_id: result.provider_id.clone(),
                                opaque_rule_ref: result.opaque_rule_ref.clone(),
                                latency_ms: started.elapsed().as_millis() as u64,
                            };
                            self.metrics
                                .observations
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .push(observation);
                            serde_json::to_value(result).map(Some).map_err(|error| {
                                NativeHostError::ProtocolViolation {
                                    reason: format!("encode S3C policy result: {error}"),
                                }
                            })
                        }
                        Err(error) => Err(error),
                    }
                }
                // The S3C fixture has no event result dependency. Keeping the
                // event branch explicit proves that event publication is not
                // used as the request-policy decision channel.
                MessageKind::EventPublishRequest => Ok(None),
                other => Err(NativeHostError::ProtocolViolation {
                    reason: format!("unexpected S3C child request: {other:?}"),
                }),
            }
        }
    }

    impl S3CHostSink {
        fn evaluate(
            &self,
            request: &RequestPolicyRequest,
        ) -> Result<RequestPolicyResult, NativeHostError> {
            let policy =
                self.policy
                    .as_ref()
                    .ok_or_else(|| NativeHostError::ProtocolViolation {
                        reason: "S3C received a policy request without an active profile"
                            .to_string(),
                    })?;
            if request.registration_id != AD_BLOCK_PROFILE_ID {
                return Err(NativeHostError::ProtocolViolation {
                    reason: format!(
                        "unexpected S3C request-policy registration '{}'",
                        request.registration_id
                    ),
                });
            }
            policy
                .evaluate(request)
                .map_err(|error| NativeHostError::ProtocolViolation {
                    reason: format!("S3C adblock evaluation failed: {error}"),
                })
        }
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn create() -> Result<Self, String> {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("clock before Unix epoch: {error}"))?
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("worldline-s3c-real-{}-{stamp}", std::process::id()));
            std::fs::create_dir_all(&path)
                .map_err(|error| format!("create S3C temporary root: {error}"))?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct RealRun {
        page_usable: bool,
        hits: Vec<String>,
        scopes: Vec<(String, String, String, String)>,
        block_results: usize,
        allow_results: usize,
        timeout_candidates: usize,
        unavailable_failures: usize,
        safe_observations: bool,
        lifecycle_terminated: bool,
    }

    struct RealRunConfiguration {
        policy: Option<Arc<AdblockPolicy>>,
        expect_blocked_origin: bool,
        label: &'static str,
    }

    fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    fn call_contract_op(
        connection: &NativeProviderConnection,
        contract: &str,
        operation: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let response = connection
            .call_with_deadline(
                serde_json::json!({
                    "contract": contract,
                    "operation": operation,
                    "payload": payload,
                }),
                Duration::from_secs(5),
            )
            .map_err(|error| {
                let stderr = connection.stderr_text();
                if stderr.trim().is_empty() {
                    format!("S3C native call '{contract}/{operation}' failed: {error}")
                } else {
                    format!("S3C native call '{contract}/{operation}' failed: {error}; stderr:\n{stderr}")
                }
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!(
                "S3C provider operation '{contract}/{operation}' failed: {error}"
            ));
        }
        response.get("result").cloned().ok_or_else(|| {
            format!("S3C provider operation '{contract}/{operation}' omitted result")
        })
    }

    fn call_op(
        connection: &NativeProviderConnection,
        operation: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let response = connection
            .call_with_deadline(
                serde_json::json!({"operation": operation, "payload": payload}),
                Duration::from_secs(5),
            )
            .map_err(|error| {
                let stderr = connection.stderr_text();
                if stderr.trim().is_empty() {
                    format!("S3C native call '{operation}' failed: {error}")
                } else {
                    format!("S3C native call '{operation}' failed: {error}; stderr:\n{stderr}")
                }
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!(
                "S3C provider operation '{operation}' failed: {error}"
            ));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("S3C provider operation '{operation}' omitted result"))
    }

    fn wait_for_page(
        connection: &NativeProviderConnection,
        page_id: &PageId,
        timeout: Duration,
    ) -> Result<PageObservation, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let observation: PageObservation = decode(call_op(
                connection,
                OP_OBSERVE,
                serde_json::to_value(ObservePageRequest {
                    page_id: page_id.clone(),
                })
                .map_err(|error| error.to_string())?,
            )?)?;
            match observation.loading_state {
                LoadingState::Complete => return Ok(observation),
                LoadingState::Failed => {
                    return Err(format!(
                        "S3C CEF page '{page_id}' failed to load '{}'; stderr:\n{}",
                        observation.url,
                        connection.stderr_text()
                    ));
                }
                LoadingState::Unloaded | LoadingState::Loading | LoadingState::Interactive => {}
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for S3C page '{page_id}'"));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_paths(
        server: &LoopbackServer,
        expected: &[String],
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let hits = server.snapshot_hits();
            if expected
                .iter()
                .all(|path| hits.iter().any(|hit| hit == path))
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for S3C origin paths {expected:?}; observed {hits:?}"
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn discover_provider_process() -> Result<PathBuf, String> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("WORLDLINE_BROWSER_PROVIDER_BOOTSTRAP") {
            candidates.push(PathBuf::from(path));
        }
        if let Ok(current_exe) = std::env::current_exe()
            && let Some(deps) = current_exe.parent()
            && let Some(target_debug) = deps.parent()
        {
            candidates.push(target_debug.join("bootstrapc.exe"));
        }
        if let Some(target_dir) = std::env::var_os("CARGO_TARGET_DIR") {
            candidates.push(
                PathBuf::from(target_dir)
                    .join("debug")
                    .join("bootstrapc.exe"),
            );
        }
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
                .join("debug")
                .join("bootstrapc.exe"),
        );
        candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                "pinned CEF bootstrapc.exe is missing; stage the verified CEF runtime before S3C-real"
                    .to_string()
            })
    }

    fn discover_provider_client(bootstrap: &Path) -> Result<PathBuf, String> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("WORLDLINE_BROWSER_PROVIDER_CLIENT") {
            candidates.push(PathBuf::from(path));
        }
        if let Some(parent) = bootstrap.parent() {
            candidates.push(parent.join("worldline_browser_provider_client.dll"));
        }
        let client = candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                "worldline_browser_provider_client.dll is missing; stage the CEF provider client before S3C-real"
                    .to_string()
            })?;
        if client.parent() != bootstrap.parent() {
            return Err(format!(
                "CEF bootstrap client '{}' must be staged beside '{}'",
                client.display(),
                bootstrap.display()
            ));
        }
        Ok(client)
    }

    fn run_one(
        program: &Path,
        client: &Path,
        server: &LoopbackServer,
        cache_root: &Path,
        configuration: RealRunConfiguration,
        nonce: &str,
    ) -> Result<RealRun, String> {
        let metrics = Arc::new(PolicyMetrics::new());
        let sink = Arc::new(S3CHostSink {
            policy: configuration.policy,
            metrics: Arc::clone(&metrics),
        });
        let identity = ExpectedIdentity {
            package_id: "worldline.browser.pkg".to_string(),
            plugin_definition_id: "worldline.browser.provider".to_string(),
        };
        let client_name = client
            .file_name()
            .ok_or_else(|| format!("CEF provider client has no file name: {}", client.display()))?;
        let mut child_args = vec![
            format!("--module={}", client_name.to_string_lossy()),
            "--disable-gpu".to_string(),
            "--in-process-gpu".to_string(),
            "--package-id".to_string(),
            identity.package_id.clone(),
            "--definition-id".to_string(),
            identity.plugin_definition_id.clone(),
            "--backend".to_string(),
            "cef".to_string(),
            "--cache-root".to_string(),
            cache_root.to_string_lossy().into_owned(),
        ];
        if sink.policy.is_some() {
            child_args.extend([
                "--request-policy-profile".to_string(),
                AD_BLOCK_PROFILE_ID.to_string(),
                "--request-policy-failure-mode".to_string(),
                "fail-open".to_string(),
            ]);
        }
        let spec = NativeChildSpec::new(
            program.to_path_buf(),
            child_args,
            MAX_FRAME_BYTES,
            64 * 1024,
        );
        let (connection, ack) = NativeProviderConnection::connect_with_required_interface(
            spec,
            &identity,
            Arc::clone(&sink) as Arc<dyn HostRequestSink>,
            MAX_POLICY_IN_FLIGHT,
            REQUEST_POLICY_INTERFACE,
        )
        .map_err(|error| format!("connect S3C native provider: {error}"))?;
        if !ack.supports_interface(REQUEST_POLICY_INTERFACE) {
            return Err("S3C provider did not negotiate request-policy interface".to_string());
        }

        let page_url = format!("{}/index.html?run={nonce}", server.base_url);
        let context: CreateContextResponse = decode(call_contract_op(
            &connection,
            "browser.context",
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some(format!("s3c-{}-{nonce}", configuration.label)),
                incognito: true,
                user_agent: None,
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let page: CreatePageResponse = decode(call_contract_op(
            &connection,
            "browser.page",
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: context.context_id,
                initial_url: Some(page_url),
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let observation = wait_for_page(&connection, &page.page_id, Duration::from_secs(15))?;
        // The active-* scripts are useful page content but are not the
        // acceptance trigger: Chromium may coalesce or skip one script after
        // a prior navigation error. The policy-specific resources below are
        // the mandatory proving paths.
        let mut expected = vec![
            "/allowed.js".to_string(),
            "/slow.js".to_string(),
            "/unavailable.js".to_string(),
        ];
        if sink.policy.is_none() || configuration.expect_blocked_origin {
            expected.push("/blocked.js".to_string());
        }
        wait_for_paths(server, &expected, Duration::from_secs(15))?;
        // The slow host response is deliberately later than the 250 ms policy
        // deadline. Give its cancelled host worker time to finish before
        // taking the report and closing the provider.
        thread::sleep(Duration::from_millis(450));
        let hits = server.snapshot_hits();
        let scopes = metrics
            .scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let observations = metrics
            .observations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let safe_observations = observations.iter().all(|observation| {
            let value = serde_json::to_value(observation).unwrap_or_default();
            value.get("url").is_none() && value.get("headers").is_none()
        });
        let shutdown_result = connection.close(Duration::from_secs(15));
        // `NativeChild::shutdown` kills and waits for the contained process
        // when CEF does not exit within the bounded deadline. For this slice
        // lifecycle cleanup means that the child is definitely terminated;
        // graceful versus deadline-forced shutdown remains visible in the
        // local diagnostic result.
        let lifecycle_terminated = shutdown_result.is_ok() || connection.try_status().is_some();
        Ok(RealRun {
            page_usable: observation.loading_state == LoadingState::Complete
                && hits.iter().any(|path| path == "/allowed.js"),
            hits,
            scopes,
            block_results: metrics.block_results.load(Ordering::SeqCst),
            allow_results: metrics.allow_results.load(Ordering::SeqCst),
            timeout_candidates: metrics.timeout_candidates.load(Ordering::SeqCst),
            unavailable_failures: metrics.unavailable_failures.load(Ordering::SeqCst),
            safe_observations,
            lifecycle_terminated,
        })
    }

    fn scope_is_consistent(run: &RealRun) -> bool {
        let Some((registration, context, page, _)) = run.scopes.first() else {
            return false;
        };
        !context.is_empty()
            && !page.is_empty()
            && registration == AD_BLOCK_PROFILE_ID
            && run
                .scopes
                .iter()
                .all(|(other_registration, other_context, other_page, _)| {
                    other_registration == registration
                        && other_context == context
                        && other_page == page
                })
    }

    /// Runs no-policy, active adblock, and replacement-profile sessions
    /// through the pinned native CEF provider. The real path never falls back
    /// to [`run_reference`].
    pub fn run() -> Result<S3CReport, String> {
        let _real_cef_guard = RealCefRunGuard::acquire()?;
        let program = discover_provider_process()?;
        let client = discover_provider_client(&program)?;
        let temp_root = TempRoot::create()?;
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos()
        );
        let server = LoopbackServer::start(&nonce)?;

        let baseline = run_one(
            &program,
            &client,
            &server,
            &temp_root.path().join("baseline-cef"),
            RealRunConfiguration {
                policy: None,
                expect_blocked_origin: true,
                label: "baseline",
            },
            &nonce,
        )?;
        let baseline_blocked_hit = baseline.hits.iter().any(|path| path == "/blocked.js");

        server.clear_hits();
        let active_policy =
            AdblockPolicy::from_text("block s3c-blocked-resource host=127.0.0.1 url=/blocked.js")
                .map_err(|error| format!("build S3C active adblock profile: {error}"))?;
        let active = run_one(
            &program,
            &client,
            &server,
            &temp_root.path().join("active-cef"),
            RealRunConfiguration {
                policy: Some(Arc::new(active_policy)),
                expect_blocked_origin: false,
                label: "active",
            },
            &nonce,
        )?;
        let active_blocked_hits = active
            .hits
            .iter()
            .filter(|path| path.as_str() == "/blocked.js")
            .count();
        let active_allowed_hits = active
            .hits
            .iter()
            .filter(|path| path.as_str() == "/allowed.js")
            .count();

        server.clear_hits();
        let replacement_policy = AdblockPolicy::from_rules(Vec::new())
            .map_err(|error| format!("build S3C replacement adblock profile: {error}"))?;
        let replacement = run_one(
            &program,
            &client,
            &server,
            &temp_root.path().join("replacement-cef"),
            RealRunConfiguration {
                policy: Some(Arc::new(replacement_policy)),
                expect_blocked_origin: true,
                label: "replacement",
            },
            &nonce,
        )?;
        let replacement_blocked_hits = replacement
            .hits
            .iter()
            .filter(|path| path.as_str() == "/blocked.js")
            .count();

        let exact_scope_isolated =
            scope_is_consistent(&active) && scope_is_consistent(&replacement);
        let replacement_isolated = active.block_results >= 1
            && active_blocked_hits == 0
            && replacement.block_results == 0
            && replacement_blocked_hits >= 1;
        let fail_open_timeout = active.timeout_candidates >= 1
            && active.hits.iter().any(|path| path == "/slow.js")
            && active.page_usable;
        let fail_open_unavailable = active.unavailable_failures >= 1
            && active.hits.iter().any(|path| path == "/unavailable.js")
            && active.page_usable;
        let safe_observations = active.safe_observations && replacement.safe_observations;
        let lifecycle_cleanup = active.lifecycle_terminated && replacement.lifecycle_terminated;
        let accepted = baseline_blocked_hit
            && active_blocked_hits == 0
            && active_allowed_hits >= 1
            && active.allow_results >= 1
            && active.page_usable
            && exact_scope_isolated
            && replacement_isolated
            && lifecycle_cleanup
            && fail_open_timeout
            && fail_open_unavailable
            && safe_observations;

        Ok(S3CReport {
            topology: format!(
                "real CEF 151.8.0 -> native provider process -> negotiated {REQUEST_POLICY_INTERFACE}; adblock profile={AD_BLOCK_PROFILE_ID}; bootstrap={}; client={}",
                program.display(),
                client.display()
            ),
            blocked_origin_hits: active_blocked_hits,
            allowed_origin_hits: active_allowed_hits,
            page_usable: active.page_usable,
            exact_scope_isolated,
            replacement_isolated,
            lifecycle_cleanup,
            fail_open_timeout,
            fail_open_unavailable,
            safe_observations,
            accepted,
        })
    }
}
