//! Formalization of external side-effect outcomes and explicit Incomplete handling.
//!
//! Architectural Invariants:
//! 1. UNKNOWN EXTERNAL SIDE EFFECT OUTCOME IS INCOMPLETE, NEVER SYNTHETIC SUCCESS OR FAILURE.
//! 2. Incomplete != Failed, Incomplete != Succeeded.
//! 3. Incomplete external action is not automatically retried unless operation contract proves retry/idempotency safe.
//! 4. Incomplete outcome carries CorrelationId / InvocationId and diagnostic reason.

use std::fmt;

use crate::{CorrelationId, security::InvocationId};

/// Lifecycle outcome of an external side-effect invocation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SideEffectOutcome {
    /// Action has not started dispatch.
    NotStarted,
    /// Local state change committed, but external dispatch not yet confirmed.
    CommittedLocal,
    /// External provider acknowledged successful completion.
    Succeeded,
    /// External provider acknowledged deterministic failure.
    Failed,
    /// Dispatched, but unprovable after crash or disconnection whether remote effect took place.
    Incomplete,
}

impl fmt::Display for SideEffectOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::NotStarted => "NotStarted",
            Self::CommittedLocal => "CommittedLocal",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Incomplete => "Incomplete",
        };
        formatter.write_str(name)
    }
}

/// Durable record of an external side-effect execution attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SideEffectRecord {
    pub invocation_id: InvocationId,
    pub correlation_id: Option<CorrelationId>,
    pub is_idempotent: bool,
    pub outcome: SideEffectOutcome,
    pub diagnostic_reason: Option<String>,
}

impl SideEffectRecord {
    #[must_use]
    pub fn new(
        invocation_id: InvocationId,
        correlation_id: Option<CorrelationId>,
        is_idempotent: bool,
    ) -> Self {
        Self {
            invocation_id,
            correlation_id,
            is_idempotent,
            outcome: SideEffectOutcome::NotStarted,
            diagnostic_reason: None,
        }
    }

    /// Evaluates whether this operation can be automatically retried after an interrupted outcome.
    #[must_use]
    pub fn is_auto_retry_safe(&self) -> bool {
        match self.outcome {
            SideEffectOutcome::NotStarted | SideEffectOutcome::CommittedLocal => true,
            SideEffectOutcome::Succeeded => false,
            SideEffectOutcome::Failed => self.is_idempotent,
            // Incomplete is ONLY retryable if formally idempotent!
            SideEffectOutcome::Incomplete => self.is_idempotent,
        }
    }

    pub fn mark_dispatched(&mut self) {
        self.outcome = SideEffectOutcome::CommittedLocal;
    }

    pub fn mark_succeeded(&mut self) {
        self.outcome = SideEffectOutcome::Succeeded;
        self.diagnostic_reason = None;
    }

    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.outcome = SideEffectOutcome::Failed;
        self.diagnostic_reason = Some(reason.into());
    }

    pub fn mark_incomplete(&mut self, reason: impl Into<String>) {
        self.outcome = SideEffectOutcome::Incomplete;
        self.diagnostic_reason = Some(reason.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_non_idempotent_is_not_auto_retry_safe() {
        let mut rec = SideEffectRecord::new(
            InvocationId::new("inv-pay-1"),
            Some(CorrelationId::new("corr-pay")),
            false, // Non-idempotent operation!
        );

        rec.mark_dispatched();
        rec.mark_incomplete("host crashed before payment provider ACK received");

        assert_eq!(rec.outcome, SideEffectOutcome::Incomplete);
        assert!(!rec.is_auto_retry_safe()); // Must NOT retry!
    }

    #[test]
    fn incomplete_idempotent_is_auto_retry_safe() {
        let mut rec = SideEffectRecord::new(
            InvocationId::new("inv-get-status"),
            Some(CorrelationId::new("corr-get")),
            true, // Formally idempotent
        );

        rec.mark_incomplete("network timeout");
        assert_eq!(rec.outcome, SideEffectOutcome::Incomplete);
        assert!(rec.is_auto_retry_safe());
    }
}
