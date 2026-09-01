use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::{Deserialize, Serialize};

/// Lightweight, safe, zero-dependency Chromium DevTools Protocol (CDP) WebSocket client.
pub struct CdpSession {
    stream: TcpStream,
    next_id: AtomicU64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CdpRequest {
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CdpResponse {
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub method: Option<String>,
    pub params: Option<serde_json::Value>,
}

impl CdpSession {
    /// Connects to a Chromium CDP WebSocket endpoint at `127.0.0.1:port` on `path`.
    pub fn connect(port: u16, path: &str) -> Result<Self, String> {
        let addr = format!("127.0.0.1:{port}");
        let mut stream =
            TcpStream::connect(&addr).map_err(|e| format!("failed to connect to {addr}: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| e.to_string())?;

        // Perform HTTP WebSocket upgrade handshake
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("failed to write upgrade request: {e}"))?;

        // Read handshake response headers byte-by-byte until \r\n\r\n
        let mut header_bytes = Vec::new();
        let mut byte = [0u8; 1];
        while header_bytes.len() < 4096 {
            stream
                .read_exact(&mut byte)
                .map_err(|e| format!("failed to read upgrade response byte: {e}"))?;
            header_bytes.push(byte[0]);
            if header_bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let response_str = String::from_utf8_lossy(&header_bytes);
        if !response_str.starts_with("HTTP/1.1 101") && !response_str.contains(" 101 ") {
            return Err(format!(
                "CDP WebSocket upgrade failed. Server responded: {response_str}"
            ));
        }

        Ok(Self {
            stream,
            next_id: AtomicU64::new(1),
        })
    }

    /// Sends a raw JSON text WebSocket frame (masked, RFC 6455).
    fn send_frame(&mut self, text: &str) -> Result<(), String> {
        let payload = text.as_bytes();
        let len = payload.len();
        let mut header = Vec::with_capacity(14 + len);

        // Byte 0: FIN (0x80) | Text opcode (0x01)
        header.push(0x81);

        // Byte 1+: Mask bit (0x80) | length
        let mask = [0x1a, 0x2b, 0x3c, 0x4d];
        if len < 126 {
            header.push(0x80 | (len as u8));
        } else if len <= 65535 {
            header.push(0x80 | 126);
            header.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            header.push(0x80 | 127);
            header.extend_from_slice(&(len as u64).to_be_bytes());
        }

        // Mask key
        header.extend_from_slice(&mask);

        // Masked payload
        for (i, byte) in payload.iter().enumerate() {
            header.push(byte ^ mask[i % 4]);
        }

        self.stream
            .write_all(&header)
            .map_err(|e| format!("failed to write WebSocket frame: {e}"))?;
        self.stream
            .flush()
            .map_err(|e| format!("failed to flush WebSocket: {e}"))?;
        Ok(())
    }

    /// Reads an incoming unmasked WebSocket text frame from Chromium.
    fn read_frame(&mut self) -> Result<String, String> {
        loop {
            let mut byte01 = [0u8; 2];
            self.stream
                .read_exact(&mut byte01)
                .map_err(|e| format!("failed to read frame header: {e}"))?;

            let opcode = byte01[0] & 0x0f;
            let masked = (byte01[1] & 0x80) != 0;
            let mut payload_len = (byte01[1] & 0x7f) as usize;

            if payload_len == 126 {
                let mut len_bytes = [0u8; 2];
                self.stream
                    .read_exact(&mut len_bytes)
                    .map_err(|e| format!("failed to read 16-bit length: {e}"))?;
                payload_len = u16::from_be_bytes(len_bytes) as usize;
            } else if payload_len == 127 {
                let mut len_bytes = [0u8; 8];
                self.stream
                    .read_exact(&mut len_bytes)
                    .map_err(|e| format!("failed to read 64-bit length: {e}"))?;
                payload_len = u64::from_be_bytes(len_bytes) as usize;
            }

            let mask = if masked {
                let mut mask_bytes = [0u8; 4];
                self.stream
                    .read_exact(&mut mask_bytes)
                    .map_err(|e| format!("failed to read mask: {e}"))?;
                Some(mask_bytes)
            } else {
                None
            };

            let mut payload = vec![0u8; payload_len];
            self.stream
                .read_exact(&mut payload)
                .map_err(|e| format!("failed to read frame payload: {e}"))?;

            if let Some(mask) = mask {
                for (i, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mask[i % 4];
                }
            }

            // Text frame (0x01)
            if opcode == 0x01 {
                return String::from_utf8(payload)
                    .map_err(|e| format!("invalid UTF-8 in frame payload: {e}"));
            }

            // Close frame (0x08)
            if opcode == 0x08 {
                return Err("WebSocket connection closed by remote peer".to_string());
            }

            // Ping frame (0x09) -> Respond with Pong (0x0a)
            if opcode == 0x09 {
                let pong = [0x8a, 0x00];
                let _ = self.stream.write_all(&pong);
            }
        }
    }

    /// Invokes a CDP method and waits for the correlated JSON-RPC response.
    pub fn call_method(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = CdpRequest {
            id,
            method: method.to_string(),
            params,
        };
        let req_json = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        self.send_frame(&req_json)?;

        // Read frames until we find response with matching id
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            let frame_text = self.read_frame()?;
            if let Ok(resp) = serde_json::from_str::<CdpResponse>(&frame_text) {
                if resp.id != Some(id) {
                    continue;
                }
                if let Some(err) = resp.error {
                    return Err(format!("CDP error from method '{method}': {err}"));
                }
                return Ok(resp.result.unwrap_or(serde_json::Value::Null));
            }
        }
        Err(format!(
            "timed out waiting for response to CDP method '{method}' (id: {id})"
        ))
    }

    /// Enables standard domains (Page, Runtime, DOM, Accessibility).
    pub fn enable_domains(&mut self) -> Result<(), String> {
        self.call_method("Page.enable", serde_json::json!({}))?;
        self.call_method("Runtime.enable", serde_json::json!({}))?;
        self.call_method("DOM.enable", serde_json::json!({}))?;
        self.call_method("Accessibility.enable", serde_json::json!({}))?;
        Ok(())
    }

    /// Navigates the page to a URL and waits for commit.
    pub fn navigate(&mut self, url: &str) -> Result<serde_json::Value, String> {
        self.call_method("Page.navigate", serde_json::json!({ "url": url }))
    }

    /// Evaluates a JavaScript expression and returns the string value.
    pub fn evaluate_string(&mut self, expression: &str) -> Result<String, String> {
        let res = self.call_method(
            "Runtime.evaluate",
            serde_json::json!({
                "expression": expression,
                "returnByValue": true,
            }),
        )?;
        let val = res
            .pointer("/result/value")
            .or_else(|| res.pointer("/value"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        Ok(val.to_string())
    }

    /// Resolves a renderer-owned DOM node into a temporary Runtime remote object.
    pub fn resolve_backend_node(&mut self, backend_node_id: i64) -> Result<String, String> {
        let res = self.call_method(
            "DOM.resolveNode",
            serde_json::json!({ "backendNodeId": backend_node_id }),
        )?;
        res.pointer("/object/objectId")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
            .ok_or_else(|| {
                format!(
                    "CDP DOM.resolveNode returned no objectId for backend node {backend_node_id}"
                )
            })
    }

    /// Calls a static JavaScript function against a resolved remote object.
    ///
    /// Caller-controlled values are supplied as CDP `CallArgument` values rather
    /// than interpolated into the function declaration or another source string.
    pub fn call_function_on(
        &mut self,
        object_id: &str,
        function_declaration: &str,
        arguments: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let res = self.call_method(
            "Runtime.callFunctionOn",
            serde_json::json!({
                "objectId": object_id,
                "functionDeclaration": function_declaration,
                "arguments": arguments,
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true,
            }),
        )?;

        if let Some(exception) = res.get("exceptionDetails") {
            return Err(format!(
                "CDP Runtime.callFunctionOn raised an exception: {exception}"
            ));
        }

        res.pointer("/result/value")
            .cloned()
            .ok_or_else(|| "CDP Runtime.callFunctionOn returned no by-value result".to_string())
    }

    /// Releases a temporary Runtime remote object returned by DOM.resolveNode.
    pub fn release_object(&mut self, object_id: &str) -> Result<(), String> {
        self.call_method(
            "Runtime.releaseObject",
            serde_json::json!({ "objectId": object_id }),
        )?;
        Ok(())
    }

    /// Retrieves the complete Accessibility Tree from Blink via CDP.
    pub fn get_full_ax_tree(&mut self) -> Result<serde_json::Value, String> {
        self.call_method("Accessibility.getFullAXTree", serde_json::json!({}))
    }

    /// Dispatches a mouse click event to coordinates (x, y).
    pub fn dispatch_click(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.call_method(
            "Input.dispatchMouseEvent",
            serde_json::json!({
                "type": "mousePressed",
                "x": x,
                "y": y,
                "button": "left",
                "clickCount": 1
            }),
        )?;
        self.call_method(
            "Input.dispatchMouseEvent",
            serde_json::json!({
                "type": "mouseReleased",
                "x": x,
                "y": y,
                "button": "left",
                "clickCount": 1
            }),
        )?;
        Ok(())
    }

    /// Inserts text via CDP Input.insertText.
    pub fn insert_text(&mut self, text: &str) -> Result<(), String> {
        self.call_method("Input.insertText", serde_json::json!({ "text": text }))?;
        Ok(())
    }

    /// Deliberately crashes the renderer process via CDP Page.crash.
    pub fn crash_renderer(&mut self) -> Result<(), String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = CdpRequest {
            id,
            method: "Page.crash".to_string(),
            params: serde_json::json!({}),
        };
        if let Ok(req_json) = serde_json::to_string(&req) {
            let _ = self.send_frame(&req_json);
        }
        Ok(())
    }

    /// Closes the target page.
    pub fn close_target(&mut self) -> Result<(), String> {
        let _ = self.call_method("Page.close", serde_json::json!({}));
        Ok(())
    }
}
