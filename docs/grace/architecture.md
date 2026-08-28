# GRACE architecture

Worldline keeps four authorities separate:

```text
User intent
  -> GRACE spec and approved plan        workflow truth
  -> ADR and roadmap                     architectural and sequencing truth
  -> source plus runtime state           implementation truth
  -> verification and CI evidence        observed proof
```

A prose report cannot overwrite any of these owners. In particular, an
approved spec is not an approved plan, a passing local command is not a hosted
CI run, and generated files are not evidence that the real product path is
wired.

## Durable layer

`.grace/context` records stable product, technology, deployment, UX, and
engineering principles. `.grace/graph` is the routed current module
projection. `.grace/verification` is the matching routed evidence design.
Active and archived `C-*` bundles describe changes to those projections.

## Change layer

`spec.xml` is normative intent. Optional `design-context.xml` explains
rationale without adding requirements. `plan.xml` is the executable contract:
baseline, target, durable scope, observed write scope, acyclic tasks, and leaf
verification commands.

Once approved, the plan is immutable. A changed baseline, new write scope, or
different acceptance contract requires a superseding change instead of an
in-place edit.

## Runtime and CI boundary

GRACE does not become a runtime registry and CI does not become a second build
system. GitHub workflows call repository-owned commands. Event delivery,
capability authority, state ownership, and agent approvals remain runtime
contracts implemented by the kernel and plugins.

## Module adoption

Initial graph entries are intentionally coarse and evidence-backed. Split a
module only through an approved change when separate ownership and
verification are real. Never manufacture fine-grained graph detail ahead of
the implementation solely to make the bureaucracy look complete.
