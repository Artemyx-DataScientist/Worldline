# ADR: Plugin Runtime v1

Статус: accepted
Change: `C-KERNEL-PLUGIN-RUNTIME-V1-20260828`
Рубеж: M0.3 — Plugin Runtime v1

## Решение

Kernel разделяет четыре уровня identity:

```text
PluginId (definition)
    -> InstallationId (persistent state)
        -> RuntimeId (one activation attempt)
            -> LifecycleScopeId (owned effects and lifecycle grants)
```

`RuntimeId` — opaque kernel-owned pair из installation incarnation и
host-local sequence. Incarnation хранится только как монотонный seed в
installation metadata, а сам `RuntimeId` не сохраняется как persistent
identity. Каждая попытка activation, включая failed/crashed/cancelled/hung,
получает новый ID. Новый runtime также получает новый principal, state lease и
lifecycle scope. Поэтому state continuity не превращается в authority
continuity.

Live runtime registry индексируется `RuntimeId`; у каждой installation есть не
более одного current runtime. Registry registrations индексируются
`InstallationId`, поэтому разные installations одной definition могут
одновременно публиковать один capability contract. Unpublish удаляет только
конкретный runtime publication.

## Lifecycle и concurrency

Lifecycle state machine явно различает `Created`,
`WaitingDependencies`/compatibility `Pending`, `Activating`, `Active`,
`Deactivating` и terminal states. Недопустимый переход возвращается как
`KernelError::InvalidRuntimeTransition` (также экспортированное lifecycle имя
`RuntimeLifecycleError`).

Синхронная reconciliation сохраняет удобный bootstrap API, а explicit
`begin_activation*` и `begin_deactivation*` используют split-phase operation:
kernel подготавливает identity, worker вызывает plugin code, затем `poll_lifecycle`
коммитит completion только если совпадают `RuntimeId`, operation identity и
current installation mapping. Plugin code не вызывается под registry lock;
обычный `RwLock` capability registry удерживается только на коротких операциях
metadata lookup/publish.

Cancellation — idempotent cooperative signal. Для trusted native in-process
plugin kernel не пытается физически убить произвольный thread. Если deadline
истёк, runtime может перейти в логическое `Hung`: его provider publications,
runtime grants, state lease и lifecycle scope authority отзываются, а late
completion отклоняется. Такой thread может физически дожить до собственного
возврата; это намеренное ограничение до process/WASM isolation.

Deactivation сначала убирает provider publication из будущего selection, затем
выполняет callback и cleanup. Уже admitted invocation сохраняет
admission-time semantics и не отменяется ретроактивно отзывом grant или
deactivation.

## Failure, boot и launch policy

`RuntimeLaunchPolicy` принадлежит installation launch, а не definition:

- `Eager`/`Lazy` задают eligibility;
- `Required`/`Optional` задают только health policy;
- `RestartPolicy` задаёт `Never` или bounded `OnFailure`, backoff и quarantine.

Quarantine не удаляет persistent installation state и не запускается снова без
явного `recover_installation`. `reconcile_with_budget` и
`start_with_budget` возвращают структурированный report: active, waiting,
failed, crashed, hung, quarantined, degraded reasons и budget exhaustion.
Необязательная ошибка не откатывает независимые runtimes; обязательная делает
boot degraded, но также не требует глобального rollback.

Lazy declarations доступны через read-only `discover_capabilities*`. Explicit
`demand_capability` может активировать только совместимый lazy provider; это не
выдаёт demanding caller ни grant, ни расширенный handle. Selection использует
стабильный порядок: highest compatible minor, затем installation ID, затем
runtime ID. Diagnostic содержит только capability metadata и rationale, без
payloads, secrets или authority contents. Major-version incompatibility не
адаптируется молча.

## Security boundary

Provider selection отвечает на вопрос «кто может обслужить contract», а broker
отвечает на вопрос «может ли этот caller вызвать operation/resource». Наличие
provider или discovery descriptor не является authorization.

`ProviderSelf` разрешён только для exact executing runtime: provider principal
должен совпадать с caller, enclosing frame и requested runtime identity.
Delegated failure не получает fallback к `ProviderSelf`. Runtime grants и их
descendants отзываются на terminal lifecycle paths и при unregister; повторный
revoke не создаёт повторный state-change audit event.

## Отклонённые альтернативы

- `PluginId` как live runtime key: не представляет две installations и смешивает
  state с ephemeral authority.
- Один `Mutex` вокруг lifecycle callback: зависший plugin блокирует unrelated
  composition и создаёт ложную гарантию cancellation.
- Tokio или production scheduler в bootstrap: M0.3 проверяет lifecycle
  contract, а не выбирает executor; std worker/channel достаточно для proof.
- Unsafe thread termination: не гарантирует cleanup и нарушает Rust safety.
- Capability discovery как implicit grant: ломает default-deny и confused-deputy
  boundary.
- Provider name/prefix priority: делает composition policy скрытой и
  недетерминированной.

## Evidence и non-goals

Evidence:

- `crates/worldline-kernel/tests/runtime_v1_acceptance.rs` — 20 M0.3 tests;
- M0.1 state/security suites, M0.2 boundary suite и S0 остаются зелёными;
- `cargo fmt --all -- --check`;
- `cargo test --workspace`;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- `cargo doc --workspace --no-deps`;
- `cargo tree -p worldline-kernel`;
- `cargo run -p worldline-demo`.

В этот change не входят production scheduler, durable event transport,
Capability RPC retry/flow-control protocol, process/WASM hard isolation,
browser engine, agent runtime, UI compositor или public third-party ABI.
