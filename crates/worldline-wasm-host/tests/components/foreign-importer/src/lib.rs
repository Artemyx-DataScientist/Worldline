//! Hostile component importing an interface the host never provides. The
//! host must fail closed before instantiation — the same code path that
//! rejects `wasi:*` ambient authority.

#![no_std]
extern crate alloc;

#[global_allocator]
static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

use alloc::{string::String, vec::Vec};

#[panic_handler]
fn worldline_panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

wit_bindgen::generate!({
    inline: r#"
        package test:vocab;
        interface plugin-operations {
            invoke: func(operation: string, payload: list<u8>) -> result<list<u8>, string>;
        }
        interface badge {
            issue: func() -> string;
        }
        world foreign-importer {
            import badge;
            export plugin-operations;
        }
    "#,
});

struct ForeignImporter;

impl exports::test::vocab::plugin_operations::Guest for ForeignImporter {
    fn invoke(_operation: String, _payload: Vec<u8>) -> Result<Vec<u8>, String> {
        // Reference the imported interface so the linker retains it; the
        // host must reject the component for demanding it.
        let _ = test::vocab::badge::issue();
        Ok(b"never reached".to_vec())
    }
}

export!(ForeignImporter);
