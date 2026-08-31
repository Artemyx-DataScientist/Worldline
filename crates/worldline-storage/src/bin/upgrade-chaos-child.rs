//! Process-kill child test binary for staged upgrade and rollback crash chaos testing.

use std::{collections::BTreeMap, env, path::PathBuf, process};

use worldline_kernel::{
    BackendState, InstallationId, InstallationRecord, InstallationStatus, PackageRevisionId,
    StateBackend, StateRevision, StateSchemaVersion, UpgradeManager,
};
use worldline_storage::SqliteStateBackend;

const INSTALLATION: &str = "installation-upgrade-chaos-child";

fn main() {
    let mut arguments = env::args().skip(1);
    let scenario = arguments.next().unwrap_or_else(|| usage_and_exit());
    let profile = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| usage_and_exit());

    let result = match scenario.as_str() {
        "during-staged-migration-copy" => staged_migration_copy_crash(&profile),
        "before-active-revision-switch" => before_active_switch_crash(&profile),
        "after-active-revision-switch" => after_active_switch_crash(&profile),
        other => Err(format!("unknown upgrade chaos scenario '{other}'")),
    };

    if let Err(error) = result {
        eprintln!("upgrade chaos child failed: {error}");
        process::exit(2);
    }
}

fn staged_migration_copy_crash(profile: &PathBuf) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    StateBackend::create(&backend, initial_state()).map_err(|error| error.to_string())?;
    let inst = InstallationId::new(INSTALLATION);
    let mut mgr = UpgradeManager::new();
    mgr.register_initial_installation(
        inst.clone(),
        PackageRevisionId::new("rev-1"),
        BTreeMap::new(),
    );
    mgr.stage_package(&inst, PackageRevisionId::new("rev-2"), true)
        .map_err(|e| e.to_string())?;
    mgr.prepare_migration_copy(&inst, &BTreeMap::new())
        .map_err(|e| e.to_string())?;
    worldline_storage::trigger_test_failpoint("during-staged-migration-copy");
    Ok(())
}

fn before_active_switch_crash(profile: &PathBuf) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    StateBackend::create(&backend, initial_state()).map_err(|error| error.to_string())?;
    let inst = InstallationId::new(INSTALLATION);
    let mut mgr = UpgradeManager::new();
    mgr.register_initial_installation(
        inst.clone(),
        PackageRevisionId::new("rev-1"),
        BTreeMap::new(),
    );
    mgr.stage_package(&inst, PackageRevisionId::new("rev-2"), true)
        .map_err(|e| e.to_string())?;
    mgr.prepare_migration_copy(&inst, &BTreeMap::new())
        .map_err(|e| e.to_string())?;
    let prov = worldline_kernel::MigrationProvenance {
        source_revision: PackageRevisionId::new("rev-1"),
        target_revision: PackageRevisionId::new("rev-2"),
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(2),
        migration_path: vec![],
        success: true,
        error_message: None,
        duration_ticks: 1,
    };
    mgr.record_migration_result(&inst, None, prov)
        .map_err(|e| e.to_string())?;
    mgr.record_health_validation(&inst, worldline_kernel::HealthProbeStatus::Healthy)
        .map_err(|e| e.to_string())?;
    mgr.begin_switch(&inst).map_err(|e| e.to_string())?;
    worldline_storage::trigger_test_failpoint("before-active-revision-switch");
    Ok(())
}

fn after_active_switch_crash(profile: &PathBuf) -> Result<(), String> {
    let backend = SqliteStateBackend::open(profile).map_err(|error| error.to_string())?;
    StateBackend::create(&backend, initial_state()).map_err(|error| error.to_string())?;
    let inst = InstallationId::new(INSTALLATION);
    let mut mgr = UpgradeManager::new();
    mgr.register_initial_installation(
        inst.clone(),
        PackageRevisionId::new("rev-1"),
        BTreeMap::new(),
    );
    mgr.stage_package(&inst, PackageRevisionId::new("rev-2"), true)
        .map_err(|e| e.to_string())?;
    mgr.prepare_migration_copy(&inst, &BTreeMap::new())
        .map_err(|e| e.to_string())?;
    let prov = worldline_kernel::MigrationProvenance {
        source_revision: PackageRevisionId::new("rev-1"),
        target_revision: PackageRevisionId::new("rev-2"),
        source_schema: StateSchemaVersion::new(1),
        target_schema: StateSchemaVersion::new(2),
        migration_path: vec![],
        success: true,
        error_message: None,
        duration_ticks: 1,
    };
    mgr.record_migration_result(&inst, None, prov)
        .map_err(|e| e.to_string())?;
    mgr.record_health_validation(&inst, worldline_kernel::HealthProbeStatus::Healthy)
        .map_err(|e| e.to_string())?;
    mgr.begin_switch(&inst).map_err(|e| e.to_string())?;
    let _ = mgr
        .commit_switch(&inst, BTreeMap::new())
        .map_err(|e| e.to_string())?;
    worldline_storage::trigger_test_failpoint("after-active-revision-switch");
    Ok(())
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
        worldline_kernel::PluginId::new("upgrade-chaos-child-plugin"),
        StateSchemaVersion::new(1),
        status,
        revision,
        0,
    )
}

fn usage_and_exit() -> ! {
    eprintln!("usage: upgrade-chaos-child <scenario> <profile-root>");
    process::exit(2);
}
