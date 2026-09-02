# Browser Provider Finite-Budget Evidence Matrix

## Status and Purpose

This document provides repository-owned evidence mapping every finite-budget
claim from the applied M1.2 specification (`C-BROWSER-ENGINE-PROVIDER-PROCESS-20260901`)
to its authoritative owner, implementation location, configured/default limit,
and concrete executable acceptance test.

Per Worldline invariant and audit findings:
- **No budget is reported as enforced solely because a configuration field, constant, constructor argument or test fixture value exists.**
- Every claim must be backed by concrete, executable limit-saturation and lifecycle tests that deterministically prove explicit bounded error on exhaustion and capacity release on resource cleanup.

---

## Evidence Matrix

| Claimed Resource / Budget | Authoritative Owner | Implementation Location | Default Production Limit | Configurable / Injected Mechanism | Concrete Acceptance Test(s) |
|---|---|---|---|---|---|
| **Max Browser Contexts** | `BrowserProviderCore` | `crates/worldline-browser-provider/src/core.rs` (`dispatch_contract`) | `32` | `ProviderBudgetLimits.max_contexts` via `BrowserProviderCore::with_limits` | `provider_max_contexts_budget_enforcement` in `recovery_health_acceptance.rs`: tests saturation at limit 2, rejection of 3rd context, exact count reporting in `list_contexts`, and slot release after `close_context`. |
| **Max Pages per Context** | `BrowserProviderCore` | `crates/worldline-browser-provider/src/core.rs` (`dispatch_contract`) | `64` | `ProviderBudgetLimits.max_pages_per_context` via `BrowserProviderCore::with_limits` | `provider_max_pages_per_context_budget_enforcement` in `recovery_health_acceptance.rs`: tests saturation at limit 2 per context, independent quota for distinct contexts, and slot release after `close_page`. |
| **Max Action Text Length** | `BrowserProviderCore` | `crates/worldline-browser-provider/src/core.rs` (`validate_and_act` / `OP_INPUT`) | `65536` bytes | `ProviderBudgetLimits.max_action_text_len` via `BrowserProviderCore::with_limits` | `provider_budget_limits_action_text_len_enforcement` in `recovery_health_acceptance.rs`: tests rejection of input actions exceeding injected byte bound with `InvalidRequest`. |
| **Pending Native RPC / In-Flight Calls** | `NativeProviderConnection` | `crates/worldline-native-host/src/connection.rs` | `16` | `max_in_flight` parameter in `NativeProviderConnection::connect` | `in_flight_concurrency_bounded_and_saturates` in `crates/worldline-native-host/tests/native_host_acceptance.rs`: verifies semaphore-bounded in-flight calls and rejection/timeout on saturation. |
| **Native Frame Size** | Stdio Native Framing / Host Supervisor | `crates/worldline-native-host/src/connection.rs`, `crates/worldline-plugin-protocol/src/framing.rs` | `4 * 1024 * 1024` bytes (4 MiB) | `NativeChildSpec.max_frame_bytes` | `oversized_frame_rejected_before_allocation` in `crates/worldline-native-host/tests/native_host_acceptance.rs` and `crates/worldline-plugin-protocol/tests/protocol_acceptance.rs`. |
| **Provider Command Queue** | Native Provider Process Loop | `crates/worldline-browser-provider-process/src/lib.rs` | `PROVIDER_COMMAND_QUEUE_CAPACITY = 64` | Fixed compile-time synchronization channel bound | Static architecture assertion in `Test-WorldlineArchitecture.ps1` checking `const PROVIDER_COMMAND_QUEUE_CAPACITY`; backpressure verified in `process_acceptance.rs`. |
| **Query Projection Bounds** | `worldline-browser-contract` DOM / AX query | `crates/worldline-browser-contract/src/query.rs` | Bounded by caller `QueryBounds`: `max_depth` (default 32), `max_nodes` (default 1000), `max_text_bytes` (default 65536) | `QueryBounds` struct passed in `QueryDocumentRequest` | `query_bounds_truncation_enforcement` in `crates/worldline-browser-contract/tests/contract_acceptance.rs` and `provider_core_acceptance.rs`: verifies `is_truncated` flag and strict node/depth ceilings. |
| **Capture Image / Pixel Bounds** | Visual Capture Subsystem | `crates/worldline-browser-contract/src/capture.rs`, `worldline-plugin-protocol` BlobStore | Bounded by viewport dimensions and blob chunk size (64 KiB chunks, max blob size enforced by Host BlobStore) | `CapturePageRequest.clip` and Host `BlobStore` policy | `capture_page_bounds_and_blob_storage_acceptance` in `crates/worldline-browser-provider/tests/semantic_action_capture_acceptance.rs`. |
| **Event & Download Mailboxes** | Provider Process Event Drain | `crates/worldline-browser-provider-process/src/lib.rs` | Bounded channel size for download events, synchronous drain per command loop turn | Internal event queue capacity | Verified in `crates/worldline-browser-provider-process/tests/process_acceptance.rs` and `crates/worldline-reference/tests/s3b_real_acceptance.rs`. |
| **Startup / Shutdown Deadlines** | Native Host Process Supervisor | `crates/worldline-native-host/src/connection.rs`, `worldline-browser-cef/src/backend.rs` | `5` seconds (`CEF_CALLBACK_TIMEOUT`), configurable shutdown deadline | `call_with_deadline` and `close(shutdown_deadline)` parameters | `graceful_and_hard_kill_shutdown_with_job_object` in `crates/worldline-native-host/tests/containment_acceptance.rs` and `recovery_health_acceptance.rs`. |

---

## Lifecycle Accounting Invariants

1. **Pre-Creation Enforcement**: Resource limits (`max_contexts`, `max_pages_per_context`) are checked prior to dispatching creation to the backend. Failure to satisfy budget bounds halts the call immediately and leaves the backend state completely unmutated.
2. **Single Truth Inventory**: Current occupancy is determined by authoritative backend listing (`backend.list_contexts()`, `backend.list_pages()`). No duplicate or desynchronized shadow inventory is maintained.
3. **Deterministic Release**: Successful `close_context` or `close_page` dispatches release budget occupancy, allowing subsequent requests within capacity to succeed.
4. **Zero-Limit Deny-All**: Setting `max_contexts: 0` or `max_pages_per_context: 0` deterministically rejects any creation attempt as exhausted.
