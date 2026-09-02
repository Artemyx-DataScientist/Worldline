//! The host side of one native provider connection.
//!
//! Host-initiated capability and lifecycle requests are correlated by
//! bounded protocol identities; child-initiated state and event requests
//! are forwarded to the [`HostRequestSink`], which routes them into the
//! existing kernel state contract and event transport. Request-policy
//! decisions use their own correlated RPC plane and never wait on event
//! publication. One malformed or oversized frame marks the connection broken,
//! fails every pending request deterministically, and never panics the host.

use std::collections::BTreeMap;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use worldline_browser_contract::request_policy::{RequestPolicyRequest, RequestPolicyResult};
use worldline_plugin_protocol::{
    Envelope, MessageKind, PROTOCOL_VERSION, REQUEST_POLICY_INTERFACE,
};

use crate::codec::{read_frame, write_frame};
use crate::error::NativeHostError;
use crate::handshake::{ChildAck, ExpectedIdentity, perform_host_handshake};
use crate::supervisor::{NativeChild, NativeChildSpec};

/// Host-side handler for child-initiated requests.
///
/// `StateRequest` payloads carry `{"action":"get"|"set","key":..,"value":..}`;
/// the reply (sent as `StateResult`) is `{"value": .. | null}`. Replies for
/// `EventPublishRequest` are sent only on failure (as `ProtocolError`),
/// because event publication is not an RPC result channel. A
/// `RequestPolicyRequest` is a bounded child-initiated RPC and receives a
/// `RequestPolicyResult` reply from the sink.
pub trait HostRequestSink: Send + Sync {
    fn on_child_request(
        &self,
        kind: MessageKind,
        correlation_id: u64,
        payload: Value,
    ) -> Result<Option<Value>, NativeHostError>;
}

struct SharedState {
    pending: Mutex<BTreeMap<u64, std::sync::mpsc::SyncSender<Result<Value, NativeHostError>>>>,
    child_policy_pending: Mutex<BTreeMap<u64, Arc<AtomicBool>>>,
    broken: Mutex<Option<NativeHostError>>,
    writer: Mutex<Option<ChildStdin>>,
    in_flight: AtomicUsize,
    next_correlation: AtomicU64,
}

impl SharedState {
    fn send(&self, envelope: &Envelope) -> Result<(), NativeHostError> {
        let mut guard = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.as_mut() {
            Some(writer) => write_frame(writer, envelope),
            None => Err(NativeHostError::TransportClosed),
        }
    }

    fn break_with(&self, error: NativeHostError) {
        {
            let mut broken = self
                .broken
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if broken.is_some() {
                return;
            }
            *broken = Some(error.clone());
        }
        let pending = self
            .child_policy_pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for cancellation in pending.values() {
            cancellation.store(true, Ordering::Release);
        }
        self.fail_all_pending(error);
    }

    fn fail_all_pending(&self, error: NativeHostError) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let correlations: Vec<u64> = pending.keys().copied().collect();
        for correlation in correlations {
            if let Some(sender) = pending.remove(&correlation) {
                let _ = sender.send(Err(error.clone()));
            }
        }
    }

    fn is_broken(&self) -> Option<NativeHostError> {
        self.broken
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// One live native provider connection over a supervised child process.
pub struct NativeProviderConnection {
    shared: Arc<SharedState>,
    child: Arc<Mutex<NativeChild>>,
    max_in_flight: usize,
    max_frame_bytes: usize,
}

impl std::fmt::Debug for NativeProviderConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeProviderConnection")
            .finish_non_exhaustive()
    }
}

impl NativeProviderConnection {
    /// Spawns the child, performs the handshake, and starts the demux
    /// reader thread.
    pub fn connect(
        spec: NativeChildSpec,
        expected: &ExpectedIdentity,
        sink: Arc<dyn HostRequestSink>,
        max_in_flight: usize,
    ) -> Result<(Self, ChildAck), NativeHostError> {
        let max_frame_bytes = spec.max_frame_bytes;
        let mut child = NativeChild::spawn(&spec)?;
        let mut stdin = child.take_stdin().ok_or(NativeHostError::SpawnFailed {
            reason: "child stdin is not piped".to_owned(),
        })?;
        let mut stdout = child.take_stdout().ok_or(NativeHostError::SpawnFailed {
            reason: "child stdout is not piped".to_owned(),
        })?;
        let ack = perform_host_handshake(&mut stdin, &mut stdout, expected, max_frame_bytes)?;

        let shared = Arc::new(SharedState {
            pending: Mutex::new(BTreeMap::new()),
            child_policy_pending: Mutex::new(BTreeMap::new()),
            broken: Mutex::new(None),
            writer: Mutex::new(Some(stdin)),
            in_flight: AtomicUsize::new(0),
            next_correlation: AtomicU64::new(0),
        });
        spawn_reader(
            Arc::clone(&shared),
            stdout,
            sink,
            max_frame_bytes,
            max_in_flight,
            ack.supports_interface(REQUEST_POLICY_INTERFACE),
        );

        Ok((
            Self {
                shared,
                child: Arc::new(Mutex::new(child)),
                max_in_flight,
                max_frame_bytes,
            },
            ack,
        ))
    }

    /// Spawns the child and requires an experimental interface to be
    /// advertised by the handshake before returning the live connection.
    /// A peer that does not negotiate the interface is terminated and cannot
    /// receive a request on that plane.
    pub fn connect_with_required_interface(
        spec: NativeChildSpec,
        expected: &ExpectedIdentity,
        sink: Arc<dyn HostRequestSink>,
        max_in_flight: usize,
        required_interface: &str,
    ) -> Result<(Self, ChildAck), NativeHostError> {
        let (connection, ack) = Self::connect(spec, expected, sink, max_in_flight)?;
        if !ack.supports_interface(required_interface) {
            connection.kill();
            return Err(NativeHostError::HandshakeFailed {
                reason: format!(
                    "child did not negotiate required interface '{required_interface}'"
                ),
            });
        }
        Ok((connection, ack))
    }

    /// The maximum accepted frame size for this connection.
    pub fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    /// Returns the bounded stderr diagnostics collected from the supervised
    /// provider process. This is intentionally a tail so a misbehaving child
    /// cannot grow host memory without bound.
    pub fn stderr_text(&self) -> String {
        self.child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stderr_text()
    }

    /// Returns the provider's exit status when it has already terminated.
    pub fn try_status(&self) -> Option<std::process::ExitStatus> {
        self.child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_status()
    }

    /// Sends one correlated capability request and awaits the reply.
    pub fn call(&self, payload: Value) -> Result<Value, NativeHostError> {
        self.call_inner(MessageKind::CapabilityRequest, payload, None)
    }

    /// Sends one correlated capability request with a deadline. On timeout
    /// a `Cancellation` envelope is sent for the same correlation and a
    /// typed deadline failure is returned.
    pub fn call_with_deadline(
        &self,
        payload: Value,
        deadline: Duration,
    ) -> Result<Value, NativeHostError> {
        self.call_inner(MessageKind::CapabilityRequest, payload, Some(deadline))
    }

    /// Performs the graceful shutdown exchange and then shuts the child
    /// down under the deadline.
    pub fn close(&self, shutdown_deadline: Duration) -> Result<(), NativeHostError> {
        let _ = self.call_inner(
            MessageKind::LifecycleRequest,
            serde_json::json!({"action": "deactivate"}),
            Some(shutdown_deadline),
        );
        self.shared
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let mut child = self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match child.shutdown(shutdown_deadline) {
            Ok(_status) => Ok(()),
            Err(NativeHostError::ShutdownTimeout { deadline_ms }) => {
                Err(NativeHostError::ShutdownTimeout { deadline_ms })
            }
            Err(error) => Err(error),
        }
    }

    /// Forcibly terminates the child. The host is unaffected.
    pub fn kill(&self) {
        if let Ok(mut child) = self.child.lock() {
            child.kill();
        }
        self.shared.break_with(NativeHostError::TransportClosed);
    }

    fn call_inner(
        &self,
        kind: MessageKind,
        payload: Value,
        deadline: Option<Duration>,
    ) -> Result<Value, NativeHostError> {
        if let Some(reason) = self.shared.is_broken() {
            return Err(reason);
        }
        if self.shared.in_flight.fetch_add(1, Ordering::SeqCst) >= self.max_in_flight {
            self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err(NativeHostError::InvocationLimitExceeded {
                limit: self.max_in_flight,
            });
        }
        let correlation = self.shared.next_correlation.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.shared
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(correlation, sender);

        let envelope = Envelope::new(kind, correlation, payload);
        if let Err(error) = self.shared.send(&envelope) {
            self.release(correlation);
            return Err(error);
        }

        let inner = match deadline {
            None => receiver
                .recv()
                .unwrap_or(Err(NativeHostError::TransportClosed)),
            Some(deadline) => match receiver.recv_timeout(deadline) {
                Ok(result) => result,
                Err(_) => {
                    self.release(correlation);
                    let _ = self.shared.send(&Envelope::new(
                        MessageKind::Cancellation,
                        correlation,
                        serde_json::json!({"reason": "deadline"}),
                    ));
                    return Err(NativeHostError::DeadlineExceeded {
                        deadline_ms: deadline.as_millis() as u64,
                    });
                }
            },
        };
        self.release(correlation);
        inner
    }

    fn release(&self, correlation: u64) {
        self.shared
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&correlation);
        self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for NativeProviderConnection {
    fn drop(&mut self) {
        self.kill();
    }
}

fn spawn_reader(
    shared: Arc<SharedState>,
    mut stdout: std::process::ChildStdout,
    sink: Arc<dyn HostRequestSink>,
    max_frame_bytes: usize,
    max_child_policy_in_flight: usize,
    policy_interface_negotiated: bool,
) {
    std::thread::spawn(move || {
        loop {
            let envelope = match read_frame(&mut stdout, max_frame_bytes) {
                Ok(envelope) => envelope,
                Err(error) => {
                    if !matches!(&error, NativeHostError::TransportClosed) {
                        eprintln!("worldline native host reader stopped: {error}");
                    }
                    shared.break_with(error);
                    break;
                }
            };
            match envelope.message_kind {
                MessageKind::CapabilityResult | MessageKind::LifecycleResult => {
                    let payload = envelope_result_payload(envelope.payload);
                    let mut pending = shared
                        .pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match pending.remove(&envelope.correlation_id) {
                        Some(sender) => {
                            let _ = sender.send(payload);
                        }
                        None => {
                            drop(pending);
                            shared.break_with(NativeHostError::ProtocolViolation {
                                reason: format!(
                                    "reply for unknown correlation {}",
                                    envelope.correlation_id
                                ),
                            });
                            break;
                        }
                    }
                }
                MessageKind::StateRequest => {
                    let outcome = sink.on_child_request(
                        MessageKind::StateRequest,
                        envelope.correlation_id,
                        envelope.payload.clone(),
                    );
                    let reply = match outcome {
                        Ok(value) => Envelope::new(
                            MessageKind::StateResult,
                            envelope.correlation_id,
                            value.unwrap_or(Value::Null),
                        ),
                        Err(error) => protocol_error_envelope(&envelope, &error),
                    };
                    if let Err(broken) = shared.send(&reply) {
                        shared.break_with(broken);
                        break;
                    }
                }
                MessageKind::BlobRequest => {
                    let outcome = sink.on_child_request(
                        MessageKind::BlobRequest,
                        envelope.correlation_id,
                        envelope.payload.clone(),
                    );
                    let reply = match outcome {
                        Ok(value) => Envelope::new(
                            MessageKind::BlobResult,
                            envelope.correlation_id,
                            value.unwrap_or(Value::Null),
                        ),
                        Err(error) => protocol_error_envelope(&envelope, &error),
                    };
                    if let Err(broken) = shared.send(&reply) {
                        shared.break_with(broken);
                        break;
                    }
                }
                MessageKind::EventPublishRequest => {
                    let outcome = sink.on_child_request(
                        MessageKind::EventPublishRequest,
                        envelope.correlation_id,
                        envelope.payload.clone(),
                    );
                    if let Err(error) = outcome {
                        let reply = protocol_error_envelope(&envelope, &error);
                        if let Err(broken) = shared.send(&reply) {
                            shared.break_with(broken);
                            break;
                        }
                    }
                }
                MessageKind::Cancellation
                    if envelope.payload.get("plane").and_then(Value::as_str)
                        == Some("request_policy") =>
                {
                    cancel_child_policy(&shared, envelope.correlation_id);
                }
                MessageKind::RequestPolicyRequest => {
                    if !policy_interface_negotiated {
                        shared.break_with(NativeHostError::ProtocolViolation {
                            reason: format!(
                                "child sent request-policy request without negotiating '{REQUEST_POLICY_INTERFACE}'"
                            ),
                        });
                        break;
                    }
                    spawn_policy_request(
                        Arc::clone(&shared),
                        Arc::clone(&sink),
                        envelope,
                        max_child_policy_in_flight,
                    );
                }
                other => {
                    shared.break_with(NativeHostError::ProtocolViolation {
                        reason: format!("child sent unexpected message kind {other:?}"),
                    });
                    break;
                }
            }
        }
    });
}

fn cancel_child_policy(shared: &SharedState, correlation_id: u64) {
    if let Some(cancellation) = shared
        .child_policy_pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&correlation_id)
    {
        cancellation.store(true, Ordering::Release);
    }
}

fn spawn_policy_request(
    shared: Arc<SharedState>,
    sink: Arc<dyn HostRequestSink>,
    request: Envelope,
    max_in_flight: usize,
) {
    if let Err(reason) = validate_policy_request_payload(&request.payload) {
        let reply = Envelope::new(
            MessageKind::RequestPolicyResult,
            request.correlation_id,
            serde_json::json!({"error": format!("invalid request-policy request: {reason}")}),
        );
        if let Err(error) = shared.send(&reply) {
            shared.break_with(error);
        }
        return;
    }

    let current = shared
        .child_policy_pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len();
    if current >= max_in_flight {
        let reply = Envelope::new(
            MessageKind::RequestPolicyResult,
            request.correlation_id,
            serde_json::json!({
                "error": format!("request-policy in-flight limit of {max_in_flight} exceeded")
            }),
        );
        if let Err(error) = shared.send(&reply) {
            shared.break_with(error);
        }
        return;
    }

    let cancellation = Arc::new(AtomicBool::new(false));
    shared
        .child_policy_pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(request.correlation_id, Arc::clone(&cancellation));

    let worker_shared = Arc::clone(&shared);
    let spawn = std::thread::Builder::new()
        .name("worldline-host-request-policy".to_owned())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                sink.on_child_request(
                    MessageKind::RequestPolicyRequest,
                    request.correlation_id,
                    request.payload,
                )
            }))
            .unwrap_or_else(|_| {
                Err(NativeHostError::ProtocolViolation {
                    reason: "request-policy host sink panicked".to_owned(),
                })
            });
            let was_cancelled = cancellation.load(Ordering::Acquire);
            worker_shared
                .child_policy_pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&request.correlation_id);
            if was_cancelled || worker_shared.is_broken().is_some() {
                return;
            }
            let payload = policy_result_payload(outcome);
            let reply = Envelope::new(
                MessageKind::RequestPolicyResult,
                request.correlation_id,
                payload,
            );
            if let Err(error) = worker_shared.send(&reply) {
                worker_shared.break_with(error);
            }
        });
    if let Err(error) = spawn {
        // The request has no worker to consume it, so return a bounded typed
        // failure on the policy plane rather than leaving the child waiting.
        shared
            .child_policy_pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&request.correlation_id);
        if shared.is_broken().is_none() {
            let reply = Envelope::new(
                MessageKind::RequestPolicyResult,
                request.correlation_id,
                serde_json::json!({"error": format!("request-policy worker spawn failed: {error}")}),
            );
            if let Err(send_error) = shared.send(&reply) {
                shared.break_with(send_error);
            }
        }
    }
}

fn validate_policy_request_payload(payload: &Value) -> Result<(), String> {
    let request: RequestPolicyRequest =
        serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
    request.validate().map_err(|error| error.to_string())
}

fn policy_result_payload(outcome: Result<Option<Value>, NativeHostError>) -> Value {
    match outcome {
        Ok(Some(value)) if value.get("error").and_then(Value::as_str).is_some() => value,
        Ok(Some(value)) => match serde_json::from_value::<RequestPolicyResult>(value) {
            Ok(result) => serde_json::to_value(result).unwrap_or_else(|error| {
                serde_json::json!({"error": format!("request-policy result encode failed: {error}")})
            }),
            Err(error) => {
                serde_json::json!({"error": format!("invalid request-policy result: {error}")})
            }
        },
        Ok(None) => serde_json::json!({
            "error": "request-policy host sink returned no result"
        }),
        Err(error) => serde_json::json!({"error": error.to_string()}),
    }
}

/// Capability results carry either `{"bytes": ..}` or `{"error": ..}`.
fn envelope_result_payload(payload: Value) -> Result<Value, NativeHostError> {
    if let Some(message) = payload.get("error").and_then(Value::as_str) {
        return Err(NativeHostError::GuestReturnedError {
            message: message.to_owned(),
        });
    }
    Ok(payload)
}

fn protocol_error_envelope(request: &Envelope, error: &NativeHostError) -> Envelope {
    Envelope::new(
        MessageKind::ProtocolError,
        request.correlation_id,
        serde_json::json!({"message": error.to_string(), "protocol_version": PROTOCOL_VERSION}),
    )
}
