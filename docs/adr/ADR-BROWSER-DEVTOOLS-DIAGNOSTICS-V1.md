# ADR: Browser DevTools Diagnostics (Experimental 0.1)

Status: Accepted for the bounded M1.3 diagnostics slice

Date: 2026-09-03

Change: `C-BROWSER-DEVTOOLS-DIAGNOSTICS-20260903`

## Context

Worldline operating environment requires operational observability over browser
pages without exposing engine-specific internals or compromising system security.
Prior to this change, diagnosing console errors, resource fetch failures, or page
lifecycle state required direct access to repository test fixtures, CEF provider
logs, or ad-hoc process stdout inspection.

Conventional approaches in Chromium-based products either:
1. Embed the full Chrome DevTools frontend and expose Chrome DevTools Protocol (CDP);
2. Open a local remote debugging port (`--remote-debugging-port`); or
3. Expose raw CDP JSON tunnels to consumers.

These approaches are rejected for Worldline:
- Embedding Chrome DevTools or CDP ties Worldline contracts, shell, and agents to
  Chromium implementation details, creating prohibitive costs for future browser
  engine replacement.
- CDP passthrough exposes extensive mutation capabilities (arbitrary script
  execution, DOM mutation, storage tampering, network interception) to consumers
  that only require read-only observation.
- Remote debugging ports create ambient, unauthenticated, network-facing control
  planes that bypass Worldline capability admission and security boundaries.
- Pushing raw console and network entries directly to the generic event bus would
  cause event flooding, violate privacy boundaries, and conflate transport with
  storage (**EVENT BUS IS NOT RPC**).

## Decision

Worldline introduces an additive experimental `browser.devtools/0.1` service
contract and a dedicated service plugin `worldline-browser-devtools` residing
above the browser engine provider.

The architecture strictly separates engine fact extraction from diagnostic state
management and consumer presentation:

```text
browser engine callback / provider fact
              -> engine-neutral diagnostic DTO
              -> bounded worldline-browser-devtools service
              -> authorized query / metadata observation
              -> shell surface, support tool, or authorized agent
```

### 1. Engine-Neutral Diagnostic DTOs

No CEF, Chromium, CDP, HWND, raw pointer, or platform-specific debugger object
crosses the contract boundary. The contract exposes only normalized DTOs:

- `ConsoleDiagnosticRecord`:
  - `page_id`: `PageId`;
  - `document_revision`: `DocumentRevision`;
  - `level`: `ConsoleLogLevel` (`Info`, `Warning`, `Error`, `Debug`);
  - `message`: length-bounded text (truncated with metadata if oversized);
  - `source`: length-bounded source file or script URL;
  - `line`: line number within the source;
  - `timestamp`: monotonic or ISO-8601 observation timestamp.

- `NetworkDiagnosticRecord`:
  - `page_id`: `PageId`;
  - `document_revision`: `DocumentRevision`;
  - `request_id`: opaque, correlation-scoped request identifier;
  - `method`: HTTP method string (e.g. `GET`, `POST`);
  - `resource_type`: normalized `RequestResourceType` (`MainFrame`, `SubFrame`,
    `Stylesheet`, `Script`, `Image`, `Font`, `Media`, `Xhr`, `Other`);
  - `url`: length-bounded request URL;
  - `status`: `NetworkRequestStatus` (`Completed`, `Failed`, `Blocked`);
  - `http_status`: `Option<u16>` (HTTP status code when available);
  - `mime_type`: `Option<String>` (response MIME type when available);
  - `received_bytes`: `Option<u64>` (transfer size when available);
  - `duration_ms`: `Option<u64>` (round-trip duration when available);
  - `timestamp`: observation timestamp.

- `PageRuntimeDiagnosticSnapshot`:
  - `context_id`: `BrowserContextId`;
  - `page_id`: `PageId`;
  - `document_revision`: `DocumentRevision`;
  - `url`: current page URL;
  - `title`: current document title;
  - `loading_state`: `LoadingState` (`Loading`, `Complete`, `Failed`);
  - `status_code`: HTTP status code for the main document.

### 2. Authority Separation

Diagnostic capabilities are split into disjoint authorities:

- `AUTH_DEVTOOLS_OBSERVE`: permits querying console records, network records,
  and page runtime snapshots for an explicitly admitted `BrowserContextId` and
  `PageId`. Ordinary page observation authority does not grant diagnostic access.
- `AUTH_DEVTOOLS_CONTROL`: permits clearing diagnostic buffers for an admitted
  `BrowserContextId` and `PageId`.
- `AUTH_DEVTOOLS_NATIVE`: an optional, high-trust escape hatch permitting the
  engine provider to show its native developer tools window locally where
  supported. Possessing `AUTH_DEVTOOLS_OBSERVE` or `AUTH_DEVTOOLS_CONTROL` never
  implies `AUTH_DEVTOOLS_NATIVE`.

Authority is scoped to `BrowserContextId` and `PageId`. An observer admitted
for Context A cannot query diagnostics from Context B, even when both contexts
load the same loopback origin.

### 3. Privacy and Sensitivity Boundaries

To safeguard user privacy and prevent data exfiltration:
- Cookies, Authorization headers, Proxy-Authorization headers, client certificates,
  request bodies, upload byte streams, and response content bodies are strictly
  excluded from diagnostic DTOs.
- Full request URLs and console message strings are treated as sensitive. They are
  accessible only through authorized queries scoped to the specific page/context,
  and must not be broadcast to generic metrics, system trajectories, or panic logs.

### 4. Bounded Ephemeral Storage & Retention

Diagnostic buffers are strictly bounded in memory and ephemeral by default:
- Ring buffers with finite entry capacity (default 500 entries per buffer per page)
  and finite byte limits prevent memory exhaustion under high-volume log floods.
- Overruns apply a drop-oldest policy while incrementing an observable
  `dropped_count` counter. Field lengths exceeding bounds are truncated and
  flagged with `truncated: true`.
- Navigation introduces a new `DocumentRevision`. Records retain their revision
  tag so that historical records are never attributed to current document content.
- Page close and context close immediately release all associated buffers.
- Diagnostics are ephemeral: restarting the service creates fresh buffers with
  fresh runtime authority. Durability is deferred to a future recorder component.

### 5. Failure Isolation & Availability

The diagnostics plugin is an ordinary, optional browser service:
- Crash, failure, quarantine, unload, or absence of `worldline-browser-devtools`
  must degrade only diagnostic visibility; normal page browsing, request policy,
  downloads, cookies, tabs, and history remain fully operational.
- Diagnostics collection must not block navigation, renderer execution, or CEF
  message loop threads.

### 6. Provider Seam & Verification

- CEF native provider extracts facts via maintained callbacks:
  - `DisplayHandler::on_console_message` for console output.
  - `ResourceRequestHandler` (`on_before_resource_load`, `on_resource_response`,
    `on_resource_load_complete`) for network request lifecycle.
  - `LoadHandler` (`on_load_error`, `on_load_end`) for page-level outcomes.
- Native provider process communicates facts across the IPC seam via bounded,
  non-blocking notification messages without raw pointer leakage.
- Deterministic loopback proving slice (S3D) proves console capture (log, warn,
  error), network capture (200 OK, 404 Not Found), isolation, and overflow drops.
- Hosted Windows real-CEF S3D suite verifies the complete pipeline against the
  pinned CEF runtime and provider process client.
