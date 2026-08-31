#![no_std]
extern crate alloc;

wit_bindgen::generate!({
    path: "../../../../../crates/worldline-plugin-protocol/wit",
    world: "external-plugin",
});

#[global_allocator]
static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

use alloc::{string::String, vec::Vec};

#[panic_handler]
fn worldline_panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct MemoryHog;

impl exports::worldline::plugin::plugin_operations::Guest for MemoryHog {
    fn invoke(_operation: String, _payload: Vec<u8>) -> Result<Vec<u8>, String> {
        // Each demand is ~6.4 GiB, far above the 64 MiB store limit; the
        // host limiter denies every growth. Bounded retries keep the call
        // inside the fuel budget.
        for _ in 0..1_000 {
            unsafe {
                core::arch::wasm32::memory_grow(0, 100_000);
            }
        }
        Ok(alloc::vec![b'g', b'r', b'o', b'w', b'n'])
    }
}

export!(MemoryHog);
