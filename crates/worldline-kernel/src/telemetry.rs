//! Per-runtime operational telemetry and metrics.
//!
//! Architectural Invariants:
//! 1. OPERATIONAL METRICS DO NOT CONTAIN RAW PRIVATE RPC/EVENT/STATE PAYLOAD.
//! 2. Metrics are diagnostic observations and do not become security decisions unless explicit policy consumes them.
//! 3. Telemetry belongs to generic runtime/plugin identity, not browser-specific concepts.

use std::collections::BTreeMap;

use crate::RuntimeId;

/// Per-runtime operational metrics counters and gauges.
///
/// Invariant: STRICTLY NO raw private RPC, event, or state payloads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeOperationalMetrics {
    /// Activation duration in host ticks/nanos.
    pub activation_duration_ticks: u64,
    /// Deactivation duration in host ticks/nanos.
    pub deactivation_duration_ticks: u64,
    /// Number of restarts experienced by this installation incarnation.
    pub restart_count: u32,
    /// Number of unexpected crashes or traps.
    pub crash_count: u32,
    /// Number of hung/timeout deactivations.
    pub hung_count: u32,
    /// Number of quarantine incidents.
    pub quarantine_count: u32,
    /// Current in-flight RPC invocations.
    pub rpc_in_flight: u32,
    /// Current queued RPC invocations.
    pub rpc_queue_depth: u32,
    /// Current subscriber event mailbox depth.
    pub event_mailbox_depth: u32,
    /// Count of dropped/backpressured events.
    pub event_drops: u64,
    /// Count of authorization denials at admission.
    pub authorization_denials: u64,
    /// Measured linear memory in bytes if reported/metered.
    pub memory_bytes_used: Option<u64>,
    /// Measured CPU / fuel execution budget consumed if metered.
    pub cpu_budget_ticks_used: Option<u64>,
}

/// Stores and aggregates operational metrics across active and historical runtimes.
#[derive(Clone, Debug, Default)]
pub struct TelemetryRegistry {
    metrics: BTreeMap<RuntimeId, RuntimeOperationalMetrics>,
}

impl TelemetryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_activation(&mut self, runtime_id: RuntimeId, duration_ticks: u64) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.activation_duration_ticks = duration_ticks;
    }

    pub fn record_deactivation(&mut self, runtime_id: RuntimeId, duration_ticks: u64) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.deactivation_duration_ticks = duration_ticks;
    }

    pub fn record_crash(&mut self, runtime_id: RuntimeId) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.crash_count = entry.crash_count.saturating_add(1);
    }

    pub fn record_restart(&mut self, runtime_id: RuntimeId) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.restart_count = entry.restart_count.saturating_add(1);
    }

    pub fn record_hung(&mut self, runtime_id: RuntimeId) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.hung_count = entry.hung_count.saturating_add(1);
    }

    pub fn record_quarantine(&mut self, runtime_id: RuntimeId) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.quarantine_count = entry.quarantine_count.saturating_add(1);
    }

    pub fn record_rpc_state(&mut self, runtime_id: RuntimeId, in_flight: u32, queue_depth: u32) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.rpc_in_flight = in_flight;
        entry.rpc_queue_depth = queue_depth;
    }

    pub fn record_event_mailbox(&mut self, runtime_id: RuntimeId, depth: u32, drops: u64) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.event_mailbox_depth = depth;
        entry.event_drops = entry.event_drops.saturating_add(drops);
    }

    pub fn record_authorization_denial(&mut self, runtime_id: RuntimeId) {
        let entry = self.metrics.entry(runtime_id).or_default();
        entry.authorization_denials = entry.authorization_denials.saturating_add(1);
    }

    pub fn record_resource_usage(
        &mut self,
        runtime_id: RuntimeId,
        memory_bytes: Option<u64>,
        cpu_ticks: Option<u64>,
    ) {
        let entry = self.metrics.entry(runtime_id).or_default();
        if let Some(mem) = memory_bytes {
            entry.memory_bytes_used = Some(mem);
        }
        if let Some(cpu) = cpu_ticks {
            entry.cpu_budget_ticks_used = Some(cpu);
        }
    }

    pub fn get_metrics(&self, runtime_id: &RuntimeId) -> Option<&RuntimeOperationalMetrics> {
        self.metrics.get(runtime_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_records_numeric_metrics_without_payload() {
        let mut registry = TelemetryRegistry::new();
        let runtime = RuntimeId::new(1, 1);

        registry.record_activation(runtime, 150);
        registry.record_authorization_denial(runtime);
        registry.record_rpc_state(runtime, 2, 5);
        registry.record_event_mailbox(runtime, 10, 1);
        registry.record_resource_usage(runtime, Some(1024 * 1024), Some(500));

        let m = registry.get_metrics(&runtime).expect("metrics exist");
        assert_eq!(m.activation_duration_ticks, 150);
        assert_eq!(m.authorization_denials, 1);
        assert_eq!(m.rpc_in_flight, 2);
        assert_eq!(m.rpc_queue_depth, 5);
        assert_eq!(m.event_mailbox_depth, 10);
        assert_eq!(m.event_drops, 1);
        assert_eq!(m.memory_bytes_used, Some(1024 * 1024));
        assert_eq!(m.cpu_budget_ticks_used, Some(500));
    }
}
