use std::sync::{Arc, Mutex};

use worldline_kernel::{
    CapabilityError, GrantLifetime, Kernel, PrincipalId, ResourceScope, RuntimeState,
};
use worldline_reference::{
    ObservationBus, ObservationDraft,
    agent_like::{AgentLikePlugin, reason_capability},
    browser_like::{
        BrowserLikeConsumer, BrowserLikeProvider, capability_from_slot, navigate_capability,
    },
    s0, s1,
    ui_like::{UiLikeConsumer, UiLikeProvider, surface_capability},
};

fn grant_for(
    kernel: &Kernel,
    principal: PrincipalId,
    capability: &worldline_kernel::CapabilityId,
    operation: &str,
) {
    kernel
        .create_root_grant(
            principal,
            capability.contract(),
            [operation],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .expect("reference grant must be accepted");
}

#[test]
fn three_reference_families_use_one_generic_plugin_contract() {
    let bus = ObservationBus::new();
    let mut kernel = Kernel::new();

    let browser = kernel
        .register(BrowserLikeProvider::new(
            "reference.browser.provider",
            bus.clone(),
            "browser",
        ))
        .expect("browser-like provider must register");
    let (agent, agent_browser_slot) = AgentLikePlugin::new(
        "reference.agent.provider",
        navigate_capability(),
        bus.clone(),
        "agent",
    );
    let agent = kernel
        .register(agent)
        .expect("agent-like provider must register");
    let (ui, _ui_agent_slot) = UiLikeProvider::new(
        "reference.ui.provider",
        reason_capability(),
        bus.clone(),
        "ui",
    );
    let ui = kernel.register(ui).expect("UI-like provider must register");
    let (browser_consumer, browser_slot) = BrowserLikeConsumer::new("reference.browser.consumer");
    let browser_consumer = kernel
        .register(browser_consumer)
        .expect("browser-like consumer must register");
    let (ui_consumer, ui_slot) = UiLikeConsumer::new("reference.ui.consumer");
    let ui_consumer = kernel
        .register(ui_consumer)
        .expect("UI-like consumer must register");

    for plugin in [&browser, &agent, &ui, &browser_consumer, &ui_consumer] {
        assert_eq!(kernel.plugin_state(plugin), Some(RuntimeState::Active));
    }

    let agent_handle = capability_from_slot(&agent_browser_slot)
        .expect("agent must receive a generic browser capability handle");
    let agent_principal = kernel
        .principal_for_plugin(&agent)
        .expect("agent runtime must exist");
    assert!(matches!(
        agent_handle.invoke("navigate", b"before-grant"),
        Err(CapabilityError::Denied {
            reason: worldline_kernel::DenialReason::NoGrant,
            ..
        })
    ));
    grant_for(&kernel, agent_principal, &navigate_capability(), "navigate");
    assert_eq!(
        agent_handle
            .invoke("navigate", b"agent-request")
            .expect("explicitly granted agent request must succeed"),
        b"browser:agent-request"
    );

    let browser_handle = capability_from_slot(&browser_slot)
        .expect("browser consumer must receive a generic capability handle");
    let browser_principal = kernel
        .principal_for_plugin(&browser_consumer)
        .expect("browser consumer runtime must exist");
    grant_for(
        &kernel,
        browser_principal,
        &navigate_capability(),
        "navigate",
    );
    assert_eq!(
        browser_handle
            .invoke("navigate", b"browser-request")
            .expect("explicitly granted browser request must succeed"),
        b"browser:browser-request"
    );

    let ui_handle = ui_slot
        .lock()
        .expect("UI slot lock is not poisoned")
        .clone()
        .expect("UI consumer must receive a generic capability handle");
    let ui_principal = kernel
        .principal_for_plugin(&ui_consumer)
        .expect("UI consumer runtime must exist");
    grant_for(&kernel, ui_principal, &surface_capability(), "render");
    assert_eq!(
        ui_handle
            .invoke("render", b"opaque-surface")
            .expect("explicitly granted UI request must succeed"),
        b"ui:opaque-surface"
    );

    assert!(kernel.is_capability_available(&navigate_capability()));
    assert!(kernel.is_capability_available(&surface_capability()));
    assert_eq!(bus.history().len(), 3);
}

#[test]
fn observation_is_independent_from_rpc_and_subscriber_failures() {
    let bus = ObservationBus::new();
    let successful_observations = Arc::new(Mutex::new(Vec::new()));
    let successful_observations_clone = Arc::clone(&successful_observations);
    bus.subscribe(move |observation| {
        successful_observations_clone
            .lock()
            .expect("subscriber log lock is not poisoned")
            .push(observation.id());
        Ok(())
    });
    bus.subscribe(|_| Err("deliberate subscriber failure".to_owned()));

    let mut kernel = Kernel::new();
    let provider = kernel
        .register(BrowserLikeProvider::new(
            "observation-provider",
            bus.clone(),
            "rpc",
        ))
        .expect("provider must register");
    let (consumer, slot) = BrowserLikeConsumer::new("observation-consumer");
    let consumer = kernel.register(consumer).expect("consumer must register");
    let handle = capability_from_slot(&slot).expect("consumer handle must exist");
    let principal = kernel
        .principal_for_plugin(&consumer)
        .expect("consumer runtime must exist");

    assert!(handle.is_available());
    assert!(matches!(
        handle.invoke("navigate", b"without-authority"),
        Err(CapabilityError::Denied {
            reason: worldline_kernel::DenialReason::NoGrant,
            ..
        })
    ));
    grant_for(&kernel, principal, &navigate_capability(), "navigate");
    assert_eq!(
        handle
            .invoke("navigate", b"rpc-result")
            .expect("provider RPC result must not depend on subscribers"),
        b"rpc:rpc-result"
    );

    let successful_observations = successful_observations
        .lock()
        .expect("subscriber log lock is not poisoned")
        .clone();
    assert_eq!(successful_observations.len(), 1);
    let history = bus.history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id().value(), 1);
    assert_eq!(
        history[0].producer(),
        &kernel.principal_for_plugin(&provider).unwrap()
    );
    assert_eq!(history[0].topic(), "reference.browser.navigation");
    assert!(history[0].causation().is_some());

    let delivery = bus.publish(ObservationDraft::new(
        kernel.system_principal(),
        "reference.test.fact",
        b"opaque",
    ));
    assert_eq!(delivery.delivered(), 1);
    assert_eq!(delivery.failures().len(), 1);
    assert_eq!(bus.history().len(), 2);
}

#[test]
fn observation_without_subscribers_still_leaves_rpc_semantics_unchanged() {
    let bus = ObservationBus::new();
    let mut kernel = Kernel::new();
    let provider = kernel
        .register(BrowserLikeProvider::new("no-observer-provider", bus, "rpc"))
        .expect("provider must register");
    let (consumer, slot) = BrowserLikeConsumer::new("no-observer-consumer");
    let consumer = kernel.register(consumer).expect("consumer must register");
    let principal = kernel
        .principal_for_plugin(&consumer)
        .expect("consumer runtime must exist");
    grant_for(&kernel, principal, &navigate_capability(), "navigate");
    assert_eq!(
        capability_from_slot(&slot)
            .expect("consumer handle must exist")
            .invoke("navigate", b"no-subscriber")
            .expect("RPC must succeed with zero subscribers"),
        b"rpc:no-subscriber"
    );
    assert!(kernel.is_capability_available(&navigate_capability()));
    assert_eq!(kernel.plugin_state(&provider), Some(RuntimeState::Active));
}

#[test]
fn s0_proves_state_continuity_and_runtime_authority_discontinuity() {
    let report = s0::run().expect("S0 proving slice must pass");
    assert_ne!(report.old_runtime, report.new_runtime);
    assert_eq!(report.state_before_restart, "1");
    assert_eq!(report.state_after_restart, "2");
    assert_eq!(report.first_result, "provider-a:https://worldline.example");
    assert_eq!(
        report.restarted_result,
        "provider-b:https://worldline.example/restarted"
    );
    assert_eq!(report.observations, 2);
    assert!(report.old_runtime_grant_revoked);
    assert!(report.new_runtime_did_not_inherit_authority);
}

#[test]
fn s1_proves_rpc_event_and_restart_composition() {
    let report = s1::run().expect("S1 proving slice must pass");
    assert_ne!(report.old_runtime_id, report.new_runtime_id);
    assert_eq!(report.state_before_restart, "1");
    assert_eq!(report.state_after_restart, "2");
    assert_eq!(report.first_result, "stateful:1:first");
    assert_eq!(report.restarted_result, "stateful:2:restarted");
    assert_eq!(report.follow_up_result, "follow-up:observer-action");
    assert_eq!(report.observed_events, 2);
    assert!(report.control_observation_was_metadata_only);
    assert!(report.old_runtime_authority_revoked);
    assert!(report.new_runtime_required_explicit_authority);
}

#[test]
fn kernel_does_not_define_reference_family_domain_types() {
    let kernel_directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../worldline-kernel/src");
    let mut source = String::new();
    for entry in std::fs::read_dir(kernel_directory).expect("kernel source directory must exist") {
        let path = entry.expect("kernel source entry must be readable").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            source.push_str(
                &std::fs::read_to_string(path).expect("kernel source file must be readable"),
            );
        }
    }
    for forbidden in [
        "struct Tab",
        "struct BrowserTab",
        "struct Page",
        "struct Document",
        "struct DOMNode",
        "struct BrowserContext",
        "struct Download",
        "struct HistoryEntry",
        "struct Agent",
        "struct Prompt",
        "struct Model",
        "struct Conversation",
        "struct Workspace",
        "struct Activity",
        "struct Message",
        "struct Panel",
        "struct Sidebar",
        "struct TabBar",
    ] {
        assert!(!source.contains(forbidden), "kernel contains {forbidden}");
    }
}

#[test]
fn observation_event_has_its_own_identity_and_producer() {
    let bus = ObservationBus::new();
    bus.subscribe(|_| Ok(()));
    let empty_kernel = Kernel::new();
    assert!(!empty_kernel.is_capability_available(&navigate_capability()));

    let delivery = bus.publish(ObservationDraft::new(
        PrincipalId::new("reference-provider"),
        "reference.test.fact",
        b"result",
    ));
    let observation = bus.history().pop().expect("published event must exist");
    assert_eq!(delivery.delivered(), 1);
    assert_eq!(observation.id().value(), 1);
    assert_eq!(observation.producer().as_str(), "reference-provider");
    assert_eq!(observation.payload(), b"result");
}
