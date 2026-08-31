# ADR-OPERABILITY-COMPATIBILITY-UPGRADE-V1: Operability, Compatibility and Upgrade Model

- **Status:** Accepted
- **Date:** 2026-08-31
- **Deciders:** Worldline Architecture Team
- **Milestone:** M0.7 (Operability, compatibility and upgrade)
- **Consults:** ADR-KERNEL-BOUNDARY-V1, ADR-PLUGIN-RUNTIME-V1, ADR-CAPABILITY-RPC-EVENT-TRANSPORT-V1, ADR-PERSISTENCE-RECOVERY-V1, ADR-EXTERNAL-PLUGIN-BOUNDARY-V1

---

## 1. Context and Problem Statement

Worldline is a userland operating environment for the Internet where browser, agent, UI, and storage capabilities are supplied by replaceable plugins above a minimal Rust kernel. Through milestones M0.1–M0.6, Worldline established state hardening, kernel boundary invariants, split-phase plugin runtime lifecycles, capability RPC, typed event transport, crash-safe persistence/recovery, and stable external WASM/native boundaries.

However, a robust operating platform must safely evolve across time:
1. It must determine whether kernel, SDK, and plugin contracts are compatible before activation.
2. It must reject incompatible upgrades before switching active provider authority.
3. It must stage new package revisions separately from currently active versions.
4. It must execute state migrations on copies before modifying active state.
5. It must validate staged runtime health within bounded deadlines and budgets.
6. It must atomically switch active revisions and recover deterministically after crash.
7. It must rollback to a Last-Known-Good revision with fresh runtime identity and clean authority.
8. It must localize broken plugins via persistent quarantine and security-preserving safe mode.
9. It must isolate offending revisions through bounded automated bisect.
10. It must observe operational telemetry without leaking private RPC, event, or state payloads.
11. It must reconstruct causal chains (`Admission -> Selection -> Invocation -> State -> Observation -> Lifecycle`) without timestamp guessing.
12. It must represent unprovable post-crash external side effects explicitly as `Incomplete`, rejecting synthetic success or automatic unsafe retries.
13. It must restrict event replay strictly to domains that explicitly opted into event sourcing.

---

## 2. Contract Stability Model

Capability and plugin contracts follow two distinct stability classes:

### 2.1 Stability Classes

1. **`Stable`**:
   - Follows semantic versioning and backward compatibility promises across supported major lines.
   - Minor releases add backward-compatible optional capabilities/fields without breaking existing consumers.
   - Breaking semantic changes require a new major version line.
   - Supported SDK/kernel baselines maintain an N / N-1 / N-2 compatibility matrix.

2. **`Experimental`**:
   - May change incompatibly between minor or pre-1.0 platform releases.
   - Compatibility guarantee: exact supported range only (no N-2 promise).
   - Incompatibility MUST still be detected and rejected deterministically prior to activation.

### 2.2 Invariants

- **Stability class is metadata of a contract version line**, not of a provider implementation.
- A provider cannot self-declare an incompatible implementation as `Stable` to bypass compatibility enforcement.
- Compatibility classification never confers capability authority; capability broker authorization remains independent and default-deny.

---

## 3. Compatibility Rules and Negotiation

1. **Stable Major**: Different major versions are incompatible unless an explicit registered adapter contract exists.
2. **Stable Minor**: Minor evolution allows backward-compatible optional semantics according to declared negotiation rules.
3. **Patch**: Patch versions preserve semantic equivalence.
4. **Machine-Readable Metadata**: Compatibility decisions are strictly evaluated from structured contract metadata (`InterfaceVersion`, `AbiVersion`, `ContractStability`, declared capability interfaces), NEVER from package filenames, plugin naming prefixes, or lexical guessing.
5. **Observable Negotiation**: Negotiated contract versions and provider selection explanations are recorded in diagnostic metadata.

---

## 4. Supported SDK/Kernel Matrix

Worldline maintains an executable compatibility matrix:
- **Current Kernel vs SDK N**: Fully supported (primary development baseline).
- **Current Kernel vs SDK N-1**: Supported for designated Stable contracts.
- **Current Kernel vs SDK N-2**: Supported for designated Stable contracts.
- **Current SDK vs Supported Kernel Baselines**: Cleanly rejects unsupported historical kernel baselines before runtime provider publication.

Versioned compatibility fixtures and golden manifests are stored directly in the repository to prevent regression.

---

## 5. Package Revision Model

To avoid conflating package distributions, installations, and runtime instances, Worldline defines four distinct identity levels:

1. **`PluginDefinitionId`**: Logical plugin definition identifier.
2. **`InstallationId`**: Durable installation identity that remains stable across compatible package updates.
3. **`PackageRevisionId`**: Content-addressed or deterministic cryptographic digest identifying an immutable installed package artifact revision.
4. **`InstallationRevision`**: Durable association of an `InstallationId` with an active `PackageRevisionId`, state revision, and schema version.
5. **`RuntimeId`**: Ephemeral execution attempt identifier (`incarnation` + `sequence`).

### Rules:
- A `RuntimeId` always belongs to exactly one `InstallationRevision`.
- Upgrading or rolling back creates a **fresh `RuntimeId`** with **fresh live authority**.
- Old opaque handles, capability grants, and state leases from previous runtimes are invalidated and never inherited.

---

## 6. Staged Upgrade Protocol and State Machine

Upgrades are transactional state transitions:

```text
Current ──► Staging ──► MigratingCopy ──► Validating ──► ReadyToSwitch ──► Switching ──► CurrentCandidate
   │           │               │              │               │               │                │
   │           ▼               ▼              ▼               │               ▼                ▼
   │     Compatibility-      Failed         Failed            │            Rollback        Rollback
   │        Rejected           │              │               │            Triggered       Triggered
   │           │               │              │               │               │                │
   │           ▼               ▼              ▼               ▼               ▼                ▼
   └───────────────────────────────► Previous Current ◄───────────────── RollingBack ──► RolledBack
```

### Invariants:
1. Only **one revision is authoritative `Current`** for any `InstallationId` at any point in time.
2. Staging a revision does not mutate the active package artifact, active state, or active provider publications.
3. A staged revision has **zero active provider authority** before the active revision switch commits.
4. Switch metadata is crash-recoverable.

---

## 7. Migration-on-Copy

1. **State Isolation**: When upgrading an installation, state migration runs exclusively against an isolated copy or snapshot of the existing state.
2. **Rollback Source Preservation**: The original Last-Known-Good state remains intact until the new revision passes health validation and the switch commits.
3. **Migration Provenance**: Every migration records:
   - `source_revision`
   - `target_revision`
   - `source_schema`
   - `target_schema`
   - `migration_path`
   - `result` (Success, Error, Duration, Timestamp)
4. **Failure Invariant**: A failed staged migration leaves the current installation state completely untouched.

---

## 8. Health Validation

Before an active revision switch, the staged runtime undergoes bounded health validation:
1. Manifest and ABI compatibility verification.
2. State migration validation on copy.
3. Runtime activation trial in an isolated harness.
4. Required capability publication checks.
5. Required dependency resolution checks.
6. Bounded health probe with explicit execution deadline and memory/CPU resource budget.
7. Trap, crash, and quota violation detection.

Passing health validation grants no authority; failure preserves the current active revision.

---

## 9. Atomic Active Revision Switch and Crash Recovery

1. **Single Durable Decision**: Switching active revisions writes a single atomic committed active-revision decision in durable storage.
2. **Crash Before Commit**: Restores the previous `Current` revision on restart.
3. **Crash After Commit**: Restores the new `Current` revision on restart.
4. **Crash During Switch**: Resolves to exactly one valid revision; never two simultaneous `Current` revisions.
5. **Single-Provider Semantics**: Capability registry never exposes both old and new revisions simultaneously.

---

## 10. LastKnownGood Rollback & Post-Switch Observation Window

1. **Durable Last-Known-Good**: The host maintains a persistent `LastKnownGoodRevision` pointer and compatible state snapshot.
2. **Rollback Protocol**:
   - Restores the Last-Known-Good package revision and state snapshot.
   - Clears all live authority, subscriptions, and provider publications of the failed candidate.
   - Spawns a fresh `RuntimeId` with clean, default-deny authority.
3. **Post-Switch Observation Window**:
   - After a technically successful switch, an observation window monitors for immediate runtime failures (activation crashes, restart loops, capability disappearance, health probe failures, resource exhaustion).
   - Configurable thresholds trigger automatic rollback to Last-Known-Good if unrecoverable failures occur.

---

## 11. Safe Mode and Persistent Quarantine

### 11.1 Persistent Quarantine
- **Triggers**: Repeated activation failures, repeated crashes, repeated WASM traps/resource violations, protocol violations, post-upgrade health failures.
- **Persistence**: Quarantine records (`installation_id`, `package_revision_id`, `reason`, `timestamp`, `originating_revision`) persist across host restarts.
- **Behavior**: Quarantined plugins are excluded from automatic activation, but their persistent state remains intact for administrative inspection, repair, or uninstallation.

### 11.2 Safe Mode
- **Purpose**: Boot a minimal known-good composition when normal composition cannot achieve health.
- **Suppression**: Excludes optional third-party plugins while booting core platform infrastructure.
- **Security Invariant**: Safe mode **reduces composition**; it **NEVER bypasses capability authorization**.

---

## 12. Bounded Automated Bisect

When composition fails after multi-plugin updates or complex dependency interactions, the automated bisect engine isolates the culprit:
1. Operates over a bounded candidate set of optional installations.
2. Preserves required core platform composition.
3. Executes bounded activation trials with explicit timeouts and budgets.
4. Returns deterministic outcomes:
   - `LikelyCulprit(InstallationId)`
   - `MultipleCandidates(Vec<InstallationId>)`
   - `Inconclusive` (when failures stem from complex multi-plugin interactions).

---

## 13. Operational Telemetry (Metrics)

To diagnose plugin health without compromising user privacy or security:
- **Metrics Collected Per Runtime**:
  - Activation and deactivation duration
  - Restart, crash, hung, and quarantine counts
  - RPC in-flight requests and queue depths
  - Event mailbox depth, delivery latency, and dropped events
  - Authorization denials count
  - Memory usage and CPU budget consumption
- **Privacy & Security Invariant**: Operational metrics contain **strictly metadata and numeric counts**, NEVER raw private RPC arguments, event payloads, or stored state values.
- Metrics are diagnostic observations and do not become security decisions unless consumed by explicit policy.

---

## 14. Diagnostic Causality (Time-Travel Queries)

Given any `EventId`, `InvocationId`, `RuntimeId`, or `CorrelationId`, the diagnostic subsystem reconstructs the exact causal graph:
`Admission -> ProviderSelection -> Invocation -> State/OutboxMutation -> Observation -> Lifecycle/Crash -> FollowUpAction`.

### Invariants:
1. Causality is derived strictly from recorded `correlation_id` and `causation_id` links, NEVER inferred from timestamp proximity.
2. Missing or pruned events are explicitly rendered as diagnostic gaps.
3. Diagnostic queries enforce caller permissions and redact sensitive contents.

---

## 15. Incomplete External Outcomes

For external side effects (e.g. network requests, hardware operations, third-party API mutations) interrupted by host crash or disconnection:

### States:
- `NotStarted`
- `CommittedLocal`
- `Succeeded`
- `Failed`
- `Incomplete`

### Semantics of `Incomplete`:
- Indicates durable evidence that an external action was dispatched, but post-crash evidence cannot prove whether the remote side effect occurred.
- `Incomplete != Failed` and `Incomplete != Succeeded`.
- `Incomplete` operations are **NEVER automatically retried** unless the capability contract formally proves idempotency.

---

## 16. Replay Policy

- **No Universal Replay Engine**: Worldline deliberately rejects building a global event-sourced replay engine for all domains.
- **Opt-In Event Sourcing**: Deterministic replay is available only for domains that explicitly selected event sourcing via an approved ADR.
- **Authoritative Recovery**: Installation state, transactional outbox, and background jobs recover from their authoritative relational stores (`worldline-storage`).

---

## 17. Cordis-rs Lineage and Deliberate Divergence

Worldline acknowledges Cordis-rs as a conceptual ancestor in its lifecycle and effect ownership model:

1. **Lineage Retained**:
   - Managed split-phase lifecycle (registration, dependency waiting, activation, deactivation).
   - Per-scope effect ownership with deterministic LIFO cleanup.
   - Compositional runtime structure.

2. **Deliberate Divergences**:
   - **Contract Stability & Upgrade**: Cordis-rs treats upgrades as dynamic in-memory replacement; Worldline enforces transactional package revisions, migration-on-copy, health validation, and crash-safe atomic switches.
   - **External Boundaries**: Cordis-rs relies on Rust in-process dynamic types (`Arc<dyn Any>`); Worldline enforces versioned WIT Component interfaces, least-authority WASI sandboxes, and native IPC envelopes.
   - **Authority & Security**: Cordis-rs has ambient trust within process; Worldline enforces default-deny capability broker authorization with runtime-scoped opaque handles.
   - **Persistence & Recovery**: Cordis-rs effects are purely in-memory; Worldline binds state and outbox to durable SQLite transactions and failpoint-tested crash recovery.
   - **Event Bus Separation**: Cordis-rs conflates event bus with messaging; Worldline enforces the strict invariant: **EVENT BUS IS NOT RPC**.

---

## 18. Rejected Alternatives

1. **In-Place Package File Overwrite**: Rejected because crashes during file copy leave corrupted installations, breaking rollback guarantees.
2. **Reverse Migration on Rollback**: Rejected because reverse migrations are lossy, untested, and fragile; snapshot-on-copy preserves known-good state.
3. **Lexical Version Guessing**: Rejected because string comparisons fail on custom tags and ignore actual ABI/capability compatibility.
4. **Automatic Retry of Interrupted External Actions**: Rejected because retrying non-idempotent side effects causes duplicate payments, messages, or external mutations.
5. **Universal Event-Sourcing for All Subsystems**: Rejected due to unbounded storage growth, schema migration complexity, and impedance mismatch with relational workspace state.

---

## 19. Reopen Conditions

This ADR may be reopened only if:
1. A new hardware or OS capability mechanism provides lower-overhead atomic state snapshots without copy overhead.
2. A formal package registry standard is introduced during milestone M3 requiring distributed multi-node upgrade consensus.
