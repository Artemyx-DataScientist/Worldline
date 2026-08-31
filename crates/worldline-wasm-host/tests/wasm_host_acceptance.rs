//! M0.6 WASM execution mode acceptance: least-authority linking, typed
//! resource limits, payload gates, and trap isolation, proven against real
//! compiled components (see `tests/components/`).

mod build_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use worldline_wasm_host::{WasmHostBroker, WasmHostError, WasmPluginHost, WasmResourceLimits};

/// Broker recording everything a component touches, backed by simple maps.
#[derive(Default)]
struct RecordingBroker {
    state: Mutex<BTreeMap<String, Vec<u8>>>,
    publications: Mutex<Vec<(String, String, Vec<u8>)>>,
}

impl WasmHostBroker for RecordingBroker {
    fn state_get(&self, key: String) -> Option<Vec<u8>> {
        self.state.lock().expect("state lock").get(&key).cloned()
    }

    fn state_set(&self, key: String, value: Vec<u8>) {
        self.state.lock().expect("state lock").insert(key, value);
    }

    fn event_publish(
        &self,
        namespace: String,
        name: String,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        self.publications
            .lock()
            .expect("publications lock")
            .push((namespace, name, payload));
        Ok(())
    }
}

fn broker() -> Arc<dyn WasmHostBroker> {
    Arc::new(RecordingBroker::default())
}

#[test]
fn benign_component_implements_the_echo_semantics() {
    let host = WasmPluginHost::new();
    let component = host
        .load_component(&build_support::components().benign_echo)
        .expect("benign component must load");
    let mut instance = host
        .make_instance(&component, broker())
        .expect("benign component must instantiate");

    let echoed = instance
        .invoke("echo", b"hello".to_vec())
        .expect("echo must succeed");
    assert_eq!(echoed, b"echo:hello");

    let first = instance
        .invoke("stateful_increment", b"abc".to_vec())
        .expect("first increment must succeed");
    let second = instance
        .invoke("stateful_increment", b"abc".to_vec())
        .expect("second increment must succeed");
    assert_eq!(first, b"incremented:1:abc");
    assert_eq!(second, b"incremented:2:abc");

    let observed = instance
        .invoke("publish_observation", b"note".to_vec())
        .expect("observation publish must succeed");
    assert_eq!(observed, b"observed:note");

    let unknown = instance
        .invoke("nonexistent", b"x".to_vec())
        .expect_err("unknown operation must be a guest error");
    assert!(matches!(unknown, WasmHostError::GuestReturnedError { .. }));
}

#[test]
fn memory_demand_beyond_the_limit_is_typed_and_contained() {
    let host = WasmPluginHost::new();
    let component = host
        .load_component(&build_support::components().memory_hog)
        .expect("memory hog must load");
    let mut instance = host
        .make_instance(&component, broker())
        .expect("memory hog must instantiate");

    let error = instance
        .invoke("echo", b"x".to_vec())
        .expect_err("memory demand must be denied");
    assert!(
        matches!(
            &error,
            WasmHostError::WasmResourceLimitExceeded { dimension }
                if dimension == "memory"
        ),
        "expected a typed memory limit failure, got {error:?}"
    );

    // Exhaustion must not consume the host: a fresh hog instance is still
    // constructible and independently enforceable, and a benign component
    // keeps working.
    let mut fresh = host
        .make_instance(&component, broker())
        .expect("host must stay usable after limit exhaustion");
    let repeated = fresh
        .invoke("echo", b"x".to_vec())
        .expect_err("a fresh hog must be limited identically");
    assert!(matches!(
        repeated,
        WasmHostError::WasmResourceLimitExceeded { .. }
    ));

    let benign = host
        .load_component(&build_support::components().benign_echo)
        .expect("benign component must still load");
    let mut working = host
        .make_instance(&benign, broker())
        .expect("benign instance must work after limit exhaustion");
    assert_eq!(
        working.invoke("echo", b"ok".to_vec()).expect("echo"),
        b"echo:ok"
    );
}

#[test]
fn infinite_loop_terminates_within_the_fuel_budget() {
    let limits = WasmResourceLimits {
        fuel_budget_per_call: 10_000_000,
        ..WasmResourceLimits::default()
    };
    let host = WasmPluginHost::with_limits(limits);
    let component = host
        .load_component(&build_support::components().spin_loop)
        .expect("spin loop must load");
    let mut instance = host
        .make_instance(&component, broker())
        .expect("spin loop must instantiate");

    let error = instance
        .invoke("echo", b"x".to_vec())
        .expect_err("infinite loop must be terminated");
    assert!(
        matches!(
            &error,
            WasmHostError::WasmResourceLimitExceeded { dimension }
                if dimension == "fuel"
        ),
        "expected a typed fuel failure, got {error:?}"
    );
}

#[test]
fn guest_trap_is_isolated_to_its_own_instance() {
    let host = WasmPluginHost::new();
    let trapper = host
        .load_component(&build_support::components().trapper)
        .expect("trapper must load");
    let benign = host
        .load_component(&build_support::components().benign_echo)
        .expect("benign component must load");

    let mut trapped = host
        .make_instance(&trapper, broker())
        .expect("trapper must instantiate");
    let error = trapped
        .invoke("echo", b"x".to_vec())
        .expect_err("the trap must surface as a failure");
    assert!(matches!(error, WasmHostError::WasmTrap { .. }));

    // The trap poisoned only the trapper's store: an independent benign
    // instance on the same host still works.
    let mut unaffected = host
        .make_instance(&benign, broker())
        .expect("benign instance must instantiate after a trap elsewhere");
    assert_eq!(
        unaffected.invoke("echo", b"alive".to_vec()).expect("echo"),
        b"echo:alive"
    );
}

#[test]
fn component_demanding_unprovided_imports_fails_closed() {
    let host = WasmPluginHost::new();
    let component = host
        .load_component(&build_support::components().foreign_importer)
        .expect("the foreign importer must compile as a component");

    let error = host
        .make_instance(&component, broker())
        .expect_err("imports the host does not provide must fail closed");
    assert!(
        matches!(error, WasmHostError::UnsupportedExternalAbi { .. }),
        "expected UnsupportedExternalAbi, got {error:?}"
    );
    // The host object remains fully usable.
    let benign = host
        .load_component(&build_support::components().benign_echo)
        .expect("host must stay usable");
    assert!(host.make_instance(&benign, broker()).is_ok());
}

#[test]
fn oversized_host_call_payloads_are_rejected_by_the_gate() {
    let limits = WasmResourceLimits {
        max_host_call_payload_bytes: 64,
        ..WasmResourceLimits::default()
    };
    let error = limits
        .check_host_call_payload(65)
        .expect_err("an oversized payload must be denied");
    assert!(matches!(
        error,
        WasmHostError::ExternalPayloadTooLarge {
            actual: 65,
            limit: 64
        }
    ));
    let oversized = vec![0u8; 10_000_000];
    let error = limits
        .check_host_call_payload(oversized.len())
        .expect_err("a multi-megabyte payload must be denied without allocation blowup");
    assert!(matches!(
        error,
        WasmHostError::ExternalPayloadTooLarge {
            actual: 10_000_000,
            limit: 64
        }
    ));
}

#[test]
fn oversized_component_binaries_never_reach_the_compiler() {
    let limits = WasmResourceLimits {
        max_component_bytes: 16,
        ..WasmResourceLimits::default()
    };
    let host = WasmPluginHost::with_limits(limits);
    let blob = vec![0u8; 4096];
    let error = host
        .load_component(&blob)
        .expect_err("an oversized binary must be rejected before compilation");
    assert!(matches!(
        error,
        WasmHostError::ComponentTooLarge {
            limit: 16,
            actual: 4096
        }
    ));
}
