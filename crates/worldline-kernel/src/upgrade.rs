//! Staged package upgrade state machine, migration-on-copy, and LastKnownGood rollback.
//!
//! Architectural Invariants (see `docs/adr/ADR-OPERABILITY-COMPATIBILITY-UPGRADE-V1.md`):
//! 1. UPGRADE IS A TRANSACTIONAL STATE TRANSITION, NOT "REPLACE SOME FILES".
//! 2. INSTALLATION IDENTITY DOES NOT CHANGE MERELY BECAUSE PACKAGE VERSION CHANGES.
//! 3. PACKAGE REVISION, INSTALLATION IDENTITY AND RUNTIME IDENTITY ARE DISTINCT.
//! 4. A STAGED VERSION HAS NO ACTIVE PROVIDER AUTHORITY BEFORE SWITCH.
//! 5. MIGRATION RUNS AGAINST STAGED/COPIED STATE BEFORE ACTIVE STATE IS IRREVERSIBLY REPLACED.
//! 6. FAILED HEALTH VALIDATION MUST NOT REPLACE THE LAST KNOWN GOOD REVISION.
//! 7. ROLLBACK RESTORES A PREVIOUSLY VALIDATED REVISION, NOT AN ASSUMED VERSION.
//! 8. CRASH RECOVERY RECONSTRUCTS EXACTLY ONE AUTHORITATIVE CURRENT REVISION.

use std::{collections::BTreeMap, fmt};

use crate::{
    InstallationId,
    state::{MigrationId, StateKey, StateSchemaVersion, StateValue},
};

/// Unique, immutable revision identity of an installed package artifact.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageRevisionId(String);

impl PackageRevisionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PackageRevisionId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PackageRevisionId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for PackageRevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Explicit upgrade lifecycle state machine.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum UpgradeState {
    Current,
    Staging,
    CompatibilityRejected,
    MigratingCopy,
    Validating,
    ReadyToSwitch,
    Switching,
    CurrentCandidate,
    RollingBack,
    RolledBack,
    Failed,
}

impl UpgradeState {
    pub const fn is_authoritative_active(self) -> bool {
        matches!(
            self,
            Self::Current | Self::CurrentCandidate | Self::RolledBack
        )
    }

    pub const fn is_staging(self) -> bool {
        matches!(
            self,
            Self::Staging
                | Self::MigratingCopy
                | Self::Validating
                | Self::ReadyToSwitch
                | Self::Switching
        )
    }

    pub const fn is_terminal_failure(self) -> bool {
        matches!(self, Self::CompatibilityRejected | Self::Failed)
    }
}

impl fmt::Display for UpgradeState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Current => "Current",
            Self::Staging => "Staging",
            Self::CompatibilityRejected => "CompatibilityRejected",
            Self::MigratingCopy => "MigratingCopy",
            Self::Validating => "Validating",
            Self::ReadyToSwitch => "ReadyToSwitch",
            Self::Switching => "Switching",
            Self::CurrentCandidate => "CurrentCandidate",
            Self::RollingBack => "RollingBack",
            Self::RolledBack => "RolledBack",
            Self::Failed => "Failed",
        };
        formatter.write_str(name)
    }
}

/// Provenance metadata recorded for every state migration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationProvenance {
    pub source_revision: PackageRevisionId,
    pub target_revision: PackageRevisionId,
    pub source_schema: StateSchemaVersion,
    pub target_schema: StateSchemaVersion,
    pub migration_path: Vec<MigrationId>,
    pub success: bool,
    pub error_message: Option<String>,
    pub duration_ticks: u64,
}

/// Outcome of pre-switch health validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HealthProbeStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
}

/// Errors occurring during the upgrade lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpgradeError {
    InstallationNotFound {
        installation: InstallationId,
    },
    InvalidStateTransition {
        from: UpgradeState,
        to: UpgradeState,
    },
    CompatibilityRejected {
        reason: String,
    },
    StagingFailed {
        reason: String,
    },
    MigrationOnCopyFailed {
        reason: String,
    },
    HealthValidationFailed {
        reason: String,
    },
    SwitchFailed {
        reason: String,
    },
    NoLastKnownGood {
        installation: InstallationId,
    },
    RollbackFailed {
        reason: String,
    },
}

impl fmt::Display for UpgradeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstallationNotFound { installation } => {
                write!(formatter, "installation not found: {installation}")
            }
            Self::InvalidStateTransition { from, to } => {
                write!(
                    formatter,
                    "invalid upgrade state transition from {from} to {to}"
                )
            }
            Self::CompatibilityRejected { reason } => {
                write!(formatter, "compatibility rejected before switch: {reason}")
            }
            Self::StagingFailed { reason } => {
                write!(formatter, "staging failed: {reason}")
            }
            Self::MigrationOnCopyFailed { reason } => {
                write!(formatter, "migration on copy failed: {reason}")
            }
            Self::HealthValidationFailed { reason } => {
                write!(formatter, "health validation failed: {reason}")
            }
            Self::SwitchFailed { reason } => {
                write!(formatter, "switch failed: {reason}")
            }
            Self::NoLastKnownGood { installation } => {
                write!(formatter, "no last-known-good revision for {installation}")
            }
            Self::RollbackFailed { reason } => {
                write!(formatter, "rollback failed: {reason}")
            }
        }
    }
}

impl std::error::Error for UpgradeError {}

/// Tracked upgrade context for one installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallationUpgradeRecord {
    pub installation_id: InstallationId,
    pub current_revision: PackageRevisionId,
    pub staged_revision: Option<PackageRevisionId>,
    pub last_known_good: Option<PackageRevisionId>,
    pub state: UpgradeState,
    pub staged_state_copy: Option<BTreeMap<StateKey, StateValue>>,
    pub last_known_good_state: Option<BTreeMap<StateKey, StateValue>>,
    pub migration_provenance: Option<MigrationProvenance>,
    pub post_switch_failures: u32,
}

/// Manages transactional upgrades, migration-on-copy, and rollback.
#[derive(Clone, Debug, Default)]
pub struct UpgradeManager {
    installations: BTreeMap<InstallationId, InstallationUpgradeRecord>,
}

impl UpgradeManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a newly installed plugin at its initial revision.
    pub fn register_initial_installation(
        &mut self,
        installation_id: InstallationId,
        initial_revision: PackageRevisionId,
        initial_state: BTreeMap<StateKey, StateValue>,
    ) {
        let record = InstallationUpgradeRecord {
            installation_id: installation_id.clone(),
            current_revision: initial_revision.clone(),
            staged_revision: None,
            last_known_good: Some(initial_revision),
            state: UpgradeState::Current,
            staged_state_copy: None,
            last_known_good_state: Some(initial_state),
            migration_provenance: None,
            post_switch_failures: 0,
        };
        self.installations.insert(installation_id, record);
    }

    pub fn get_record(
        &self,
        installation_id: &InstallationId,
    ) -> Option<&InstallationUpgradeRecord> {
        self.installations.get(installation_id)
    }

    pub fn current_revision(&self, installation_id: &InstallationId) -> Option<&PackageRevisionId> {
        self.installations
            .get(installation_id)
            .map(|r| &r.current_revision)
    }

    pub fn staged_revision(&self, installation_id: &InstallationId) -> Option<&PackageRevisionId> {
        self.installations
            .get(installation_id)
            .and_then(|r| r.staged_revision.as_ref())
    }

    pub fn last_known_good(&self, installation_id: &InstallationId) -> Option<&PackageRevisionId> {
        self.installations
            .get(installation_id)
            .and_then(|r| r.last_known_good.as_ref())
    }

    pub fn upgrade_state(&self, installation_id: &InstallationId) -> Option<UpgradeState> {
        self.installations.get(installation_id).map(|r| r.state)
    }

    /// Stages a new package revision.
    ///
    /// Invariant: if compatibility evaluation fails, moves to `CompatibilityRejected`
    /// and grants no active authority.
    pub fn stage_package(
        &mut self,
        installation_id: &InstallationId,
        target_revision: PackageRevisionId,
        is_compatible: bool,
    ) -> Result<(), UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        if !is_compatible {
            record.state = UpgradeState::CompatibilityRejected;
            record.staged_revision = Some(target_revision);
            return Err(UpgradeError::CompatibilityRejected {
                reason: "incompatible contract or ABI baseline".to_string(),
            });
        }

        record.staged_revision = Some(target_revision);
        record.state = UpgradeState::Staging;
        record.staged_state_copy = None;
        record.migration_provenance = None;
        record.post_switch_failures = 0;
        Ok(())
    }

    /// Prepares an isolated copy of current state for staged migration.
    pub fn prepare_migration_copy(
        &mut self,
        installation_id: &InstallationId,
        active_state: &BTreeMap<StateKey, StateValue>,
    ) -> Result<(), UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        if record.state != UpgradeState::Staging {
            return Err(UpgradeError::InvalidStateTransition {
                from: record.state,
                to: UpgradeState::MigratingCopy,
            });
        }

        record.staged_state_copy = Some(active_state.clone());
        record.state = UpgradeState::MigratingCopy;
        Ok(())
    }

    /// Records migration execution against the staged copy.
    pub fn record_migration_result(
        &mut self,
        installation_id: &InstallationId,
        mutated_copy: Option<BTreeMap<StateKey, StateValue>>,
        provenance: MigrationProvenance,
    ) -> Result<(), UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        if record.state != UpgradeState::MigratingCopy {
            return Err(UpgradeError::InvalidStateTransition {
                from: record.state,
                to: UpgradeState::Validating,
            });
        }

        let success = provenance.success;
        record.migration_provenance = Some(provenance);

        if success {
            if let Some(copy) = mutated_copy {
                record.staged_state_copy = Some(copy);
            }
            record.state = UpgradeState::Validating;
            Ok(())
        } else {
            record.state = UpgradeState::Failed;
            record.staged_state_copy = None;
            Err(UpgradeError::MigrationOnCopyFailed {
                reason: "staged migration script failed".to_string(),
            })
        }
    }

    /// Records health probe validation outcome.
    pub fn record_health_validation(
        &mut self,
        installation_id: &InstallationId,
        status: HealthProbeStatus,
    ) -> Result<(), UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        if record.state != UpgradeState::Validating {
            return Err(UpgradeError::InvalidStateTransition {
                from: record.state,
                to: UpgradeState::ReadyToSwitch,
            });
        }

        match status {
            HealthProbeStatus::Healthy | HealthProbeStatus::Degraded { .. } => {
                record.state = UpgradeState::ReadyToSwitch;
                Ok(())
            }
            HealthProbeStatus::Unhealthy { reason } => {
                record.state = UpgradeState::Failed;
                record.staged_state_copy = None;
                Err(UpgradeError::HealthValidationFailed { reason })
            }
        }
    }

    /// Begins the atomic switch protocol.
    pub fn begin_switch(&mut self, installation_id: &InstallationId) -> Result<(), UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        if record.state != UpgradeState::ReadyToSwitch {
            return Err(UpgradeError::InvalidStateTransition {
                from: record.state,
                to: UpgradeState::Switching,
            });
        }

        record.state = UpgradeState::Switching;
        Ok(())
    }

    /// Atomically commits active revision switch.
    ///
    /// Returns the newly active revision and state.
    pub fn commit_switch(
        &mut self,
        installation_id: &InstallationId,
        pre_switch_active_state: BTreeMap<StateKey, StateValue>,
    ) -> Result<(PackageRevisionId, BTreeMap<StateKey, StateValue>), UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        if record.state != UpgradeState::Switching {
            return Err(UpgradeError::InvalidStateTransition {
                from: record.state,
                to: UpgradeState::CurrentCandidate,
            });
        }

        let staged_rev =
            record
                .staged_revision
                .take()
                .ok_or_else(|| UpgradeError::SwitchFailed {
                    reason: "no staged revision present".to_string(),
                })?;

        let staged_state = record
            .staged_state_copy
            .take()
            .unwrap_or_else(|| pre_switch_active_state.clone());

        // Update LastKnownGood
        record.last_known_good = Some(record.current_revision.clone());
        record.last_known_good_state = Some(pre_switch_active_state);

        // Commit new revision
        record.current_revision = staged_rev.clone();
        record.state = UpgradeState::CurrentCandidate;
        record.post_switch_failures = 0;

        Ok((staged_rev, staged_state))
    }

    /// Observes post-switch health. If failures exceed threshold, indicates rollback is required.
    pub fn record_post_switch_observation(
        &mut self,
        installation_id: &InstallationId,
        is_failure: bool,
        threshold: u32,
    ) -> Result<bool, UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        if is_failure {
            record.post_switch_failures = record.post_switch_failures.saturating_add(1);
            if record.post_switch_failures >= threshold {
                return Ok(true); // Rollback triggered
            }
        } else if record.state == UpgradeState::CurrentCandidate {
            record.state = UpgradeState::Current;
        }

        Ok(false)
    }

    /// Executes rollback to the LastKnownGood revision and state.
    pub fn execute_rollback(
        &mut self,
        installation_id: &InstallationId,
    ) -> Result<(PackageRevisionId, BTreeMap<StateKey, StateValue>), UpgradeError> {
        let record = self.installations.get_mut(installation_id).ok_or_else(|| {
            UpgradeError::InstallationNotFound {
                installation: installation_id.clone(),
            }
        })?;

        record.state = UpgradeState::RollingBack;

        let lkg_rev =
            record
                .last_known_good
                .clone()
                .ok_or_else(|| UpgradeError::NoLastKnownGood {
                    installation: installation_id.clone(),
                })?;

        let lkg_state = record.last_known_good_state.clone().unwrap_or_default();

        record.current_revision = lkg_rev.clone();
        record.staged_revision = None;
        record.staged_state_copy = None;
        record.state = UpgradeState::RolledBack;
        record.post_switch_failures = 0;

        Ok((lkg_rev, lkg_state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_upgrade_happy_path_state_transitions() {
        let mut mgr = UpgradeManager::new();
        let inst = InstallationId::new("inst-1");
        let rev1 = PackageRevisionId::new("rev-1");
        let rev2 = PackageRevisionId::new("rev-2");

        let mut initial_state = BTreeMap::new();
        initial_state.insert(StateKey::new("k1"), vec![1]);

        mgr.register_initial_installation(inst.clone(), rev1.clone(), initial_state.clone());
        assert_eq!(mgr.current_revision(&inst), Some(&rev1));
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::Current));

        // 1. Stage package
        mgr.stage_package(&inst, rev2.clone(), true)
            .expect("stage ok");
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::Staging));
        assert_eq!(mgr.current_revision(&inst), Some(&rev1)); // Current is unchanged!

        // 2. Prepare migration copy
        mgr.prepare_migration_copy(&inst, &initial_state)
            .expect("prep copy ok");
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::MigratingCopy));

        // 3. Record migration result
        let mut migrated_state = initial_state.clone();
        migrated_state.insert(StateKey::new("k2"), vec![2]);
        let prov = MigrationProvenance {
            source_revision: rev1.clone(),
            target_revision: rev2.clone(),
            source_schema: StateSchemaVersion::new(1),
            target_schema: StateSchemaVersion::new(2),
            migration_path: vec![],
            success: true,
            error_message: None,
            duration_ticks: 10,
        };
        mgr.record_migration_result(&inst, Some(migrated_state.clone()), prov)
            .expect("migration ok");
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::Validating));

        // 4. Validate health
        mgr.record_health_validation(&inst, HealthProbeStatus::Healthy)
            .expect("health ok");
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::ReadyToSwitch));

        // 5. Begin switch
        mgr.begin_switch(&inst).expect("begin switch ok");
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::Switching));

        // 6. Commit switch
        let (active_rev, active_st) = mgr
            .commit_switch(&inst, initial_state.clone())
            .expect("commit switch ok");
        assert_eq!(active_rev, rev2);
        assert_eq!(active_st, migrated_state);
        assert_eq!(mgr.current_revision(&inst), Some(&rev2));
        assert_eq!(mgr.last_known_good(&inst), Some(&rev1));
        assert_eq!(
            mgr.upgrade_state(&inst),
            Some(UpgradeState::CurrentCandidate)
        );
    }

    #[test]
    fn incompatible_package_rejected_before_staging() {
        let mut mgr = UpgradeManager::new();
        let inst = InstallationId::new("inst-1");
        let rev1 = PackageRevisionId::new("rev-1");
        let rev2 = PackageRevisionId::new("rev-2");

        mgr.register_initial_installation(inst.clone(), rev1.clone(), BTreeMap::new());

        let err = mgr
            .stage_package(&inst, rev2, false)
            .expect_err("must reject");
        assert!(matches!(err, UpgradeError::CompatibilityRejected { .. }));
        assert_eq!(
            mgr.upgrade_state(&inst),
            Some(UpgradeState::CompatibilityRejected)
        );
        assert_eq!(mgr.current_revision(&inst), Some(&rev1)); // Current is unchanged!
    }

    #[test]
    fn failed_migration_on_copy_preserves_current() {
        let mut mgr = UpgradeManager::new();
        let inst = InstallationId::new("inst-1");
        let rev1 = PackageRevisionId::new("rev-1");
        let rev2 = PackageRevisionId::new("rev-2");
        let initial_state = BTreeMap::new();

        mgr.register_initial_installation(inst.clone(), rev1.clone(), initial_state.clone());
        mgr.stage_package(&inst, rev2.clone(), true)
            .expect("stage ok");
        mgr.prepare_migration_copy(&inst, &initial_state)
            .expect("prep copy ok");

        let prov = MigrationProvenance {
            source_revision: rev1.clone(),
            target_revision: rev2,
            source_schema: StateSchemaVersion::new(1),
            target_schema: StateSchemaVersion::new(2),
            migration_path: vec![],
            success: false,
            error_message: Some("syntax error in migration script".to_string()),
            duration_ticks: 5,
        };
        let err = mgr
            .record_migration_result(&inst, None, prov)
            .expect_err("migration must fail");
        assert!(matches!(err, UpgradeError::MigrationOnCopyFailed { .. }));
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::Failed));
        assert_eq!(mgr.current_revision(&inst), Some(&rev1)); // Current is intact!
    }

    #[test]
    fn post_switch_failures_trigger_rollback() {
        let mut mgr = UpgradeManager::new();
        let inst = InstallationId::new("inst-1");
        let rev1 = PackageRevisionId::new("rev-1");
        let rev2 = PackageRevisionId::new("rev-2");
        let initial_state = BTreeMap::new();

        mgr.register_initial_installation(inst.clone(), rev1.clone(), initial_state.clone());
        mgr.stage_package(&inst, rev2.clone(), true)
            .expect("stage ok");
        mgr.prepare_migration_copy(&inst, &initial_state)
            .expect("prep copy ok");
        let prov = MigrationProvenance {
            source_revision: rev1.clone(),
            target_revision: rev2,
            source_schema: StateSchemaVersion::new(1),
            target_schema: StateSchemaVersion::new(1),
            migration_path: vec![],
            success: true,
            error_message: None,
            duration_ticks: 1,
        };
        mgr.record_migration_result(&inst, None, prov)
            .expect("mig ok");
        mgr.record_health_validation(&inst, HealthProbeStatus::Healthy)
            .expect("health ok");
        mgr.begin_switch(&inst).expect("begin switch ok");
        mgr.commit_switch(&inst, initial_state)
            .expect("commit switch ok");

        // Post switch observation
        let need_rb1 = mgr
            .record_post_switch_observation(&inst, true, 2)
            .expect("obs 1");
        assert!(!need_rb1);
        let need_rb2 = mgr
            .record_post_switch_observation(&inst, true, 2)
            .expect("obs 2");
        assert!(need_rb2); // Threshold reached!

        // Rollback
        let (restored_rev, _) = mgr.execute_rollback(&inst).expect("rollback ok");
        assert_eq!(restored_rev, rev1);
        assert_eq!(mgr.current_revision(&inst), Some(&rev1));
        assert_eq!(mgr.upgrade_state(&inst), Some(UpgradeState::RolledBack));
    }
}
