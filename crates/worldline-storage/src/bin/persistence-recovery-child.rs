use std::{collections::BTreeMap, env, path::PathBuf, process};

use worldline_kernel::{
    BackendState, CausationRef, CorrelationId, DeliveryMode, EventContract, EventEnvelope, EventId,
    EventJournal, InstallationId, InstallationRecord, InstallationStatus, InterfaceVersion,
    InvocationId, JobRecord, JobState, OutboxId, OutboxRecord, PluginId, PrincipalId, StateBackend,
    StateRevision, StateSchemaVersion, StateTransactionId,
};
use worldline_storage::{FilesystemBlobStore, SqliteEventJournal, SqliteStateBackend};

const INSTALLATION: &str = "installation-recovery-child";

fn main() {
    let mut arguments = env::args().skip(1);
    let scenario = arguments.next().unwrap_or_else(|| usage_and_exit());
    let profile = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| usage_and_exit());

    let result = match scenario.as_str() {
        "before-state-commit" => state_commit(&profile),
        "after-state-commit" => state_commit(&profile),
        "after-state-outbox-commit-before-publish" => state_outbox_commit(&profile),
        "after-event-publish-before-delivered" => publish_before_delivered(&profile),
        "during-migration" => metadata_transition(&profile, InstallationStatus::Migrating),
        "during-uninstall-metadata-transition" => {
            metadata_transition(&profile, InstallationStatus::Uninstalling)
        }
        "after-job-running-before-completion" => running_job(&profile),
        "during-blob-temporary-write" => partial_blob(&profile),
        other => Err(format!("unknown recovery scenario '{other}'")),
    };
    if let Err(error) = result {
        eprintln!("persistence recovery child failed: {error}");
        process::exit(2);
    }
}

fn state_commit(profile: &PathBuf) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    StateBackend::create(&backend, initial_state()).map_err(|error| error.to_string())?;
    let mut values = BTreeMap::new();
    values.insert(
        worldline_kernel::StateKey::new("committed"),
        b"child-commit".to_vec(),
    );
    backend
        .commit_if_revision(
            &InstallationId::new(INSTALLATION),
            &StateTransactionId::new("child-state-commit"),
            StateRevision::new(0),
            BackendState::new(
                record(StateRevision::new(1), InstallationStatus::Ready),
                values,
            ),
        )
        .map_err(|error| error.to_string())
}

fn state_outbox_commit(profile: &PathBuf) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    StateBackend::create(&backend, initial_state()).map_err(|error| error.to_string())?;
    let installation = InstallationId::new(INSTALLATION);
    let event = durable_event("child-outbox-event");
    let outbox = OutboxRecord::new(
        OutboxId::new("child-outbox"),
        installation.clone(),
        event,
        1,
        100,
    );
    let mut values = BTreeMap::new();
    values.insert(
        worldline_kernel::StateKey::new("committed"),
        b"child-outbox-commit".to_vec(),
    );
    backend
        .commit_if_revision_with_outbox(
            &installation,
            &StateTransactionId::new("child-outbox-commit"),
            StateRevision::new(0),
            BackendState::new(
                record(StateRevision::new(1), InstallationStatus::Ready),
                values,
            ),
            &outbox,
        )
        .map_err(|error| error.to_string())
}

fn publish_before_delivered(profile: &PathBuf) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    StateBackend::create(&backend, initial_state()).map_err(|error| error.to_string())?;
    let installation = InstallationId::new(INSTALLATION);
    let event = durable_event("child-publish-event");
    let outbox = OutboxRecord::new(
        OutboxId::new("child-publish-outbox"),
        installation.clone(),
        event,
        1,
        100,
    );
    let mut values = BTreeMap::new();
    values.insert(
        worldline_kernel::StateKey::new("committed"),
        b"child-publish-commit".to_vec(),
    );
    backend
        .commit_if_revision_with_outbox(
            &installation,
            &StateTransactionId::new("child-publish-commit"),
            StateRevision::new(0),
            BackendState::new(
                record(StateRevision::new(1), InstallationStatus::Ready),
                values,
            ),
            &outbox,
        )
        .map_err(|error| error.to_string())?;
    let delivering = worldline_kernel::OutboxStore::mark_delivering(
        &backend,
        &OutboxId::new("child-publish-outbox"),
    )
    .map_err(|error| error.to_string())?;
    let journal = SqliteEventJournal::open(profile).map_err(|error| error.to_string())?;
    journal
        .append(delivering.event())
        .map_err(|error| error.to_string())?;
    #[cfg(feature = "test-failpoints")]
    worldline_storage::trigger_test_failpoint("after-event-publish-before-delivered");
    Ok(())
}

fn metadata_transition(profile: &PathBuf, status: InstallationStatus) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    StateBackend::create(&backend, initial_state()).map_err(|error| error.to_string())?;
    backend
        .update_record_if_revision(StateRevision::new(0), record(StateRevision::new(1), status))
        .map_err(|error| error.to_string())
}

fn running_job(profile: &PathBuf) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    let job = JobRecord::new("child-running-job", PrincipalId::new("child-owner"))
        .with_state(JobState::Running)
        .with_attempt(1);
    worldline_kernel::JobStore::create(&backend, job).map_err(|error| error.to_string())
}

fn partial_blob(profile: &PathBuf) -> Result<(), String> {
    let blob = FilesystemBlobStore::open(profile).map_err(|error| error.to_string())?;
    worldline_kernel::BlobStore::put(&blob, b"child partial blob")
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn initial_state() -> BackendState {
    BackendState::new(
        record(StateRevision::new(0), InstallationStatus::Ready),
        BTreeMap::new(),
    )
}

fn record(revision: StateRevision, status: InstallationStatus) -> InstallationRecord {
    InstallationRecord::from_parts(
        InstallationId::new(INSTALLATION),
        PluginId::new("recovery-child-plugin"),
        StateSchemaVersion::new(1),
        status,
        revision,
        0,
    )
}

fn durable_event(event_id: &str) -> EventEnvelope {
    EventEnvelope::from_parts(
        EventId::new(event_id),
        EventContract::new(
            "worldline.recovery",
            "child-event",
            InterfaceVersion::new(1, 0),
        ),
        PrincipalId::new("child-owner"),
        None,
        1,
        CorrelationId::new("child-correlation"),
        Some(CausationRef::Invocation(InvocationId::new(
            "child-invocation",
        ))),
        DeliveryMode::Durable,
        b"child event payload".to_vec(),
        None,
    )
}

fn usage_and_exit() -> ! {
    eprintln!("usage: persistence-recovery-child <scenario> <profile-root>");
    process::exit(2);
}
