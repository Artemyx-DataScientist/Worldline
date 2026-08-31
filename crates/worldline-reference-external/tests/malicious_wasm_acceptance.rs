//! M0.6 malicious WASM acceptance suite: verifies least-authority linking,
//! typed resource limits, fuel exhaustion, trap containment, and state/handle
//! isolation through the reference fixture against real compiled hostile WASM
//! components.

mod support;

use worldline_kernel::CapabilityError;
use worldline_reference_external::{
    BuiltinEchoPlugin, EchoFixture, OPERATION_ECHO, WasmEchoPlugin,
};

#[test]
fn malicious_trapper_traps_safely_without_damaging_other_runtimes() {
    let components = support::test_components();
    let hostile_plugin =
        WasmEchoPlugin::from_component("reference.echo.trapper", components.trapper.clone());
    let hostile_fixture = EchoFixture::boot(hostile_plugin).expect("hostile fixture boots");

    let outcome = hostile_fixture.call(OPERATION_ECHO, b"trigger-trap", "trap-req-1");
    assert!(
        outcome.is_err(),
        "invoking a trapping component must return an error outcome, got {outcome:?}"
    );

    // Host and unrelated runtimes stay healthy: a builtin or benign wasm fixture boots and works.
    let benign_plugin =
        WasmEchoPlugin::from_component("reference.echo.benign", components.benign_echo.clone());
    let benign_fixture = EchoFixture::boot(benign_plugin).expect("benign fixture boots");
    let result = benign_fixture
        .call(OPERATION_ECHO, b"still-alive", "benign-req-1")
        .expect("benign component must work after a trap in another runtime");
    assert_eq!(result, b"echo:still-alive");

    let builtin_fixture = EchoFixture::boot(BuiltinEchoPlugin::new("reference.echo.builtin"))
        .expect("builtin fixture boots");
    let result = builtin_fixture
        .call(OPERATION_ECHO, b"builtin-alive", "builtin-req-1")
        .expect("builtin provider must work");
    assert_eq!(result, b"echo:builtin-alive");
}

#[test]
fn malicious_memory_hog_is_contained_by_memory_limits() {
    let components = support::test_components();
    let hog_plugin =
        WasmEchoPlugin::from_component("reference.echo.hog", components.memory_hog.clone());
    let hog_fixture = EchoFixture::boot(hog_plugin).expect("hog fixture boots");

    let outcome = hog_fixture.call(OPERATION_ECHO, b"alloc-excess", "hog-req-1");
    assert!(
        outcome.is_err(),
        "memory hog exceeding quota must fail, got {outcome:?}"
    );

    // Unrelated instance works without issue.
    let benign_plugin =
        WasmEchoPlugin::from_component("reference.echo.benign", components.benign_echo.clone());
    let benign_fixture = EchoFixture::boot(benign_plugin).expect("benign fixture boots");
    let result = benign_fixture
        .call(OPERATION_ECHO, b"healthy", "benign-req-2")
        .expect("benign component works");
    assert_eq!(result, b"echo:healthy");
}

#[test]
fn malicious_spin_loop_is_contained_by_fuel_budget() {
    let components = support::test_components();
    let spin_plugin =
        WasmEchoPlugin::from_component("reference.echo.spin", components.spin_loop.clone());
    let spin_fixture = EchoFixture::boot(spin_plugin).expect("spin fixture boots");

    let outcome = spin_fixture.call(OPERATION_ECHO, b"loop", "spin-req-1");
    assert!(
        outcome.is_err(),
        "infinite spin loop must be terminated by fuel budget, got {outcome:?}"
    );

    // Host remains operational.
    let benign_plugin =
        WasmEchoPlugin::from_component("reference.echo.benign", components.benign_echo.clone());
    let benign_fixture = EchoFixture::boot(benign_plugin).expect("benign fixture boots");
    let result = benign_fixture
        .call(OPERATION_ECHO, b"fast", "benign-req-3")
        .expect("benign component works");
    assert_eq!(result, b"echo:fast");
}

#[test]
fn foreign_importer_demanding_wasi_fails_at_instantiation() {
    let components = support::test_components();
    let foreign_plugin = WasmEchoPlugin::from_component(
        "reference.echo.foreign",
        components.foreign_importer.clone(),
    );
    let fixture = EchoFixture::boot(foreign_plugin).expect("fixture registration boots");

    // Instantiation happens on the first call with context and fails closed because the host
    // provides zero ambient WASI imports.
    let outcome = fixture.call(OPERATION_ECHO, b"wasi-attempt", "foreign-req-1");
    assert!(
        outcome.is_err(),
        "component demanding unauthorized WASI imports must fail closed, got {outcome:?}"
    );
}

#[test]
fn ungranted_caller_cannot_reach_sandboxed_wasm_provider() {
    let components = support::test_components();
    let plugin =
        WasmEchoPlugin::from_component("reference.echo.wasm", components.benign_echo.clone());
    let fixture = EchoFixture::boot(plugin).expect("fixture boots");

    let intruder = fixture
        .register_unauthorized_subject("intruder")
        .expect("intruder registration");

    let outcome = fixture
        .call_unauthorized(&intruder, OPERATION_ECHO, b"probe")
        .expect("handle acquisition succeeds for registered subject");

    assert!(
        matches!(outcome, Err(CapabilityError::Denied { .. })),
        "broker must deny call before it ever reaches WASM instance, got {outcome:?}"
    );
}
