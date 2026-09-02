use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use worldline_kernel::{
    AuditOutcome, AuditRecord, AuditStore, BackendState, BlobStore, CausationRef, CorrelationId,
    DeliveryMode, EventContract, EventCursor, EventEnvelope, EventId, EventJournal, InstallationId,
    InstallationRecord, InstallationStatus, InterfaceVersion, InvocationId, JobRecord, JobState,
    JobStore, OutboxId, OutboxRecord, OutboxStatus, OutboxStore, PluginId, PrincipalId,
    StateBackend, StateError, StateKey, StateRevision, StateSchemaVersion, StateTransactionId,
};
use worldline_storage::{
    BLOB_READ_CAPABILITY, BlobReadBroker, FilesystemBlobStore, SqliteEventJournal,
    SqliteStateBackend,
};

struct TempProfile(PathBuf);

impl TempProfile {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "worldline-storage-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary profile must be created");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn initial_state() -> BackendState {
    let installation = InstallationId::new("installation-contract");
    let record = InstallationRecord::from_parts(
        installation,
        PluginId::new("contract-plugin"),
        StateSchemaVersion::new(1),
        InstallationStatus::Ready,
        StateRevision::new(0),
        0,
    );
    BackendState::new(record, BTreeMap::new())
}

fn durable_event(event_id: &str, sequence: u64) -> EventEnvelope {
    EventEnvelope::from_parts(
        EventId::new(event_id),
        EventContract::new(
            "worldline.test",
            "state.changed",
            InterfaceVersion::new(1, 2),
        ),
        PrincipalId::new("plugin-installation:installation-contract"),
        None,
        sequence,
        CorrelationId::new("correlation-contract"),
        Some(CausationRef::Invocation(InvocationId::new(
            "invocation-contract",
        ))),
        DeliveryMode::Durable,
        b"opaque payload".to_vec(),
        None,
    )
}

#[test]
fn production_backend_persists_state_across_reopen() {
    let profile = TempProfile::new("reopen");
    {
        let backend = SqliteStateBackend::open(profile.path()).expect("backend must open");
        StateBackend::create(&backend, initial_state()).expect("installation creation must commit");
        let installation = InstallationId::new("installation-contract");
        let record = InstallationRecord::from_parts(
            installation.clone(),
            PluginId::new("contract-plugin"),
            StateSchemaVersion::new(1),
            InstallationStatus::Ready,
            StateRevision::new(1),
            0,
        );
        let mut values = BTreeMap::new();
        values.insert(StateKey::new("value"), b"survives".to_vec());
        backend
            .commit_if_revision(
                &installation,
                &StateTransactionId::new("tx-reopen"),
                StateRevision::new(0),
                BackendState::new(record, values),
            )
            .expect("state commit must be durable");
    }
    let backend = SqliteStateBackend::open(profile.path()).expect("reopen must succeed");
    let snapshot = backend
        .snapshot(&InstallationId::new("installation-contract"))
        .expect("reopened snapshot must be available");
    assert_eq!(
        snapshot.values().get(&StateKey::new("value")),
        Some(&b"survives".to_vec())
    );
}

#[test]
fn production_backend_commits_complete_key_set_and_rejects_stale_cas() {
    let profile = TempProfile::new("cas");
    let backend = SqliteStateBackend::open(profile.path()).expect("backend must open");
    StateBackend::create(&backend, initial_state()).expect("installation creation must commit");
    let installation = InstallationId::new("installation-contract");
    let mut values = BTreeMap::new();
    values.insert(StateKey::new("a"), b"one".to_vec());
    values.insert(StateKey::new("b"), b"two".to_vec());
    let next = InstallationRecord::from_parts(
        installation.clone(),
        PluginId::new("contract-plugin"),
        StateSchemaVersion::new(1),
        InstallationStatus::Ready,
        StateRevision::new(1),
        0,
    );
    backend
        .commit_if_revision(
            &installation,
            &StateTransactionId::new("tx-1"),
            StateRevision::new(0),
            BackendState::new(next, values),
        )
        .expect("first CAS commit must succeed");

    let stale = InstallationRecord::from_parts(
        installation.clone(),
        PluginId::new("contract-plugin"),
        StateSchemaVersion::new(1),
        InstallationStatus::Ready,
        StateRevision::new(1),
        0,
    );
    let error = backend
        .commit_if_revision(
            &installation,
            &StateTransactionId::new("stale"),
            StateRevision::new(0),
            BackendState::new(stale, BTreeMap::new()),
        )
        .expect_err("stale CAS must fail");
    assert!(matches!(error, StateError::TransactionConflict { .. }));

    let snapshot = backend.snapshot(&installation).expect("snapshot must work");
    assert_eq!(snapshot.record().state_revision(), StateRevision::new(1));
    assert_eq!(
        snapshot.values().get(&StateKey::new("a")),
        Some(&b"one".to_vec())
    );
    assert_eq!(
        snapshot.values().get(&StateKey::new("b")),
        Some(&b"two".to_vec())
    );
}

#[test]
fn newer_storage_format_fails_closed() {
    let profile = TempProfile::new("format");
    {
        let backend = SqliteStateBackend::open(profile.path()).expect("backend must open");
        drop(backend);
    }
    let connection = rusqlite::Connection::open(profile.path().join("worldline.sqlite3"))
        .expect("database opens");
    connection
        .execute(
            "UPDATE worldline_storage_meta SET value = '999' WHERE key = 'format_version'",
            [],
        )
        .expect("format marker must be mutable for the test");
    drop(connection);

    let error = SqliteStateBackend::open(profile.path())
        .err()
        .expect("newer format must fail");
    assert!(matches!(
        error,
        worldline_kernel::PersistenceError::UnsupportedStorageFormat { .. }
    ));
}

#[test]
fn state_and_outbox_commit_atomically_and_redeliver_after_reopen() {
    let profile = TempProfile::new("outbox-recovery");
    let installation = InstallationId::new("installation-contract");
    let event = durable_event("event-contract-1", 9);
    {
        let backend = SqliteStateBackend::open(profile.path()).expect("backend must open");
        StateBackend::create(&backend, initial_state()).expect("state must exist");
        let record = InstallationRecord::from_parts(
            installation.clone(),
            PluginId::new("contract-plugin"),
            StateSchemaVersion::new(1),
            InstallationStatus::Ready,
            StateRevision::new(1),
            0,
        );
        let mut values = BTreeMap::new();
        values.insert(StateKey::new("committed"), b"with-outbox".to_vec());
        backend
            .commit_if_revision_with_outbox(
                &installation,
                &StateTransactionId::new("tx-outbox"),
                StateRevision::new(0),
                BackendState::new(record, values),
                &OutboxRecord::new(
                    OutboxId::new("outbox-contract-1"),
                    installation.clone(),
                    event.clone(),
                    1,
                    100,
                ),
            )
            .expect("state and outbox must commit together");
        let pending = OutboxStore::get(&backend, &OutboxId::new("outbox-contract-1"))
            .expect("outbox lookup must work")
            .expect("outbox must exist");
        assert_eq!(pending.status(), OutboxStatus::Pending);
        assert_eq!(pending.event().event_id(), event.event_id());
        assert_eq!(
            backend
                .snapshot(&installation)
                .expect("state lookup must work")
                .values()
                .get(&StateKey::new("committed")),
            Some(&b"with-outbox".to_vec())
        );

        // A duplicate outbox identity must roll back the second state change,
        // proving the outbox insert is inside the same SQLite transaction.
        let second_record = InstallationRecord::from_parts(
            installation.clone(),
            PluginId::new("contract-plugin"),
            StateSchemaVersion::new(1),
            InstallationStatus::Ready,
            StateRevision::new(2),
            0,
        );
        let mut second_values = BTreeMap::new();
        second_values.insert(StateKey::new("should-rollback"), b"no".to_vec());
        let duplicate = backend
            .commit_if_revision_with_outbox(
                &installation,
                &StateTransactionId::new("tx-duplicate"),
                StateRevision::new(1),
                BackendState::new(second_record, second_values),
                &OutboxRecord::new(
                    OutboxId::new("outbox-contract-1"),
                    installation.clone(),
                    event.clone(),
                    2,
                    200,
                ),
            )
            .expect_err("duplicate outbox must fail atomically");
        assert!(matches!(
            duplicate,
            StateError::Persistence(worldline_kernel::PersistenceError::OutboxAppendFailed { .. })
        ));
        let unchanged = backend
            .snapshot(&installation)
            .expect("state remains readable");
        assert_eq!(unchanged.record().state_revision(), StateRevision::new(1));
        assert!(
            !unchanged
                .values()
                .contains_key(&StateKey::new("should-rollback"))
        );
    }

    {
        let backend = SqliteStateBackend::open(profile.path()).expect("reopen must succeed");
        let pending = backend
            .list_pending(10)
            .expect("pending outbox must survive restart");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event().event_id(), event.event_id());
        let delivering = backend
            .mark_delivering(&OutboxId::new("outbox-contract-1"))
            .expect("delivery transition must commit");
        assert_eq!(delivering.status(), OutboxStatus::Delivering);
        assert_eq!(delivering.attempt_count(), 1);
        backend
            .mark_delivered(&OutboxId::new("outbox-contract-1"))
            .expect("delivered transition must commit");
    }

    let backend = SqliteStateBackend::open(profile.path()).expect("second reopen must succeed");
    assert!(
        backend
            .list_pending(10)
            .expect("pending listing must work")
            .is_empty()
    );
    assert_eq!(
        OutboxStore::get(&backend, &OutboxId::new("outbox-contract-1"))
            .expect("delivered outbox lookup must work")
            .expect("delivered outbox must remain durable")
            .event()
            .event_id(),
        event.event_id()
    );
}

#[test]
fn production_event_journal_preserves_envelope_and_detects_corruption() {
    let profile = TempProfile::new("journal-recovery");
    let event = durable_event("event-journal-1", 17);
    {
        let journal = SqliteEventJournal::open(profile.path()).expect("journal must open");
        journal.append(&event).expect("event append must commit");
        journal
            .append(&event)
            .expect("same EventId and bytes must be idempotent");
        assert_eq!(
            journal.read_from(EventCursor::new(0)).unwrap(),
            vec![event.clone()]
        );
    }
    {
        let journal = SqliteEventJournal::open(profile.path()).expect("journal must reopen");
        let replayed = journal
            .read_from(EventCursor::new(0))
            .expect("replay must survive restart");
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].event_id(), event.event_id());
        assert_eq!(replayed[0].sequence(), event.sequence());
        assert_eq!(replayed[0].correlation_id(), event.correlation_id());
        assert_eq!(replayed[0].causation(), event.causation());
        assert_eq!(replayed[0].payload(), event.payload());
    }

    let connection = rusqlite::Connection::open(profile.path().join("worldline.sqlite3"))
        .expect("database opens for corruption setup");
    connection
        .execute(
            "UPDATE event_journal SET record = X'00' WHERE event_id = ?1",
            [&"event-journal-1"],
        )
        .expect("corruption setup must commit");
    drop(connection);
    let journal = SqliteEventJournal::open(profile.path()).expect("journal must still open");
    let error = journal
        .read_from(EventCursor::new(0))
        .expect_err("corrupt journal must fail explicitly");
    assert!(error.to_string().contains("JournalReplayFailed"));
}

#[test]
fn audit_blob_and_job_stores_preserve_safe_metadata_and_recovery_state() {
    let profile = TempProfile::new("audit-blob-jobs");
    let installation = InstallationId::new("installation-contract");

    {
        let backend = SqliteStateBackend::open(profile.path()).expect("backend must open");
        StateBackend::create(&backend, initial_state()).expect("state must exist");

        let mut metadata = BTreeMap::new();
        metadata.insert("operation".to_owned(), "state.commit".to_owned());
        metadata.insert("attempt".to_owned(), "1".to_owned());
        let audit = AuditRecord::new(1, "StateTransaction", AuditOutcome::Committed, metadata)
            .with_installation(installation.clone())
            .with_trace(
                CorrelationId::new("audit-correlation"),
                Some(CausationRef::Invocation(InvocationId::new(
                    "audit-invocation",
                ))),
            );
        AuditStore::append(&backend, audit.clone()).expect("audit append must commit");
        assert_eq!(
            AuditStore::list(&backend, 10).expect("audit list must work"),
            vec![audit]
        );

        let rejected = AuditRecord::new(
            2,
            "StateTransaction",
            AuditOutcome::Failed,
            [("raw_payload".to_owned(), "must-not-log".to_owned())]
                .into_iter()
                .collect(),
        );
        assert!(matches!(
            AuditStore::append(&backend, rejected),
            Err(worldline_kernel::PersistenceError::InvalidRecord { .. })
        ));

        let blob = FilesystemBlobStore::open(profile.path()).expect("blob store must open");
        let blob_id = BlobStore::put(&blob, b"immutable content").expect("blob put must work");
        assert_eq!(
            BlobStore::put(&blob, b"immutable content").unwrap(),
            blob_id
        );
        assert_eq!(
            BlobStore::get(&blob, &blob_id).unwrap(),
            b"immutable content"
        );
        BlobStore::verify(&blob, &blob_id).expect("fresh blob must verify");
        let broker = BlobReadBroker::new();
        assert!(
            broker
                .issue(
                    "downloads-metadata-reader",
                    "browser.downloads.read",
                    blob_id.as_str()
                )
                .is_err()
        );
        let grant = broker
            .issue(
                "generic-blob-reader",
                BLOB_READ_CAPABILITY,
                blob_id.as_str(),
            )
            .expect("generic blob broker must issue an exact grant");
        assert_eq!(
            blob.get_with_authority(&blob_id, &grant)
                .expect("authorized generic read must succeed"),
            b"immutable content"
        );

        let job = JobRecord::new(
            "job-contract",
            PrincipalId::new("plugin-installation:installation-contract"),
        )
        .with_installation(installation.clone())
        .with_state(JobState::Running)
        .with_deadline_millis(Some(500))
        .with_wakeup_millis(Some(200))
        .with_cancellation_requested(false)
        .with_attempt(3)
        .with_resource_budget(Some(8))
        .with_recovery_policy(worldline_kernel::JobRecoveryPolicy::Manual)
        .with_trace(
            CorrelationId::new("job-correlation"),
            Some(CausationRef::Event(EventId::new("event-journal-1"))),
        );
        JobStore::create(&backend, job.clone()).expect("job create must commit");
        assert_eq!(JobStore::get(&backend, job.job_id()).unwrap(), Some(job));
    }

    let backend = SqliteStateBackend::open(profile.path()).expect("reopen must succeed");
    let recovered = JobStore::recover_running(&backend).expect("running jobs must recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].job_id(),
        &worldline_kernel::JobId::new("job-contract")
    );
    assert_eq!(recovered[0].state(), JobState::Interrupted);
    assert_eq!(
        JobStore::get(&backend, &worldline_kernel::JobId::new("job-contract"))
            .unwrap()
            .unwrap()
            .state(),
        JobState::Interrupted
    );

    let blob = FilesystemBlobStore::open(profile.path()).expect("blob store must reopen");
    let blob_id = BlobStore::put(&blob, b"corruptible content").expect("blob put must work");
    std::fs::write(blob.blob_root().join(blob_id.as_str()), b"corrupted")
        .expect("test corruption must be writable");
    assert!(matches!(
        BlobStore::get(&blob, &blob_id),
        Err(worldline_kernel::PersistenceError::BlobCorrupt { .. })
    ));
}

#[test]
fn online_backup_validates_and_restores_into_a_fresh_profile() {
    let source_profile = TempProfile::new("backup-source");
    let restore_profile = TempProfile::new("backup-restore");
    let backup_path = source_profile.path().join("metadata-backup.sqlite3");
    let installation = InstallationId::new("installation-contract");
    {
        let backend = SqliteStateBackend::open(source_profile.path()).expect("backend must open");
        StateBackend::create(&backend, initial_state()).expect("state must exist");
        backend
            .backup_to(&backup_path)
            .expect("online backup must publish");
    }
    SqliteStateBackend::validate_backup(&backup_path).expect("backup must validate");
    let restored = SqliteStateBackend::restore_from(&backup_path, restore_profile.path())
        .expect("restore must reopen through production backend");
    let snapshot = restored
        .snapshot(&installation)
        .expect("restored installation must be readable");
    assert_eq!(snapshot.record().installation_id(), &installation);
    assert_eq!(snapshot.record().state_revision(), StateRevision::new(0));
}
