use std::{collections::BTreeMap, sync::Arc};

use worldline_kernel::{
    BackendState, CausationRef, CorrelationId, DeliveryMode, EventContract, EventEnvelope, EventId,
    InstallationId, InstallationRecord, InstallationStatus, InterfaceVersion, InvocationId,
    OutboxId, OutboxRecord, OutboxStatus, OutboxStore, PluginId, PrincipalId, StateBackend,
    StateError, StateKey, StateRevision, StateSchemaVersion, StateTransactionId,
};

fn initial_state() -> BackendState {
    let installation = InstallationId::new("kernel-persistence-installation");
    BackendState::new(
        InstallationRecord::from_parts(
            installation,
            PluginId::new("kernel-persistence-plugin"),
            StateSchemaVersion::new(1),
            InstallationStatus::Ready,
            StateRevision::new(0),
            0,
        ),
        BTreeMap::new(),
    )
}

fn event() -> EventEnvelope {
    EventEnvelope::from_parts(
        EventId::new("kernel-persistence-event"),
        EventContract::new(
            "worldline.kernel.persistence",
            "state-committed",
            InterfaceVersion::new(1, 0),
        ),
        PrincipalId::new("kernel-persistence-producer"),
        None,
        1,
        CorrelationId::new("kernel-persistence-correlation"),
        Some(CausationRef::Invocation(InvocationId::new(
            "kernel-persistence-invocation",
        ))),
        DeliveryMode::Durable,
        b"opaque".to_vec(),
        None,
    )
}

#[test]
fn in_memory_backend_keeps_state_and_outbox_atomic_and_recoverable() {
    let backend = Arc::new(worldline_kernel::InMemoryStateBackend::new());
    StateBackend::create(&*backend, initial_state()).expect("initial state must commit");
    let installation = InstallationId::new("kernel-persistence-installation");
    let mut values = BTreeMap::new();
    values.insert(StateKey::new("committed"), b"yes".to_vec());
    let outbox = OutboxRecord::new(
        OutboxId::new("kernel-persistence-outbox"),
        installation.clone(),
        event(),
        1,
        10,
    );
    StateBackend::commit_if_revision_with_outbox(
        &*backend,
        &installation,
        &StateTransactionId::new("kernel-persistence-transaction"),
        StateRevision::new(0),
        BackendState::new(
            InstallationRecord::from_parts(
                installation.clone(),
                PluginId::new("kernel-persistence-plugin"),
                StateSchemaVersion::new(1),
                InstallationStatus::Ready,
                StateRevision::new(1),
                0,
            ),
            values,
        ),
        &outbox,
    )
    .expect("state plus outbox must commit atomically");

    let pending = OutboxStore::list_pending(&*backend, 10).expect("pending list must work");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status(), OutboxStatus::Pending);
    assert_eq!(pending[0].event().event_id(), outbox.event().event_id());
    assert_eq!(
        StateBackend::snapshot(&*backend, &installation)
            .expect("state snapshot must work")
            .values()
            .get(&StateKey::new("committed")),
        Some(&b"yes".to_vec())
    );

    let delivered = OutboxStore::mark_delivering(&*backend, outbox.outbox_id())
        .expect("delivery transition must work");
    assert_eq!(delivered.attempt_count(), 1);
    OutboxStore::mark_delivered(&*backend, outbox.outbox_id())
        .expect("delivery completion must work");
    assert!(
        OutboxStore::list_pending(&*backend, 10)
            .expect("pending list must work")
            .is_empty()
    );
}

#[test]
fn non_durable_outbox_is_rejected_without_state_mutation() {
    let backend = Arc::new(worldline_kernel::InMemoryStateBackend::new());
    StateBackend::create(&*backend, initial_state()).expect("initial state must commit");
    let installation = InstallationId::new("kernel-persistence-installation");
    let mut non_durable = event();
    non_durable = EventEnvelope::from_parts(
        non_durable.event_id().clone(),
        non_durable.contract().clone(),
        non_durable.producer().clone(),
        non_durable.producer_runtime_id(),
        non_durable.sequence(),
        non_durable.correlation_id().clone(),
        non_durable.causation().cloned(),
        DeliveryMode::Ephemeral,
        non_durable.payload().to_vec(),
        None,
    );
    let result = StateBackend::commit_if_revision_with_outbox(
        &*backend,
        &installation,
        &StateTransactionId::new("kernel-persistence-non-durable"),
        StateRevision::new(0),
        BackendState::new(
            InstallationRecord::from_parts(
                installation.clone(),
                PluginId::new("kernel-persistence-plugin"),
                StateSchemaVersion::new(1),
                InstallationStatus::Ready,
                StateRevision::new(1),
                0,
            ),
            BTreeMap::from([(StateKey::new("must-not-commit"), b"no".to_vec())]),
        ),
        &OutboxRecord::new(
            OutboxId::new("kernel-persistence-invalid-outbox"),
            installation.clone(),
            non_durable,
            1,
            10,
        ),
    );
    assert!(matches!(
        result,
        Err(StateError::Persistence(
            worldline_kernel::PersistenceError::OutboxAppendFailed { .. }
        ))
    ));
    let snapshot =
        StateBackend::snapshot(&*backend, &installation).expect("state snapshot must work");
    assert_eq!(snapshot.record().state_revision(), StateRevision::new(0));
    assert!(
        !snapshot
            .values()
            .contains_key(&StateKey::new("must-not-commit"))
    );
}
