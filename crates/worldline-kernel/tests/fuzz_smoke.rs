//! Fuzzing smoke tests for parser, envelope, compatibility, and diagnostic inputs.
//!
//! Invariant: Arbitrary fuzz inputs must NOT panic privileged host code.
//! All invalid inputs must fail closed cleanly.

use worldline_kernel::{
    CapabilityId, ContractStability, CorrelationId, DiagnosticCausalityGraph, InterfaceVersion,
    InvocationId,
};

#[test]
fn fuzz_smoke_capability_id_inputs() {
    let long_str = "a".repeat(1000);
    let inputs = [
        "",
        "   ",
        "\0\0\0",
        "../../etc/passwd",
        long_str.as_str(),
        "namespace/name@99999999999999999999999999",
        "invalid:prefix///",
        "---",
        "123@abc",
        "unicode: \u{1F600} / test",
    ];

    for input in inputs {
        let cap = CapabilityId::new(input, input, InterfaceVersion::new(1, 0));
        let _ = cap.is_well_formed();
        let _ = cap.contract();
        let _ = cap.to_string();
    }
}

#[test]
fn fuzz_smoke_diagnostic_graph_queries() {
    let graph = DiagnosticCausalityGraph::new();
    let long_str = "a".repeat(2048);
    let fuzz_ids = [
        "",
        " ",
        "\0",
        "corr-////\\\\",
        long_str.as_str(),
        "123.456.789",
    ];

    for fuzz_id in fuzz_ids {
        let corr = CorrelationId::new(fuzz_id);
        let chain = graph.query_by_correlation(&corr);
        assert_eq!(chain.facts.len(), 0);

        let inv = InvocationId::new(fuzz_id);
        let chain_inv = graph.query_by_invocation(&inv);
        assert_eq!(chain_inv.facts.len(), 0);
    }
}

#[test]
fn fuzz_smoke_compatibility_resolution_randomized() {
    for seed in 0..100 {
        let major_a = (seed * 7) % 10;
        let minor_a = (seed * 13) % 20;
        let major_b = (seed * 11) % 10;
        let minor_b = (seed * 17) % 20;

        let stability = if seed % 2 == 0 {
            ContractStability::Stable
        } else {
            ContractStability::Experimental
        };

        let cap_a = CapabilityId::with_stability(
            "ns",
            "name",
            InterfaceVersion::new(major_a as u16, minor_a as u16),
            stability,
        );
        let cap_b = CapabilityId::with_stability(
            "ns",
            "name",
            InterfaceVersion::new(major_b as u16, minor_b as u16),
            stability,
        );

        let _ = cap_a.is_compatible_with(&cap_b);
        let _ = cap_b.is_compatible_with(&cap_a);
    }
}
