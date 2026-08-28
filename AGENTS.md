# Worldline GRACE 4 Engineering Protocol

## Keywords

Rust, plugin runtime, capability security, event transport, browser engine, agent runtime, UI composition, userland Internet environment

## Annotation

Worldline is a userland operating environment for the Internet. The Rust kernel
owns generic lifecycle, capabilities, messaging, scheduling, persistence
boundaries, and composition contracts; browser, agent, UI, memory, search, and
integration behavior is supplied by plugins.

## Sources of truth

This repository uses the GRACE 4 `.grace` artifact model as its engineering
control plane.

- Product and technical context: `.grace/context/*.xml`
- Current module projection: `.grace/graph/index.xml` and its routed documents
- Current verification projection: `.grace/verification/index.xml` and its routed documents
- Active changes: `.grace/changes/active/C-*/spec.xml` and `plan.xml`
- Terminal changes: `.grace/changes/archive/C-*/*`
- Long-lived architecture decisions: `docs/adr/*.md`
- CI/CD engineering policy: `docs/CI-CD.md`
- Product direction and milestone gates: `ROADMAP.md`
- Runtime truth: the implementation and fresh execution evidence

GRACE owns workflow truth, scope, acceptance intent, and durable engineering
projections. It does not replace runtime state, ADRs, the roadmap, or test
evidence. Legacy `docs/*.xml` files are not GRACE 4 state.

## Required reading order

Before an implementation change, read:

1. `AGENTS.md`
2. `ROADMAP.md` sections relevant to the change
3. relevant `docs/adr/*.md` and `docs/CI-CD.md` when applicable
4. `.grace/context/*.xml`
5. the graph module routed by `.grace/graph/index.xml`
6. the matching verification entry routed by `.grace/verification/index.xml`
7. the active change's approved `spec.xml`
8. the same change's separately approved `plan.xml`

For GRACE mechanics, start at `docs/grace/README.md`. Vendored upstream skills
live under `.agents/skills/grace/`; their provenance is recorded in
`.agents/GRACE_UPSTREAM.md`.

## Change lifecycle

1. Create one draft `GraceChangeSpec` under `.grace/changes/active/C-*`.
2. Obtain explicit approval of the spec. Spec approval does not approve a plan.
3. Create a draft `GraceChangePlan` from the approved spec and current durable state.
4. Obtain explicit approval of that exact plan.
5. Treat an approved plan as immutable. Conflicts require supersession and replanning.
6. Execute only dependency-ready tasks within `ObservedWriteScope`.
7. Apply graph and verification deltas centrally after implementation evidence passes.
8. Run final evidence, request explicit apply confirmation, then mark applied and archive.

Production behavior, CI gates, security policy, persistent schema, and release
semantics require an approved bundle. A user may explicitly authorize a tiny
docs-only or mechanical direct fix, but that exception must not be inferred.

## Admission contract

Before observed writes, establish:

- operation ID and authority mode: `codex_led`, `external_direct`, or `parallel_mixed`;
- operation root and, when delegated, technical owner;
- branch/worktree isolation and dirty-tree coexistence facts;
- change, task, module, and verification IDs;
- exact allowed and forbidden write scope;
- test owner, acceptance owner, rollback boundary, and stop conditions.

Workers do not mutate approved plans or shared `.grace/context`, graph,
verification, or active bundle artifacts. The operation root reconciles those
shared artifacts after reviewing the implementation result.

## Worldline invariants

Every change must preserve these current project invariants unless an explicit
approved architecture change replaces one:

1. Nothing above the kernel is special; product capabilities come from plugins.
2. The kernel contains no browser-, model-, provider-, workspace-, or UI-specific domain policy.
3. Capability authority is default-deny and is checked at every privileged boundary.
4. **EVENT BUS IS NOT RPC.** Event delivery cannot determine or replace an RPC result.
5. Events are not automatically persistence; event sourcing is opt-in per bounded domain.
6. Installation identity and durable state outlive runtime identity and runtime authority.
7. S0 and every promoted proving slice remain permanent end-to-end acceptance gates.
8. Scaffolded, wired, and verified are distinct evidence states.

## Verification and status language

- `scaffolded`: files or types exist, but the real path is not established.
- `wired`: the real product path reaches the authoritative owner and consumes the result.
- `verified`: the exact claim is wired and supported by named fresh evidence.

Do not claim that CI works without a named CI run. A local run is local
evidence. Do not promote an integration test to device or manual-product
evidence. Hypotheses and unresolved blockers survive handoff explicitly.

Protected acceptance tests are owned independently from implementation work.
Do not weaken, delete, skip, or rewrite a failing invariant test to make a
change green unless the approved spec explicitly changes that invariant and
the designated test owner accepts the replacement.

## Semantic anchors

GRACE anchors are attribute-free XML tags: `<M-EXAMPLE />`, not
`<Module ref="M-EXAMPLE" />`. Use `M-*` for modules, `GD-*` for graph
documents, `V-M-*` for verification entries, `VD-*` for verification
documents, `C-*` for changes, and `T-NNN` for plan tasks.

## Commands

```text
grace lint --path . --assertions current
grace lint --path . --parallel-preflight
grace status --path . --with modules --json --fail-on errors
```

Project verification commands are declared in `.grace/verification` and in
the selected change plan. GRACE lifecycle commands remain outer gates; do not
nest them inside `MustPassCommand`.

## Completion packet

Every implementation result reports:

- operation ID, authority mode, change ID, task IDs, module and verification IDs;
- branch/worktree and dirty-tree coexistence status;
- files read and exact files changed;
- scope delta or an explicit no-delta statement;
- exact commands and results;
- scaffolded, wired, and verified claims separately;
- unrun checks, unresolved gaps, remaining risks, and rollback notes.
