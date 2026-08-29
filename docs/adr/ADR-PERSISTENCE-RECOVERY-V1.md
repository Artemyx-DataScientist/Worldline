# ADR: Persistence and Recovery Model v1

Статус: accepted
Change: C-KERNEL-PERSISTENCE-RECOVERY-MODEL-20260828
Рубеж: M0.5 — Persistence and Recovery Model

## Решение

Worldline вводит гибридную persistence boundary для single-host profile.
Каждый bounded domain имеет одну authoritative source of truth. Persistence
semantics принадлежат generic kernel contracts, а storage implementation
остаётся host-side заменяемым adapter-ом.

### Persistence domains

| Domain | Source of truth | Не является source of truth |
| --- | --- | --- |
| InstallationState | Transactional State Store | Event journal, audit, derived index |
| InstallationMetadata | Transactional State Store | Runtime registry or trajectory |
| NotificationIntent | Transactional Outbox | Subscriber mailbox or event history |
| DurableEventTransport | Event Journal | Installation/application state |
| Audit | Selective append-only Audit Store | Current state or authorization |
| Blob | Content-addressed Blob Store | BlobId authority or metadata row |
| ScheduledJob | Persistent Job Store | Event transport or in-memory queue |
| DerivedIndex | None; rebuild from authoritative state | Any persisted projection |

Event sourcing is not selected for any domain in this change. The state store
needs current mutable state and CAS rather than global replay; the outbox needs
delivery intent and status; the journal needs transport replay; audit needs
selective control-plane facts; blobs need immutable content identity; jobs need
explicit work state. Existing event transport does not justify turning any of
these domains into an event-sourced database. A future bounded domain may opt in
only through a separate ADR with replay, retention, and fork semantics.

## Source-of-truth and contract boundary

The kernel owns opaque, database-neutral contracts:

- StateBackend owns installation records, opaque key/value state, revisions,
  CAS and migration-visible metadata transitions.
- Transactional state and outbox contracts carry already-admitted notification
  intent without exposing storage handles.
- EventJournal owns accepted durable transport envelopes and stable cursors.
- Audit, blob, job, backup and format contracts expose structured values and
  explicit errors only.

Plugins receive handles and capability results, never SQLite connections,
database transactions, profile roots or raw filesystem handles. The kernel does
not depend on rusqlite, SQLite schema names, or product persistence types.

## Kernel versus host/system boundary

The host constructs a production backend from trusted profile configuration and
passes it to Kernel::with_state_backend. The production backend is in the
worldline-storage crate and depends on worldline-kernel; the dependency
direction is:

    host configuration -> worldline-storage -> worldline-kernel contracts

The kernel's Kernel::new remains an explicit in-memory bootstrap for fast unit
tests. Production boot must call the fallible storage constructor and surface
open/corruption/durability errors. It must never replace a failed production
open with InMemoryStateBackend.

## Production backend selection

SQLite is selected as the initial production metadata backend because it gives
local atomic transactions, unique constraints, durable cursors, and a small
operational surface while keeping the implementation replaceable. The
worldline-storage crate hides all SQLite-specific types.

The connection is opened with the following explicit configuration:

    PRAGMA foreign_keys = ON;
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = FULL;
    PRAGMA busy_timeout = 5000;

The database is accepted only when the requested journal mode is confirmed.
The storage format version is stored independently from Worldline, plugin,
capability, and installation state schema versions. A newer format fails closed
with UnsupportedStorageFormat; no automatic destructive downgrade exists.

The profile root is host configuration, created and canonicalized before
opening. Plugins cannot select it. Blobs live below a separate content-addressed
directory under that configured root.

## Transaction and durability semantics

State mutations use one SQLite transaction guarded by the installation's
expected StateRevision. The transaction validates the current revision, writes
the complete replacement key set and metadata, and increments the revision
exactly once. Metadata transitions use the same CAS rule. A successful commit
acknowledgement is returned only after SQLite has committed with
synchronous=FULL under the supported local-filesystem assumptions.

Before commit acknowledgement, a process kill may expose either the previous
committed snapshot or the new fully committed snapshot if the database commit
already crossed its durability boundary. It may not expose a torn logical
transaction. Transaction identity is process-local and is never resurrected
after reopen.

Supported assumptions are a local filesystem whose SQLite locking, WAL and
flush guarantees are honored, a profile root owned by one Worldline profile,
and no concurrent unsupported file mutation. Network filesystems, manual
database copying while active, disk/controller failure, and filesystem
corruption are explicit unsupported or surfaced-failure cases.

## Transactional outbox

The outbox is stored transactionally with the state mutation whenever the
notification is required to survive the same commit. A record contains an
outbox identity, the original EventId, event contract/version, trusted
producer identity, correlation and causation references, opaque payload,
Pending/Delivering/Delivered/Failed status, attempt count, and creation
sequence/time metadata.

Delivery is completion of an already admitted intent. The dispatcher feeds the
existing M0.4 event transport and uses the original EventId on every retry.
Crash after publish and before Delivered may produce at-least-once redelivery.
Exactly-once subscriber side effects are not promised. Subscription authority
is evaluated at delivery time; producer authority is captured at admission and
cannot be forged by payload bytes.

## Event journal

The production journal stores accepted EventEnvelope records separately from
application state. Append is atomic, and replay uses a stable sequence cursor
that remains valid across reopen. EventId, producer sequence, correlation,
causation, contract and payload identity are preserved. Records with an
incompatible format or invalid integrity marker fail explicitly.

Retention is explicit and conservative by default. The journal does not
reconstruct installation state and does not promise global ordering across
producers.

## Audit store

Audit records are selective control-plane facts: record type, sequence,
principal/installation/runtime identities when relevant, correlation/causation,
outcome classification, and safe structured metadata. Raw RPC requests/
responses, raw event payloads, raw state values, credentials, tokens and
approval secrets are excluded at write time by default.

Audit failure policy is explicit per record class. An audit record can explain
what happened, but it never determines current authoritative state or grants
authority.

## Blob model

Blob identity is a versioned SHA-256 digest encoded as
sha256-v1-<lowercase-hex>. Bytes are written to a temporary file inside the
configured blob root, flushed, and atomically renamed to the final digest path.
An existing final blob is verified before being treated as success. Reads and
verify recompute the digest and return BlobCorrupt on mismatch.

Temporary files may remain after a crash and are removable by an explicit
maintenance operation. Digest/path validation rejects traversal and absolute
paths. Knowing a BlobId creates no plugin authority; access remains behind the
normal capability layer. Cross-domain garbage collection is deferred.

## Scheduler recovery semantics

The persistent job store is generic and records JobId, owning principal or
installation, Pending/Waiting/Runnable/Running/Completed/Cancelled/Failed/
Interrupted state, deadline, wakeup, cancellation, attempt, budget, recovery
policy, and correlation/causation.

On reopen, a Running job is recovered as Interrupted unless a documented
recovery policy explicitly proves a safe idempotent retry. Manual jobs are
never automatically re-executed. Unknown external outcomes remain
Interrupted/incomplete and never become synthetic Completed.

## Backup and restore

Backup uses SQLite's consistent snapshot mechanism while the database is active
and copies the blob tree only through an explicit storage operation. A backup is
not accepted because files merely exist. The restore test opens the copy in a
fresh profile root, validates format/schema before boot, reopens all
authoritative stores, and verifies state, outbox/journal cursors, jobs and blob
identity.

Restore rejects unsupported newer formats and corrupt/incomplete snapshots. No
automatic downgrade or destructive overwrite of the source profile occurs.

## Retention and deletion

State, outbox, journal, audit, job, and blob retention are independent. The
default M0.5 policy is conservative: no automatic cross-domain deletion,
explicit Delivered/Failed and journal retention hooks, and no blob garbage
collection without ownership/reference policy. State deletion is an
installation lifecycle operation, not audit retention. Sensitive data is
excluded at write time where possible.

## Rejected alternatives

- Global event sourcing: rejected because mutable installation state, delivery
  intent, transport replay, audit, blobs and jobs have different truth and
  retention semantics.
- Event bus as database: rejected because publication is not persistence and
  event delivery is not RPC.
- Audit as state store: rejected because audit is selective and append-only.
- One generic database API exposed to plugins: rejected because it would leak
  authority, storage layout and product policy through the plugin boundary.
- In-memory fallback after production open failure: rejected because it would
  silently lose acknowledged state.
- Distributed database/scheduler: deferred; M0.5 is single-host.
- Exactly-once external effects or unsafe automatic retries: rejected because a
  crash can leave external outcome unknown.

## Known limitations

M0.5 does not provide cloud synchronization, distributed coordination,
workspace/browser persistence, automatic cross-domain blob GC, a full backup
UX, WASM persistence adapters, or exactly-once external side effects. SQLite
durability is bounded by the supported local filesystem assumptions. Event
ordering is per accepted journal sequence, not a global producer order.

## Reopen conditions

Revisit this ADR before adding distributed profiles, cloud sync, multi-writer
storage, workspace-specific persistence in the kernel, global replay/fork
requirements, automatic unsafe job retry, or a filesystem/database backend
whose durability semantics differ from SQLite. A new backend must pass the
same contract and process-kill evidence without changing kernel/plugin
contracts.
