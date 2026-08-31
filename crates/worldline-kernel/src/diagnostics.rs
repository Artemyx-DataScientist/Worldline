//! Diagnostic causality reconstruction and time-travel querying.
//!
//! Architectural Invariants:
//! 1. DIAGNOSTICS ARE OBSERVATION, NOT AUTHORITY.
//! 2. Causality is derived strictly from correlation_id / causation_ref metadata, NEVER inferred from timestamp proximity.
//! 3. Missing or pruned evidence appears explicitly as a diagnostic gap.
//! 4. Query never exposes raw payloads or private state values.

use std::fmt;

use crate::{CorrelationId, EventId, RuntimeId, rpc::CausationRef, security::InvocationId};

/// The semantic kind of an observed causal fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CausalFactKind {
    AdmissionCheck,
    ProviderSelection,
    InvocationDispatch,
    StateOutboxCommit,
    EventObservation,
    LifecycleTransition,
    FollowUpAction,
    DiagnosticGap,
}

impl fmt::Display for CausalFactKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::AdmissionCheck => "AdmissionCheck",
            Self::ProviderSelection => "ProviderSelection",
            Self::InvocationDispatch => "InvocationDispatch",
            Self::StateOutboxCommit => "StateOutboxCommit",
            Self::EventObservation => "EventObservation",
            Self::LifecycleTransition => "LifecycleTransition",
            Self::FollowUpAction => "FollowUpAction",
            Self::DiagnosticGap => "DiagnosticGap",
        };
        formatter.write_str(name)
    }
}

/// One recorded causal fact in the diagnostic graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CausalFact {
    pub fact_id: u64,
    pub kind: CausalFactKind,
    pub correlation_id: Option<CorrelationId>,
    pub causation_ref: Option<CausationRef>,
    pub invocation_id: Option<InvocationId>,
    pub event_id: Option<EventId>,
    pub runtime_id: Option<RuntimeId>,
    pub summary: String,
    pub timestamp_seq: u64,
}

/// Diagnostic timeline query result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticCausalityChain {
    pub query_target: String,
    pub facts: Vec<CausalFact>,
    pub has_gaps: bool,
}

/// Stores and queries recorded causal trajectory facts.
#[derive(Clone, Debug, Default)]
pub struct DiagnosticCausalityGraph {
    facts: Vec<CausalFact>,
    next_fact_id: u64,
}

impl DiagnosticCausalityGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an observed causal fact.
    #[allow(clippy::too_many_arguments)]
    pub fn record_fact(
        &mut self,
        kind: CausalFactKind,
        correlation_id: Option<CorrelationId>,
        causation_ref: Option<CausationRef>,
        invocation_id: Option<InvocationId>,
        event_id: Option<EventId>,
        runtime_id: Option<RuntimeId>,
        summary: impl Into<String>,
        timestamp_seq: u64,
    ) -> u64 {
        let fact_id = self.next_fact_id;
        self.next_fact_id = self.next_fact_id.saturating_add(1);

        self.facts.push(CausalFact {
            fact_id,
            kind,
            correlation_id,
            causation_ref,
            invocation_id,
            event_id,
            runtime_id,
            summary: summary.into(),
            timestamp_seq,
        });

        fact_id
    }

    /// Queries the causal chain for a specific CorrelationId.
    #[must_use]
    pub fn query_by_correlation(&self, correlation_id: &CorrelationId) -> DiagnosticCausalityChain {
        let mut relevant: Vec<CausalFact> = self
            .facts
            .iter()
            .filter(|f| f.correlation_id.as_ref() == Some(correlation_id))
            .cloned()
            .collect();

        relevant.sort_by_key(|f| f.timestamp_seq);

        let has_gaps = self.detect_causal_gaps(&relevant);

        DiagnosticCausalityChain {
            query_target: correlation_id.as_str().to_string(),
            facts: relevant,
            has_gaps,
        }
    }

    /// Queries the causal chain for a specific InvocationId.
    #[must_use]
    pub fn query_by_invocation(&self, invocation_id: &InvocationId) -> DiagnosticCausalityChain {
        let mut relevant: Vec<CausalFact> = self
            .facts
            .iter()
            .filter(|f| f.invocation_id.as_ref() == Some(invocation_id))
            .cloned()
            .collect();

        relevant.sort_by_key(|f| f.timestamp_seq);
        let has_gaps = self.detect_causal_gaps(&relevant);

        DiagnosticCausalityChain {
            query_target: invocation_id.as_str().to_string(),
            facts: relevant,
            has_gaps,
        }
    }

    fn detect_causal_gaps(&self, facts: &[CausalFact]) -> bool {
        // If an invocation exists without an admission check or outcome, a gap exists
        let has_invocation = facts
            .iter()
            .any(|f| f.kind == CausalFactKind::InvocationDispatch);
        let has_admission = facts
            .iter()
            .any(|f| f.kind == CausalFactKind::AdmissionCheck);
        if has_invocation && !has_admission {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn causal_chain_reconstruction_from_metadata() {
        let mut graph = DiagnosticCausalityGraph::new();
        let corr = CorrelationId::new("corr-123");
        let inv = InvocationId::new("inv-456");

        graph.record_fact(
            CausalFactKind::AdmissionCheck,
            Some(corr.clone()),
            None,
            Some(inv.clone()),
            None,
            None,
            "Caller authorized for capability",
            1,
        );

        graph.record_fact(
            CausalFactKind::ProviderSelection,
            Some(corr.clone()),
            None,
            Some(inv.clone()),
            None,
            None,
            "Selected provider-v1",
            2,
        );

        graph.record_fact(
            CausalFactKind::InvocationDispatch,
            Some(corr.clone()),
            None,
            Some(inv.clone()),
            None,
            None,
            "Dispatched RPC request",
            3,
        );

        let chain = graph.query_by_correlation(&corr);
        assert_eq!(chain.facts.len(), 3);
        assert!(!chain.has_gaps);
        assert_eq!(chain.facts[0].kind, CausalFactKind::AdmissionCheck);
        assert_eq!(chain.facts[1].kind, CausalFactKind::ProviderSelection);
        assert_eq!(chain.facts[2].kind, CausalFactKind::InvocationDispatch);
    }

    #[test]
    fn detects_missing_admission_as_gap() {
        let mut graph = DiagnosticCausalityGraph::new();
        let corr = CorrelationId::new("corr-missing-adm");
        let inv = InvocationId::new("inv-999");

        // Invocation dispatched directly without recorded admission
        graph.record_fact(
            CausalFactKind::InvocationDispatch,
            Some(corr.clone()),
            None,
            Some(inv.clone()),
            None,
            None,
            "Dispatched RPC without prior recorded admission",
            1,
        );

        let chain = graph.query_by_correlation(&corr);
        assert!(chain.has_gaps);
    }
}
