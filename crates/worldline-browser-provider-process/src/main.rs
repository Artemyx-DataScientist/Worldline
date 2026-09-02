//! Executable entrypoint for the supervised browser provider.

fn main() {
    std::process::exit(worldline_browser_provider_client::run_main(
        std::env::args().collect(),
        0,
    ));
}
