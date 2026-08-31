//! Persistent plugin quarantine subsystem.
//!
//! Architectural Invariants:
//! 1. QUARANTINE REMOVES A RUNTIME/PACKAGE FROM AUTOMATIC ACTIVATION; IT DOES NOT DELETE ITS STATE.
//! 2. Quarantine state is persistent across host restart.
//! 3. Quarantine has reason, timestamp/sequence and originating revision.
//! 4. Quarantine prevents automatic activation but not inspection, removal, or explicit recovery.
//! 5. Quarantine is per installation/revision according to recorded reason; it is not a global ban.

use std::{collections::BTreeMap, fmt};

use crate::{InstallationId, upgrade::PackageRevisionId};

/// Trigger reason that placed an installation into quarantine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuarantineReason {
    RepeatedActivationFailure { attempts: u32 },
    RepeatedCrash { crash_count: u32 },
    RepeatedTrapOrResourceViolation { violation: String },
    ProtocolViolation { error: String },
    FailedPostUpgradeHealthPolicy { reason: String },
    ManualQuarantine { operator_reason: String },
}

impl fmt::Display for QuarantineReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RepeatedActivationFailure { attempts } => {
                write!(
                    formatter,
                    "repeated activation failure ({attempts} attempts)"
                )
            }
            Self::RepeatedCrash { crash_count } => {
                write!(formatter, "repeated crashes ({crash_count} crashes)")
            }
            Self::RepeatedTrapOrResourceViolation { violation } => {
                write!(formatter, "resource violation: {violation}")
            }
            Self::ProtocolViolation { error } => {
                write!(formatter, "protocol violation: {error}")
            }
            Self::FailedPostUpgradeHealthPolicy { reason } => {
                write!(formatter, "post-upgrade health policy failed: {reason}")
            }
            Self::ManualQuarantine { operator_reason } => {
                write!(formatter, "operator quarantine: {operator_reason}")
            }
        }
    }
}

/// Durable record of an installation placed into quarantine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuarantineRecord {
    pub installation_id: InstallationId,
    pub package_revision_id: PackageRevisionId,
    pub reason: QuarantineReason,
    pub timestamp_tick: u64,
    pub originating_revision: PackageRevisionId,
}

/// Manages persistent quarantine state across host restarts.
#[derive(Clone, Debug, Default)]
pub struct QuarantineManager {
    records: BTreeMap<InstallationId, QuarantineRecord>,
}

impl QuarantineManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Places an installation into quarantine.
    pub fn quarantine(&mut self, record: QuarantineRecord) {
        self.records.insert(record.installation_id.clone(), record);
    }

    /// Releases an installation from quarantine.
    pub fn lift_quarantine(
        &mut self,
        installation_id: &InstallationId,
    ) -> Option<QuarantineRecord> {
        self.records.remove(installation_id)
    }

    /// Checks whether an installation is currently quarantined.
    #[must_use]
    pub fn is_quarantined(&self, installation_id: &InstallationId) -> bool {
        self.records.contains_key(installation_id)
    }

    /// Returns the quarantine record for an installation if present.
    #[must_use]
    pub fn get_quarantine(&self, installation_id: &InstallationId) -> Option<&QuarantineRecord> {
        self.records.get(installation_id)
    }

    /// Lists all currently quarantined installations.
    #[must_use]
    pub fn list_quarantined(&self) -> Vec<&QuarantineRecord> {
        self.records.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarantine_and_lift_lifecycle() {
        let mut qm = QuarantineManager::new();
        let inst = InstallationId::new("broken-plugin");
        let rev = PackageRevisionId::new("rev-1");

        assert!(!qm.is_quarantined(&inst));

        let record = QuarantineRecord {
            installation_id: inst.clone(),
            package_revision_id: rev.clone(),
            reason: QuarantineReason::RepeatedCrash { crash_count: 5 },
            timestamp_tick: 42,
            originating_revision: rev,
        };

        qm.quarantine(record.clone());
        assert!(qm.is_quarantined(&inst));
        assert_eq!(qm.get_quarantine(&inst), Some(&record));
        assert_eq!(qm.list_quarantined().len(), 1);

        let lifted = qm.lift_quarantine(&inst).expect("lift");
        assert_eq!(lifted, record);
        assert!(!qm.is_quarantined(&inst));
    }
}
