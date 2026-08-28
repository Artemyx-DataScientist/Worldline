# ADR: Capability RPC and Typed Event Transport v1

Статус: accepted
Change: `C-KERNEL-CAPABILITY-RPC-EVENT-TRANSPORT-20260828`
Рубеж: M0.4 — Capability RPC and Event Transport

## Решение

Worldline фиксирует два независимых communication plane на общей kernel
vocabulary identity, authority, correlation и causality:

```text
Capability RPC:  one caller -> one selected provider -> one result/error
Event transport: one producer -> zero or more independent subscribers
Persistence:     separate authoritative state/journal boundary
```

Event subscribers не входят в capability provider registry и не могут
удовлетворить RPC. Публикация event никогда не подтверждает успех command, а
RPC result устанавливается без ожидания subscriber delivery.

## RPC contract

Каждый broker attempt получает новый `InvocationId`. Логический caller intent
может передаваться через `RpcRequestId` между explicit retry attempts. Trusted
caller, capability, operation, resource, selected runtime, deadline,
cancellation, retry classification, idempotency key и `TraceContext` находятся
в структурированном request metadata; opaque payload туда не парсится.

Operation contract принадлежит provider и не может быть расширен caller-ом:
`NeverRetry`, `Safe` и `Idempotent`. M0.4 не запускает automatic retries;
explicit idempotent retry требует `IdempotencyKey`, а kernel не обещает
exactly-once external side effects.

Deadline использует monotonic `Instant` и проверяется до authorization,
enqueue, dispatch и перед caller-visible outcome. Cancellation — idempotent
cooperative signal: до dispatch provider не вызывается, после dispatch provider
видит token, но in-process code не убивается unsafe способом. Ни deadline, ни
cancellation не отзывает authority.

Для каждой конкретной `RuntimeId`/capability publication задаются конечные
`max_in_flight` и `queue_capacity`. Full queue возвращает явный RPC flow error;
fallback unbounded queue отсутствует. Saturation одного publication не
блокирует другой provider.

## Event contract

`EventContract` — ABI-neutral namespace/name и major/minor schema version.
`EventEnvelope` создаётся только transport и содержит kernel-stamped
`EventId`, producer identity, optional runtime identity, producer-local
sequence, `CorrelationId`, optional `CausationRef`, delivery mode и opaque
payload. Payload не может подменить trusted envelope metadata.

Publish и Subscribe default-deny и используют существующую generic grant
machinery с отдельными operations. Subscription имеет exact subscriber
principal/runtime и finite pull mailbox. `RejectForSubscriber`, `DropNewest` и
`DropOldest` отражаются в `PublishReport` и trajectory. Revoked subscription
authority проверяется перед последующей delivery; terminal runtime lifecycle
закрывает его subscriptions и отзывает runtime grants/descendants.

`Ephemeral` доставляется только live subscriptions. `Durable` сначала требует
успешный `EventJournal::append`; отсутствие journal или его failure не
понижается до ephemeral. `InMemoryEventJournal` — deterministic acceptance
fixture, не crash-safe storage. Persistent cursors, transactional outbox,
crash recovery и production durability относятся к M0.5.

## Causality and observation

Provider observation через `InvocationContext` получает correlation текущего
RPC и causation `Invocation(invocation_id)`. Event handler через
`EventContext` сохраняет event correlation и получает causation `Event(event_id)`;
follow-up RPC создаётся под subscriber authority и не наследует producer grant.

Broker может публиковать metadata-only
`worldline.control/invocation-completed@1.0`. В него входят request/attempt,
caller, provider runtime, contract, operation, outcome и trace metadata, но не
raw request/result payload. Failure публикации observation не меняет уже
установленный RPC outcome.

Trajectory остаётся отдельным append-only diagnostic mechanism: она содержит
metadata и payload sizes, но не raw RPC/event payloads. Event transport не
становится database, reply channel или trajectory queue.

## Evidence and future boundary

Acceptance suite покрывает request/attempt IDs, retry rules, deadline,
cancellation, bounded flow, event authority, schema compatibility, fan-out,
mailbox QoS, revocation, causality, journal seam и observation independence.
S1 проходит через реальные `Kernel` boundaries: state commit, RPC result,
bounded event observation, subscriber follow-up RPC и host restart с новым
`RuntimeId`/runtime authority.

Native IPC, WASM adapters, production crash-safe event persistence, persistent
subscription cursors и recovery semantics намеренно не входят в M0.4.
