//! Replaceable structured URL-search provider plugin for Worldline.
//!
//! Provides the experimental `browser.search/0.1` capability without
//! browser page mutation or navigation authority.

pub mod config;
pub mod plugin;
pub mod service;

pub use config::{SearchConfigError, SearchProviderConfig, is_loopback_host};
pub use plugin::{SearchProviderPlugin, search_capability};
pub use service::SearchProviderService;

#[cfg(test)]
mod tests {
    use super::*;
    use worldline_browser_services_contract::{
        MAX_SEARCH_QUERY_LENGTH, MAX_SEARCH_TARGET_URL_LENGTH, OP_RESOLVE_SEARCH,
        SearchContractError, SearchResolveRequest,
    };
    use worldline_kernel::CapabilityService;

    #[test]
    fn valid_production_https_config() {
        let config = SearchProviderConfig::new("DuckDuckGo", "https://duckduckgo.com/html/", "q")
            .with_static_parameter("kl", "wt-wt")
            .with_static_parameter("k1", "-1");

        assert!(config.validate().is_ok());
    }

    #[test]
    fn reject_insecure_remote_http_in_production() {
        let config =
            SearchProviderConfig::new("RemoteHttp", "http://insecure-search.example.com/", "q");
        match config.validate() {
            Err(SearchConfigError::InsecureScheme { scheme }) => {
                assert_eq!(scheme, "http");
            }
            other => panic!("expected InsecureScheme, got {other:?}"),
        }
    }

    #[test]
    fn allow_http_for_explicit_loopback_test_configuration() {
        let loopback_v4 =
            SearchProviderConfig::new("LocalTest", "http://127.0.0.1:8080/search", "q")
                .with_loopback_http(true);
        assert!(loopback_v4.validate().is_ok());

        let loopback_name =
            SearchProviderConfig::new("LocalTest", "http://localhost:3000/search", "q")
                .with_loopback_http(true);
        assert!(loopback_name.validate().is_ok());

        let loopback_v6 = SearchProviderConfig::new("LocalTest", "http://[::1]:9090/search", "q")
            .with_loopback_http(true);
        assert!(loopback_v6.validate().is_ok());

        // Still rejected if allow_loopback_http is false
        let disabled_loopback =
            SearchProviderConfig::new("LocalTest", "http://127.0.0.1:8080/search", "q");
        assert!(disabled_loopback.validate().is_err());
    }

    #[test]
    fn reject_forbidden_schemes() {
        for scheme_url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,test",
            "ftp://ftp.example.com/search",
            "about:blank",
        ] {
            let config = SearchProviderConfig::new("BadScheme", scheme_url, "q");
            assert!(
                config.validate().is_err(),
                "expected rejection for scheme URL: {scheme_url}"
            );
        }
    }

    #[test]
    fn reject_url_userinfo_credentials() {
        let config = SearchProviderConfig::new(
            "Credentials",
            "https://admin:secret@example.com/search",
            "q",
        );
        assert_eq!(
            config.validate(),
            Err(SearchConfigError::UserInfoNotAllowed)
        );
    }

    #[test]
    fn reject_empty_query_parameter_name() {
        let config = SearchProviderConfig::new("BadParam", "https://example.com/search", "   ");
        assert_eq!(
            config.validate(),
            Err(SearchConfigError::EmptyQueryParameterName)
        );
    }

    #[test]
    fn reject_duplicate_and_conflicting_static_parameters() {
        let duplicate_key = SearchProviderConfig::new("Dup", "https://example.com/search", "q")
            .with_static_parameter("param1", "val1")
            .with_static_parameter("param1", "val2");
        assert_eq!(
            duplicate_key.validate(),
            Err(SearchConfigError::DuplicateStaticParameterKey(
                "param1".to_string()
            ))
        );

        let conflict_key = SearchProviderConfig::new("Conflict", "https://example.com/search", "q")
            .with_static_parameter("q", "static_override");
        assert_eq!(
            conflict_key.validate(),
            Err(SearchConfigError::StaticParameterConflictsWithQueryName(
                "q".to_string()
            ))
        );
    }

    #[test]
    fn structural_url_encoding_prevents_escape() {
        let config = SearchProviderConfig::new("DuckDuckGo", "https://duckduckgo.com/html/", "q")
            .with_static_parameter("kl", "wt-wt");
        let service = SearchProviderService::new(config).expect("valid service");

        // Test with tricky inputs that would break naive string concatenation:
        // spaces, ampersand, equals, question mark, hash/fragment, unicode, percent signs
        let input_cases = [
            ("hello world", "q=hello+world"),
            ("foo&bar=baz", "q=foo%26bar%3Dbaz"),
            ("test#fragment", "q=test%23fragment"),
            ("question?mark", "q=question%3Fmark"),
            ("100% genuine", "q=100%25+genuine"),
            ("Rust 🦀", "q=Rust+%F0%9F%A6%80"),
            (
                "Привет мир",
                "q=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82+%D0%BC%D0%B8%D1%80",
            ),
            (
                "日本語 検索",
                "q=%E6%97%A5%E6%9C%AC%E8%AA%9E+%E6%A4%9C%E7%B4%A2",
            ),
        ];

        for (raw_query, expected_substring) in input_cases {
            let req = SearchResolveRequest::new(raw_query).expect("valid request");
            let target = service.resolve(&req).expect("resolved");
            let target_url = target.url();

            // Must preserve base origin and path
            assert!(target_url.starts_with("https://duckduckgo.com/html/?"));
            // Must contain static parameters intact
            assert!(target_url.contains("kl=wt-wt"));
            // Must contain safely escaped query parameter
            assert!(
                target_url.contains(expected_substring),
                "target URL '{target_url}' did not contain expected substring '{expected_substring}'"
            );

            // Structure verification: must parse cleanly back to URL
            let parsed = url::Url::parse(target_url).expect("must parse back to Url");
            assert_eq!(parsed.scheme(), "https");
            assert_eq!(parsed.host_str(), Some("duckduckgo.com"));
            assert_eq!(parsed.path(), "/html/");
            assert_eq!(
                parsed.fragment(),
                None,
                "fragment was escaped and must not exist!"
            );

            // Query parameter value must roundtrip exactly to the original raw query
            let mut found_query = None;
            for (k, v) in parsed.query_pairs() {
                if k == "q" {
                    found_query = Some(v.into_owned());
                }
            }
            assert_eq!(
                found_query.as_deref(),
                Some(raw_query),
                "decoded query parameter must equal raw input query"
            );
        }
    }

    #[test]
    fn invoke_capability_service_roundtrip() {
        let config = SearchProviderConfig::new("Loopback", "http://127.0.0.1:8080/search", "query")
            .with_loopback_http(true);
        let service = SearchProviderService::new(config).expect("valid service");

        let req = SearchResolveRequest::new("worldline rust microkernel").expect("valid");
        let payload = serde_json::to_vec(&req).expect("serialize");

        let response_bytes = service
            .invoke(OP_RESOLVE_SEARCH, &payload)
            .expect("invoke succeeds");

        let target: worldline_browser_services_contract::SearchNavigationTarget =
            serde_json::from_slice(&response_bytes).expect("deserialize response");

        assert_eq!(target.query_parameter_name(), "query");
        assert!(
            target
                .url()
                .starts_with("http://127.0.0.1:8080/search?query=worldline+rust+microkernel")
        );
    }

    #[test]
    fn invoke_unsupported_operation_fails() {
        let config = SearchProviderConfig::new("DDG", "https://duckduckgo.com/", "q");
        let service = SearchProviderService::new(config).expect("valid");
        let err = service.invoke("browser.navigate", b"{}");
        assert!(err.is_err());
    }

    #[test]
    fn oversized_target_url_fails_gracefully() {
        // Create config near limit
        let base_prefix =
            "https://example.com/very/long/path/".to_string() + &"segment/".repeat(500);
        // Even if base is long, total URL beyond limit must return TargetUrlTooLong error
        let config = SearchProviderConfig::new("LongBase", base_prefix, "q");
        if let Ok(service) = SearchProviderService::new(config) {
            let req = SearchResolveRequest::new("a".repeat(MAX_SEARCH_QUERY_LENGTH)).unwrap();
            let res = service.resolve(&req);
            match res {
                Err(SearchContractError::TargetUrlTooLong { length, max }) => {
                    assert!(length > max);
                    assert_eq!(max, MAX_SEARCH_TARGET_URL_LENGTH);
                }
                other => panic!("expected TargetUrlTooLong or valid rejection, got {other:?}"),
            }
        }
    }
}
