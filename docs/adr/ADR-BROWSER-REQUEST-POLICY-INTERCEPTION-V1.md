# ADR: Browser Request-Policy Interception (Experimental 0.1)

Status: Accepted for the bounded M1.3 request-policy slice

Date: 2026-09-02

Change: `C-BROWSER-REQUEST-POLICY-INTERCEPTION-20260902`

## Decision

Worldline introduces an additive experimental `browser.request-policy/0.1`
contract. It is an engine-neutral pre-dispatch decision boundary for an
authorized browser context and optional page. The first consumer is a small,
replaceable deterministic adblock profile; this ADR does not claim a general
purpose filter engine.

The contract carries only:

- `BrowserContextId`;
- an optional `PageId`;
- the request URL, only in the exact-scope decision request;
- HTTP method;
- neutral resource type;
- reliable initiator and top-level HTTP(S) origins when available.

It carries no CEF/Chromium objects, HWNDs, raw pointers, provider handles,
cookies, authorization headers, client certificates, request bodies, uploads,
response content, or unrestricted telemetry. Decision observations omit the
request URL and sensitive request data.

## Authority and scope

Observation and interception/decision are independent authorities:

- `ObserveRequestPolicy` permits only post-outcome observation;
- `DecideRequestPolicy` permits registration, unregistration, and a bounded
  pre-dispatch decision.

Every registration names an exact `BrowserContextId` and may name an exact
`PageId`. Identifiers, URLs, registration IDs, and opaque rule references are
not authority. The broker validates the requested scope against the authority
and registration before invoking a policy evaluator.

## Failure policy

Failure semantics belong to the policy registration/profile. The generic
interception broker applies an explicitly declared `RequestPolicyFailureMode`
and does not globally assume either fail-open or fail-closed behavior. The
optional first adblock profile explicitly selects `FailOpen`, so timeout,
evaluator failure, unload, quarantine, restart, crash, or absence degrades to
`Allow` with a bounded diagnostic outcome. A future security profile may
explicitly select `FailClosed` without changing the broker's generic contract.

Every decision has a finite deadline and bounded in-flight capacity. A failure
must not deadlock navigation. Registration and pending callback state are
invalidated on page/context close, provider shutdown, browser/renderer crash,
policy unload, and provider restart; a stale runtime or authority cannot be
reused.

## CEF boundary and transport

CEF owns only native request interception mechanics. Its resource request
handler marshals neutral metadata, invokes the explicit request/result path,
and translates `Allow`/`Block` to CEF continue/cancel behavior. Rule parsing,
filter lists, tracker policy, and failure-profile choice remain outside CEF.

The provider-process boundary uses a correlated, versioned
`RequestPolicyRequest`/`RequestPolicyResult` exchange. It is distinct from
`EventPublishRequest`; **EVENT BUS IS NOT RPC**. Frames, pending requests,
queueing, and cancellation are bounded. The duplex demultiplexer must not hold
the provider core mutex while waiting for a policy result or call back into
the event plane.

## Hot-path feasibility gate

Before the full adblock profile or S3C proving slice is built, one real CEF
request must pass through:

```text
CEF IO callback -> neutral policy boundary -> decision -> CEF callback
```

The early gate measures no-policy and active passthrough/decision paths at the
declared concurrency, including provider/runtime topology, throughput,
latency distribution, queue pressure, in-flight saturation, callback
completion/cancellation exactly once, deadline behavior, and transport cost.
The accepted budget and topology are recorded with the evidence. Deadlock,
unbounded growth, stale callback use, cross-context decisions, navigation
hanging, missed finite deadlines, or an unacceptable transport cost is a
stop-and-replan condition. The implementation must not hide that cost in CEF
or the kernel.

### Accepted early-gate evidence (T-004)

Run `T004-real-20260902-local-01` passed on the hosted Windows development
machine with the repository-pinned CEF runtime and the staged
`worldline_browser_provider_client.dll`. The named command was:

```text
cargo test -p worldline-reference --test request_policy_feasibility_real_acceptance -- --nocapture
```

The run exercised both a no-policy baseline and an explicitly configured
`t004-feasibility-profile` using `fail-open`; the provider negotiated
`browser.request-policy/v0.1` through `bootstrapc.exe`. The active path
reported 13 policy requests, 10 Allow decisions, 2 Block decisions, one
finite-deadline fallback, a declared concurrency/queue bound of 8, and a
maximum observed host-side in-flight count of 2. The host-side policy span
was p50 2.374 ms, p95 2.578 ms, and maximum 2.613 ms. The no-policy page
load was 287 ms and the active page load was 566 ms; the active fixture's
deliberately slow resource accounts for the controlled fail-open deadline
probe, so this page-load delta is a boundary-cost indicator rather than an
isolated transport-only estimate.

All 13 active decisions reached a terminal callback outcome, including two
CEF cancellations, the blocked resource produced no loopback-origin hit, the
slow resource continued under the profile's FailOpen fallback, and the page
completed without a navigation hang. The real report marked `accepted: true`.
The deterministic reference-only fixture remains separate evidence and is
not used for this real-CEF claim. This is the T-004 stop/replan decision:
the bounded per-request transport is accepted for the narrow M1.3 slice, with
the declared limits and profile-specific failure semantics frozen for the
following tasks; no policy rule engine is moved into CEF or the kernel.

### S3C proving evidence (T-006)

The deterministic S3C acceptance is local contract/broker evidence and is
kept separate from the hosted browser claim:

```text
cargo test -p worldline-reference --test s3c_acceptance
```

It proves zero origin hits for the matching blocked resource, one origin hit
for the allowed resource, exact scope rejection, replacement and lifecycle
cleanup, profile-scoped FailOpen timeout/unavailable outcomes, and URL-free
post-outcome observations.

Named hosted run `S3C-real-20260902-local-01` used:

```text
cargo test -p worldline-reference --test s3c_real_acceptance -- --nocapture
```

with `CEF_PATH`, `WORLDLINE_BROWSER_PROVIDER_BOOTSTRAP`, and
`WORLDLINE_BROWSER_PROVIDER_CLIENT` pointing at the staged repository-pinned
runtime/client. The real report was:

```text
real CEF 151.8.0 -> native provider process -> negotiated browser.request-policy/v0.1
blocked_origin_hits=0, allowed_origin_hits=1, page_usable=true
exact_scope_isolated=true, replacement_isolated=true, lifecycle_cleanup=true
fail_open_timeout=true, fail_open_unavailable=true, safe_observations=true
accepted=true
```

The hosted slice ran a no-policy baseline, an explicit
`worldline.browser.adblock.profile.v0` with `FailOpen`, and a fresh empty
replacement profile. The loopback origin was the only network dependency;
the blocked path never reached it, while allowed, unavailable, and
deliberately slow resources remained usable under the declared fallback.

### Architecture and CI guard decision (T-007)

`Test-WorldlineArchitecture.ps1` now checks the request-policy dependency
direction, engine-neutral contract, adblock/CEF separation, service-domain
isolation, bounded native transport declarations, profile-configurable
failure semantics, and preservation of the existing architecture checks.
`Invoke-WorldlineCi.ps1 -Suite BrowserRequestPolicy` is a required Windows
suite: it runs deterministic and real request-policy/S3C tests and fails when
the staged CEF variables are absent. Local deterministic results and
Windows-hosted real-CEF results remain distinct evidence classes.

## Compatibility and non-goals

The stable Browser Contract v1 and existing experimental engine primitives
remain additive and compatible. This slice does not implement full
uBlock/AdGuard syntax, cosmetic or DOM filtering, scriptlets, response
rewriting, arbitrary header mutation, redirects beyond allow/block, malware
policy, subscription synchronization, UI, DevTools, Search, or complete M1.3
closure.

The existing S0, M1.1/M1.2, downloads/cookies reality, and real S3B gates are
permanent regression gates. Deterministic loopback tests and hosted Windows
real-CEF tests are reported as separate evidence classes; a reference backend
cannot satisfy the real-CEF proving claim.
