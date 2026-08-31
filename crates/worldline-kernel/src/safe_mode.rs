//! Generic Safe Mode composition and policy subsystem.
//!
//! Architectural Invariants:
//! 1. SAFE MODE REDUCES COMPOSITION; IT DOES NOT BYPASS AUTHORIZATION.
//! 2. Safe mode boots a minimal known-good composition when normal composition cannot become healthy.
//! 3. Safe mode does not automatically activate optional third-party plugins.
//! 4. Core diagnostic/control facilities remain available.
//! 5. Persistent user/plugin data is not deleted.
//! 6. Disabled/quarantined plugins and reasons are observable.
//! 7. Safe mode contains NO special-case browser or agent domain logic.

use std::fmt;

use crate::{InstallationId, runtime::RuntimeCriticality};

/// Safe mode activation policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SafeModeState {
    /// Normal host composition: all declared and healthy plugins are activated.
    #[default]
    Normal,
    /// Safe mode active: only Mandatory core plugins activate; optional plugins are suppressed.
    SafeModeActive { reason_code: SafeModeReason },
}

/// Trigger reason for entering Safe Mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafeModeReason {
    ExplicitOperatorRequest,
    RepeatedHostCompositionFailure,
    CriticalPluginCrashLoop,
}

impl fmt::Display for SafeModeReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExplicitOperatorRequest => formatter.write_str("explicit operator request"),
            Self::RepeatedHostCompositionFailure => {
                formatter.write_str("repeated host composition failure")
            }
            Self::CriticalPluginCrashLoop => formatter.write_str("critical plugin crash loop"),
        }
    }
}

/// Controls safe mode composition filtering.
#[derive(Clone, Debug, Default)]
pub struct SafeModeManager {
    state: SafeModeState,
}

impl SafeModeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn is_safe_mode(&self) -> bool {
        matches!(self.state, SafeModeState::SafeModeActive { .. })
    }

    pub const fn state(&self) -> SafeModeState {
        self.state
    }

    pub fn enter_safe_mode(&mut self, reason: SafeModeReason) {
        self.state = SafeModeState::SafeModeActive {
            reason_code: reason,
        };
    }

    pub fn exit_safe_mode(&mut self) {
        self.state = SafeModeState::Normal;
    }

    /// Evaluates whether a plugin installation should be activated under current safe mode policy.
    ///
    /// Invariant: In safe mode, Optional plugins are suppressed. Mandatory/Core plugins activate.
    /// Security checks and capability authorization remain 100% enforced in all modes.
    #[must_use]
    pub fn should_activate_installation(
        &self,
        _installation_id: &InstallationId,
        criticality: RuntimeCriticality,
        is_quarantined: bool,
    ) -> bool {
        if is_quarantined {
            return false;
        }

        match self.state {
            SafeModeState::Normal => true,
            SafeModeState::SafeModeActive { .. } => criticality == RuntimeCriticality::Required,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_mode_filters_optional_plugins_but_preserves_required() {
        let mut sm = SafeModeManager::new();
        let inst_opt = InstallationId::new("third-party-tool");
        let inst_core = InstallationId::new("core-storage-adapter");

        assert!(!sm.is_safe_mode());
        assert!(sm.should_activate_installation(&inst_opt, RuntimeCriticality::Optional, false));
        assert!(sm.should_activate_installation(&inst_core, RuntimeCriticality::Required, false));

        // Enter safe mode
        sm.enter_safe_mode(SafeModeReason::RepeatedHostCompositionFailure);
        assert!(sm.is_safe_mode());

        // Optional plugin is suppressed
        assert!(!sm.should_activate_installation(&inst_opt, RuntimeCriticality::Optional, false));
        // Required plugin is permitted
        assert!(sm.should_activate_installation(&inst_core, RuntimeCriticality::Required, false));
        // Quarantined is always suppressed
        assert!(!sm.should_activate_installation(&inst_core, RuntimeCriticality::Required, true));

        sm.exit_safe_mode();
        assert!(!sm.is_safe_mode());
        assert!(sm.should_activate_installation(&inst_opt, RuntimeCriticality::Optional, false));
    }
}
