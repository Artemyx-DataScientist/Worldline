use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use worldline_kernel::{
    ActivationContext, CapabilityId, CapabilityService, CausationRef, DeliveryMode, EventContract,
    EventEnvelope, EventJournal, EventPublishOptions, GrantLifetime, InMemoryStateBackend,
    InterfaceVersion, Kernel, NoopRuntime, OutboxRecord, OutboxStore, Plugin, PluginDefinition,
    PluginError, PluginRuntime, PrincipalId, PrincipalKind, ResourceId, ResourceScope,
    RpcCallOptions, RuntimeStateHandle, StateBackend, SubscriptionHandle, SubscriptionOptions,
    TraceContext,
};
use worldline_storage::{SqliteEventJournal, SqliteStateBackend};

fn source_capability() -> CapabilityId {
    CapabilityId::new(
        "worldline.s1",
        "stateful-source",
        InterfaceVersion::new(1, 0),
    )
}

fn follow_up_capability() -> CapabilityId {
    CapabilityId::new("worldline.s1", "follow-up", InterfaceVersion::new(1, 0))
}

fn observation_contract() -> EventContract {
    EventContract::new(
        "worldline.s1",
        "state-committed",
        InterfaceVersion::new(1, 0),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S1Report {
    pub installation_id: String,
    pub state_before_restart: String,
    pub state_after_restart: String,
    pub old_runtime_id: String,
    pub new_runtime_id: String,
    pub first_result: String,
    pub restarted_result: String,
    pub follow_up_result: String,
    pub observed_events: usize,
    pub control_observation_was_metadata_only: bool,
    pub old_runtime_authority_revoked: bool,
    pub new_runtime_required_explicit_authority: bool,
}

struct StatefulService {
    state: Arc<Mutex<Option<RuntimeStateHandle>>>,
    event: EventContract,
    durable_outbox: bool,
}

impl CapabilityService for StatefulService {
    fn invoke(&self, _operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(payload.to_vec())
    }

    fn invoke_with_context(
        &self,
        context: &worldline_kernel::InvocationContext,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "state slot is poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "state handle is not initialized".to_owned())?;
        let current = state
            .get("committed-count")
            .map_err(|error| error.to_string())?
            .and_then(|value| String::from_utf8(value).ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_default()
            .checked_add(1)
            .ok_or_else(|| "committed count exhausted".to_owned())?;
        let mut transaction = state.transaction().map_err(|error| error.to_string())?;
        transaction
            .put("committed-count", current.to_string().as_bytes())
            .map_err(|error| error.to_string())?;
        if self.durable_outbox {
            let event = EventEnvelope::from_parts(
                worldline_kernel::EventId::new(format!("s1-outbox-event-{current}")),
                self.event.clone(),
                context.provider().clone(),
                Some(context.provider_runtime_id()),
                current,
                context.trace_context().correlation_id().clone(),
                Some(CausationRef::Invocation(context.invocation_id().clone())),
                DeliveryMode::Durable,
                payload.to_vec(),
                None,
            );
            let outbox = OutboxRecord::new(
                format!("s1-outbox-{current}"),
                state.installation_id().clone(),
                event,
                current,
                current,
            );
            transaction
                .commit_with_outbox(outbox)
                .map_err(|error| error.to_string())?;
        } else {
            transaction.commit().map_err(|error| error.to_string())?;
        }
        context
            .publish_event(self.event.clone(), payload, EventPublishOptions::default())
            .map_err(|error| error.to_string())?;
        Ok(format!("stateful:{current}:{}", String::from_utf8_lossy(payload)).into_bytes())
    }
}

struct StatefulProvider {
    definition: PluginDefinition,
    capability: CapabilityId,
    service: Arc<StatefulService>,
}

impl StatefulProvider {
    fn new_with_outbox(
        plugin: &str,
        capability: CapabilityId,
        event: EventContract,
        durable_outbox: bool,
    ) -> Self {
        let state = Arc::new(Mutex::new(None));
        Self {
            definition: PluginDefinition::new(plugin).provides(capability.clone()),
            capability,
            service: Arc::new(StatefulService {
                state,
                event,
                durable_outbox,
            }),
        }
    }
}

impl Plugin for StatefulProvider {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        *self
            .service
            .state
            .lock()
            .map_err(|_| PluginError::new("state slot is poisoned"))? =
            Some(context.state().clone());
        let service: Arc<dyn CapabilityService> = self.service.clone();
        context.publish_capability(self.capability.clone(), service)?;
        Ok(Box::new(NoopRuntime))
    }
}

struct FollowUpService;

impl CapabilityService for FollowUpService {
    fn invoke(&self, _operation: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        Ok(format!("follow-up:{}", String::from_utf8_lossy(payload)).into_bytes())
    }
}

struct FollowUpProvider {
    definition: PluginDefinition,
    capability: CapabilityId,
}

impl Plugin for FollowUpProvider {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        context.publish_capability(self.capability.clone(), Arc::new(FollowUpService))?;
        Ok(Box::new(NoopRuntime))
    }
}

fn grant(
    kernel: &Kernel,
    subject: &PrincipalId,
    capability: &CapabilityId,
    operation: &str,
) -> worldline_kernel::GrantId {
    kernel
        .create_root_grant(
            subject.clone(),
            capability.contract(),
            [operation],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .expect("S1 grant must be accepted")
}

fn register_subjects(kernel: &Kernel) -> (PrincipalId, PrincipalId) {
    let caller = PrincipalId::new("s1-caller");
    let observer = PrincipalId::new("s1-observer");
    kernel
        .register_principal_id(caller.clone(), PrincipalKind::Agent)
        .expect("S1 caller must register");
    kernel
        .register_principal_id(observer.clone(), PrincipalKind::Agent)
        .expect("S1 observer must register");
    (caller, observer)
}

fn subscribe_pair(
    kernel: &Kernel,
    observer: &PrincipalId,
    event: &EventContract,
) -> Result<(SubscriptionHandle, SubscriptionHandle), String> {
    let custom = kernel
        .subscribe(
            observer.clone(),
            event.clone(),
            SubscriptionOptions::default(),
        )
        .map_err(|error| error.to_string())?;
    let control = kernel
        .subscribe(
            observer.clone(),
            worldline_kernel::invocation_completed_event_contract(),
            SubscriptionOptions::default(),
        )
        .map_err(|error| error.to_string())?;
    Ok((custom, control))
}

fn follow_up(
    custom_subscription: &SubscriptionHandle,
    event: EventEnvelope,
    capability: CapabilityId,
) -> Result<String, String> {
    let context = custom_subscription.context(event);
    let result = context
        .invoke(
            capability.clone(),
            "run",
            ResourceId::root(capability.namespace()),
            b"observer-action",
        )
        .map_err(|error| error.to_string())?;
    String::from_utf8(result).map_err(|error| error.to_string())
}

fn read_committed_count(
    kernel: &Kernel,
    installation: &worldline_kernel::InstallationId,
) -> Result<String, String> {
    let value = kernel
        .state_handle(installation)
        .map_err(|error| error.to_string())?
        .get("committed-count")
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "committed count is missing".to_owned())?;
    String::from_utf8(value).map_err(|error| error.to_string())
}

/// Runs S1 through two real Kernel instances over one caller-owned state
/// backend.  Event transport is pull-based and remains independent from the
/// RPC result and persisted installation state.
pub fn run() -> Result<S1Report, String> {
    let backend: Arc<dyn worldline_kernel::StateBackend> = Arc::new(InMemoryStateBackend::new());
    run_with_backend(backend, None, None)
}

/// Runs S1 with the production SQLite StateBackend, SQLite EventJournal, and
/// transactional outbox dispatcher. The dispatcher reuses the admitted
/// envelope from the outbox, so redelivery never allocates a new EventId.
pub fn run_production(profile_root: impl AsRef<Path>) -> Result<S1Report, String> {
    let state_backend = Arc::new(
        SqliteStateBackend::open(profile_root.as_ref()).map_err(|error| error.to_string())?,
    );
    let backend: Arc<dyn StateBackend> = state_backend.clone();
    let outbox: Arc<dyn OutboxStore> = state_backend;
    let journal: Arc<dyn EventJournal> = Arc::new(
        SqliteEventJournal::open(profile_root.as_ref()).map_err(|error| error.to_string())?,
    );
    run_with_backend(backend, Some(outbox), Some(journal))
}

fn run_with_backend(
    backend: Arc<dyn StateBackend>,
    outbox: Option<Arc<dyn OutboxStore>>,
    journal: Option<Arc<dyn EventJournal>>,
) -> Result<S1Report, String> {
    let source = source_capability();
    let follow_up_capability = follow_up_capability();
    let event = observation_contract();
    let control = worldline_kernel::invocation_completed_event_contract();
    let durable_outbox = outbox.is_some();

    let mut first =
        Kernel::with_state_backend(Arc::clone(&backend)).map_err(|error| error.to_string())?;
    if let Some(journal) = journal.as_ref() {
        first.set_event_journal(Arc::clone(journal));
    }
    let source_plugin = first
        .register(StatefulProvider::new_with_outbox(
            "s1.stateful.provider",
            source.clone(),
            event.clone(),
            durable_outbox,
        ))
        .map_err(|error| error.to_string())?;
    let installation = first
        .installation_id_for_plugin(&source_plugin)
        .ok_or_else(|| "S1 source installation is missing".to_owned())?;
    let old_runtime_id = first
        .runtime_id_for_plugin(&source_plugin)
        .ok_or_else(|| "S1 source runtime is missing".to_owned())?;
    let old_runtime_principal = first
        .principal_for_runtime(&old_runtime_id)
        .ok_or_else(|| "S1 source principal is missing".to_owned())?;
    let follow_up_plugin = first
        .register(FollowUpProvider {
            definition: PluginDefinition::new("s1.follow-up.provider")
                .provides(follow_up_capability.clone()),
            capability: follow_up_capability.clone(),
        })
        .map_err(|error| error.to_string())?;
    let _follow_up_runtime = first
        .runtime_id_for_plugin(&follow_up_plugin)
        .ok_or_else(|| "S1 follow-up runtime is missing".to_owned())?;
    let (caller, observer) = register_subjects(&first);
    grant(&first, &caller, &source, "run");
    grant(
        &first,
        &old_runtime_principal,
        &event.capability_id(),
        "publish",
    );
    grant(&first, &observer, &event.capability_id(), "subscribe");
    grant(&first, &observer, &control.capability_id(), "subscribe");
    grant(&first, &observer, &follow_up_capability, "run");
    let (custom_subscription, control_subscription) = subscribe_pair(&first, &observer, &event)?;

    let first_result = String::from_utf8(
        first
            .capability_for(caller.clone(), source.clone())
            .map_err(|error| error.to_string())?
            .invoke_with_options(
                "run",
                b"first",
                RpcCallOptions::new()
                    .with_request_id("s1-first")
                    .with_trace_context(TraceContext::new("s1-activity")),
            )
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let first_event = custom_subscription
        .try_recv()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "S1 custom observation is missing".to_owned())?;
    let follow_up_result = follow_up(
        &custom_subscription,
        first_event,
        follow_up_capability.clone(),
    )?;
    let control_event = control_subscription
        .try_recv()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "S1 InvocationCompleted observation is missing".to_owned())?;
    let control_observation_was_metadata_only =
        control_event.payload().is_empty() && control_event.invocation_completed().is_some();
    let state_before_restart = read_committed_count(&first, &installation)?;

    let old_runtime_grant = grant(&first, &old_runtime_principal, &source, "run");
    first
        .unregister(&source_plugin)
        .map_err(|error| error.to_string())?;
    let old_runtime_authority_revoked = !first.is_grant_active(&old_runtime_grant)
        && first
            .publish_event_for_runtime(
                old_runtime_id,
                event.clone(),
                b"old-runtime",
                EventPublishOptions::default(),
            )
            .is_err();
    drop(custom_subscription);
    drop(control_subscription);
    drop(first);
    if let (Some(outbox), Some(journal)) = (outbox.as_ref(), journal.as_ref()) {
        drain_outbox(outbox.as_ref(), journal.as_ref())?;
    }

    let mut restarted =
        Kernel::with_state_backend(Arc::clone(&backend)).map_err(|error| error.to_string())?;
    if let Some(journal) = journal.as_ref() {
        restarted.set_event_journal(Arc::clone(journal));
    }
    let restarted_source = restarted
        .register_for_installation(
            StatefulProvider::new_with_outbox(
                "s1.stateful.provider",
                source.clone(),
                event.clone(),
                durable_outbox,
            ),
            &installation,
        )
        .map_err(|error| error.to_string())?;
    let new_runtime_id = restarted
        .runtime_id_for_plugin(&restarted_source)
        .ok_or_else(|| "S1 restarted source runtime is missing".to_owned())?;
    let new_runtime_principal = restarted
        .principal_for_runtime(&new_runtime_id)
        .ok_or_else(|| "S1 restarted source principal is missing".to_owned())?;
    let _follow_up_plugin = restarted
        .register(FollowUpProvider {
            definition: PluginDefinition::new("s1.follow-up.provider")
                .provides(follow_up_capability.clone()),
            capability: follow_up_capability.clone(),
        })
        .map_err(|error| error.to_string())?;
    let (caller, observer) = register_subjects(&restarted);
    grant(&restarted, &caller, &source, "run");
    let new_runtime_required_explicit_authority = restarted
        .publish_event_for_runtime(
            new_runtime_id,
            event.clone(),
            b"before-new-grant",
            EventPublishOptions::default(),
        )
        .is_err();
    grant(
        &restarted,
        &new_runtime_principal,
        &event.capability_id(),
        "publish",
    );
    grant(&restarted, &observer, &event.capability_id(), "subscribe");
    grant(&restarted, &observer, &control.capability_id(), "subscribe");
    grant(&restarted, &observer, &follow_up_capability, "run");
    let (custom_subscription, control_subscription) =
        subscribe_pair(&restarted, &observer, &event)?;
    let restarted_result = String::from_utf8(
        restarted
            .capability_for(caller, source)
            .map_err(|error| error.to_string())?
            .invoke("run", b"restarted")
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let _ = custom_subscription
        .try_recv()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "S1 restarted observation is missing".to_owned())?;
    let _ = control_subscription
        .try_recv()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "S1 restarted control observation is missing".to_owned())?;
    let state_after_restart = read_committed_count(&restarted, &installation)?;
    drop(custom_subscription);
    drop(control_subscription);
    drop(restarted);
    if let (Some(outbox), Some(journal)) = (outbox.as_ref(), journal.as_ref()) {
        drain_outbox(outbox.as_ref(), journal.as_ref())?;
    }

    Ok(S1Report {
        installation_id: installation.to_string(),
        state_before_restart,
        state_after_restart,
        old_runtime_id: old_runtime_id.to_string(),
        new_runtime_id: new_runtime_id.to_string(),
        first_result,
        restarted_result,
        follow_up_result,
        observed_events: 2,
        control_observation_was_metadata_only,
        old_runtime_authority_revoked,
        new_runtime_required_explicit_authority,
    })
}

fn drain_outbox(outbox: &dyn OutboxStore, journal: &dyn EventJournal) -> Result<(), String> {
    for pending in outbox.list_pending(64).map_err(|error| error.to_string())? {
        let delivering = outbox
            .mark_delivering(pending.outbox_id())
            .map_err(|error| error.to_string())?;
        journal
            .append(delivering.event())
            .map_err(|error| error.to_string())?;
        outbox
            .mark_delivered(delivering.outbox_id())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
