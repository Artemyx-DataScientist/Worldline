# ADR: Browser Search Providers (Experimental 0.1)

Status: Accepted for the bounded M1.3 search-provider slice

Date: 2026-09-05

Change: `C-BROWSER-SEARCH-PROVIDERS-20260904`

## Context and Problem

In Worldline milestone M1.3, the browser subsystem has acquired core engine primitives (CEF out-of-process provider), tabs/history, downloads/cookies, request-policy/adblock interception, and DevTools diagnostics. However, there is still no product contract for web search providers.

Traditional browser designs typically solve this in one of three ways:
1. **Hardcoding search engine templates in the shell/omnibox UI**: couples shell code to specific commercial search engines, prevents dynamic user provider replacement, and collapses omnibox text classification with search-provider policy.
2. **Implementing search inside the browser engine (CEF/Chromium)**: pollutes the engine layer with product-level business policy that CEF does not need and cannot generically own across engine replacements.
3. **Granting search providers direct page navigation authority**: collapses two fundamentally distinct capability authorities — disclosure of user query text to an external provider vs. mutation of browser tabs and page state.

Furthermore, user search queries represent sensitive personal data. If search providers or the generic execution framework indiscriminately broadcast or log queries, privacy boundaries are breached.

## Core Architectural Invariants

1. **Search as Target Resolution (`browser.search/0.1`)**:
   Search is purely a target-resolution capability, not a navigation capability and not a structured SERP/search-results retrieval engine.
   - Input: bounded, already-classified query text (`SearchResolveRequest`).
   - Output: bounded navigation target data (`SearchNavigationTarget`), containing a validated target URL and metadata.
   - Result is data only: the search provider never holds `NavigatePage` or `ActOnPage` authority.

2. **Strict Authority Separation**:
   - `AUTH_SEARCH_RESOLVE`: authority required by a caller to invoke `browser.search/resolve`. Discloses user-entered text only to the selected search provider installation.
   - `AUTH_BROWSER_NAVIGATE` (`browser.navigate`): separate authority required to actually navigate a browser page to the resolved target URL.
   - A search provider cannot mutate page state, inject scripts, inspect cookies, or intercept network traffic.

3. **Address vs. Search Classification is External**:
   The future omnibox / shell layer is responsible for determining whether user input is a direct URL, an internal navigation command, or a web search. The search provider does not guess or classify input.

4. **Multi-Installation Provider Targeting via Generic Kernel Mechanism**:
   Rather than building a search-specific registry or preferred-engine table into the microkernel, Worldline reuses generic capability provider targeting (`CapabilityTarget::Installation(InstallationId)`), established in `ADR-KERNEL-CAPABILITY-PROVIDER-TARGETING-V1.md`.
   - Multiple installations of the same `PluginDefinition` (`worldline-browser-search`) can be active concurrently (e.g. `search-duckduckgo`, `search-google`, `search-kagi`, `search-local`).
   - The consumer or shell selects an engine by explicitly addressing its durable `InstallationId`: `kernel.capability_for_installation(caller, capability, installation_id)`.
   - If a target installation is unavailable, the kernel fails closed with typed `CapabilityError::TargetUnavailable` without silent fallback to another provider.

5. **Structural URL Construction without Template String Interpolation**:
   The `worldline-browser-search` provider avoids fragile `{searchTerms}` string replacement. Each installation configures:
   - A parsed base origin and path (e.g. `https://html.duckduckgo.com/html/`);
   - Exactly one query parameter name (e.g. `q`);
   - Optional bounded static query parameters (e.g. `kl=wt-wt`).
   - Query text is strictly passed through the URL encoder as the value of the designated parameter, preventing scheme/authority/path escape, parameter injection, or credential leakage.

6. **Privacy and Telemetry Boundaries**:
   - Query text is sensitive payload.
   - Debug and Display implementations for search DTOs redact or omit raw query text.
   - Trajectory audit logs record invocation metadata (`request_id`, `invocation_id`, target `InstallationId`, resolved `RuntimeId`, outcome code), but strictly omit raw query strings.

7. **Production HTTPS vs. Test Loopback HTTP**:
   Production configurations require HTTPS. Unencrypted HTTP is rejected unless explicitly operating under loopback test configuration (`127.0.0.1` / `::1`), enabling deterministic local proving (S3E) without Internet access.

8. **EVENT BUS IS NOT RPC**:
   Search resolution is a synchronous Capability RPC returning a typed `SearchNavigationTarget` data value. Event delivery cannot determine, initiate, or replace an RPC result. Search queries are never broadcast over the event bus.

## Verification and Proving Slices

- **T-001 Feasibility Evidence**:
  Generic multi-installation provider targeting was validated and verified in `crates/worldline-kernel/tests/runtime_v1_acceptance.rs`:
  - `multi_installation_provider_targeting_coexistence`: independent concurrent invocation of two installations of the same capability.
  - `multi_installation_provider_targeting_restart_and_authority_isolation`: new `RuntimeId` on restart with zero stale authority inheritance.
  - `multi_installation_provider_targeting_fail_closed_without_fallback`: fail-closed `TargetUnavailable` semantics.
  - `multi_installation_provider_targeting_caller_authorization_enforced`: strict caller authorization.
  - `multi_installation_provider_targeting_trajectory_audit`: metadata-only audit logging.
- **Deterministic S3E Slice** (`worldline-reference/src/s3e.rs`):
  Proves loopback search resolution and separate navigation in isolated process space without network access.
- **Native Real-CEF S3E Slice** (`worldline-reference/tests/s3e_real_acceptance.rs`):
  Proves search resolution through real Windows CEF provider, confirming that resolution produces 0 network hits and only a subsequent authorized `browser.navigate` navigates to the resolved target.
