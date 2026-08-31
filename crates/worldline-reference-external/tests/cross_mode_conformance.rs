//! Cross-mode conformance for `reference.echo/v1`. The same unchanged
//! consumer (EchoFixture) runs against every execution mode; these are the
//! builtin cases. Native-process and WASM cases register their adapters
//! through the same fixture and assert identical observable semantics.

use worldline_kernel::CapabilityError;
use worldline_reference_external::{
    BuiltinEchoPlugin, EchoFixture, OPERATION_ECHO, OPERATION_PUBLISH_OBSERVATION,
    OPERATION_STATEFUL_INCREMENT, format_echo_result, format_increment_result,
    format_observation_result,
};

fn builtin_fixture() -> Result<EchoFixture, String> {
    EchoFixture::boot(BuiltinEchoPlugin::new("reference.echo.builtin"))
}

#[test]
fn builtin_authorized_echo_returns_the_shared_result() {
    let fixture = builtin_fixture().expect("builtin fixture must boot");
    let result = fixture
        .call(OPERATION_ECHO, b"hello", "builtin-echo-1")
        .expect("authorized echo must succeed");
    assert_eq!(result, format_echo_result(b"hello"));
}

#[test]
fn builtin_stateful_increment_persists_through_installation_state() {
    let fixture = builtin_fixture().expect("builtin fixture must boot");
    let first = fixture
        .call(OPERATION_STATEFUL_INCREMENT, b"a", "builtin-inc-1")
        .expect("first increment must succeed");
    let second = fixture
        .call(OPERATION_STATEFUL_INCREMENT, b"b", "builtin-inc-2")
        .expect("second increment must succeed");
    assert_eq!(first, format_increment_result(1, b"a"));
    assert_eq!(second, format_increment_result(2, b"b"));
    assert_eq!(fixture.committed_count().expect("counter must exist"), 2);
}

#[test]
fn builtin_publish_observation_emits_event_and_metadata_only_control() {
    let fixture = builtin_fixture().expect("builtin fixture must boot");
    let result = fixture
        .call(OPERATION_PUBLISH_OBSERVATION, b"note", "builtin-obs-1")
        .expect("authorized observation publish must succeed");
    assert_eq!(result, format_observation_result(b"note"));

    let observation = fixture.next_observation().expect("observation must arrive");
    assert_eq!(observation.payload(), b"note");

    let control = fixture.next_control().expect("control must arrive");
    assert!(control.payload().is_empty());
    assert!(control.invocation_completed().is_some());
}

#[test]
fn builtin_denies_invocation_without_authority() {
    let fixture = builtin_fixture().expect("builtin fixture must boot");
    let subject = fixture
        .register_unauthorized_subject("echo-stranger")
        .expect("subject registration must succeed");
    let outcome = fixture
        .call_unauthorized(&subject, OPERATION_ECHO, b"x")
        .expect("handle acquisition must succeed for a registered principal");
    assert!(
        matches!(outcome, Err(CapabilityError::Denied { .. })),
        "unauthorized echo must be denied by the broker, got {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// Native-process execution mode: the SAME unchanged consumer drives the
// SAME logical capability through the IPC adapter.
// ---------------------------------------------------------------------------

mod support;

use worldline_reference_external::{NativeEchoOptions, NativeEchoPlugin, WasmEchoPlugin};

fn native_fixture() -> Result<EchoFixture, String> {
    let options = NativeEchoOptions::new(support::native_provider_program());
    EchoFixture::boot(NativeEchoPlugin::new("reference.echo.native", options))
}

fn wasm_fixture() -> Result<EchoFixture, String> {
    EchoFixture::boot(WasmEchoPlugin::from_component(
        "reference.echo.wasm",
        support::benign_echo_component().clone(),
    ))
}

#[test]
fn native_authorized_echo_returns_the_shared_result() {
    let fixture = native_fixture().expect("native fixture must boot");
    let result = fixture
        .call(OPERATION_ECHO, b"hello", "native-echo-1")
        .expect("authorized echo must succeed over IPC");
    assert_eq!(result, format_echo_result(b"hello"));
}

#[test]
fn native_stateful_increment_persists_through_installation_state() {
    let fixture = native_fixture().expect("native fixture must boot");
    let first = fixture
        .call(OPERATION_STATEFUL_INCREMENT, b"a", "native-inc-1")
        .expect("first increment must succeed over IPC");
    let second = fixture
        .call(OPERATION_STATEFUL_INCREMENT, b"b", "native-inc-2")
        .expect("second increment must succeed over IPC");
    assert_eq!(first, format_increment_result(1, b"a"));
    assert_eq!(second, format_increment_result(2, b"b"));
    assert_eq!(fixture.committed_count().expect("counter must exist"), 2);
}

#[test]
fn native_publish_observation_emits_host_stamped_event() {
    let fixture = native_fixture().expect("native fixture must boot");
    let result = fixture
        .call(OPERATION_PUBLISH_OBSERVATION, b"note", "native-obs-1")
        .expect("authorized observation publish must succeed over IPC");
    assert_eq!(result, format_observation_result(b"note"));
    let observation = fixture.next_observation().expect("observation must arrive");
    assert_eq!(observation.payload(), b"note");
    let control = fixture.next_control().expect("control must arrive");
    assert!(control.payload().is_empty());
    assert!(control.invocation_completed().is_some());
}

#[test]
fn native_denies_invocation_without_authority() {
    let fixture = native_fixture().expect("native fixture must boot");
    let subject = fixture
        .register_unauthorized_subject("native-stranger")
        .expect("subject registration must succeed");
    let outcome = fixture
        .call_unauthorized(&subject, OPERATION_ECHO, b"x")
        .expect("handle acquisition must succeed for a registered principal");
    assert!(
        matches!(outcome, Err(CapabilityError::Denied { .. })),
        "unauthorized echo must be denied by the broker, got {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// WASM Component execution mode: identical consumer, sandboxed provider.
// ---------------------------------------------------------------------------

#[test]
fn wasm_authorized_echo_returns_the_shared_result() {
    let fixture = wasm_fixture().expect("wasm fixture must boot");
    let result = fixture
        .call(OPERATION_ECHO, b"hello", "wasm-echo-1")
        .expect("authorized echo must succeed in the sandbox");
    assert_eq!(result, format_echo_result(b"hello"));
}

#[test]
fn wasm_stateful_increment_persists_through_installation_state() {
    let fixture = wasm_fixture().expect("wasm fixture must boot");
    let first = fixture
        .call(OPERATION_STATEFUL_INCREMENT, b"a", "wasm-inc-1")
        .expect("first increment must succeed in the sandbox");
    let second = fixture
        .call(OPERATION_STATEFUL_INCREMENT, b"b", "wasm-inc-2")
        .expect("second increment must succeed in the sandbox");
    assert_eq!(first, format_increment_result(1, b"a"));
    assert_eq!(second, format_increment_result(2, b"b"));
    assert_eq!(fixture.committed_count().expect("counter must exist"), 2);
}

#[test]
fn wasm_publish_observation_emits_host_stamped_event() {
    let fixture = wasm_fixture().expect("wasm fixture must boot");
    let result = fixture
        .call(OPERATION_PUBLISH_OBSERVATION, b"note", "wasm-obs-1")
        .expect("authorized observation publish must succeed in the sandbox");
    assert_eq!(result, format_observation_result(b"note"));
    let observation = fixture.next_observation().expect("observation must arrive");
    assert_eq!(observation.payload(), b"note");
    let control = fixture.next_control().expect("control must arrive");
    assert!(control.payload().is_empty());
    assert!(control.invocation_completed().is_some());
}

#[test]
fn wasm_denies_invocation_without_authority() {
    let fixture = wasm_fixture().expect("wasm fixture must boot");
    let subject = fixture
        .register_unauthorized_subject("wasm-stranger")
        .expect("subject registration must succeed");
    let outcome = fixture
        .call_unauthorized(&subject, OPERATION_ECHO, b"x")
        .expect("handle acquisition must succeed for a registered principal");
    assert!(
        matches!(outcome, Err(CapabilityError::Denied { .. })),
        "unauthorized echo must be denied by the broker, got {outcome:?}"
    );
}
