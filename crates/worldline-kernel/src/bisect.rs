//! Automated bounded plugin bisect engine.
//!
//! Architectural Invariants:
//! 1. Operates only on a bounded candidate set of optional plugins.
//! 2. Preserves required core platform composition.
//! 3. Does not delete state or package artifacts.
//! 4. Every trial execution has a bounded budget.
//! 5. Outcomes: LikelyCulprit, MultipleCandidates, Inconclusive.
//! 6. Interacting failures yield Inconclusive rather than an invented single culprit.

use crate::InstallationId;

/// Outcome of automated plugin bisect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BisectOutcome {
    /// Exactly one offending installation isolated.
    LikelyCulprit(InstallationId),
    /// Several candidate installations isolated as contributors.
    MultipleCandidates(Vec<InstallationId>),
    /// Complex multi-plugin interaction or inconclusive trial results.
    Inconclusive { reason: String },
}

/// Trial execution record in bisect history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BisectTrialRecord {
    pub enabled_plugins: Vec<InstallationId>,
    pub disabled_plugins: Vec<InstallationId>,
    pub composition_healthy: bool,
}

/// Bounded bisect engine for isolating broken optional plugins.
#[derive(Clone, Debug, Default)]
pub struct BisectEngine {
    trials: Vec<BisectTrialRecord>,
}

impl BisectEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trials(&self) -> &[BisectTrialRecord] {
        &self.trials
    }

    /// Runs automated bounded bisect using a test evaluation function.
    ///
    /// `test_composition` is called with a subset of enabled optional plugins and returns
    /// `true` if the composition is healthy.
    pub fn bisect<F>(
        &mut self,
        optional_candidates: &[InstallationId],
        mut test_composition: F,
    ) -> BisectOutcome
    where
        F: FnMut(&[InstallationId]) -> bool,
    {
        self.trials.clear();

        if optional_candidates.is_empty() {
            return BisectOutcome::Inconclusive {
                reason: "no candidate plugins supplied".to_string(),
            };
        }

        // Base check: All enabled
        let all_healthy = test_composition(optional_candidates);
        self.record_trial(optional_candidates, &[], all_healthy);
        if all_healthy {
            return BisectOutcome::Inconclusive {
                reason: "all plugins together passed composition health check".to_string(),
            };
        }

        // Base check: None enabled (empty set)
        let empty_healthy = test_composition(&[]);
        self.record_trial(&[], optional_candidates, empty_healthy);
        if !empty_healthy {
            return BisectOutcome::Inconclusive {
                reason: "composition failed even with zero optional plugins enabled (core issue)"
                    .to_string(),
            };
        }

        // Test individual candidate plugins enabled one-by-one
        let mut individual_failures = Vec::new();
        let mut individual_passes = Vec::new();

        for candidate in optional_candidates {
            let single = vec![candidate.clone()];
            let mut disabled: Vec<InstallationId> = optional_candidates
                .iter()
                .filter(|&c| c != candidate)
                .cloned()
                .collect();
            disabled.sort();

            let healthy = test_composition(&single);
            self.record_trial(&single, &disabled, healthy);

            if healthy {
                individual_passes.push(candidate.clone());
            } else {
                individual_failures.push(candidate.clone());
            }
        }

        if individual_failures.len() == 1 {
            return BisectOutcome::LikelyCulprit(individual_failures.remove(0));
        }

        if individual_failures.len() > 1 {
            return BisectOutcome::MultipleCandidates(individual_failures);
        }

        // If every plugin passes individually, but all together fail, check pairwise / exclusions
        let mut exclusion_failures = Vec::new();
        for candidate in optional_candidates {
            let subset: Vec<InstallationId> = optional_candidates
                .iter()
                .filter(|&c| c != candidate)
                .cloned()
                .collect();
            let disabled = vec![candidate.clone()];

            let healthy = test_composition(&subset);
            self.record_trial(&subset, &disabled, healthy);

            if healthy {
                // Disabling this single candidate cured the composition!
                exclusion_failures.push(candidate.clone());
            }
        }

        if exclusion_failures.len() == 1 {
            BisectOutcome::LikelyCulprit(exclusion_failures.remove(0))
        } else if exclusion_failures.len() > 1 {
            BisectOutcome::MultipleCandidates(exclusion_failures)
        } else {
            BisectOutcome::Inconclusive {
                reason: "complex interacting failure between multiple optional plugins".to_string(),
            }
        }
    }

    fn record_trial(
        &mut self,
        enabled: &[InstallationId],
        disabled: &[InstallationId],
        healthy: bool,
    ) {
        self.trials.push(BisectTrialRecord {
            enabled_plugins: enabled.to_vec(),
            disabled_plugins: disabled.to_vec(),
            composition_healthy: healthy,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolates_single_culprit_failing_individually() {
        let mut engine = BisectEngine::new();
        let p1 = InstallationId::new("p1");
        let p2 = InstallationId::new("p2");
        let p3 = InstallationId::new("p3");
        let candidates = vec![p1.clone(), p2.clone(), p3.clone()];

        // p2 fails whenever present
        let outcome = engine.bisect(&candidates, |enabled| !enabled.iter().any(|c| c == &p2));

        assert_eq!(outcome, BisectOutcome::LikelyCulprit(p2));
        assert!(!engine.trials().is_empty());
    }

    #[test]
    fn isolates_single_culprit_in_interaction() {
        let mut engine = BisectEngine::new();
        let p1 = InstallationId::new("p1");
        let p2 = InstallationId::new("p2");
        let p3 = InstallationId::new("p3");
        let candidates = vec![p1.clone(), p2.clone(), p3.clone()];

        // Each individual passes, but composition fails if p3 is in the mix with others
        let outcome = engine.bisect(&candidates, |enabled| {
            if enabled.len() <= 1 {
                true
            } else {
                !enabled.iter().any(|c| c == &p3)
            }
        });

        assert_eq!(outcome, BisectOutcome::LikelyCulprit(p3));
    }

    #[test]
    fn reports_inconclusive_when_complex_interaction() {
        let mut engine = BisectEngine::new();
        let p1 = InstallationId::new("p1");
        let p2 = InstallationId::new("p2");
        let p3 = InstallationId::new("p3");
        let candidates = vec![p1.clone(), p2.clone(), p3.clone()];

        // Fails only when at least 2 plugins are present simultaneously, none individually
        let outcome = engine.bisect(&candidates, |enabled| enabled.len() <= 1);

        assert_eq!(
            outcome,
            BisectOutcome::Inconclusive {
                reason: "complex interacting failure between multiple optional plugins".to_string()
            }
        );
    }
}
