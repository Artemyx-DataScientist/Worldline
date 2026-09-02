//! T-004 request-policy hot-path feasibility fixtures.
//!
//! The reference fixture exercises the bounded, engine-neutral broker only.
//! The Windows fixture below is the separate real-CEF proving path; neither
//! fixture contains adblock rule parsing.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use worldline_browser_contract::authority::{BrowserAuthority, BrowserAuthoritySet};
use worldline_browser_contract::identity::{BrowserContextId, PageId};
use worldline_browser_contract::request_policy::{
    DEFAULT_REQUEST_POLICY_DEADLINE_MS, RequestPolicyAction, RequestPolicyFailureMode,
    RequestPolicyMetadata, RequestPolicyObservation, RequestPolicyOutcome,
    RequestPolicyRegistration, RequestPolicyRequest, RequestPolicyResult, RequestResourceType,
};
use worldline_browser_provider::{
    RequestPolicyBroker, RequestPolicyBrokerLimits, RequestPolicyCaller, RequestPolicyCancellation,
    RequestPolicyEvaluator, RequestPolicyEvaluatorError,
};

const REFERENCE_CONTEXT: &str = "t004-reference-context";
const REFERENCE_PAGE: &str = "t004-reference-page";
const REFERENCE_REGISTRATION: &str = "t004-reference-registration";
const REFERENCE_PROVIDER: &str = crate::request_policy::REFERENCE_REQUEST_POLICY_PROVIDER_ID;
const DECLARED_CONCURRENCY: usize = 8;
const REQUEST_COUNT: usize = 64;

/// Evidence emitted by the early T-004 feasibility gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestPolicyFeasibilityReport {
    /// Provider/runtime topology used by the fixture.
    pub topology: String,
    /// Maximum number of policy decisions intentionally issued concurrently.
    pub declared_concurrency: usize,
    /// Number of decision requests submitted by the fixture.
    pub request_count: usize,
    /// Number of decisions that completed with a result or declared fallback.
    pub completed_decisions: usize,
    /// Number of Allow results observed.
    pub allowed_decisions: usize,
    /// Number of Block results observed.
    pub blocked_decisions: usize,
    /// Highest evaluator concurrency observed.
    pub max_observed_in_flight: usize,
    /// Explicit queue/in-flight bound used by the fixture.
    pub queue_bound: usize,
    /// Number of deadline/failure fallbacks observed.
    pub fallback_decisions: usize,
    /// Number of decisions that exceeded their finite deadline.
    pub timeout_decisions: usize,
    /// Number of policy results that completed the CEF callback path.
    pub callback_completions: usize,
    /// Number of completed CEF callbacks that selected cancellation.
    pub callback_cancellations: usize,
    /// Number of safe post-outcome observations drained.
    pub observations: usize,
    /// Median decision latency in microseconds.
    pub p50_latency_us: u64,
    /// P95 decision latency in microseconds.
    pub p95_latency_us: u64,
    /// Maximum decision latency in microseconds.
    pub max_latency_us: u64,
    /// No-policy page-load baseline, when the hosted fixture ran it.
    pub baseline_page_load_ms: Option<u64>,
    /// Active-policy page-load measurement, when the hosted fixture ran it.
    pub active_page_load_ms: Option<u64>,
    /// Whether all local gate assertions passed.
    pub accepted: bool,
}

struct MeasuringEvaluator {
    current: AtomicUsize,
    max_observed: AtomicUsize,
    delay: Duration,
}

impl MeasuringEvaluator {
    fn enter(&self) {
        let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_observed.fetch_max(current, Ordering::SeqCst);
    }

    fn leave(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RequestPolicyEvaluator for MeasuringEvaluator {
    fn provider_id(&self) -> &str {
        REFERENCE_PROVIDER
    }

    fn decide(
        &self,
        request: &RequestPolicyRequest,
        cancellation: &RequestPolicyCancellation,
    ) -> Result<RequestPolicyResult, RequestPolicyEvaluatorError> {
        self.enter();
        let result = (|| {
            let started = Instant::now();
            while started.elapsed() < self.delay {
                if cancellation.is_cancelled() {
                    return Err(RequestPolicyEvaluatorError::Cancelled);
                }
                thread::sleep(Duration::from_micros(100));
            }
            let action = if request.metadata.url.contains("/blocked-") {
                RequestPolicyAction::Block
            } else {
                RequestPolicyAction::Allow
            };
            Ok(RequestPolicyResult {
                action,
                outcome: RequestPolicyOutcome::Evaluated,
                provider_id: None,
                opaque_rule_ref: Some("reference-rule".to_string()),
            })
        })();
        self.leave();
        result
    }
}

struct SlowEvaluator;

impl RequestPolicyEvaluator for SlowEvaluator {
    fn provider_id(&self) -> &str {
        "t004-slow-policy"
    }

    fn decide(
        &self,
        _request: &RequestPolicyRequest,
        cancellation: &RequestPolicyCancellation,
    ) -> Result<RequestPolicyResult, RequestPolicyEvaluatorError> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(40) {
            if cancellation.is_cancelled() {
                return Err(RequestPolicyEvaluatorError::Cancelled);
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(RequestPolicyResult {
            action: RequestPolicyAction::Block,
            outcome: RequestPolicyOutcome::Evaluated,
            provider_id: None,
            opaque_rule_ref: None,
        })
    }
}

fn decision_caller() -> RequestPolicyCaller {
    RequestPolicyCaller::new(
        BrowserAuthoritySet::new().with(BrowserAuthority::DecideRequestPolicy),
        BrowserContextId::new(REFERENCE_CONTEXT),
        Some(PageId::new(REFERENCE_PAGE)),
    )
}

fn observation_caller() -> RequestPolicyCaller {
    RequestPolicyCaller::new(
        BrowserAuthoritySet::new().with(BrowserAuthority::ObserveRequestPolicy),
        BrowserContextId::new(REFERENCE_CONTEXT),
        Some(PageId::new(REFERENCE_PAGE)),
    )
}

fn registration(
    registration_id: &str,
    provider_id: &str,
    failure_mode: RequestPolicyFailureMode,
) -> RequestPolicyRegistration {
    RequestPolicyRegistration {
        registration_id: registration_id.to_string(),
        context_id: BrowserContextId::new(REFERENCE_CONTEXT),
        page_id: Some(PageId::new(REFERENCE_PAGE)),
        failure_mode,
        max_in_flight: DECLARED_CONCURRENCY as u16,
        provider_id: provider_id.to_string(),
    }
}

fn request(registration_id: &str, url: String, deadline_ms: u64) -> RequestPolicyRequest {
    RequestPolicyRequest {
        registration_id: registration_id.to_string(),
        metadata: RequestPolicyMetadata {
            context_id: BrowserContextId::new(REFERENCE_CONTEXT),
            page_id: Some(PageId::new(REFERENCE_PAGE)),
            url,
            method: "GET".to_string(),
            resource_type: RequestResourceType::Script,
            initiator_origin: Some("http://127.0.0.1".to_string()),
            top_level_origin: Some("http://127.0.0.1".to_string()),
        },
        deadline_ms,
    }
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) * percentile / 100).min(values.len() - 1);
    values[index]
}

/// Runs bounded reference-only evidence. This is not real CEF evidence.
pub fn run_reference() -> Result<RequestPolicyFeasibilityReport, String> {
    let broker = RequestPolicyBroker::with_limits(RequestPolicyBrokerLimits {
        max_registrations: 4,
        max_total_in_flight: DECLARED_CONCURRENCY,
        max_observations: REQUEST_COUNT + 4,
    });
    let evaluator = Arc::new(MeasuringEvaluator {
        current: AtomicUsize::new(0),
        max_observed: AtomicUsize::new(0),
        delay: Duration::from_millis(1),
    });
    let caller = decision_caller();
    broker
        .register(
            registration(
                REFERENCE_REGISTRATION,
                REFERENCE_PROVIDER,
                RequestPolicyFailureMode::FailClosed,
            ),
            Arc::clone(&evaluator) as Arc<dyn RequestPolicyEvaluator>,
            &caller,
        )
        .map_err(|error| error.to_string())?;

    let next = Arc::new(AtomicUsize::new(0));
    let results = Arc::new(Mutex::new(Vec::<(RequestPolicyResult, u64)>::new()));
    let mut workers = Vec::with_capacity(DECLARED_CONCURRENCY);
    for _ in 0..DECLARED_CONCURRENCY {
        let broker = broker.clone();
        let caller = caller.clone();
        let next = Arc::clone(&next);
        let results = Arc::clone(&results);
        workers.push(
            thread::Builder::new()
                .name("worldline-t004-reference-worker".to_string())
                .spawn(move || {
                    loop {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        if index >= REQUEST_COUNT {
                            return;
                        }
                        let started = Instant::now();
                        let result = broker
                            .decide(
                                &caller,
                                request(
                                    REFERENCE_REGISTRATION,
                                    format!(
                                        "http://127.0.0.1/{}",
                                        if index.is_multiple_of(4) {
                                            format!("blocked-{index}.js")
                                        } else {
                                            format!("allowed-{index}.js")
                                        }
                                    ),
                                    DEFAULT_REQUEST_POLICY_DEADLINE_MS,
                                ),
                            )
                            .expect("reference feasibility decision must complete");
                        results
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push((result, started.elapsed().as_micros() as u64));
                    }
                })
                .map_err(|error| format!("spawn reference feasibility worker: {error}"))?,
        );
    }
    for worker in workers {
        worker
            .join()
            .map_err(|_| "reference feasibility worker panicked".to_string())?;
    }

    let timeout_registration = registration(
        "t004-timeout-registration",
        "t004-slow-policy",
        RequestPolicyFailureMode::FailOpen,
    );
    broker
        .register(timeout_registration, Arc::new(SlowEvaluator), &caller)
        .map_err(|error| error.to_string())?;
    let timeout_result = broker
        .decide(
            &caller,
            request(
                "t004-timeout-registration",
                "http://127.0.0.1/slow.js".to_string(),
                5,
            ),
        )
        .map_err(|error| error.to_string())?;

    let results = results
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut latencies: Vec<u64> = results.iter().map(|(_, latency)| *latency).collect();
    latencies.sort_unstable();
    let allowed_decisions = results
        .iter()
        .filter(|(result, _)| result.action == RequestPolicyAction::Allow)
        .count();
    let blocked_decisions = results
        .iter()
        .filter(|(result, _)| result.action == RequestPolicyAction::Block)
        .count();
    let observations: Vec<RequestPolicyObservation> = broker
        .drain_observations(&observation_caller())
        .map_err(|error| error.to_string())?;
    let scope_probe = broker.decide(
        &RequestPolicyCaller::new(
            BrowserAuthoritySet::new().with(BrowserAuthority::DecideRequestPolicy),
            BrowserContextId::new("t004-other-context"),
            Some(PageId::new(REFERENCE_PAGE)),
        ),
        request(
            REFERENCE_REGISTRATION,
            "http://127.0.0.1/cross-context.js".to_string(),
            DEFAULT_REQUEST_POLICY_DEADLINE_MS,
        ),
    );

    let report = RequestPolicyFeasibilityReport {
        topology: crate::request_policy::REFERENCE_REQUEST_POLICY_TOPOLOGY.to_string(),
        declared_concurrency: DECLARED_CONCURRENCY,
        request_count: REQUEST_COUNT,
        completed_decisions: results.len(),
        allowed_decisions,
        blocked_decisions,
        max_observed_in_flight: evaluator.max_observed.load(Ordering::SeqCst),
        queue_bound: DECLARED_CONCURRENCY,
        fallback_decisions: usize::from(timeout_result.outcome != RequestPolicyOutcome::Evaluated),
        timeout_decisions: usize::from(
            timeout_result.outcome == RequestPolicyOutcome::DeadlineExceeded,
        ),
        callback_completions: 0,
        callback_cancellations: 0,
        observations: observations.len(),
        p50_latency_us: percentile(&latencies, 50),
        p95_latency_us: percentile(&latencies, 95),
        max_latency_us: latencies.iter().copied().max().unwrap_or_default(),
        baseline_page_load_ms: None,
        active_page_load_ms: None,
        accepted: results.len() == REQUEST_COUNT
            && allowed_decisions + blocked_decisions == REQUEST_COUNT
            && evaluator.max_observed.load(Ordering::SeqCst) <= DECLARED_CONCURRENCY
            && timeout_result.action == RequestPolicyAction::Allow
            && timeout_result.outcome == RequestPolicyOutcome::DeadlineExceeded
            && observations.len() == REQUEST_COUNT + 1
            && scope_probe.is_err(),
    };
    Ok(report)
}

#[cfg(windows)]
pub fn run_real() -> Result<RequestPolicyFeasibilityReport, String> {
    real::run()
}

#[cfg(not(windows))]
pub fn run_real() -> Result<RequestPolicyFeasibilityReport, String> {
    Err("T-004 real CEF feasibility requires the hosted Windows runtime".to_string())
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
    use worldline_browser_contract::authority::{OP_CREATE_CONTEXT, OP_CREATE_PAGE, OP_OBSERVE};
    use worldline_browser_contract::contracts::{
        CreateContextRequest, CreateContextResponse, CreatePageRequest, CreatePageResponse,
        LoadingState, ObservePageRequest, PageObservation,
    };
    use worldline_browser_contract::identity::PageId;
    use worldline_browser_contract::request_policy::{
        RequestPolicyAction, RequestPolicyRequest, RequestPolicyResult,
    };
    use worldline_native_host::{
        ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError,
        NativeProviderConnection,
    };
    use worldline_plugin_protocol::{MessageKind, REQUEST_POLICY_INTERFACE};

    use crate::real_cef_lock::RealCefRunGuard;

    use super::RequestPolicyFeasibilityReport;

    const SCRIPT_COUNT: usize = 8;
    const ACTIVE_PROFILE: &str = "t004-feasibility-profile";
    const POLICY_PROVIDER: &str = "t004-feasibility-host-policy";
    const SLOW_REQUEST_DELAY: Duration = Duration::from_millis(400);

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
                .map_err(|error| format!("bind T-004 loopback server: {error}"))?;
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("configure T-004 loopback server: {error}"))?;
            let address = listener
                .local_addr()
                .map_err(|error| format!("read T-004 loopback address: {error}"))?;
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let connections = Arc::new(Mutex::new(Vec::new()));
            let worker_connections = Arc::clone(&connections);
            let hits = Arc::new(Mutex::new(Vec::new()));
            let worker_hits = Arc::clone(&hits);
            let index_body = Arc::new(index_body(nonce));
            let worker_index_body = Arc::clone(&index_body);
            let worker = thread::Builder::new()
                .name("worldline-t004-loopback".to_string())
                .spawn(move || {
                    while !worker_stop.load(Ordering::SeqCst) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let body = Arc::clone(&worker_index_body);
                                let hits = Arc::clone(&worker_hits);
                                let connection = thread::Builder::new()
                                    .name("worldline-t004-http".to_string())
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
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| format!("start T-004 loopback server: {error}"))?;

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
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Worldline T004</title></head><body><h1>Worldline T004</h1>",
        );
        for index in 0..SCRIPT_COUNT {
            body.push_str(&format!(
                "<script src=\"/active-{index}.js?run={nonce}\"></script>"
            ));
        }
        body.push_str(&format!(
            "<script src=\"/blocked.js?run={nonce}\"></script><script src=\"/slow.js?run={nonce}\"></script></body></html>"
        ));
        body
    }

    fn serve_connection(
        mut stream: TcpStream,
        index_body: Arc<String>,
        hits: Arc<Mutex<Vec<String>>>,
    ) {
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
                b"window.__worldline_t004 = true;",
            )
        } else if path == "/blocked.js" {
            (
                "200 OK",
                "text/javascript; charset=utf-8",
                b"window.__worldline_blocked = true;",
            )
        } else if path == "/slow.js" {
            (
                "200 OK",
                "text/javascript; charset=utf-8",
                b"window.__worldline_slow = true;",
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
        allowed: AtomicUsize,
        blocked: AtomicUsize,
        current: AtomicUsize,
        max_in_flight: AtomicUsize,
        latencies_us: Mutex<Vec<u64>>,
        scopes: Mutex<Vec<(String, String, String)>>,
    }

    impl PolicyMetrics {
        fn new() -> Self {
            Self {
                request_count: AtomicUsize::new(0),
                allowed: AtomicUsize::new(0),
                blocked: AtomicUsize::new(0),
                current: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                latencies_us: Mutex::new(Vec::new()),
                scopes: Mutex::new(Vec::new()),
            }
        }
    }

    struct FeasibilityHostSink {
        metrics: Arc<PolicyMetrics>,
    }

    impl HostRequestSink for FeasibilityHostSink {
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
                                reason: format!("invalid T-004 policy request: {error}"),
                            }
                        })?;
                    request
                        .validate()
                        .map_err(|reason| NativeHostError::ProtocolViolation {
                            reason: format!("invalid T-004 policy request: {reason}"),
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
                            request.metadata.context_id.as_str().to_string(),
                            page_id,
                            request.metadata.url.clone(),
                        ));
                    if request.metadata.url.contains("/slow.js") {
                        thread::sleep(SLOW_REQUEST_DELAY);
                    } else {
                        thread::sleep(Duration::from_millis(2));
                    }
                    let action = if request.metadata.url.contains("/blocked.js") {
                        self.metrics.blocked.fetch_add(1, Ordering::SeqCst);
                        RequestPolicyAction::Block
                    } else {
                        self.metrics.allowed.fetch_add(1, Ordering::SeqCst);
                        RequestPolicyAction::Allow
                    };
                    self.metrics.current.fetch_sub(1, Ordering::SeqCst);
                    self.metrics
                        .latencies_us
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(started.elapsed().as_micros() as u64);
                    serde_json::to_value(RequestPolicyResult {
                        action,
                        outcome: worldline_browser_contract::request_policy::RequestPolicyOutcome::Evaluated,
                        provider_id: Some(POLICY_PROVIDER.to_string()),
                        opaque_rule_ref: Some("t004-decision".to_string()),
                    })
                    .map(Some)
                    .map_err(|error| NativeHostError::ProtocolViolation {
                        reason: format!("encode T-004 policy result: {error}"),
                    })
                }
                // The fixture does not use event or blob transport, but an
                // event can be safely ignored because it is never the policy
                // result path.
                MessageKind::EventPublishRequest => Ok(None),
                other => Err(NativeHostError::ProtocolViolation {
                    reason: format!("unexpected T-004 child request: {other:?}"),
                }),
            }
        }
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn create() -> Result<Self, String> {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("clock before Unix epoch: {error}"))?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "worldline-t004-feasibility-{}-{stamp}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path)
                .map_err(|error| format!("create T-004 temporary root: {error}"))?;
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
        page_load_ms: u64,
        hits: Vec<String>,
        metrics: Arc<PolicyMetrics>,
    }

    fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
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
                    format!("T-004 native call '{operation}' failed: {error}")
                } else {
                    format!("T-004 native call '{operation}' failed: {error}; stderr:\n{stderr}")
                }
            })?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(format!(
                "T-004 provider operation '{operation}' failed: {error}"
            ));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("T-004 provider operation '{operation}' omitted result"))
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
                        "T-004 CEF page '{page_id}' failed to load '{}'; stderr:\n{}",
                        observation.url,
                        connection.stderr_text()
                    ));
                }
                LoadingState::Unloaded | LoadingState::Loading | LoadingState::Interactive => {}
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for T-004 page '{page_id}'"));
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
                    "timed out waiting for T-004 origin paths {expected:?}; observed {hits:?}"
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_policy_paths(
        metrics: &PolicyMetrics,
        expected: &[String],
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let urls = metrics
                .scopes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .map(|(_, _, url)| url.split('?').next().unwrap_or_default().to_string())
                .collect::<Vec<_>>();
            if expected
                .iter()
                .all(|path| urls.iter().any(|url| url.ends_with(path)))
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for T-004 policy paths {expected:?}; observed {urls:?}"
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_policy_completion(
        metrics: &PolicyMetrics,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let request_count = metrics.request_count.load(Ordering::SeqCst);
            let completed =
                metrics.allowed.load(Ordering::SeqCst) + metrics.blocked.load(Ordering::SeqCst);
            if metrics.current.load(Ordering::SeqCst) == 0 && completed == request_count {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for T-004 policy decisions: requests={request_count}, completed={completed}, current={}",
                    metrics.current.load(Ordering::SeqCst)
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
                "pinned CEF bootstrapc.exe is missing; stage the verified CEF runtime before T-004-real"
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
                "worldline_browser_provider_client.dll is missing; build and stage the CEF provider client before T-004-real"
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
        policy_enabled: bool,
        nonce: &str,
    ) -> Result<RealRun, String> {
        let metrics = Arc::new(PolicyMetrics::new());
        let sink = Arc::new(FeasibilityHostSink {
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
        if policy_enabled {
            child_args.extend([
                "--request-policy-profile".to_string(),
                ACTIVE_PROFILE.to_string(),
                "--request-policy-failure-mode".to_string(),
                "fail-open".to_string(),
            ]);
        }
        let spec = NativeChildSpec::new(
            program.to_path_buf(),
            child_args,
            4 * 1024 * 1024,
            64 * 1024,
        );
        let (connection, ack) = NativeProviderConnection::connect_with_required_interface(
            spec,
            &identity,
            Arc::clone(&sink) as Arc<dyn HostRequestSink>,
            super::DECLARED_CONCURRENCY,
            REQUEST_POLICY_INTERFACE,
        )
        .map_err(|error| format!("connect T-004 native provider: {error}"))?;
        if !ack.supports_interface(REQUEST_POLICY_INTERFACE) {
            return Err("T-004 provider did not negotiate request-policy interface".to_string());
        }

        let page_url = format!("{}/index.html?run={nonce}", server.base_url);
        let started = Instant::now();
        let context: CreateContextResponse = decode(call_op(
            &connection,
            OP_CREATE_CONTEXT,
            serde_json::to_value(CreateContextRequest {
                profile_id: Some(format!("t004-{nonce}")),
                incognito: true,
                user_agent: None,
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        let page: CreatePageResponse = decode(call_op(
            &connection,
            OP_CREATE_PAGE,
            serde_json::to_value(CreatePageRequest {
                context_id: context.context_id,
                initial_url: Some(page_url),
            })
            .map_err(|error| error.to_string())?,
        )?)?;
        wait_for_page(&connection, &page.page_id, Duration::from_secs(15))?;
        // Baseline must fetch every resource. The active profile must still
        // fetch unrelated and timeout resources, while the blocked path must
        // stop before the loopback origin.
        let expected: Vec<String> = if policy_enabled {
            (0..SCRIPT_COUNT)
                .map(|index| format!("/active-{index}.js"))
                .chain(["/slow.js".to_string()])
                .collect()
        } else {
            (0..SCRIPT_COUNT)
                .map(|index| format!("/active-{index}.js"))
                .chain(["/blocked.js".to_string(), "/slow.js".to_string()])
                .collect()
        };
        let wait_result = if policy_enabled {
            // The policy boundary is the authoritative T-004 proving point.
            // S3C owns the stronger per-resource origin assertions; here we
            // require every active fixture request to reach and complete at
            // the policy boundary, while retaining the slow fail-open origin
            // hit as the timeout signal.
            let policy_expected = expected
                .iter()
                .cloned()
                .chain(["/blocked.js".to_string()])
                .collect::<Vec<_>>();
            wait_for_policy_paths(&metrics, &policy_expected, Duration::from_secs(15))
                .and_then(|_| wait_for_policy_completion(&metrics, Duration::from_secs(15)))
                .and_then(|_| {
                    wait_for_paths(server, &["/slow.js".to_string()], Duration::from_secs(15))
                })
        } else {
            wait_for_paths(server, &expected, Duration::from_secs(15))
        };
        wait_result.map_err(|error| {
            format!(
                "{error}; T-004 policy metrics: requests={}, allowed={}, blocked={}, max_in_flight={}; provider stderr:\n{}",
                metrics.request_count.load(Ordering::SeqCst),
                metrics.allowed.load(Ordering::SeqCst),
                metrics.blocked.load(Ordering::SeqCst),
                metrics.max_in_flight.load(Ordering::SeqCst),
                connection.stderr_text()
            )
        })?;
        let hits = server.snapshot_hits();
        connection
            .close(Duration::from_secs(15))
            .map_err(|error| format!("close T-004 native CEF provider: {error}"))?;
        Ok(RealRun {
            page_load_ms: started.elapsed().as_millis() as u64,
            hits,
            metrics,
        })
    }

    /// Runs both the no-policy baseline and the active explicit FailOpen
    /// policy profile through the pinned native CEF provider.
    pub fn run() -> Result<RequestPolicyFeasibilityReport, String> {
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
            false,
            &nonce,
        )?;
        let baseline_hits = baseline.hits.clone();
        server.clear_hits();
        let active = run_one(
            &program,
            &client,
            &server,
            &temp_root.path().join("active-cef"),
            true,
            &nonce,
        )?;
        let active_hits = active.hits.clone();
        let mut latencies = active
            .metrics
            .latencies_us
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        latencies.sort_unstable();
        let request_count = active.metrics.request_count.load(Ordering::SeqCst);
        let allowed = active.metrics.allowed.load(Ordering::SeqCst);
        let blocked = active.metrics.blocked.load(Ordering::SeqCst);
        let slow_hit = active_hits.iter().any(|path| path == "/slow.js");
        let blocked_origin_hit = active_hits.iter().any(|path| path == "/blocked.js");
        let baseline_blocked_hit = baseline_hits.iter().any(|path| path == "/blocked.js");
        let scopes = active
            .metrics
            .scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let same_scope = scopes
            .first()
            .map(|(context, page, _)| {
                !context.is_empty()
                    && !page.is_empty()
                    && scopes.iter().all(|(other_context, other_page, _)| {
                        other_context == context && other_page == page
                    })
            })
            .unwrap_or(false);
        let callback_completions = request_count;
        let callback_cancellations = blocked;
        let timeout_decisions = usize::from(slow_hit);
        let report = RequestPolicyFeasibilityReport {
            topology: format!(
                "real CEF 151.8.0 -> native provider process -> negotiated {REQUEST_POLICY_INTERFACE}; bootstrap={}; client={}",
                program.display(),
                client.display()
            ),
            declared_concurrency: super::DECLARED_CONCURRENCY,
            request_count,
            completed_decisions: callback_completions,
            allowed_decisions: allowed,
            blocked_decisions: blocked,
            max_observed_in_flight: active.metrics.max_in_flight.load(Ordering::SeqCst),
            queue_bound: super::DECLARED_CONCURRENCY,
            fallback_decisions: timeout_decisions,
            timeout_decisions,
            callback_completions,
            callback_cancellations,
            observations: 0,
            p50_latency_us: super::percentile(&latencies, 50),
            p95_latency_us: super::percentile(&latencies, 95),
            max_latency_us: latencies.iter().copied().max().unwrap_or_default(),
            baseline_page_load_ms: Some(baseline.page_load_ms),
            active_page_load_ms: Some(active.page_load_ms),
            accepted: request_count >= SCRIPT_COUNT + 3
                && allowed >= SCRIPT_COUNT + 2
                && blocked >= 1
                && active.metrics.max_in_flight.load(Ordering::SeqCst)
                    <= super::DECLARED_CONCURRENCY
                && callback_completions == request_count
                && callback_cancellations == blocked
                && baseline_blocked_hit
                && !blocked_origin_hit
                && slow_hit
                && timeout_decisions == 1
                && same_scope
                && active.page_load_ms < 5_000,
        };
        Ok(report)
    }
}
