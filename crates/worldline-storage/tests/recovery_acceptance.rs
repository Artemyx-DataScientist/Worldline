#![cfg(feature = "test-failpoints")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use worldline_kernel::{
    BlobStore, EventCursor, EventJournal, InstallationId, InstallationStatus, JobState,
    OutboxStatus, StateBackend, StateKey,
};
use worldline_storage::{FilesystemBlobStore, SqliteEventJournal, SqliteStateBackend};

struct TempProfile(PathBuf);

impl TempProfile {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "worldline-recovery-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary profile must be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_child(scenario: &str, profile: &Path) {
    let child = option_env!("CARGO_BIN_EXE_persistence-recovery-child")
        .map(PathBuf::from)
        .expect("Cargo must provide the recovery child executable path");
    let status = Command::new(child)
        .arg(scenario)
        .arg(profile)
        .env("WORLDLINE_FAILPOINT", scenario)
        .status()
        .expect("recovery child must start");
    assert!(
        !status.success(),
        "child scenario '{scenario}' must terminate at its hard failpoint"
    );
}

#[test]
fn hard_kill_before_commit_leaves_only_previous_state() {
    let profile = TempProfile::new("before-commit");
    run_child("before-state-commit", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let state = backend
        .snapshot(&InstallationId::new("installation-recovery-child"))
        .expect("previous state must remain readable");
    assert_eq!(state.record().state_revision().value(), 0);
    assert!(!state.values().contains_key(&StateKey::new("committed")));
}

#[test]
fn hard_kill_after_commit_preserves_committed_state() {
    let profile = TempProfile::new("after-commit");
    run_child("after-state-commit", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let state = backend
        .snapshot(&InstallationId::new("installation-recovery-child"))
        .expect("committed state must be readable");
    assert_eq!(state.record().state_revision().value(), 1);
    assert_eq!(
        state.values().get(&StateKey::new("committed")),
        Some(&b"child-commit".to_vec())
    );
}

#[test]
fn hard_kill_after_state_outbox_commit_recovers_pending_delivery() {
    let profile = TempProfile::new("outbox");
    run_child("after-state-outbox-commit-before-publish", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let state = backend
        .snapshot(&InstallationId::new("installation-recovery-child"))
        .expect("committed state must be readable");
    assert_eq!(state.record().state_revision().value(), 1);
    let pending = worldline_kernel::OutboxStore::list_pending(&backend, 10)
        .expect("outbox must be recoverable");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status(), OutboxStatus::Pending);
    assert_eq!(pending[0].event().event_id().as_str(), "child-outbox-event");
}

#[test]
fn hard_kill_after_publish_redelivers_same_event_and_finishes_delivery() {
    let profile = TempProfile::new("publish");
    run_child("after-event-publish-before-delivered", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let pending = worldline_kernel::OutboxStore::list_pending(&backend, 10)
        .expect("delivering record must be recoverable");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status(), OutboxStatus::Delivering);
    let journal = SqliteEventJournal::open(profile.path()).expect("journal must reopen");
    let events = journal
        .read_from(EventCursor::new(0))
        .expect("published event must be replayable");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id().as_str(), "child-publish-event");
    worldline_kernel::OutboxStore::mark_delivered(&backend, pending[0].outbox_id())
        .expect("recovered delivery must be markable");
    assert!(
        worldline_kernel::OutboxStore::list_pending(&backend, 10)
            .expect("outbox must be empty after delivery")
            .is_empty()
    );
}

#[test]
fn hard_kill_during_metadata_transitions_is_explicit() {
    for (scenario, expected_status) in [
        ("during-migration", InstallationStatus::Migrating),
        (
            "during-uninstall-metadata-transition",
            InstallationStatus::Uninstalling,
        ),
    ] {
        let profile = TempProfile::new(scenario);
        run_child(scenario, profile.path());
        let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
        let state = backend
            .snapshot(&InstallationId::new("installation-recovery-child"))
            .expect("metadata must remain readable");
        assert_eq!(state.record().status(), expected_status);
    }
}

#[test]
fn hard_kill_after_running_job_never_invents_completion() {
    let profile = TempProfile::new("running-job");
    run_child("after-job-running-before-completion", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let recovered = worldline_kernel::JobStore::recover_running(&backend)
        .expect("running job must recover explicitly");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state(), JobState::Interrupted);
    assert_eq!(
        worldline_kernel::JobStore::get(
            &backend,
            &worldline_kernel::JobId::new("child-running-job")
        )
        .expect("job lookup must work")
        .expect("job must remain durable")
        .state(),
        JobState::Interrupted
    );
}

#[test]
fn hard_kill_during_blob_write_publishes_no_partial_blob() {
    let profile = TempProfile::new("blob");
    run_child("during-blob-temporary-write", profile.path());
    let blob = FilesystemBlobStore::open(profile.path()).expect("blob store must reopen");
    let entries = fs::read_dir(blob.blob_root())
        .expect("blob root must be readable")
        .map(|entry| {
            entry
                .expect("blob directory entry must be readable")
                .file_name()
        })
        .collect::<Vec<_>>();
    assert!(
        !entries.is_empty(),
        "hard kill should leave only a temp artifact"
    );
    assert!(entries.iter().all(|name| {
        let name = name.to_string_lossy();
        name.starts_with('.') && name.ends_with(".tmp")
    }));
    assert_eq!(
        BlobStore::exists(
            &blob,
            &worldline_kernel::BlobId::new(
                "sha256-v1-0000000000000000000000000000000000000000000000000000000000000000"
            )
            .expect("synthetic id must parse")
        ),
        Ok(false)
    );
}
