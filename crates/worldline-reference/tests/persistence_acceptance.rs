use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use worldline_kernel::{EventCursor, EventJournal, StateBackend};
use worldline_reference::s1;
use worldline_storage::{SqliteEventJournal, SqliteStateBackend};

struct TempProfile(PathBuf);

impl TempProfile {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "worldline-reference-{label}-{}-{nonce}",
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

#[test]
fn production_s1_preserves_state_and_redelivers_original_event_identities() {
    let profile = TempProfile::new("s1");
    let report = s1::run_production(profile.path()).expect("production S1 must pass");
    assert_eq!(report.state_before_restart, "1");
    assert_eq!(report.state_after_restart, "2");
    assert_ne!(report.old_runtime_id, report.new_runtime_id);
    assert!(report.old_runtime_authority_revoked);
    assert!(report.new_runtime_required_explicit_authority);
    assert_eq!(report.observed_events, 2);

    let installation = worldline_kernel::InstallationId::new(report.installation_id.clone());
    let backend = SqliteStateBackend::open(profile.path()).expect("state backend must reopen");
    let state = backend
        .snapshot(&installation)
        .expect("production S1 state must survive a second reopen");
    assert_eq!(
        state
            .values()
            .get(&worldline_kernel::StateKey::new("committed-count")),
        Some(&b"2".to_vec())
    );
    assert_eq!(
        worldline_kernel::OutboxStore::list_pending(&backend, 64)
            .expect("outbox listing must work after S1"),
        Vec::new()
    );

    let journal = SqliteEventJournal::open(profile.path()).expect("journal must reopen");
    let events = journal
        .read_from(EventCursor::new(0))
        .expect("S1 outbox envelopes must be replayable");
    assert_eq!(events.len(), 2);
    assert_ne!(events[0].event_id(), events[1].event_id());
    assert_ne!(
        events[0].producer_runtime_id(),
        events[1].producer_runtime_id()
    );
    assert_eq!(events[0].sequence(), 1);
    assert_eq!(events[1].sequence(), 2);
}
