//! Acceptance tests for multi-installation search provider composition,
//! generic targeting, authority separation, and query privacy.

use worldline_browser_search::{SearchProviderConfig, SearchProviderPlugin, search_capability};
use worldline_browser_services_contract::{SearchNavigationTarget, SearchResolveRequest};
use worldline_kernel::{
    CapabilityTarget, GrantLifetime, InstallationId, Kernel, PrincipalKind, ResourceScope,
    StateSchemaVersion,
};

fn install(kernel: &mut Kernel, name: &str) -> InstallationId {
    kernel
        .create_installation(name, StateSchemaVersion::default())
        .unwrap_or_else(|err| panic!("install {name} failed: {err:?}"))
}

#[test]
fn multi_installation_search_coexistence_and_generic_targeting() {
    let mut kernel = Kernel::new();
    let def_id = "worldline-browser-search";
    let inst_a = install(&mut kernel, def_id);
    let inst_b = install(&mut kernel, def_id);

    let config_a = SearchProviderConfig::new("Alpha", "http://127.0.0.1:8081/search-a/", "q")
        .with_static_parameter("engine", "alpha")
        .with_loopback_http(true);
    let config_b = SearchProviderConfig::new("Beta", "http://127.0.0.1:8082/search-b/", "term")
        .with_static_parameter("engine", "beta")
        .with_loopback_http(true);

    let plugin_a = SearchProviderPlugin::new(def_id, config_a);
    let plugin_b = SearchProviderPlugin::new(def_id, config_b);

    kernel
        .register_for_installation(plugin_a, &inst_a)
        .expect("inst_a registration must succeed");
    kernel
        .register_for_installation(plugin_b, &inst_b)
        .expect("inst_b registration must succeed");

    let caller = kernel
        .register_principal_id("search-consumer", PrincipalKind::User)
        .expect("caller principal must register");

    let search_cap = search_capability();

    // Grant caller search resolution authority
    kernel
        .create_root_grant(
            caller.clone(),
            search_cap.contract(),
            ["resolve"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .expect("grant must succeed");

    // Case 1: Untargeted handle defaults to lowest InstallationId (inst_a)
    let untargeted_handle = kernel
        .capability_for(caller.clone(), search_cap.clone())
        .expect("untargeted handle");
    assert_eq!(untargeted_handle.target(), &CapabilityTarget::AnyCompatible);

    let req = SearchResolveRequest::new("rust kernel").unwrap();
    let payload = serde_json::to_vec(&req).unwrap();

    let resp_bytes = untargeted_handle
        .invoke("resolve", &payload)
        .expect("default invoke succeeds");
    let target: SearchNavigationTarget = serde_json::from_slice(&resp_bytes).unwrap();
    assert_eq!(target.query_parameter_name(), "q");
    assert!(
        target
            .url()
            .contains("8081/search-a/?engine=alpha&q=rust+kernel")
    );

    // Case 2: Explicit targeting to inst_a
    let handle_a = kernel
        .capability_for_installation(caller.clone(), search_cap.clone(), &inst_a)
        .expect("handle_a");
    assert_eq!(
        handle_a.target(),
        &CapabilityTarget::Installation(inst_a.clone())
    );

    let resp_a = handle_a
        .invoke("resolve", &payload)
        .expect("handle_a invoke succeeds");
    let target_a: SearchNavigationTarget = serde_json::from_slice(&resp_a).unwrap();
    assert_eq!(target_a.query_parameter_name(), "q");
    assert!(
        target_a
            .url()
            .contains("8081/search-a/?engine=alpha&q=rust+kernel")
    );

    // Case 3: Explicit targeting to inst_b (not shadowed!)
    let handle_b = kernel
        .capability_for_installation(caller.clone(), search_cap.clone(), &inst_b)
        .expect("handle_b");
    assert_eq!(
        handle_b.target(),
        &CapabilityTarget::Installation(inst_b.clone())
    );

    let resp_b = handle_b
        .invoke("resolve", &payload)
        .expect("handle_b invoke succeeds");
    let target_b: SearchNavigationTarget = serde_json::from_slice(&resp_b).unwrap();
    assert_eq!(target_b.query_parameter_name(), "term");
    assert!(
        target_b
            .url()
            .contains("8082/search-b/?engine=beta&term=rust+kernel")
    );

    // Case 4: Target unknown installation -> fails closed without fallback
    let unknown_inst = InstallationId::new("search-unknown");
    let handle_unknown = kernel
        .capability_for_installation(caller, search_cap, &unknown_inst)
        .expect("handle_unknown");
    assert!(!handle_unknown.is_available());
    let err = handle_unknown
        .invoke("resolve", &payload)
        .expect_err("unknown target must fail");
    assert!(
        matches!(
            err,
            worldline_kernel::CapabilityError::TargetUnavailable { .. }
        ),
        "expected TargetUnavailable, got {err:?}"
    );
}

#[test]
fn search_authority_separation_from_browser_navigation() {
    let mut kernel = Kernel::new();
    let def_id = "worldline-browser-search";
    let inst = install(&mut kernel, def_id);
    let config = SearchProviderConfig::new("Search", "http://127.0.0.1:8080/s", "q")
        .with_loopback_http(true);
    let plugin = SearchProviderPlugin::new(def_id, config);

    kernel.register_for_installation(plugin, &inst).unwrap();

    let search_cap = search_capability();
    let caller = kernel
        .register_principal_id("search-only-caller", PrincipalKind::User)
        .unwrap();

    // Grant ONLY search resolve authority
    kernel
        .create_root_grant(
            caller.clone(),
            search_cap.contract(),
            ["resolve"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .unwrap();

    // 1. Search resolution succeeds
    let search_handle = kernel
        .capability_for_installation(caller.clone(), search_cap.clone(), &inst)
        .unwrap();
    let req = SearchResolveRequest::new("privacy query").unwrap();
    let payload = serde_json::to_vec(&req).unwrap();
    let res = search_handle.invoke("resolve", &payload);
    assert!(res.is_ok(), "search resolve must succeed");

    // 2. Attempting browser navigation through a hypothetical browser capability fails closed
    let nav_cap = worldline_kernel::CapabilityId::new(
        "browser.navigate",
        "navigate",
        worldline_kernel::InterfaceVersion::new(1, 0),
    );
    let nav_handle = kernel.capability_for(caller, nav_cap);
    // Either the handle cannot be created or invocation is denied because caller lacks NavigatePage authority
    if let Ok(handle) = nav_handle {
        let nav_res = handle.invoke("navigate", b"{}");
        assert!(
            nav_res.is_err(),
            "search-only caller must not be authorized to navigate!"
        );
    }
}

#[test]
fn search_privacy_invariants_in_trajectory() {
    let mut kernel = Kernel::new();
    let def_id = "worldline-browser-search";
    let inst = install(&mut kernel, def_id);
    let config = SearchProviderConfig::new("PrivacySearch", "http://127.0.0.1:8080/search", "q")
        .with_loopback_http(true);
    let plugin = SearchProviderPlugin::new(def_id, config);

    kernel.register_for_installation(plugin, &inst).unwrap();

    let search_cap = search_capability();
    let caller = kernel
        .register_principal_id("privacy-caller", PrincipalKind::User)
        .unwrap();

    kernel
        .create_root_grant(
            caller.clone(),
            search_cap.contract(),
            ["resolve"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .unwrap();

    let handle = kernel
        .capability_for_installation(caller, search_cap, &inst)
        .unwrap();

    let sensitive_query = "confidential-personal-health-data-987654";
    let req = SearchResolveRequest::new(sensitive_query).unwrap();
    let payload = serde_json::to_vec(&req).unwrap();

    let res = handle.invoke("resolve", &payload).unwrap();
    let target: SearchNavigationTarget = serde_json::from_slice(&res).unwrap();
    assert!(target.url().contains(sensitive_query));

    // Verify all emitted trajectory events
    for event in kernel.trajectory() {
        let event_debug = format!("{event:?}");
        assert!(
            !event_debug.contains(sensitive_query),
            "Trajectory leaked raw query text! Event: {event_debug}"
        );
    }
}

#[test]
fn search_lifecycle_isolation_and_degradation() {
    let mut kernel = Kernel::new();
    let def_id = "worldline-browser-search";
    let inst_a = install(&mut kernel, def_id);
    let inst_b = install(&mut kernel, def_id);

    let config_a = SearchProviderConfig::new("SearchA", "http://127.0.0.1:8081/s", "q")
        .with_loopback_http(true);
    let config_b = SearchProviderConfig::new("SearchB", "http://127.0.0.1:8082/s", "q")
        .with_loopback_http(true);

    let plugin_a = SearchProviderPlugin::new(def_id, config_a);
    let plugin_b = SearchProviderPlugin::new(def_id, config_b);

    kernel.register_for_installation(plugin_a, &inst_a).unwrap();
    kernel.register_for_installation(plugin_b, &inst_b).unwrap();

    let caller = kernel
        .register_principal_id("caller", PrincipalKind::User)
        .unwrap();
    let search_cap = search_capability();

    kernel
        .create_root_grant(
            caller.clone(),
            search_cap.contract(),
            ["resolve"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .unwrap();

    let req = SearchResolveRequest::new("query").unwrap();
    let payload = serde_json::to_vec(&req).unwrap();

    // Verify both are working initially
    let handle_a = kernel
        .capability_for_installation(caller.clone(), search_cap.clone(), &inst_a)
        .unwrap();
    let handle_b = kernel
        .capability_for_installation(caller.clone(), search_cap.clone(), &inst_b)
        .unwrap();
    assert!(handle_a.invoke("resolve", &payload).is_ok());
    assert!(handle_b.invoke("resolve", &payload).is_ok());

    // Uninstall inst_a
    kernel.uninstall(&inst_a).expect("uninstall inst_a");

    // inst_a invocation fails closed with TargetUnavailable
    let err_a = handle_a
        .invoke("resolve", &payload)
        .expect_err("uninstalled provider must fail");
    assert!(
        matches!(
            err_a,
            worldline_kernel::CapabilityError::TargetUnavailable { .. }
        ),
        "expected TargetUnavailable, got {err_a:?}"
    );

    // inst_b remains completely functional!
    let res_b = handle_b
        .invoke("resolve", &payload)
        .expect("inst_b must remain functional after inst_a removal");
    let target_b: SearchNavigationTarget = serde_json::from_slice(&res_b).unwrap();
    assert!(target_b.url().contains("8082"));
}
