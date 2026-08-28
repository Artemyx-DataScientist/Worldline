# GRACE in Worldline

Worldline uses a full GRACE 4 control layer with phased module adoption. The
canonical state is `.grace/`; the repository-local upstream workflows live in
`.agents/skills/grace/`; Worldline-specific rules live in `AGENTS.md` and this
directory.

## Current state

| Surface | State |
|---|---|
| GRACE v4.0.4 skills | vendored |
| Context, graph, verification | bootstrapped |
| Kernel and proving-slice modules | projected |
| CI/CD policy | documented in `docs/CI-CD.md` |
| `C-INFRA-CI-BASELINE-20260828` spec | approved by the user's 2026-08-28 instruction |
| `C-INFRA-CI-BASELINE-20260828` plan | draft; implementation is not admitted until separate approval |

## Boundaries

GRACE owns engineering workflow truth: change intent, explicit scope, task
dependencies, durable graph and verification projections, and acceptance
evidence. It does not own runtime truth. ADRs own long-lived architectural
decisions; `ROADMAP.md` owns product sequencing; `docs/CI-CD.md` owns CI/CD
policy; implementation and fresh runs own behavioral evidence.

## First commands

```powershell
grace lint --path . --assertions current
grace status --path . --with modules --json --fail-on errors
```

For a new change:

1. use the vendored `grace-spec` workflow to create a draft spec;
2. obtain explicit spec approval;
3. use `grace-plan` to create a draft plan from current state;
4. obtain separate approval of that exact plan;
5. execute with `grace-execute` and preserve its recovery gates;
6. apply durable projections centrally only after fresh evidence;
7. request explicit apply confirmation before archive.

See `architecture.md`, `execution-protocol.md`, and `review-checklist.md` for
the local interpretation.
