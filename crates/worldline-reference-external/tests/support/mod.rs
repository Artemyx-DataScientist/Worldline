//! Test support for the cross-mode conformance suite: locates the native
//! provider child executable and produces the benign WASM component.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Locates the native provider child built by `worldline-native-host`.
/// Integration test binaries live under `<target>/<profile>/deps`, so the
/// profile directory is one level up.
pub fn native_provider_program() -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("profile directory")
        .to_path_buf();
    for name in ["reference-native-provider.exe", "reference-native-provider"] {
        let candidate = profile_dir.join(name);
        if candidate.exists() {
            return candidate;
        }
    }
    panic!(
        "reference-native-provider executable not found under {}",
        profile_dir.display()
    );
}

#[allow(dead_code)]
pub struct TestComponents {
    pub benign_echo: Vec<u8>,
    pub memory_hog: Vec<u8>,
    pub spin_loop: Vec<u8>,
    pub trapper: Vec<u8>,
    pub foreign_importer: Vec<u8>,
}

static COMPONENTS: OnceLock<TestComponents> = OnceLock::new();

pub fn test_components() -> &'static TestComponents {
    COMPONENTS.get_or_init(build_all_components)
}

pub fn benign_echo_component() -> &'static Vec<u8> {
    &test_components().benign_echo
}

#[allow(dead_code)]
pub fn native_violator_program() -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("profile directory")
        .to_path_buf();
    for name in ["test-violator.exe", "test-violator"] {
        let candidate = profile_dir.join(name);
        if candidate.exists() {
            return candidate;
        }
    }
    panic!(
        "test-violator executable not found under {}",
        profile_dir.display()
    );
}

fn build_all_components() -> TestComponents {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../worldline-wasm-host/tests/components/Cargo.toml");
    let manifest = manifest.canonicalize().unwrap_or_else(|error| {
        panic!(
            "component workspace at {} must exist: {error}",
            manifest.display()
        )
    });
    let target_dir = manifest
        .parent()
        .expect("component workspace dir")
        .join("target");
    let status = Command::new("cargo")
        .args([
            "build",
            "--manifest-path",
            manifest.to_str().expect("utf-8 manifest"),
            "--target",
            "wasm32-unknown-unknown",
        ])
        .env("CARGO_TARGET_DIR", &target_dir)
        .status()
        .expect("cargo must build the test components");
    assert!(status.success(), "test component build failed");

    let core_dir = target_dir.join("wasm32-unknown-unknown/debug");
    let artifact = |name: &str| -> Vec<u8> {
        std::fs::read(core_dir.join(format!("{name}.wasm")))
            .unwrap_or_else(|error| panic!("component {name}.wasm must exist: {error}"))
    };

    let component = |name: &str| -> Vec<u8> {
        let core = artifact(name);
        wit_component::ComponentEncoder::default()
            .module(&core)
            .expect("core module must parse")
            .validate(true)
            .encode()
            .expect("core module must encode into a component")
    };

    TestComponents {
        benign_echo: component("benign_echo"),
        memory_hog: component("memory_hog"),
        spin_loop: component("spin_loop"),
        trapper: component("trapper"),
        foreign_importer: component("foreign_importer"),
    }
}
