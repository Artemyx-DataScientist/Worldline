use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use worldline_native_host::{
    HostRequestSink, NativeChild, NativeChildSpec, ProcessTreeContainment,
};
use worldline_plugin_protocol::{BlobAction, BlobRequest, BlobResult, MessageKind};

struct MockBlobSink {
    received_blob: AtomicBool,
}

impl HostRequestSink for MockBlobSink {
    fn on_child_request(
        &self,
        kind: MessageKind,
        _correlation_id: u64,
        payload: Value,
    ) -> Result<Option<Value>, worldline_native_host::NativeHostError> {
        if kind == MessageKind::BlobRequest {
            self.received_blob.store(true, Ordering::SeqCst);
            let req: BlobRequest = serde_json::from_value(payload).map_err(|e| {
                worldline_native_host::NativeHostError::ProtocolViolation {
                    reason: e.to_string(),
                }
            })?;
            match req.action {
                BlobAction::Put { blob_id, bytes } => {
                    let res = BlobResult::PutSuccess {
                        blob_id,
                        byte_len: bytes.len(),
                    };
                    Ok(Some(serde_json::to_value(res).unwrap()))
                }
                BlobAction::Get {
                    blob_id,
                    offset: _,
                    max_bytes: _,
                } => {
                    let res = BlobResult::GetSuccess {
                        blob_id,
                        data: vec![0xCA, 0xFE],
                        is_truncated: false,
                        total_bytes: 2,
                    };
                    Ok(Some(serde_json::to_value(res).unwrap()))
                }
                BlobAction::Verify { blob_id } => {
                    let res = BlobResult::VerifySuccess {
                        blob_id,
                        exists: true,
                        byte_len: Some(2),
                    };
                    Ok(Some(serde_json::to_value(res).unwrap()))
                }
            }
        } else {
            Ok(None)
        }
    }
}

#[test]
fn process_tree_containment_initialization() {
    let containment = ProcessTreeContainment::new();
    assert!(
        containment.is_ok(),
        "Containment must initialize on supported platforms"
    );
}

#[test]
fn native_child_with_containment_lifecycle() {
    // Spawn a quick command (ping / hostname or current exe)
    #[cfg(windows)]
    let program = "cmd.exe";
    #[cfg(not(windows))]
    let program = "echo";

    #[cfg(windows)]
    let args = vec!["/C".to_string(), "ping 127.0.0.1 -n 2".to_string()];
    #[cfg(not(windows))]
    let args = vec!["hello".to_string()];

    let spec = NativeChildSpec {
        program: program.into(),
        args,
        max_frame_bytes: 64 * 1024,
        stderr_max_bytes: 4 * 1024,
        enable_process_tree_containment: true,
    };

    let mut child = NativeChild::spawn(&spec).expect("must spawn with containment");
    // Kill child and ensure no hang
    child.kill();
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn blob_request_sink_handling() {
    let sink = MockBlobSink {
        received_blob: AtomicBool::new(false),
    };

    let req = BlobRequest {
        action: BlobAction::Put {
            blob_id: "sha256-test".to_string(),
            bytes: vec![1, 2, 3, 4],
        },
    };
    let payload = serde_json::to_value(&req).unwrap();

    let res = sink.on_child_request(MessageKind::BlobRequest, 1, payload);
    assert!(res.is_ok());
    assert!(sink.received_blob.load(Ordering::SeqCst));

    let result_val = res.unwrap().unwrap();
    let blob_result: BlobResult = serde_json::from_value(result_val).unwrap();
    match blob_result {
        BlobResult::PutSuccess { blob_id, byte_len } => {
            assert_eq!(blob_id, "sha256-test");
            assert_eq!(byte_len, 4);
        }
        _ => panic!("Expected PutSuccess"),
    }
}
