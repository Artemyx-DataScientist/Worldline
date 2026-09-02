use std::sync::Arc;

use worldline_browser_contract::identity::BrowserContextId;
use worldline_browser_contract::primitives::{
    CookieV0_2, GetCookiesRequest, SetCookieRequestV0_2, StorageType,
};
use worldline_browser_cookies::{CookieEngineBackend, CookiesService, InMemoryCookieEngine};
use worldline_browser_services_contract::{
    ClearSiteDataRequest, DeleteCookieServiceRequest, GetCookieMetadataRequest,
    GetCookieValueRequest, SetCookieServiceRequest,
};

#[test]
fn metadata_inspection_discloses_no_raw_values() {
    let engine = Arc::new(InMemoryCookieEngine::new());
    let service = CookiesService::new(engine);

    let context_id = BrowserContextId::new("ctx-user");
    let secret_token = "sess_secret_xyz123_token";

    // Set cookie
    let set_res = service
        .set_cookie(SetCookieServiceRequest {
            context_id: context_id.clone(),
            name: "auth_session".to_string(),
            value: secret_token.to_string(),
            domain: "worldline.test".to_string(),
            path: Some("/".to_string()),
            secure: Some(true),
            http_only: Some(true),
            same_site: Some("Strict".to_string()),
            expires_epoch_sec: Some(1800000000),
        })
        .expect("Set cookie should succeed");
    assert!(set_res.success);

    // Get metadata
    let meta_res = service
        .get_cookie_metadata(GetCookieMetadataRequest {
            context_id,
            url: None,
            domain: Some("worldline.test".to_string()),
        })
        .expect("Get metadata should succeed");

    assert_eq!(meta_res.cookies.len(), 1);
    let meta = &meta_res.cookies[0];
    assert_eq!(meta.name, "auth_session");
    assert_eq!(meta.domain, "worldline.test");
    assert_eq!(meta.path, "/");
    assert!(meta.secure);
    assert!(meta.http_only);
    assert_eq!(meta.same_site, Some("Strict".to_string()));
    assert_eq!(meta.expires_epoch_sec, Some(1800000000));
    // CookieMetadata struct has no value field by design.
}

#[test]
fn secret_value_disclosure_and_redaction() {
    let engine = Arc::new(InMemoryCookieEngine::new());
    let service = CookiesService::new(engine);

    let context_id = BrowserContextId::new("ctx-secure");
    let secret = "sensitive-api-key-9999";

    service
        .set_cookie(SetCookieServiceRequest {
            context_id: context_id.clone(),
            name: "api_key".to_string(),
            value: secret.to_string(),
            domain: "api.worldline.test".to_string(),
            path: Some("/v1".to_string()),
            secure: Some(true),
            http_only: Some(true),
            same_site: None,
            expires_epoch_sec: None,
        })
        .unwrap();

    let val_res = service
        .get_cookie_value(GetCookieValueRequest {
            context_id,
            domain: "api.worldline.test".to_string(),
            name: "api_key".to_string(),
            path: Some("/v1/data".to_string()),
            url: None,
        })
        .unwrap();

    let cookie_val = val_res.cookie.expect("Cookie value should be found");
    assert_eq!(cookie_val.expose_value(), secret);

    let debug_repr = format!("{:?}", cookie_val);
    assert!(debug_repr.contains("[REDACTED]"));
    assert!(!debug_repr.contains(secret));
}

#[test]
fn policy_metadata_only_mode_blocks_value_disclosure() {
    let engine = Arc::new(InMemoryCookieEngine::new());
    let service = CookiesService::new(engine);

    let context_id = BrowserContextId::new("ctx-restricted");
    let secret = "restricted_secret";

    service
        .set_cookie(SetCookieServiceRequest {
            context_id: context_id.clone(),
            name: "token".to_string(),
            value: secret.to_string(),
            domain: "worldline.test".to_string(),
            path: None,
            secure: None,
            http_only: None,
            same_site: None,
            expires_epoch_sec: None,
        })
        .unwrap();

    // Enable metadata-only policy
    service.set_metadata_only(context_id.clone(), true);

    // Metadata is still accessible
    let meta_res = service
        .get_cookie_metadata(GetCookieMetadataRequest {
            context_id: context_id.clone(),
            url: None,
            domain: Some("worldline.test".to_string()),
        })
        .unwrap();
    assert_eq!(meta_res.cookies.len(), 1);

    // Value disclosure is blocked by policy
    let val_res = service.get_cookie_value(GetCookieValueRequest {
        context_id,
        domain: "worldline.test".to_string(),
        name: "token".to_string(),
        path: None,
        url: None,
    });
    assert!(val_res.is_err(), "Policy must deny value disclosure");
}

#[test]
fn cross_context_cookie_isolation() {
    let engine = Arc::new(InMemoryCookieEngine::new());
    let service = CookiesService::new(engine);

    let context_a = BrowserContextId::new("ctx-A");
    let context_b = BrowserContextId::new("ctx-B");

    // Both contexts set a cookie for same domain and name with different values
    service
        .set_cookie(SetCookieServiceRequest {
            context_id: context_a.clone(),
            name: "session".to_string(),
            value: "token_for_context_A".to_string(),
            domain: "127.0.0.1".to_string(),
            path: Some("/".to_string()),
            secure: None,
            http_only: None,
            same_site: None,
            expires_epoch_sec: None,
        })
        .unwrap();

    service
        .set_cookie(SetCookieServiceRequest {
            context_id: context_b.clone(),
            name: "session".to_string(),
            value: "token_for_context_B".to_string(),
            domain: "127.0.0.1".to_string(),
            path: Some("/".to_string()),
            secure: None,
            http_only: None,
            same_site: None,
            expires_epoch_sec: None,
        })
        .unwrap();

    // Check Context A
    let val_a = service
        .get_cookie_value(GetCookieValueRequest {
            context_id: context_a.clone(),
            domain: "127.0.0.1".to_string(),
            name: "session".to_string(),
            path: None,
            url: None,
        })
        .unwrap()
        .cookie
        .unwrap();
    assert_eq!(val_a.expose_value(), "token_for_context_A");

    // Check Context B
    let val_b = service
        .get_cookie_value(GetCookieValueRequest {
            context_id: context_b.clone(),
            domain: "127.0.0.1".to_string(),
            name: "session".to_string(),
            path: None,
            url: None,
        })
        .unwrap()
        .cookie
        .unwrap();
    assert_eq!(val_b.expose_value(), "token_for_context_B");

    // Delete in Context A
    let del_res = service
        .delete_cookie(DeleteCookieServiceRequest {
            context_id: context_a.clone(),
            domain: "127.0.0.1".to_string(),
            name: "session".to_string(),
            path: None,
            url: None,
        })
        .unwrap();
    assert_eq!(del_res.deleted_count, 1);

    // Context A cookie is gone
    let meta_a = service
        .get_cookie_metadata(GetCookieMetadataRequest {
            context_id: context_a,
            url: None,
            domain: Some("127.0.0.1".to_string()),
        })
        .unwrap();
    assert_eq!(meta_a.cookies.len(), 0);

    // Context B cookie is unaffected
    let meta_b = service
        .get_cookie_metadata(GetCookieMetadataRequest {
            context_id: context_b,
            url: None,
            domain: Some("127.0.0.1".to_string()),
        })
        .unwrap();
    assert_eq!(meta_b.cookies.len(), 1);
}

#[test]
fn site_data_origin_scoped_clear_isolation() {
    let engine = Arc::new(InMemoryCookieEngine::new());
    let service = CookiesService::new(engine.clone());

    let context_a = BrowserContextId::new("ctx-A");
    let context_b = BrowserContextId::new("ctx-B");
    let origin = "http://127.0.0.1:8080";

    // Write localStorage items in both contexts
    engine.insert_storage_item(
        &context_a,
        origin,
        StorageType::LocalStorage,
        "pref".to_string(),
        "dark".to_string(),
    );
    engine.insert_storage_item(
        &context_b,
        origin,
        StorageType::LocalStorage,
        "pref".to_string(),
        "light".to_string(),
    );

    // Clear Context A storage
    let clear_res = service
        .clear_site_data(ClearSiteDataRequest {
            context_id: context_a.clone(),
            origin: origin.to_string(),
            storage_type: StorageType::LocalStorage,
        })
        .unwrap();
    assert!(clear_res.cleared);

    // Context A is cleared
    assert_eq!(
        engine.get_storage_item(&context_a, origin, StorageType::LocalStorage, "pref"),
        None
    );

    // Context B remains intact
    assert_eq!(
        engine.get_storage_item(&context_b, origin, StorageType::LocalStorage, "pref"),
        Some("light".to_string())
    );
}

#[test]
fn restart_recovery_preserves_engine_as_source_of_truth() {
    let engine = Arc::new(InMemoryCookieEngine::new());
    let service = CookiesService::new(engine.clone());

    let context_id = BrowserContextId::new("ctx-persist");
    service
        .set_cookie(SetCookieServiceRequest {
            context_id: context_id.clone(),
            name: "remember_me".to_string(),
            value: "true_12345".to_string(),
            domain: "worldline.test".to_string(),
            path: None,
            secure: None,
            http_only: None,
            same_site: None,
            expires_epoch_sec: None,
        })
        .unwrap();

    // Export service policy
    let policy = service.export_policy();

    // Simulate service restart: new CookiesService instance wrapping same engine backend
    let restarted_service = CookiesService::from_policy(policy, engine);

    // Cookie values are retrieved from the engine profile store without duplicate DB
    let val_res = restarted_service
        .get_cookie_value(GetCookieValueRequest {
            context_id,
            domain: "worldline.test".to_string(),
            name: "remember_me".to_string(),
            path: None,
            url: None,
        })
        .unwrap();

    assert_eq!(val_res.cookie.unwrap().expose_value(), "true_12345");
}

#[test]
fn cookie_matching_uses_label_boundaries_and_host_only_semantics() {
    let engine = InMemoryCookieEngine::new();
    let context_id = BrowserContextId::new("ctx-domain-boundary");

    engine
        .set_cookie_v0_2(SetCookieRequestV0_2 {
            context_id: context_id.clone(),
            cookie: CookieV0_2 {
                name: "parent".to_string(),
                value: "parent-value".to_string(),
                domain: ".Example.COM".to_string(),
                path: "/".to_string(),
                secure: false,
                http_only: false,
                same_site: None,
                expires_epoch_sec: None,
                host_only: false,
            },
        })
        .unwrap();
    engine
        .set_cookie_v0_2(SetCookieRequestV0_2 {
            context_id: context_id.clone(),
            cookie: CookieV0_2 {
                name: "host".to_string(),
                value: "host-value".to_string(),
                domain: "EXAMPLE.COM".to_string(),
                path: "/".to_string(),
                secure: false,
                http_only: false,
                same_site: None,
                expires_epoch_sec: None,
                host_only: true,
            },
        })
        .unwrap();
    engine
        .set_cookie_v0_2(SetCookieRequestV0_2 {
            context_id: context_id.clone(),
            cookie: CookieV0_2 {
                name: "evil".to_string(),
                value: "evil-value".to_string(),
                domain: "evil-example.com".to_string(),
                path: "/".to_string(),
                secure: false,
                http_only: false,
                same_site: None,
                expires_epoch_sec: None,
                host_only: false,
            },
        })
        .unwrap();

    let subdomain = engine
        .get_cookies_v0_2(GetCookiesRequest {
            context_id: context_id.clone(),
            url: Some("https://sub.Example.COM/path".to_string()),
            domain: None,
        })
        .unwrap();
    let names: Vec<_> = subdomain
        .cookies
        .iter()
        .map(|cookie| cookie.name.as_str())
        .collect();
    assert!(names.contains(&"parent"));
    assert!(!names.contains(&"host"));
    assert!(!names.contains(&"evil"));

    let exact_host = engine
        .get_cookies_v0_2(GetCookiesRequest {
            context_id: context_id.clone(),
            url: Some("https://EXAMPLE.com/".to_string()),
            domain: None,
        })
        .unwrap();
    let names: Vec<_> = exact_host
        .cookies
        .iter()
        .map(|cookie| cookie.name.as_str())
        .collect();
    assert!(names.contains(&"parent"));
    assert!(names.contains(&"host"));
    assert!(!names.contains(&"evil"));

    let selected = engine
        .get_cookies_v0_2(GetCookiesRequest {
            context_id,
            url: None,
            domain: Some("example.com".to_string()),
        })
        .unwrap();
    assert!(selected.cookies.iter().all(|cookie| {
        cookie.domain == "example.com" || cookie.domain.ends_with(".example.com")
    }));
    assert!(!selected.cookies.iter().any(|cookie| cookie.name == "evil"));

    let trailing_dot = engine
        .get_cookies_v0_2(GetCookiesRequest {
            context_id: BrowserContextId::new("ctx-domain-boundary"),
            url: Some("https://sub.example.com./path".to_string()),
            domain: None,
        })
        .unwrap();
    assert!(
        trailing_dot
            .cookies
            .iter()
            .any(|cookie| cookie.name == "parent")
    );
}

#[test]
fn invalid_cookie_domains_are_rejected_and_clear_reports_actual_change() {
    let engine = InMemoryCookieEngine::new();
    let context_id = BrowserContextId::new("ctx-invalid-domain");
    for domain in [
        "",
        ".",
        "..example.com",
        "example.com..",
        "example.com/path",
        "bad domain",
    ] {
        let result = engine.set_cookie_v0_2(SetCookieRequestV0_2 {
            context_id: context_id.clone(),
            cookie: CookieV0_2 {
                name: "bad".to_string(),
                value: "value".to_string(),
                domain: domain.to_string(),
                path: "/".to_string(),
                secure: false,
                http_only: false,
                same_site: None,
                expires_epoch_sec: None,
                host_only: true,
            },
        });
        assert!(result.is_err(), "domain {domain:?} must be rejected");
    }

    let first_clear = engine
        .clear_storage(
            worldline_browser_contract::primitives::ClearStorageRequest {
                context_id: context_id.clone(),
                origin: "https://example.com".to_string(),
                storage_type: StorageType::LocalStorage,
            },
        )
        .unwrap();
    assert!(!first_clear.cleared);
}
