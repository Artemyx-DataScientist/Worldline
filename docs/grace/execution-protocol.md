# GRACE execution protocol

## Safe profile

Worldline uses the safe GRACE profile by default. Before writes, the operation
root records the authority mode, isolation, dirty-tree coexistence, selected
change, approved artifacts, write scope, protected scope, test owner, rollback
boundary, and stop conditions.

## Admission gates

Implementation is admitted only when:

- one matching active spec and plan both have `status="approved"`;
- current and selected baseline lint passes before observed writes;
- observed and durable scopes are explicit;
- target commands are leaf project evidence, not nested GRACE lifecycle calls;
- task dependencies are acyclic and the selected task is ready;
- parallel work, if any, passes `grace lint --parallel-preflight`.

A draft plan is a deliberate stop condition, not administrative decoration.

## Execution

Run one dependency-ready task or one verified non-overlapping batch at a time.
Verify each task immediately. Do not edit approved XML to match unexpected
writes. Unknown drift stops execution until it is explained, reverted by its
owner, or handled through a superseding plan.

Shared context, graph, verification, and change artifacts are reconciled by
the operation root after implementation review; delegated workers return
proposed deltas instead of editing them directly.

## Recovery

| Observed state | Required action |
|---|---|
| Clean to start | Run selected baseline, then execute. |
| Partial declared writes | Inspect and explicitly resume or revert. |
| Durable GRACE state changed | Stop, supersede, and replan. |
| Target already satisfied | Re-run final evidence and reconcile before apply. |
| Unknown or out-of-scope drift | Stop and report exact files. |

## Acceptance

After tasks and durable reconciliation, run selected target commands and outer
final GRACE lint. Report local, CI, device, and manual evidence without
promotion. Only explicit apply confirmation permits setting the bundle to
`applied` and moving it to archive.
