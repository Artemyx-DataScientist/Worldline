#![no_std]
extern crate alloc;

wit_bindgen::generate!({
    path: "../../../../../crates/worldline-plugin-protocol/wit",
    world: "external-plugin",
});

use alloc::{string::String, vec::Vec};

#[global_allocator]
static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[panic_handler]
fn worldline_panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Trapper;

impl exports::worldline::plugin::plugin_operations::Guest for Trapper {
    fn invoke(_operation: String, _payload: Vec<u8>) -> Result<Vec<u8>, String> {
        unsafe {
            core::arch::wasm32::unreachable()
        }
    }
}

export!(Trapper);
