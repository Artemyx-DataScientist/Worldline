#![cfg(feature = "test-failpoints")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use worldline_kernel::{InstallationId, InstallationStatus, StateBackend};
use worldline_storage::SqliteStateBackend;

struct TempProfile(PathBuf);

impl TempProfile {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "worldline-upgrade-chaos-{label}-{}-{nonce}",
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
    let child = option_env!("CARGO_BIN_EXE_upgrade-chaos-child")
        .map(PathBuf::from)
        .expect("Cargo must provide the upgrade chaos child executable path");
    let status = Command::new(child)
        .arg(scenario)
        .arg(profile)
        .env("WORLDLINE_FAILPOINT", scenario)
        .status()
        .expect("upgrade chaos child must start");
    assert!(
        !status.success(),
        "child scenario '{scenario}' must terminate at its hard failpoint"
    );
}

#[test]
fn hard_kill_during_staged_migration_copy_preserves_current_state() {
    let profile = TempProfile::new("staged-migration-copy");
    run_child("during-staged-migration-copy", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let state = backend
        .snapshot(&InstallationId::new("installation-upgrade-chaos-child"))
        .expect("current state must remain readable");
    assert_eq!(state.record().state_revision().value(), 0);
    assert_eq!(state.record().status(), InstallationStatus::Ready);
}

#[test]
fn hard_kill_before_active_revision_switch_preserves_previous_revision() {
    let profile = TempProfile::new("before-active-switch");
    run_child("before-active-revision-switch", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let state = backend
        .snapshot(&InstallationId::new("installation-upgrade-chaos-child"))
        .expect("previous state must remain readable");
    assert_eq!(state.record().state_revision().value(), 0);
    assert_eq!(state.record().status(), InstallationStatus::Ready);
}

#[test]
fn hard_kill_after_active_revision_switch_recovers_state_atomically() {
    let profile = TempProfile::new("after-active-switch");
    run_child("after-active-revision-switch", profile.path());
    let backend = SqliteStateBackend::open(profile.path()).expect("profile must reopen");
    let state = backend
        .snapshot(&InstallationId::new("installation-upgrade-chaos-child"))
        .expect("state must remain readable");
    assert_eq!(state.record().status(), InstallationStatus::Ready);
}
