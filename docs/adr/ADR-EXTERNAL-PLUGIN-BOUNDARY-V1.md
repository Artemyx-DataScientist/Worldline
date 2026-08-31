# ADR: External Plugin Boundary v1

Статус: accepted
Change: C-KERNEL-STABLE-IPC-WASM-COMPONENT-BOUNDARY-20260831
Рубеж: M0.6 — Stable IPC and WASM Component Boundary

## Context

M0.1–M0.5 закрыты: kernel владеет identity chain
(`PluginDefinitionId -> InstallationId -> RuntimeId -> LifecycleScopeId`),
default-deny capability authority, bounded capability RPC, отдельным typed
event transport, installation-owned state с CAS и миграциями, production
SQLite persistence и transactional outbox. При этом всё это доступно только
через internal Rust API: trait-объекты и in-process вызовы не являются
внешним ABI и не дают изоляции для сторонних plugin-ов.

Roadmap определяет три trust zone: builtin Rust, trusted native и untrusted
third-party. M0.6 фиксирует единую внешнюю границу, на которой один и тот же
logical capability contract реализуется в трёх execution modes, а consumer не
знает, каким из них реализован provider:

    same consumer
       ↓
    same capability
       ↓
    ┌─────────┬──────────────┬──────────────┐
    │ builtin │ native proc  │ WASM sandbox │
    └─────────┴──────────────┴──────────────┘
       ↓
    same observable semantics

Ограничения решения: public Rust ABI (struct layout, trait object vtables,
`Arc<dyn Any>`, panic unwinding) не является внешним ABI; process isolation не
создаёт authority; sandbox не заменяет capability authorization; событийная
шина не становится транспортом для RPC. Windows-first, `#![forbid(unsafe_code)]`
в kernel/storage сохраняются.

## Execution modes

| Mode | Boundary | Доверие | Общее с другими modes |
| --- | --- | --- | --- |
| Builtin | Статически связанный Rust-код в host-процессе | Доверенный platform adapter | Обычные `RuntimeId`, authority, lifecycle, state lease, capability contracts |
| NativeProcess | Отдельный supervised child process за versioned IPC | Доверенный, но только по provenance/host policy: trust расширяет допустимые ресурсы, но никогда не обходит capability broker | То же |
| WasmComponent | Component Model runtime с least-authority host imports | Недоверенный third-party | То же |

Invariant: `PluginDefinition` semantics не ветвятся по product role. Execution
mode — свойство package/manifest и host policy, а не часть logical contract.

## Logical contract versus physical transport

Kernel-контракты остаются ABI-neutral. Новый крейт `worldline-plugin-protocol`
владеет transport-neutral словарём: package/plugin identities, manifest schema,
versioned envelopes. Adapters переводят логические контракты в физические
формы: WIT interfaces/resources для WASM; versioned IPC envelopes для native
process.

Через внешнюю границу запрещено пересекать:

- Rust struct memory layout как wire ABI;
- trait object vtables;
- raw pointers;
- `Arc`/`Rc`;
- panic unwinding;
- serde-представление без явного protocol version contract.

Неизвестные поля и версии обрабатываются явными правилами совместимости
(см. Compatibility policy v1), а не случайным поведением парсера. Размер
boundary payload ограничивается до выделения памяти.

Adapters обязаны переиспользовать существующие semantics, а не строить вторую
модель: capability вызовы идут через существующий broker и `RpcOutcome`
семантику, публикации событий — через существующий M0.4 event transport,
state — через installation-owned state contract. Ни один adapter не вводит
второй RPC или event механизм.

### Kernel interfaces, сохраняемые adapters

Inventory интерфейсов, которые external adapters обязаны сохранять
(адаптация, не замена):

- `Kernel`, `ReconcileReport`, `RuntimeMetadata` — lifecycle и reconciliation;
- `StateBackend`, `StateHandle`, `RuntimeStateHandle`, `StateTransaction`,
  `StateRevision` CAS, `InstallationId`, migration plan — state ownership;
- `CapabilityHandle`, `InvocationContext`,
  `MAX_NESTED_INVOCATION_DEPTH` — bounded invocation;
- `RpcOperationContract`, `RpcRequestId`/`InvocationId`, `RpcCallOptions`
  (deadline, cancellation, retry class), `RpcOutcomeClass`, `TraceContext` —
  RPC semantics;
- `EventContract`, `EventEnvelope`, `EventJournal`, `SubscriptionHandle`,
  `EventQoS`, `OverflowPolicy`, `DeliveryMode` — event semantics;
- `AuthoritySet`, `CapabilityGrant`, `GrantId`, `GrantLifetime`, `Principal`,
  `ResourceScope`, `OperationId`, `DenialReason` — authority model;
- `OwnedEffect`, `TrajectoryEvent` — effects и наблюдаемость;
- `AuditStore`, `BlobStore`, `OutboxStore`, `JobStore` остаются host-side
  persistence contracts и внешней границей не выставляются.

## WASM Component Model selection

Зафиксированный baseline v1:

- **WASI 0.3** — stable release от 11 июня 2026 (reference implementation —
  wasmtime); async встроен в Component Model, есть stream types и полный
  socket surface на уровне платформы.
- **Component Model** в исполнении **wasmtime 48.x** (48.0.0 выпущен
  2026-08-20) — выбранная host runtime линейка.
- Гостевой toolchain для reference-компоненты фиксируется при реализации
  `worldline-wasm-host` (T-006) в манифестах крейтов: точная версия wasmtime
  crate, wit-bindgen и гостевого Rust target записываются там, а не в этом
  ADR «по памяти».

Правила изменения baseline: обновление внутри линейки WASI 0.3 (новый minor
wasmtime, обновление wit-bindgen) — записанная замена версий, не редизайн;
переход на новый WASI/Component Model baseline (например 0.4) открывает этот
ADR заново. Слово «latest» baseline-ом не является.

Разрешённый зафиксированный fallback: если к моменту T-006 стабильный гостевой
Rust toolchain для WASI 0.3-миров отсутствует, v1 reference-компонента
собирается под WASI 0.2 world при том же wasmtime host; отклонение от baseline
записывается в манифестах крейтов и в completion packet как явная дельта, а не
молчаливая подмена.

## Supported WASI surface

Принцип: WASM-компонента стартует без ambient capability. Классы разрешений
явные и host-scoped:

| Класс | Default | Явная выдача |
| --- | --- | --- |
| filesystem | Запрещён | Только явно scoped preopened directories; выход за разрешённый root невозможен |
| network | Запрещён | Явно включённый/scoped доступ; без permissions нет inbound/outbound |
| clock | Ограничивается по необходимости | Может быть выдан или ограничен host policy |
| random | Не выдаётся неявно | Явный источник по запросу/approval |
| environment | Наследование запрещено | Переменные окружения не передаются по умолчанию |

Правила: запрос в manifest не равен выданному разрешению; guest импорты
перечислены явно и минимальны; полный WASI world не выставляется с последующим
запретом внутри host functions — запреты существуют до первого вызова гостя.

## Native IPC transport selection

v1 transport: length-prefixed versioned envelopes поверх stdio-пайпов child
process (stdin — host→child, stdout — child→host), stderr захватывается
отдельно с ограниченным draining. Кодирование envelope — JSON с явным полем
версии протокола (versioned representation разрешена; unversioned — нет).

Обоснование: stdio-пайпы не создают имён каналов/сокетов, коллизий портов и
сетевой поверхности firewall, тривиально supervise-ятся и кроссплатформенны;
производительности framed JSON достаточно для v1 proving boundary. Handshake —
первый обмен: protocol version, host nonce/session identity, package identity,
plugin definition identity, supported ABI range, declared capability
interfaces. Ограничение frame size проверяется до выделения буфера.

Shared memory и zero-copy в v1 запрещены (см. Rejected alternatives).

## Handle authority model

External plugin ссылается на делегированную authority через opaque handles,
не получая kernel-внутренностей (`Grant`, `SecurityStore`, `StateBackend`).

Свойства handle: opaque для guest/process; создаётся host-ом; scoped к
конкретному `RuntimeId` и сессии; не персистентен (если отдельный контракт
явно не представляет иное); attenuated до разрешённых operations/resources;
revocable.

Правила проверки: каждое использование handle резолвится host-ом против
handle table конкретного runtime с проверкой точного владельца; значения
handle — высокоэнтропийные, но security не опирается только на
непредсказуемость значений: даже известное значение чужого handle не работает
вне owner runtime/session.
Handle из crashed, replaced или restarted runtime невалиден: новая таблица на
новый `RuntimeId`. Guest не может расширить scope, закодированный в handle
metadata; delegation даёт только equal-or-narrower authority. Значения
`Grant`/`Principal` никогда не сериализуются в гостевой payload.

## Resource isolation

Обязательные dimensions: linear memory; component/resource table entries;
host-call concurrency; host-call payload bytes; invocation count/rate budget;
wall-time/deadline; CPU/execution budget (fuel/epoch-эквивалент выбранного
runtime).

Механизм исполнения runtime-specific, наблюдаемая семантика общая:
WASM — store limits и execution budget wasmtime; native — supervision
deadlines graceful shutdown/kill плюс платформенные ограничения child process
(например Job Objects на Windows) как defense in depth. Истощение лимита
отображается в явный typed failure class, не потребляет навсегда слот provider
registry и имеет явную политику reset/restart. Process isolation не считается
authority isolation: broker-проверки обязательны для всех modes.

## Package identity and manifest

Идентичности: `PluginPackageId` — distribution/package identity;
`PluginDefinitionId` — логическая plugin identity; `PackageVersion` независима
от версий capability contracts; `AbiVersion` — требуемый внешний baseline.
Package identity сама по себе authority не создаёт.

Manifest (schema version фиксирована) несёт: package identity и version,
предоставляемые plugin definitions, execution mode, required ABI baseline,
provided/required capability contracts, requested host permissions, resource
limit hints, entrypoint/component artifact. Инвариант: manifest описывает
запрошенные/декларированные permissions и никогда не выдаёт их. Unknown
обязательные поля или неподдерживаемая schema version — fail closed. Пути
внутри package не могут выйти за package root; digest артефакта вычисляется и
записывается для provenance/diagnostics. Загрузка package никогда не активирует
plugin authority автоматически.

## Failure mapping

| Источник | Failure | Отображение |
| --- | --- | --- |
| Guest | Trap | Изоляция в exact invocation/runtime; existing RPC/runtime failure semantics; host живёт |
| Guest | GuestReturnedError | Обычный RPC error outcome |
| Guest | ResourceLimitExceeded | Явный typed class, не generic `PluginError` |
| Guest | DeadlineExceeded | Existing RPC deadline semantics |
| Native | ProcessExit / ProcessCrash | Crash/restart/quarantine policy только этого `RuntimeId`; provider publications снимаются |
| Native | ProtocolViolation | Terminate/quarantine offending runtime; deterministic protocol error |
| Native | TransportDisconnected | Явный transport failure class до классификации broker-ом |
| Host | AdapterFailure | Ошибка host-side adapter, не приписывается плагину |

Классификация наблюдаема (trajectory/audit, metadata-only). Runtime terminal
cleanup отзывает handles, state leases, grants, subscriptions и provider
publications по всем modes. Installation state не принадлежит процессу или
WASM instance и переживает их смерть.

## Compatibility policy v1

Версионирование явное на трёх уровнях: `AbiVersion` (Component Model/WASI
baseline), protocol version native IPC, manifest schema version. v1 поддерживает
ровно зафиксированный baseline. Unknown mandatory — fail closed; unknown
optional поля envelope игнорируются и учитываются в diagnostics счётчиках, но
не меняют семантику. Совместимость replacement provider-ов между execution
modes определяется существующей capability contract compatibility логикой
kernel-а. Матрица SDK N/N-1/N-2 — NonGoal этого рубежа (M0.7).

## Cordis-rs lineage and deliberate divergence

cordis-rs — концептуальный предок Worldline plugin/runtime модели и остаётся
контрольным референсом при review решений. Это родословная, а не зависимость:
cordis-rs не входит в Cargo-граф, не vendored и не является целью API- или
поведенческой совместимости. Roadmap фиксирует это как «концептуальное
родство, а не требование повторить».

Что сознательно унаследовано из lifecycle/Fiber/effects-модели cordis-rs:

- управляемая split-phase активация/деактивация как first-class операции
  runtime, а не побочный эффект загрузки кода;
- ownership эффектов: каждый живой effect принадлежит scope и детерминированно
  сворачивается, LIFO-порядок очистки;
- идея композиционного runtime, в котором host компонует replaceable
  providers за едиными контрактами;
- детерминированная reconciliation желаемого и фактического состояния plugins.

Сознательные расхождения и их причины:

- **`Arc<dyn Any>`-подобный capability ABI не переносим.** Public Rust ABI не
  является внешним ABI Worldline: через границу идут versioned WIT и IPC
  envelopes (см. Logical contract versus physical transport). Причина:
  нестабильность Rust ABI, отсутствие изоляции, link-time coupling.
- **Effect cleanup не смешивается с rollback.** LIFO-cleanup — это teardown
  live effects scope-а; rollback/recovery — семантика persistence-слоя
  (M0.1 CAS, M0.5 recovery). Это разные plane-ы с разными failure modes, и их
  конъюнкция в cordis-модели здесь не воспроизводится.
- **Блокирующие lifecycle-lock-и не переносимы.** M0.3 требует async
  split-phase lifecycle без глобального transition lock на время plugin-кода;
  hung-обнаружение и quarantine заменяют «заморозку всех».
- **Event bus не универсальный механизм.** В cordis-подобных моделях шина
  легко становится универсальным transport-ом; Worldline закрепляет
  `EVENT BUS IS NOT RPC` (M0.4): command ждёт admission и outcome, а не
  ищет ответ среди subscribers.

Чего cordis-rs не предоставлял и что эта граница добавляет: identity chain
от definition до lifecycle scope, explicit default-deny authority с
admission-time проверками, crash/quarantine isolation, opaque attenuated
handles, installation-owned persistence/recovery. Родословная сохранена
намеренно: через полгода должно быть видно не только что решено, но и откуда
решение выросло и где оно сознательно отклонилось от предка.

## Rejected alternatives

- **Public Rust dylib ABI для third-party plugins** — отвергнут: Rust ABI не
  стабилен, sandbox отсутствует, обновление host ломает плагины.
- **Raw C ABI как единственная граница** — отвергнут: unsafe по умолчанию,
  нет типизированных контрактов и versioning story по сравнению с WIT.
- **WASM-only для всех providers без native path** — отвергнут: тяжёлые
  trusted providers (CEF, GPU, media) требуют отдельных процессов за IPC.
- **Unversioned ad-hoc JSON/serde representation** — отвергнут: запрещён spec-ом
  change; представление без явного protocol version contract даёт случайную
  совместимость.
- **Shared memory / zero-copy IPC в v1** — отвергнут до стабилизации semantics
  (NonGoal change).
- **Event bus как транспорт RPC через границу** — отвергнут: `EVENT BUS IS NOT
  RPC`.
- **Выставить полный WASI world и запрещать внутри host functions** — отвергнут:
  запреты должны существовать до первого вызова гостя.
- **Передача объектов `Grant`/`Principal` через границу** — отвергнута:
  authority пересекает границу только как opaque attenuated handles.

## Known limitations

v1 не оптимизирует производительность границы: framed JSON без zero-copy и
shared memory. Cooperative cancellation через process boundary имеет задержку
доставки. Baseline WASI 0.3 без guest threads/shared memory. Package signing,
registry, marketplace, автоматические обновления — NonGoals. Нативные
платформенные ограничения child process покрывают не все платформы равномерно;
гарантируется supervision-уровень (deadlines, kill), а не полноценный OS
sandbox. Reference consumer v1 — Rust; другие языки SDK — за пределами рубежа.

## Reopen conditions

Открывать ADR заново при: новом WASI/Component Model baseline; недоступности
стабильного гостевого toolchain сверх зафиксированного fallback; доказанной
непригодности stdio-фрейминга по измерениям производительности; требованиях
multi-language SDK, меняющих WIT-контракты; работе M0.7, требующей матрицы
совместимости шире v1; платформах, где stdio supervision недостаточен. Любая
ревизия обязана сохранить инвариант одного logical contract для трёх execution
modes и переиспользование broker/event/state semantics.
