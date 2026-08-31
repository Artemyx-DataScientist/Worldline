//! Test-support builder: compiles the hostile/benign test components under
//! `tests/components/` for `wasm32-wasip2` and wraps the resulting core
//! modules into Component Model binaries. Components are built once per
//! test binary run and shared through a `OnceLock`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

pub struct TestComponents {
    pub benign_echo: Vec<u8>,
    pub memory_hog: Vec<u8>,
    pub spin_loop: Vec<u8>,
    pub trapper: Vec<u8>,
    pub foreign_importer: Vec<u8>,
}

static COMPONENTS: OnceLock<TestComponents> = OnceLock::new();

pub fn components() -> &'static TestComponents {
    COMPONENTS.get_or_init(build_all)
}

fn build_all() -> TestComponents {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/components/Cargo.toml");
    // Isolated target dir: keeps wasm artifacts out of the shared workspace
    // target and makes clean rebuilds cheap.
    let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/components/target");
    let status = Command::new("cargo")
        .args([
            "build",
            "--manifest-path",
            manifest.to_str().expect("utf-8 manifest path"),
            "--target",
            "wasm32-unknown-unknown",
        ])
        .env("CARGO_TARGET_DIR", &target_dir)
        .status()
        .expect("cargo must be able to build the test components");
    assert!(
        status.success(),
        "test component build failed; run cargo build --manifest-path {} --target wasm32-wasip2 for details",
        manifest.display()
    );

    let core_dir = target_dir.join("wasm32-unknown-unknown/debug");
    let artifact = |name: &str| -> Vec<u8> {
        std::fs::read(core_dir.join(format!("{name}.wasm")))
            .unwrap_or_else(|error| panic!("component {name}.wasm must exist: {error}"))
    };

    let component = |name: &str| encode_component(&artifact(name));

    // The unknown-unknown target emits plain core modules with wit-bindgen
    // component-type metadata, which the encoder wraps into Component Model
    // binaries with zero WASI imports: guests are least-authority by
    // construction.
    TestComponents {
        benign_echo: component("benign_echo"),
        memory_hog: component("memory_hog"),
        spin_loop: component("spin_loop"),
        trapper: component("trapper"),
        foreign_importer: component("foreign_importer"),
    }
}

/// Wraps one wit-bindgen core module into a Component Model binary. The
/// module already carries component-type metadata for its exports, so the
/// encoder derives the component shape without extra inputs.
fn encode_component(core_module: &[u8]) -> Vec<u8> {
    wit_component::ComponentEncoder::default()
        .module(core_module)
        .expect("core module must parse")
        .validate(true)
        .encode()
        .expect("core module must encode into a component")
}
