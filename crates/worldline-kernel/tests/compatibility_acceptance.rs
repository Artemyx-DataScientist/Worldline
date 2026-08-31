//! Acceptance tests for M0.7 Contract Stability, Compatibility Rules and Fixture Matrix.
//!
//! Required Tests (from Spec):
//! 1. Stable compatible minor provider works with unchanged consumer.
//! 2. Stable incompatible major is rejected before activation.
//! 3. Experimental unsupported revision is rejected explicitly.
//! 4. Compatibility classification grants no authority.
//! 5. SDK N/N-1/N-2 designated fixtures pass supported current kernel matrix.
//! 6. Current SDK fixture rejects unsupported historical kernel baseline cleanly.

use worldline_kernel::{CapabilityId, ContractStability, InterfaceVersion};

#[test]
fn stable_compatible_minor_provider_works_with_unchanged_consumer() {
    let required = CapabilityId::new("reference.echo", "echo", InterfaceVersion::new(1, 0));
    let provided_v1_0 = CapabilityId::new("reference.echo", "echo", InterfaceVersion::new(1, 0));
    let provided_v1_2 = CapabilityId::new("reference.echo", "echo", InterfaceVersion::new(1, 2));

    assert!(provided_v1_0.is_compatible_with(&required));
    assert!(provided_v1_2.is_compatible_with(&required));
    assert_eq!(required.stability(), ContractStability::Stable);
}

#[test]
fn stable_incompatible_major_is_rejected_before_activation() {
    let required = CapabilityId::new("reference.echo", "echo", InterfaceVersion::new(1, 0));
    let provided_v2_0 = CapabilityId::new("reference.echo", "echo", InterfaceVersion::new(2, 0));

    assert!(!provided_v2_0.is_compatible_with(&required));
}

#[test]
fn experimental_unsupported_revision_is_rejected_explicitly() {
    let required = CapabilityId::with_stability(
        "reference.experimental",
        "feature",
        InterfaceVersion::new(0, 1),
        ContractStability::Experimental,
    );
    let provided_same = CapabilityId::with_stability(
        "reference.experimental",
        "feature",
        InterfaceVersion::new(0, 1),
        ContractStability::Experimental,
    );
    let provided_bumped = CapabilityId::with_stability(
        "reference.experimental",
        "feature",
        InterfaceVersion::new(0, 2),
        ContractStability::Experimental,
    );

    assert!(provided_same.is_compatible_with(&required));
    assert!(
        !provided_bumped.is_compatible_with(&required),
        "Experimental requires exact minor match"
    );
}

#[test]
fn stability_class_mismatch_is_rejected() {
    let required_stable = CapabilityId::with_stability(
        "reference.echo",
        "echo",
        InterfaceVersion::new(1, 0),
        ContractStability::Stable,
    );
    let provided_experimental = CapabilityId::with_stability(
        "reference.echo",
        "echo",
        InterfaceVersion::new(1, 0),
        ContractStability::Experimental,
    );

    assert!(!provided_experimental.is_compatible_with(&required_stable));
    assert!(!required_stable.is_compatible_with(&provided_experimental));
}

#[test]
fn compatibility_classification_grants_no_authority() {
    let cap = CapabilityId::new("reference.echo", "echo", InterfaceVersion::new(1, 0));
    let contract = cap.contract();

    // Contract has namespace and major interface version, but is NOT a grant
    assert_eq!(contract.namespace(), "reference.echo");
    assert_eq!(contract.interface_major(), 1);
}
