#![no_std]
extern crate alloc;

wit_bindgen::generate!({
    path: "../../../../../crates/worldline-plugin-protocol/wit",
    world: "external-plugin",
});

#[global_allocator]
static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

use alloc::{format, string::{String, ToString}, vec::Vec};

#[panic_handler]
fn worldline_panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct EchoComponent;

impl exports::worldline::plugin::plugin_operations::Guest for EchoComponent {
    fn invoke(operation: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        match operation.as_str() {
            "echo" => Ok(format!("echo:{}", String::from_utf8_lossy(&payload)).into_bytes()),
            "stateful_increment" => {
                let current = worldline::plugin::state_access::get("reference-echo-count")
                    .and_then(|value| String::from_utf8(value).ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or_default();
                let next = current.checked_add(1).ok_or_else(|| alloc::string::String::from("echo count exhausted"))?;
                worldline::plugin::state_access::set("reference-echo-count", &next.to_string().into_bytes());
                Ok(format!("incremented:{next}:{}", String::from_utf8_lossy(&payload)).into_bytes())
            }
            "publish_observation" => {
                worldline::plugin::event_publish::publish("reference.echo", "observation", &payload)?;
                Ok(format!("observed:{}", String::from_utf8_lossy(&payload)).into_bytes())
            }
            other => Err(format!("unsupported echo operation '{other}'")),
        }
    }
}

export!(EchoComponent);
