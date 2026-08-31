//! Native-process execution of `reference.echo/v1`.
//!
//! The plugin running in-process is a thin proxy: every capability call is
//! forwarded over the versioned IPC transport to a supervised child process
//! that performs the actual semantics. The child never keeps authoritative
//! state and never gains authority: its state round-trips are routed back
//! to the installation-owned state contract through the host sink, and its
//! event publications are replayed through the invocation context so the
//! producer identity stays host-stamped.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use worldline_kernel::{
    CapabilityId, CapabilityService, EventContract, EventPublishOptions, InterfaceVersion,
    InvocationContext, Plugin, PluginDefinition, PluginError, PluginId, PluginRuntime,
    RuntimeStateHandle,
};
use worldline_native_host::{
    ExpectedIdentity, HostRequestSink, NativeChildSpec, NativeHostError, NativeProviderConnection,
};
use worldline_plugin_protocol::MessageKind;

use crate::echo::{OPERATION_ECHO, echo_capability};

/// Configuration for one native echo provider child.
#[derive(Clone, Debug)]
pub struct NativeEchoOptions {
    pub package_id: String,
    pub plugin_definition_id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub max_frame_bytes: usize,
    pub stderr_max_bytes: usize,
    pub max_in_flight: usize,
}

impl NativeEchoOptions {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        let package_id = "reference.echo.pkg".to_owned();
        let plugin_definition_id = "reference.echo.native".to_owned();
        Self {
            args: vec![
                "--package-id".to_owned(),
                package_id.clone(),
                "--definition-id".to_owned(),
                plugin_definition_id.clone(),
            ],
            package_id,
            plugin_definition_id,
            program: program.into(),
            max_frame_bytes: 4 * 1024 * 1024,
            stderr_max_bytes: 64 * 1024,
            max_in_flight: 16,
        }
    }
}

type Publications = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

/// Routes child-initiated state requests into the installation-owned state
/// contract and records child event publications for replay through the
/// invocation context under the runtime's own authority.
struct NativeEchoSink {
    state: Arc<Mutex<Option<RuntimeStateHandle>>>,
    publications: Publications,
}

impl NativeEchoSink {
    fn locked_state(&self) -> Result<RuntimeStateHandle, NativeHostError> {
        self.state
            .lock()
            .map_err(|_| slot_poisoned())?
            .clone()
            .ok_or_else(|| transport_failure("native echo state handle is not initialized"))
    }
}

fn slot_poisoned() -> NativeHostError {
    NativeHostError::ProtocolViolation {
        reason: "state slot poisoned".to_owned(),
    }
}

fn transport_failure(reason: &str) -> NativeHostError {
    NativeHostError::ProtocolViolation {
        reason: reason.to_owned(),
    }
}

impl HostRequestSink for NativeEchoSink {
    fn on_child_request(
        &self,
        kind: MessageKind,
        _correlation_id: u64,
        payload: Value,
    ) -> Result<Option<Value>, NativeHostError> {
        match kind {
            MessageKind::StateRequest => {
                let key = payload
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                match payload.get("action").and_then(Value::as_str) {
                    Some("get") => {
                        let value = self
                            .locked_state()?
                            .get(key.as_str())
                            .map_err(|error| transport_failure(&error.to_string()))?;
                        Ok(Some(json!({ "value": value })))
                    }
                    Some("set") => {
                        let value = payload_bytes(&payload, "value");
                        let state = self.locked_state()?;
                        let mut transaction = state
                            .transaction()
                            .map_err(|error| transport_failure(&error.to_string()))?;
                        transaction
                            .put(key.as_str(), &value)
                            .map_err(|error| transport_failure(&error.to_string()))?;
                        transaction
                            .commit()
                            .map_err(|error| transport_failure(&error.to_string()))?;
                        Ok(None)
                    }
                    other => Err(NativeHostError::ProtocolViolation {
                        reason: format!("unknown state action {other:?}"),
                    }),
                }
            }
            MessageKind::EventPublishRequest => {
                let namespace = string_field(&payload, "namespace");
                let name = string_field(&payload, "name");
                let bytes = payload_bytes(&payload, "bytes");
                self.publications
                    .lock()
                    .map_err(|_| slot_poisoned())?
                    .push((namespace, name, bytes));
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

fn string_field(payload: &Value, field: &str) -> String {
    payload
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn payload_bytes(payload: &Value, field: &str) -> Vec<u8> {
    payload
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_u64)
                .map(|value| value as u8)
                .collect::<Vec<u8>>()
        })
        .unwrap_or_default()
}

fn decode_reply(reply: Value) -> Result<Vec<u8>, String> {
    if let Some(message) = reply.get("error").and_then(Value::as_str) {
        return Err(message.to_owned());
    }
    reply
        .get("bytes")
        .map(payload_bytes_of)
        .ok_or_else(|| "native reply carries no bytes".to_owned())
}

fn payload_bytes_of(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_u64)
                .map(|value| value as u8)
                .collect::<Vec<u8>>()
        })
        .unwrap_or_default()
}

/// The native execution mode of `reference.echo/v1`.
pub struct NativeEchoPlugin {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<NativeEchoService>,
}

struct NativeEchoService {
    options: NativeEchoOptions,
    state: Arc<Mutex<Option<RuntimeStateHandle>>>,
    publications: Publications,
    connection: Arc<Mutex<Option<Arc<NativeProviderConnection>>>>,
}

impl NativeEchoService {
    fn connection(&self) -> Result<Arc<NativeProviderConnection>, String> {
        let mut guard = self
            .connection
            .lock()
            .map_err(|_| "connection slot poisoned".to_owned())?;
        if let Some(connection) = guard.as_ref() {
            return Ok(Arc::clone(connection));
        }
        let identity = ExpectedIdentity {
            package_id: self.options.package_id.clone(),
            plugin_definition_id: self.options.plugin_definition_id.clone(),
        };
        let spec = NativeChildSpec {
            program: self.options.program.clone(),
            args: self.options.args.clone(),
            max_frame_bytes: self.options.max_frame_bytes,
            stderr_max_bytes: self.options.stderr_max_bytes,
        };
        let sink = Arc::new(NativeEchoSink {
            state: Arc::clone(&self.state),
            publications: Arc::clone(&self.publications),
        });
        let (connection, _ack) =
            NativeProviderConnection::connect(spec, &identity, sink, self.options.max_in_flight)
                .map_err(|error| error.to_string())?;
        let connection = Arc::new(connection);
        *guard = Some(Arc::clone(&connection));
        Ok(connection)
    }

    fn replay_publications(&self, context: &InvocationContext) -> Result<(), String> {
        let drained: Vec<(String, String, Vec<u8>)> = self
            .publications
            .lock()
            .map_err(|_| "publications queue poisoned".to_owned())?
            .drain(..)
            .collect();
        for (namespace, name, bytes) in drained {
            context
                .publish_event(
                    EventContract::new(namespace, name, InterfaceVersion::new(1, 0)),
                    &bytes,
                    EventPublishOptions::default(),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

impl CapabilityService for NativeEchoService {
    fn invoke(&self, operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        if operation == OPERATION_ECHO {
            self.connection()?;
            Ok(crate::echo::semantics::echo(payload))
        } else {
            Err(format!(
                "operation '{operation}' requires an invocation context"
            ))
        }
    }

    fn invoke_with_context(
        &self,
        context: &InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let connection = self.connection()?;
        let reply = connection
            .call(json!({
                "operation": context.operation().as_str(),
                "bytes": payload,
            }))
            .map_err(|error| error.to_string())?;
        let bytes = decode_reply(reply)?;
        // Child-side publications are replayed here so the producer identity
        // stays host-stamped under this runtime's own authority.
        self.replay_publications(context)?;
        Ok(bytes)
    }
}

/// The native execution mode plugin.
impl NativeEchoPlugin {
    pub fn new(plugin: impl Into<String>, options: NativeEchoOptions) -> Self {
        let capability = echo_capability();
        let state = Arc::new(Mutex::new(None));
        let publications: Publications = Arc::new(Mutex::new(Vec::new()));
        Self {
            definition: PluginDefinition::new(PluginId::new(plugin)).provides(capability.clone()),
            capability,
            service: Arc::new(NativeEchoService {
                options,
                state: Arc::clone(&state),
                publications: Arc::clone(&publications),
                connection: Arc::new(Mutex::new(None)),
            }),
        }
    }
}

impl Plugin for NativeEchoPlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut worldline_kernel::ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        *self
            .service
            .state
            .lock()
            .map_err(|_| PluginError::new("native echo state slot is poisoned"))? =
            Some(context.state().clone());
        let service: Arc<dyn CapabilityService> = Arc::clone(&self.service) as _;
        context.publish_capability(self.capability.clone(), service)?;
        Ok(Box::new(NativeEchoRuntime::for_service(Arc::clone(
            &self.service,
        ))))
    }
}

/// Folds the live external effect (the child process) on deactivation.
struct NativeEchoRuntime {
    service: Arc<NativeEchoService>,
}

impl NativeEchoRuntime {
    fn for_service(service: Arc<NativeEchoService>) -> Self {
        Self { service }
    }
}

impl PluginRuntime for NativeEchoRuntime {
    fn deactivate(&mut self) -> Result<(), PluginError> {
        let connection = self
            .service
            .connection
            .lock()
            .map_err(|_| PluginError::new("connection slot poisoned"))?
            .take();
        if let Some(connection) = connection {
            connection
                .close(std::time::Duration::from_secs(5))
                .map_err(|error| PluginError::new(error.to_string()))?;
        }
        Ok(())
    }
}
