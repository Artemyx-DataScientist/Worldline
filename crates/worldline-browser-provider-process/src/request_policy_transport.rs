//! Child-side bounded request/result transport for browser interception.
//!
//! This module owns only the physical native-process translation. The
//! request-policy contract remains in `worldline-browser-contract`, and the
//! engine adapter receives the transport through the provider-owned neutral
//! trait. The event plane is intentionally absent from this implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use worldline_browser_contract::request_policy::{RequestPolicyRequest, RequestPolicyResult};
use worldline_browser_provider::{RequestPolicyTransport, RequestPolicyTransportError};
use worldline_native_host::{NativeHostError, write_frame};
use worldline_plugin_protocol::{Envelope, MessageKind};

/// Shared writer used by the provider command loop and engine callback
/// threads. The mutex is held only while encoding/writing one bounded frame.
pub type SharedProviderWriter = Arc<Mutex<Box<dyn Write + Send>>>;

const RETIRED_CORRELATION_FACTOR: usize = 2;

struct PendingPolicyRequest {
    sender: std::sync::mpsc::SyncSender<Result<serde_json::Value, RequestPolicyTransportError>>,
}

struct TransportState {
    writer: SharedProviderWriter,
    pending: Mutex<BTreeMap<u64, PendingPolicyRequest>>,
    retired: Mutex<BTreeSet<u64>>,
    broken: Mutex<Option<RequestPolicyTransportError>>,
    in_flight: AtomicUsize,
    next_correlation: AtomicU64,
    max_in_flight: usize,
    max_frame_bytes: usize,
}

/// Cloneable child-side handle for one bounded request-policy RPC plane.
#[derive(Clone)]
pub struct ProviderPolicyTransport {
    state: Arc<TransportState>,
}

impl std::fmt::Debug for ProviderPolicyTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderPolicyTransport")
            .field("max_in_flight", &self.state.max_in_flight)
            .field("max_frame_bytes", &self.state.max_frame_bytes)
            .finish_non_exhaustive()
    }
}

impl ProviderPolicyTransport {
    pub fn new(writer: SharedProviderWriter, max_in_flight: usize, max_frame_bytes: usize) -> Self {
        Self {
            state: Arc::new(TransportState {
                writer,
                pending: Mutex::new(BTreeMap::new()),
                retired: Mutex::new(BTreeSet::new()),
                broken: Mutex::new(None),
                in_flight: AtomicUsize::new(0),
                next_correlation: AtomicU64::new(1),
                max_in_flight,
                max_frame_bytes,
            }),
        }
    }

    pub(crate) fn fail(&self, error: RequestPolicyTransportError) {
        let mut broken = self
            .state
            .broken
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if broken.is_some() {
            return;
        }
        *broken = Some(error.clone());
        drop(broken);
        let mut pending = self
            .state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = std::mem::take(&mut *pending);
        for (_, request) in pending {
            let _ = request.sender.send(Err(error.clone()));
        }
    }

    pub(crate) fn failure(&self) -> Option<RequestPolicyTransportError> {
        self.state
            .broken
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Completes a child-initiated policy request from the input demuxer.
    /// Unknown non-retired correlations break this policy transport.
    pub(crate) fn complete_result(
        &self,
        correlation_id: u64,
        payload: serde_json::Value,
    ) -> Result<(), RequestPolicyTransportError> {
        let pending = self
            .state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&correlation_id);
        if let Some(request) = pending {
            let _ = request.sender.send(Ok(payload));
            return Ok(());
        }

        let retired = self
            .state
            .retired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&correlation_id);
        if retired {
            return Ok(());
        }

        let error = RequestPolicyTransportError::ProtocolViolation(format!(
            "request-policy result for unknown correlation {correlation_id}"
        ));
        self.fail(error.clone());
        Err(error)
    }

    /// Converts a host `ProtocolError` response into a typed policy failure.
    pub(crate) fn complete_error(
        &self,
        correlation_id: u64,
        payload: serde_json::Value,
    ) -> Result<(), RequestPolicyTransportError> {
        let reason = payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("host rejected request-policy frame")
            .to_owned();
        let pending = self
            .state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&correlation_id);
        if let Some(request) = pending {
            let _ = request
                .sender
                .send(Err(RequestPolicyTransportError::ProtocolViolation(reason)));
            return Ok(());
        }
        let retired = self
            .state
            .retired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&correlation_id);
        if retired {
            return Ok(());
        }
        let error = RequestPolicyTransportError::ProtocolViolation(format!(
            "request-policy error for unknown correlation {correlation_id}"
        ));
        self.fail(error.clone());
        Err(error)
    }

    /// Applies a host cancellation notification to a pending policy request.
    /// Cancellation is idempotent for already-retired requests.
    pub(crate) fn cancel(&self, correlation_id: u64) {
        let pending = self
            .state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&correlation_id);
        if let Some(request) = pending {
            let _ = request
                .sender
                .send(Err(RequestPolicyTransportError::Cancelled));
        }
    }

    fn send(&self, envelope: &Envelope) -> Result<(), RequestPolicyTransportError> {
        let encoded = envelope.encode().map_err(|error| {
            RequestPolicyTransportError::ProtocolViolation(format!(
                "request-policy envelope encode failed: {error}"
            ))
        })?;
        if encoded.len() > self.state.max_frame_bytes {
            return Err(RequestPolicyTransportError::PayloadTooLarge {
                limit: self.state.max_frame_bytes,
                actual: encoded.len(),
            });
        }
        let mut writer = self
            .state
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let writer_ref: &mut dyn Write = &mut **writer;
        write_frame(writer_ref, envelope).map_err(|error| match error {
            NativeHostError::PayloadTooLarge { limit, actual } => {
                RequestPolicyTransportError::PayloadTooLarge { limit, actual }
            }
            _other => RequestPolicyTransportError::TransportClosed,
        })
    }

    fn remember_retired(&self, correlation_id: u64) {
        let mut retired = self
            .state
            .retired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        retired.insert(correlation_id);
        let limit = self
            .state
            .max_in_flight
            .saturating_mul(RETIRED_CORRELATION_FACTOR)
            .max(16);
        while retired.len() > limit {
            let Some(first) = retired.iter().next().copied() else {
                break;
            };
            retired.remove(&first);
        }
    }

    fn request_policy(
        &self,
        request: RequestPolicyRequest,
    ) -> Result<RequestPolicyResult, RequestPolicyTransportError> {
        request.validate().map_err(|reason| {
            RequestPolicyTransportError::ProtocolViolation(format!(
                "request-policy request is invalid: {reason}"
            ))
        })?;
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if self.state.in_flight.fetch_add(1, Ordering::SeqCst) >= self.state.max_in_flight {
            self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err(RequestPolicyTransportError::CapacityExceeded);
        }

        let correlation_id = self.state.next_correlation.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(correlation_id, PendingPolicyRequest { sender });

        let payload = serde_json::to_value(&request).map_err(|error| {
            RequestPolicyTransportError::ProtocolViolation(format!(
                "request-policy request encode failed: {error}"
            ))
        });
        let payload = match payload {
            Ok(payload) => payload,
            Err(error) => {
                self.remove_pending(correlation_id);
                self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
                return Err(error);
            }
        };
        let envelope = Envelope::new(MessageKind::RequestPolicyRequest, correlation_id, payload);
        if let Err(error) = self.send(&envelope) {
            self.remove_pending(correlation_id);
            self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.fail(error.clone());
            return Err(error);
        }

        let deadline = Duration::from_millis(request.deadline_ms);
        let received = receiver.recv_timeout(deadline);
        match received {
            Ok(Ok(payload)) => {
                self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
                if let Some(message) = payload.get("error").and_then(serde_json::Value::as_str) {
                    return Err(RequestPolicyTransportError::Unavailable(message.to_owned()));
                }
                let result: RequestPolicyResult =
                    serde_json::from_value(payload).map_err(|error| {
                        RequestPolicyTransportError::ProtocolViolation(format!(
                            "request-policy result is malformed: {error}"
                        ))
                    })?;
                result.validate().map_err(|reason| {
                    RequestPolicyTransportError::ProtocolViolation(format!(
                        "request-policy result is invalid: {reason}"
                    ))
                })?;
                Ok(result)
            }
            Ok(Err(error)) => {
                self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
                Err(error)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                self.remove_pending(correlation_id);
                self.remember_retired(correlation_id);
                self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
                let _ = self.send(&Envelope::new(
                    MessageKind::Cancellation,
                    correlation_id,
                    serde_json::json!({
                        "plane": "request_policy",
                        "reason": "deadline"
                    }),
                ));
                Err(RequestPolicyTransportError::DeadlineExceeded {
                    deadline_ms: request.deadline_ms,
                })
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.remove_pending(correlation_id);
                self.state.in_flight.fetch_sub(1, Ordering::SeqCst);
                Err(self
                    .failure()
                    .unwrap_or(RequestPolicyTransportError::TransportClosed))
            }
        }
    }

    fn remove_pending(&self, correlation_id: u64) {
        self.state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&correlation_id);
    }
}

impl RequestPolicyTransport for ProviderPolicyTransport {
    fn decide(
        &self,
        request: RequestPolicyRequest,
    ) -> Result<RequestPolicyResult, RequestPolicyTransportError> {
        self.request_policy(request)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Result as IoResult};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use serde_json::Value;
    use worldline_browser_contract::identity::BrowserContextId;
    use worldline_browser_contract::request_policy::{
        RequestPolicyAction, RequestPolicyMetadata, RequestPolicyOutcome, RequestResourceType,
    };
    use worldline_native_host::read_frame;

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    fn request(deadline_ms: u64) -> RequestPolicyRequest {
        RequestPolicyRequest {
            registration_id: "adblock-profile".to_owned(),
            metadata: RequestPolicyMetadata {
                context_id: BrowserContextId::new("ctx-a"),
                page_id: None,
                url: "http://127.0.0.1/asset.js".to_owned(),
                method: "GET".to_owned(),
                resource_type: RequestResourceType::Script,
                initiator_origin: Some("http://127.0.0.1".to_owned()),
                top_level_origin: Some("http://127.0.0.1".to_owned()),
            },
            deadline_ms,
        }
    }

    fn result() -> RequestPolicyResult {
        RequestPolicyResult {
            action: RequestPolicyAction::Allow,
            outcome: RequestPolicyOutcome::Evaluated,
            provider_id: Some("adblock".to_owned()),
            opaque_rule_ref: None,
        }
    }

    fn wait_for_frame(bytes: &Arc<Mutex<Vec<u8>>>) -> Envelope {
        for _ in 0..100 {
            let snapshot = bytes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Ok(envelope) = read_frame(&mut Cursor::new(snapshot), 4096) {
                return envelope;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("policy request frame was not written");
    }

    #[test]
    fn request_result_roundtrip_is_correlated_on_dedicated_plane() {
        let recording = RecordingWriter::default();
        let writer: SharedProviderWriter = Arc::new(Mutex::new(
            Box::new(recording.clone()) as Box<dyn Write + Send>
        ));
        let transport = ProviderPolicyTransport::new(writer, 1, 4096);
        let waiting = transport.clone();
        let join = thread::spawn(move || waiting.decide(request(100)));

        let request_frame = wait_for_frame(&recording.bytes);
        assert_eq!(
            request_frame.message_kind,
            MessageKind::RequestPolicyRequest
        );
        assert_eq!(request_frame.correlation_id, 1);
        transport
            .complete_result(
                request_frame.correlation_id,
                serde_json::to_value(result()).expect("result json"),
            )
            .expect("known result correlation");

        let received = join.join().expect("transport waiter").expect("result");
        assert_eq!(received, result());
    }

    #[test]
    fn timeout_sends_plane_specific_cancellation() {
        let recording = RecordingWriter::default();
        let writer: SharedProviderWriter = Arc::new(Mutex::new(
            Box::new(recording.clone()) as Box<dyn Write + Send>
        ));
        let transport = ProviderPolicyTransport::new(writer, 1, 4096);

        let error = transport.decide(request(5)).expect_err("must time out");
        assert_eq!(
            error,
            RequestPolicyTransportError::DeadlineExceeded { deadline_ms: 5 }
        );
        let bytes = recording
            .bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let mut cursor = Cursor::new(bytes);
        let request_frame = read_frame(&mut cursor, 4096).expect("request frame");
        let cancellation = read_frame(&mut cursor, 4096).expect("cancellation frame");
        assert_eq!(
            request_frame.message_kind,
            MessageKind::RequestPolicyRequest
        );
        assert_eq!(cancellation.message_kind, MessageKind::Cancellation);
        assert_eq!(cancellation.correlation_id, request_frame.correlation_id);
        assert_eq!(
            cancellation.payload.get("plane").and_then(Value::as_str),
            Some("request_policy")
        );
    }

    #[test]
    fn unknown_result_correlation_breaks_policy_transport() {
        let recording = RecordingWriter::default();
        let writer: SharedProviderWriter =
            Arc::new(Mutex::new(Box::new(recording) as Box<dyn Write + Send>));
        let transport = ProviderPolicyTransport::new(writer, 1, 4096);
        let error = transport
            .complete_result(999, serde_json::to_value(result()).expect("result json"))
            .expect_err("unknown result must fail closed");
        assert!(matches!(
            error,
            RequestPolicyTransportError::ProtocolViolation(_)
        ));
        assert_eq!(transport.failure(), Some(error));
    }
}
