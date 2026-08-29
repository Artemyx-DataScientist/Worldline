/// Test-only hard-termination hooks.
///
/// The production feature set compiles the same call sites to no-ops. The
/// environment variable is therefore not an accidental runtime switch in a
/// normal build; only the dedicated test-failpoints acceptance build can
/// terminate at a named point.
pub(crate) fn hit(name: &str) {
    #[cfg(feature = "test-failpoints")]
    {
        if std::env::var("WORLDLINE_FAILPOINT").ok().as_deref() == Some(name) {
            eprintln!("worldline test failpoint reached: {name}");
            std::process::abort();
        }
    }

    #[cfg(not(feature = "test-failpoints"))]
    {
        let _ = name;
    }
}
