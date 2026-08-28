# GRACE review checklist

## Integrity

- All canonical context artifacts use `graceVersion="4.0"`.
- Graph and verification indexes route every owned anchor exactly once.
- Every current module has meaningful deterministic verification or an explicit planned exception.
- Active bundle directory, wrapper ID, spec ID, and plan ID match.
- Spec and plan statuses reflect distinct explicit approvals.
- Approved assertions, scopes, and tasks were not mutated during execution.

## Worldline architecture

- Kernel dependencies still point outward to no browser, agent, UI, or workspace domain.
- Capability authorization remains default-deny at privileged boundaries.
- Event transport is not used as request/response control flow.
- Event publishing or subscriber failure cannot change an RPC result.
- Persistence choices are explicit; no domain acquires event sourcing by implication.
- Installation state and runtime authority retain separate lifetimes.
- The earliest affected proving slice remains runnable and permanent.

## Scope and evidence

- Actual writes are a subset of `ObservedWriteScope`.
- Unrelated dirty-tree files are named and left untouched.
- Protected tests were not weakened or skipped by the implementer.
- Commands are exact, fresh, deterministic, and run from the documented cwd.
- Local evidence is not described as hosted CI, device, or manual-product evidence.
- Scaffolded, wired, and verified claims are separated.
- Unrun checks, unresolved blockers, and rollback risks are explicit.
