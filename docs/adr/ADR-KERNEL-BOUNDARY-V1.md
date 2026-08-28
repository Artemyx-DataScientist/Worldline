# ADR-KERNEL-BOUNDARY-V1: Kernel Boundary and Reference Composition

- Status: accepted for M0.2
- Date: 2026-08-28
- Scope: `worldline-kernel`, `worldline-reference`, and the S0 host proving slice
- Next gate: M0.3 Plugin Runtime v1

## Context

Worldline is intended to be a modular userland environment for the Internet.
Browser engines, agent providers, model adapters, renderers, and UI surfaces
must be replaceable plugins. The kernel is a trust boundary, not a product
shell.

The current bootstrap is synchronous and native Rust. It already provides
plugin lifecycle, capability publication and resolution, invocation
authorization, lifecycle-scoped effects and grants, installation-owned state,
schema migration, and an append-only trajectory. This decision tests whether
those mechanisms are genuinely generic by composing three structurally
different reference families:

- browser-like: opaque navigation request/result and navigation observation;
- agent-like: a reasoning-shaped capability consumer/provider;
- UI-like: opaque surface and command capabilities.

The reference implementations are architectural probes. They are deliberately
small and are not browser, agent, renderer, scheduler, or production event
implementations.

## Architectural invariants

1. NOTHING ABOVE THE KERNEL IS SPECIAL.
2. EVERYTHING IS A CAPABILITY PROVIDED BY A PLUGIN.
3. NOTHING GETS AUTHORITY MERELY BECAUSE IT IS A PLUGIN.
4. PRODUCT ROLE DOES NOT CREATE PRIVILEGE.
5. EVENT TRANSPORT IS NOT RPC.
6. PERSISTENCE IS NOT IMPLICIT EVENT SOURCING.
7. A kernel primitive exists to enforce an invariant, not merely because it is
   convenient to implement there.
8. Installation state continuity does not imply runtime authority continuity.
9. Reference families must exercise the same logical platform contract.

## Current kernel responsibilities

The kernel currently owns only generic mechanisms:

- trusted principal identity registration and runtime-principal allocation;
- installation identity and the binding between installation, state, and a
  runtime activation;
- lifecycle scopes, owned-effect cleanup, and lifecycle grant revocation;
- capability contract publication, compatibility resolution, and provider
  selection;
- the point-to-point invocation broker, admission-time authorization, causal
  nesting limit, ProviderSelf validation, and provider dispatch;
- the kernel-managed state boundary, transaction/CAS semantics, schema
  metadata, migration execution, and uninstall lifecycle;
- append-only trajectory metadata that excludes raw payloads and state values.

The kernel does not contain browser, agent, model, prompt, page, document,
workspace, panel, renderer, or other product-domain entities.

## Candidate boundary classifications

The classification rule is evidence-based: a KernelPrimitive must protect a
system invariant that cannot be enforced by an ordinary plugin. A
MandatorySystemPlugin is a replaceable implementation required by a selected
boot composition. An OpenQuestion is not silently promoted into the kernel;
it has a named spike and kill test.

| Mechanism | Classification | Invariant requiring it | Current evidence / next action |
| --- | --- | --- | --- |
| Principal identity registry | KernelPrimitive | Authority must bind to trusted subjects rather than names or roles | `SecurityStore` owns registration and existence checks; future process/WASM identity boundary may reopen the representation |
| Installation identity | KernelPrimitive | Persistent state belongs to an installation, not a runtime or `PluginId` | `InstallationId` and `InstallationRecord` are kernel-owned; multiple records per logical plugin are supported |
| Runtime identity allocation | KernelPrimitive (transitional) | Runtime authority must be ephemeral and bound to exactly one installation | Kernel allocates `PluginRuntime` principals and revokes them on lifecycle termination; final `RuntimeId` registry is an M0.3 reopen |
| Lifecycle scopes | KernelPrimitive | Effects and lifecycle grants must be reclaimed on deactivation, crash, and replacement | `LifecycleScope` cleanup and grant revocation are generic and family-independent |
| Capability contract registry | KernelPrimitive | Providers must be resolved by compatible contract, not product identity | `CapabilityRegistry` stores generic `CapabilityId` contracts and provider principals |
| Provider resolution | KernelPrimitive for resolution; selection policy is OpenQuestion | A capability invocation has one provider and one result; subscribers cannot become responders | Deterministic current selection works for the proving slice; explicit policy/explanation is an M0.3 spike |
| Invocation broker | KernelPrimitive | Admission, provider dispatch, causal depth, and one request/result path need one trusted arbiter | `InvocationBroker` authorizes before dispatch, enforces depth, and never searches an event stream for a result |
| Authority enforcement | KernelPrimitive | Availability and plugin role must not create authority; delegation must not widen or fall back | `SecurityStore` evaluates grants, ProviderSelf, delegated authority, revocation, and runtime lineage |
| State installation binding | KernelPrimitive | A runtime may access only the installation selected by the kernel | `RuntimeStateHandle` carries an unforgeable-in-context installation binding and a revocable lifecycle lease |
| State backend abstraction | KernelPrimitive contract; implementation OpenQuestion | Plugins must not receive ambient filesystem/database access and state commits need atomic/CAS semantics | `StateBackend` is hidden behind the public state API; in-memory backend is bootstrap-only, production persistence is a later spike |
| Event transport | OpenQuestion | Observations must not become hidden commands, RPC replies, or implicit persistence | S0 uses `worldline-reference::ObservationBus` as a host-local typed fixture; production bounded transport is M0.4 |
| Scheduler primitives | OpenQuestion | Persistent tasks need budgets, cancellation, and restart semantics without hidden execution authority | No scheduler exists yet; M0.3 async lifecycle spike and kill test: a hung task cannot block unrelated reconciliation |
| Trajectory/audit primitives | KernelPrimitive | Security and lifecycle decisions need append-only, payload-safe evidence | `Trajectory` is kernel-owned; state values and invocation payloads are never recorded |
| Blob persistence | OpenQuestion | Large opaque data needs explicit ownership, retention, and crash-consistency semantics | No blob API is exposed; persistence spike must compare blob store, state store, and external provider ownership |
| UI composition bootstrap | OpenQuestion with a future MandatorySystemPlugin candidate | UI composition must not add product entities or privilege to the kernel | Current kernel only composes generic plugins; wgpu/CEF/window ownership spike decides the mandatory host surface |
| Plugin discovery/loading | OpenQuestion | Loading and update policy must preserve identity, provenance, sandbox, and rollback boundaries | Current public path registers an already supplied `Plugin`; install/sign/loader policy is a future system/plugin boundary spike |

Every current KernelPrimitive above is connected to an enforceable invariant.
No browser, agent, or UI role is used as an authorization input.

## Decision

Keep the current kernel boundary generic and add no family discriminator or
product-specific lifecycle path. Place the reference compositions in the
separate `worldline-reference` crate, whose dependency direction is:

```text
worldline-demo / worldline-reference -> worldline-kernel
worldline-kernel -X-> reference family crates
```

The three families use the same `PluginDefinition`, `ActivationContext`,
`CapabilityHandle`, `CapabilityService`, `RuntimeStateHandle`, installation
state transaction, and ordinary explicit grants:

| Family | Reference implementation | Generic contract exercised | Evidence |
| --- | --- | --- | --- |
| Browser-like | `BrowserLikeProvider` and `BrowserLikeConsumer` | opaque `CapabilityId`, provider result, installation state, independent navigation observation | `three_reference_families_use_one_generic_plugin_contract`, `observation_without_subscribers_still_leaves_rpc_semantics_unchanged` |
| Agent-like | `AgentLikePlugin` | ordinary capability dependency and provider publication; no role-based authority | the agent request is denied before its explicit grant in `three_reference_families_use_one_generic_plugin_contract` |
| UI-like | `UiLikeProvider` and `UiLikeConsumer` | opaque surface/command capabilities and generic consumer handle | the UI surface invocation succeeds only after its explicit grant in the same test |

The reference crate owns the minimal proving observation mechanism. An
`Observation` has its own identity, producer principal, topic, opaque payload,
and optional causation/correlation metadata. `ObservationBus` supports zero or
more subscribers, records facts separately from RPC, continues after a
subscriber error, and returns delivery diagnostics without changing the
producer's result. This is deliberately not a production event bus and does
not participate in provider resolution or authorization.

## S0 evidence

`worldline_reference::s0::run` is a host-facing, public-API proving slice:

```text
host boot
  -> shared in-memory StateBackend discovery
  -> browser-like installation/runtime activation
  -> consumer installation/runtime activation
  -> explicit caller grant
  -> point-to-point navigation RPC result
  -> independent navigation observation
  -> first host unregister/stop
  -> second Kernel over the same backend
  -> same installations registered again
  -> installation state continues from 1 to 2
  -> runtime principal changes
  -> old runtime grant is inactive
  -> new runtime has no inherited authority
  -> compatible consumer receives the replacement provider result
```

The persisted runtime-generation epoch is a transitional identity seed for
this pre-M0.3 proof. It ensures that a new host instance cannot reuse the
previous runtime principal string merely because its in-memory allocator was
reset. It is not the final `RuntimeId` model.

The acceptance tests also prove:

- a provider exists and RPC remains unauthorized until an explicit grant;
- zero subscribers do not affect an RPC result;
- multiple subscribers observe independently;
- a failing subscriber does not alter a successful RPC result or prevent the
  other subscriber from receiving the fact;
- an observation subscriber cannot make a missing provider available;
- reference implementations do not require forbidden kernel domain types.

## Rejected alternatives

### Browser/agent/UI family enum in the kernel

Rejected. A family discriminator would make product role part of privileged
policy and would force every future family through a special path. Generic
capability and lifecycle contracts are sufficient evidence for the current
slice.

### Browser, agent, or UI domain entities in `worldline-kernel`

Rejected. `Tab`, `Page`, `DOMNode`, `Agent`, `Model`, `Prompt`, `Workspace`,
`Panel`, and related entities belong to replaceable plugins or composition
hosts. Their presence would make the boundary depend on product semantics.

### Event bus as an RPC response channel

Rejected. The provider returns the RPC result directly through the invocation
broker. Observers receive a separate fact with a separate identity. A
subscriber cannot select a provider, replace a result, or grant itself
authority.

### Event publication as implicit persistence/event sourcing

Rejected. The state transaction remains the source of truth for installation
state. Event sourcing is a per-domain decision that needs its own ADR and is
not implied by having an observation transport.

### Role-based or availability-based authority

Rejected. Reference status, plugin naming, capability availability, and
provider publication do not create grants. The same explicit grant path is
used for browser-like, agent-like, and UI-like callers.

### Reference implementations inside the kernel

Rejected. Keeping the probes in `worldline-reference` makes dependency
direction and accidental privileged APIs visible in review. The kernel
dependency tree remains free of reference, browser, inference, and renderer
dependencies.

### Global singleton observation service

Rejected for this gate. The host-local bus is explicitly passed to reference
providers. A process-global bus would introduce ambient coupling and hide
ownership; production transport remains an OpenQuestion.

## Known unresolved questions

These are intentionally not solved by M0.2:

1. M0.3 must introduce a first-class `RuntimeId` and allow simultaneous active
   installations of one plugin definition without using `PluginId` as the
   runtime slot key.
2. Async lifecycle, cancellation, deadlines, hung-plugin detection,
   restart/backoff, quarantine, and partial activation are still absent.
3. Provider selection needs an explicit policy, compatibility negotiation, and
   an observable explanation once multiple providers are normal.
4. Production event transport needs versioned envelopes, bounded mailboxes,
   backpressure/QoS, ordering, retry, persistence, and subscriber isolation.
5. Scheduler, blob persistence, production disk state, crash consistency, and
   synchronization need separate contracts and failure models.
6. CEF/Chromium, wgpu, accessibility, window ownership, and UI composition may
   change which host surface is mandatory.
7. Native plugins are still trusted in-process code. WASM, IPC, capability
   mediation, package provenance, and public ABI are future security gates.

## Reopen conditions and kill tests

The boundary decision must be revisited when evidence triggers one of these
conditions:

| Trigger | Required spike | Kill test |
| --- | --- | --- |
| CEF or another browser engine needs a kernel-only type | embedding and ownership spike | browser provider cannot run without adding a browser entity to kernel |
| wgpu/window integration needs privileged UI semantics | UI composition spike | UI provider requires a kernel `Panel`/`TabBar` or gains authority by being the shell |
| WASM component boundary is added | identity/state/capability mediation spike | a component can access another installation or retain a revoked lease |
| async lifecycle is introduced | runtime transition and cancellation spike | a hung plugin blocks unrelated activation/deactivation or leaks a live effect |
| multiple providers become normal | provider-selection policy spike | an invocation has two providers, an unexplained choice, or an event subscriber response |
| durable observations are required | event transport spike | a slow/failing subscriber changes RPC success, provider availability, or command authority |
| production persistence is introduced | crash/CAS/recovery spike | restart loses committed state, accepts stale writes, or silently resets incompatible state |
| external loading/update is introduced | provenance/sandbox/rollback spike | an untrusted package receives ambient authority or update crosses installation identity |

An OpenQuestion may become a KernelPrimitive only when the spike demonstrates
that its invariant cannot be enforced at a replaceable boundary.

## Consequences for M0.3 Runtime v1

M0.3 can proceed on a boundary that is already exercised by three different
families. Its implementation work should preserve these contracts:

- `PluginDefinitionId -> InstallationId -> RuntimeId -> LifecycleScopeId` is
  explicit and no registry key conflates definition and runtime identity;
- runtime leases and authority are revoked on every terminal lifecycle path;
- provider replacement is expressed through generic capability contracts;
- state continuity is independent from authority continuity;
- event observation remains separate from RPC and persistence;
- new kernel APIs require a corresponding invariant and ADR evidence.

The reference families and S0 are permanent regression fixtures. They are not
throw-away demos: every later runtime, event, persistence, or WASM change must
keep this proving slice green or update this ADR with new evidence.
