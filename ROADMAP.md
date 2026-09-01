# Worldline Roadmap v0.1

Статус документа: рабочий roadmap  
Зафиксирован: 28 августа 2026 года  
Единица планирования: проверяемый архитектурный или продуктовый рубеж, а не
календарная дата

## Текущий статус рубежей

- **M0.1 — Done.** State hardening закрыт revision/CAS, runtime leases,
  recovery errors и installation authority retirement.
- **M0.2 — Done.** Это был active gate текущего boundary review; он закрыт
  [ADR-KERNEL-BOUNDARY-V1](docs/adr/ADR-KERNEL-BOUNDARY-V1.md), тремя
  reference families и постоянным S0 proving slice.
- **M0.3 — Done.** Plugin Runtime v1 закрыт явным runtime identity,
  multi-install cardinality, split-phase lifecycle, recovery policy,
  discovery и deterministic provider selection.
- **M0.4 — Done.** Capability RPC and typed event transport закрыты bounded
  RPC, default-deny event authority, runtime-scoped subscriptions, logical
  durable delivery и S1 proving slice.
- **M0.5 — Done.** Persistence and Recovery Model закрыт production SQLite
  state/outbox/journal, selective audit, CAS blobs, persistent jobs,
  backup/restore, hard-kill recovery tests и production-backed S1.
- **M0.6 — Done.** Stable IPC and WASM Component Boundary закрыт
  `worldline-plugin-protocol`, WIT/IPC envelopes, opaque handles,
  least-authority WASI/quotas, supervised native host, sandboxed WASM host,
  malicious WASM containment, protocol robustness, 3-mode cross-conformance
  и external S1 proving paths.
- **M0.7 — Done.** Operability, compatibility and upgrade закрыт machine-checkable
  compatibility matrix (SDK N/N-1/N-2), staged updates, migration-on-copy,
  LastKnownGood rollback, persistent quarantine, safe mode, automated bisect,
  payload-free operational telemetry, causality diagnostics, property tests,
  negative security tests, fuzzing smoke tests, process-kill chaos matrix и
  explicit incomplete side-effects.
- **M0 — Complete.** Базовая архитектура платформы Worldline (M0.1–M0.7)
  полностью построена, проверена сквозными CI-гейтами и готова к разработке
  первого прикладного плагина — браузерного движка (Milestone M1).
- **M1.1 — Done.** Browser Contract v1 and engine spike закрыты
  [ADR-BROWSER-ENGINE-V1](docs/adr/ADR-BROWSER-ENGINE-V1.md),
  8 engine-neutral capability contracts (`worldline-browser-contract`),
  строгим разделением authority (observation vs mutation),
  защитой от confused-deputy привязкой `InvocationContext` к целевым `PageId`/`ContextId`,
  бюджетированными запросами (`QueryBounds` с флагами `is_truncated`),
  логическими идентификаторами профилей без утечки путей хоста,
  публикацией типизированных событий через M0.4 транспорт,
  реальным out-of-process спайком Chromium на Windows с навигацией по локальным HTML,
  извлечением дерева доступности Blink, действиями в DOM и изоляцией сбоя рендерера,
  а также детерминированным контрактным эталоном со всеми 8 рабочими контрактами
  и обоснованным выбором Chromium/CEF для M1.2.
- **M1.2 — Done.** Browser engine provider process закрыт
  [ADR-BROWSER-ENGINE-PROVIDER-PROCESS-V1](docs/adr/ADR-BROWSER-ENGINE-PROVIDER-PROCESS-V1.md),
  реализацией supervised native child process `worldline-browser-provider-process`,
  CEF/Chromium C FFI и потокобезопасным UI message loop runner (`worldline-browser-cef`),
  Windows Job Object containment (`worldline-native-host`),
  CAS-валидацией генераций и строгой проверкой устаревания `ElementRef`,
  бюджетированной проекцией деревьев доступности, контентно-адресуемым visual capture,
  полной изоляцией cookies/storage по контекстам и сквозным proving slice S2.
- **M1.3 — Active.** Browser service plugins.

## 1. Куда идёт Worldline

**Worldline is a userland operating environment for the Internet.**

Он не заменяет Windows, Linux или macOS и не является браузером с отдельным
AI-чатом. Это рабочая среда над обычной ОС, которая координирует деятельность
пользователя в интернете и хранит не только страницы: над чем человек работал,
зачем открывал источники, какие действия выполнял, к каким выводам пришёл и что
осталось сделать.

Целевое ощущение от продукта:

> Я продолжаю своё дело, а система уже знает его контекст и помогает прямо в
> рабочей среде.

Центральным объектом Worldline должен стать `Workspace` (или `Activity`), а не
окно, вкладка или чат. Страницы, документы, репозитории, сообщения, заметки,
загрузки, результаты поиска и agent runs — это связанные объекты внутри
workspace. Вкладка остаётся полезным временным представлением, но не является
единицей памяти продукта.

У Worldline есть отдельная визуальная гипотеза: содержимое остаётся привычным и
читаемым, а управляющий слой среды получает оптический glass-язык. Стекло здесь
не decoration theme и не свойство каждой карточки. Оно показывает границу:
«это Worldline управляет контентом, контекстом или действием поверх него».
Веб-страница не должна становиться полупрозрачной ради стиля; navigation chrome,
floating commands, agent actions и временные contextual surfaces могут
ощущаться как самостоятельный системный слой над ней.

Успех Worldline определяется не количеством AI-функций, а тем, может ли
пользователь:

- вернуться к делу через месяц и продолжить с восстановленным контекстом;
- спросить о текущей странице, выделении, соседней вкладке или проекте без
  повторного пересказа контекста;
- поручить системе многошаговую работу и видеть ход каждого действия;
- позволить агенту подготовить внешнее действие, сохранив за человеком право
  окончательного подтверждения;
- получать важные изменения из интернета без постоянного ручного обновления
  страниц.

## 2. Рабочая гипотеза архитектуры

Схема ниже фиксирует текущее направление размышлений, а не окончательную
component topology. Она нужна, чтобы декомпозировать риски и поставить
эксперименты. Граница kernel, набор обязательных services, CEF, wgpu, WASM и
форма event transport должны быть подтверждены ADR и работающими spikes.

```text
                         Rust microkernel
                                  │
             identity / lifecycle / registry / authority
              ┌──────────────┼──────────────┐
       capability RPC     event transport    storage primitives
       request / result   publish / observe  state / log / blob
                         UI composition host
                                  │
          ┌───────────────────────┼───────────────────────┐
          │                       │                       │
    Browser plugins         Agent plugins            UI plugins
          │                       │                       │
    Chromium / CEF          GLM / OpenAI / local      wgpu renderer
    tabs / history          agent loop / planner      tab bar
    downloads / cookies     memory / search           command palette
    DevTools / adblock      GitHub / MCP              workspaces / overlays
          │                       │                       │
          └───────────────────────┴───────────────────────┘
                                  │
                    versioned capability contracts
```

В этой гипотезе Browser, Agent и UI — не три подсистемы, зашитые в kernel, а три
первые семьи plugins, с помощью которых проверяется общая платформа. В
дальнейшем рядом с ними могут появляться другие families и `whatever.wasm` без
нового привилегированного пути.

Более устойчивые продуктовые и security-инварианты:

> **Nothing above the kernel is special.**

> **Everything is a capability provided by a plugin.**

> **Nothing gets authority merely because it is a plugin.**

Предварительное следствие: kernel должен стремиться быть минимальным арбитром
механизмов, не предоставляющим продуктовую функцию. Какие именно части
identity, authority, isolation, causality, scheduling, persistence и UI
composition обязаны жить в kernel, а какие можно безопасно вынести в системные
plugins, — отдельный design gate M0. Policy может расширяться plugins, но
обязательность enforcement capability checks не должна зависеть от случайной
composition профиля.

### Реестр ключевых гипотез

| Гипотеза | Текущий статус | Как принимается или отвергается |
| --- | --- | --- |
| Rust microkernel — минимальная несменяемая основа | Направление | Boundary review: в kernel остаётся только то, без чего нельзя обеспечить инварианты всех plugins |
| Browser, agent и UI образуют равноправные plugin families | Направление | Три reference plugins проходят один lifecycle/capability contract без специальных kernel APIs |
| Typed event transport нужен в kernel | Подтверждено в M0.4 | Acceptance/S1 доказывают bounded publish/subscribe и isolation; transport не получает RPC или storage semantics |
| Scheduler и persistence являются kernel primitives | Открытый вопрос | Crash/restart и authority tests определяют минимально необходимую часть |
| UI composition host относится к kernel | Открытый вопрос | UI spike проверяет, можно ли оставить в host только bootstrap/window handles, а composition вынести в plugins |
| CEF — первый browser engine provider | Кандидат | Engine spike по embedding, isolation, semantics, packaging, licensing и upgrade cost |
| wgpu — первый renderer/compositor provider | Кандидат | UI/OSR spike по latency, accessibility и cross-platform fallback |
| Optical system glass — визуальный язык управляющего слоя | Направление | M1 UI spike доказывает hierarchy, readability, bounded backdrop sampling, degraded fallback и frame-time budget на opaque web/workspace content |
| WIT Components — внешний third-party ABI | Сильный кандидат | ABI compatibility и sandbox prototype против альтернативного out-of-process IPC |
| Hybrid persistence: state stores + selective logs + blobs + indexes | Сильный кандидат | Каждый bounded domain обосновывает свою source-of-truth model; event sourcing остаётся opt-in |

Roadmap может менять конкретную гипотезу без смены продуктовой цели. Смена
продуктового или security-инварианта требует отдельного решения владельца
проекта.

### Три независимых plane

> **EVENT BUS IS NOT RPC.**

| Plane | Семантика | Чего он не делает |
| --- | --- | --- |
| Capability invocation / RPC | Один caller выбирает capability operation через broker; один provider возвращает result/error; есть admission, deadline, cancellation и authority | Не ищет ответ среди event subscribers и не кодирует request/reply парой событий |
| Event transport | Producer публикует факт или сигнал; ноль или несколько независимых subscribers наблюдают его через bounded delivery | Не является command dispatcher, provider discovery или гарантией persistence |
| Persistence | Domain store атомарно сохраняет authoritative state, audit facts, blobs или indexes согласно собственной модели | Не возникает автоматически из публикации event и не обязана быть event-sourced |

Planes могут связываться correlation/causation IDs, но не подменяют друг друга.
Например, capability invocation может после commit опубликовать observation;
это не превращает observation в ответ RPC и не означает, что event bus стал
источником истины.

## 3. Архитектурная конституция

Worldline следует тому же базовому принципу, который формулирует
[DeepSeek Harness](https://www.deepseek.com/harness/en/): **everything is a
plugin**. Это концептуальное родство, а не требование повторить Cordis или
внутреннее устройство DeepSeek Harness.

Для Worldline принцип означает следующее.

1. Всё продуктовое и заменяемое поставляется plugins: browser engines, tabs,
   history, downloads, inference providers, agent loops, tools, workspace
   stores, semantic indexes, event sources, integrations, renderers и UI
   surfaces.
2. Privileged kernel core может содержать только необходимые механизмы
   композиции и доверия. Точная граница определяется в M0; независимо от неё в
   core нет специальных знаний о браузере, модели, GitHub или конкретном UI.
3. Consumer зависит от capability contract, а не от конкретного provider.
   Встроенный provider использует тот же логический контракт, что и сторонний.
4. Наличие capability не создаёт authority. Доступ выдаётся явно, с минимальным
   scope, может быть ослаблен при delegation и отозван.
5. Цепочка identity не смешивается:
   `PluginDefinitionId -> InstallationId -> RuntimeId -> LifecycleScopeId`.
   Состояние принадлежит installation; полномочия и живые handles — runtime.
6. Каждый живой effect принадлежит lifecycle scope и должен сворачиваться при
   unload, crash или replacement plugin.
7. Каждое действие агента наблюдаемо и имеет provenance. Model-visible context,
   вызовы capabilities, результаты, approvals и значимые изменения workspace
   оставляют durable audit records в подходящем domain store; это не требует
   одного глобального event-sourced stream.
8. Необратимые внешние действия требуют human confirmation в точке исполнения,
   а не только согласия в начале длинной задачи.
9. Никакая продуктовая фаза не получает browser-specific исключение в kernel.
   Если контракт недостаточен, меняется общий capability protocol.
10. Данные workspace локальны по умолчанию. Передача модели, синхронизация и
    внешние integrations являются отдельными разрешаемыми capabilities.
11. Event transport не используется вместо capability invocation/RPC. Событие
    сообщает о факте; command ожидает конкретный admission и outcome.
12. Event sourcing применяется только к bounded domain, где доказана потребность
    в полном replay/audit/fork. Для остальных данных выбирается более простая
    authoritative storage model.
13. С самого раннего runnable build существует один end-to-end proving slice.
    Он всегда остаётся зелёным и расширяется вместе с платформой.

Практическая проверка принципа проста: заменить provider, отключить plugin или
перенести его в sandbox должно быть возможно без изменения consumer и без
потери контроля над authority, state и наблюдаемостью.

### Текущие design constraints для проверки

Это defaults для следующего spike, а не навсегда замороженный стек. Решение
становится обязательным после ADR с evidence; неудачный kill test обязан его
отменить или сузить.

| Область | Решение |
| --- | --- |
| External ABI | Версионированные WIT Component interfaces или stable IPC; никакого public Rust `dylib` ABI |
| Authority | Default deny и capability broker между каждым principal и ресурсом; process isolation не считается authority isolation |
| Native providers | Высокопроизводительные trusted providers работают в отдельных restartable processes за versioned IPC |
| Browser engine | CEF/Chromium скрыт за `BrowserEngine` contract; CEF types никогда не пересекают provider boundary |
| Chromium strategy | Не форкать Chromium, пока физически невозможно решить задачу через upstream CEF/provider layer |
| State | Один authoritative source, entity revisions и CAS; stale операции всегда отвергаются |
| Capability RPC | Point-to-point request/result через broker с admission, deadline, cancellation, authority и явным provider |
| Events | Отдельный typed publish/subscribe plane: versioned envelopes, bounded mailboxes, backpressure/QoS и causation/correlation IDs |
| Persistence | Явный выбор per bounded domain: transactional state, selective audit log, blob store или derived index; event sourcing не является default |
| Agent writes | `prepare -> authorize -> commit`; approval связан с exact action digest, target revision и TTL |
| Failure model | Partial activation, degraded mode, quarantine и safe mode являются штатными состояниями |
| Upgrade | `stage -> migrate copy -> validate -> switch -> rollback`; новая версия не портит единственную рабочую копию state |
| Diagnostics | Replay только для явно event-sourced domains; plugin telemetry и plugin bisect проектируются до публичной ecosystem |
| Visual layering | Web/workspace content в основном opaque; glass зарезервирован для system chrome, controls over content, agent/action overlays и temporary context UI |

### Предварительная модель трёх trust zones

| Тип plugin | Boundary | Назначение |
| --- | --- | --- |
| Builtin Rust | Statically linked, но тот же logical capability contract | Малые доверенные platform adapters |
| Trusted native | Отдельный process + stable IPC + OS sandbox по возможности | CEF, GPU, media и другие тяжёлые providers |
| Untrusted third-party | WASM Component + ограниченный WIT world + quotas + OS-level defense in depth | Ecosystem plugins |

Если эта модель будет принята, WASM останется одной границей защиты, а не всей
sandbox model. Runtime, host functions, capability broker и OS sandbox входят в
trusted computing base.

## 4. Карта крупных рубежей

| Рубеж | Результат для пользователя | Статус |
| --- | --- | --- |
| **S0–S4 — Proving Slice** | Самый тонкий реальный end-to-end путь всегда runnable | Непрерывный track |
| **M0 — Kernel** | Безопасная и заменяемая plugin-платформа | В работе |
| **M1 — Browser** | Workspace-first браузер, пригодный для обычного browsing | Запланирован |
| **M2 — Agentic Workspace** | Агент работает внутри среды и сохраняет контекст деятельности | Запланирован |
| **M3 — Internet Activity OS** | Фоновые события и persistent activities работают без открытой страницы | Горизонт |

Зависимость рубежей:

```text
M0 Kernel
  -> M1 Browser
       -> M2 Agentic Workspace
            -> M3 Internet Activity OS
```

Эта стрелка задаёт gates для product claims, но не разрешает горизонтально
строить весь M0 до первого сквозного запуска. Через все рубежи проходит
постоянный proving slice.

### Proving Slice — непрерывный track

Цель — с самого начала сохранять тонкий, настоящий путь от user intent до
наблюдаемого результата. Он использует production boundaries и расширяется
вместе с системой; breadth каждого слоя добавляется только после его включения
в slice.

| Версия slice | Минимальный сквозной путь |
| --- | --- |
| **S0 — Kernel** | user command -> host -> capability RPC/broker -> replaceable demo provider -> result/error -> diagnostic observation |
| **S1 — Durable runtime** | boot profile -> installation/runtime -> RPC -> state commit -> independent event observation -> restart/restore |
| **S2 — Browser** | shell command -> browser provider process -> navigation -> opaque rendered page under Worldline system chrome -> history/state restore |
| **S3 — Agentic** | selection/intent -> agent plugin -> inference provider -> browser observation/action -> visible result or exact approval |
| **S4 — Background** | internet event source -> subscriber -> persistent task wakeup -> semantic filter -> notification |

Правила track:

- slice запускается в CI как end-to-end acceptance, а не собирается из
  subsystem mocks;
- fake provider допустим только за тем же contract/IPC boundary, что и будущий
  real provider;
- event subscriber в S1 наблюдает RPC outcome, но не возвращает сам outcome;
- каждый следующий milestone сначала делает slice тоньше сквозным, и только
  затем расширяет capability breadth;
- ни одна горизонтальная platform-ветка не остаётся без интеграции в slice
  дольше одного планового цикла;
- поломка slice блокирует дальнейшее расширение roadmap.

Исследовательские spikes следующих фаз допустимы раньше. Их код не становится
production dependency, пока предыдущий системный gate не закрыт.

Названия M1 и M2 описывают product outcomes, а не привилегии архитектуры.
Browser, agent и UI plugins используют один runtime; браузер выбран первым
вертикальным срезом только потому, что он быстрее всего проверяет полезность
среды.

## 5. Текущая точка

В репозитории уже реализован bootstrap `worldline-kernel` и acceptance-набор,
который описывает текущую часть M0:

- plugin definitions, dependencies, activation/deactivation и deterministic
  reconciliation;
- lifecycle scopes и owned effects с обратным сворачиванием;
- capability registry, compatible provider replacement и live broker handles;
- principals, grants, operation/resource scopes, delegation, attenuation,
  transitive revocation и `ProviderSelf` без confused-deputy fallback;
- admission-time authorization, causal invocation и bounded recursion;
- отдельная installation identity, installation-owned state, atomic
  transactions, directed migrations и explicit uninstall;
- append-only trajectory без записи raw payload/state values.
- bounded capability RPC с logical request/attempt identity, deadline,
  cooperative cancellation и operation-owned retry classification;
- отдельный typed event transport с kernel-stamped envelope, bounded
  subscriber mailboxes, explicit QoS/backpressure и Ephemeral/Durable modes;
- S1 proving slice, который связывает RPC result, независимое event
  observation и state continuity после host restart.

`worldline-demo` сохраняет S0 и запускает S1: показывает availability,
authorization, grant/revoke, provider replacement, независимое typed event
observation и state continuity после host restart. M0.5 расширяет тот же
proving path production-backed persistence/recovery evidence, не заменяя его
новым изолированным subsystem demo.

M0.1–M0.5 закрыты своими acceptance gates. Текущий активный gate — M0.6:
stable IPC and WASM Component Boundary. Логический `InMemoryEventJournal` из
M0.4 намеренно не является crash-safe persistence; production persistence
теперь принадлежит M0.5 и реализован в `worldline-storage`.

Статус **Done** разрешено ставить только когда соответствующий exit criterion
покрыт acceptance-тестами и весь workspace проходит:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 6. M0 — Kernel

### Цель

Получить минимальное browser-agnostic ядро, на котором можно безопасно
компоновать trusted native и untrusted third-party plugins без скрытого общего
состояния и неявной authority.

### M0.1 — State hardening — Done

Довести `STATE-HARDENING-1` до зелёного acceptance gate:

- монотонная revision для state values и installation metadata;
- compare-and-swap на commit и metadata transitions;
- детерминированный отказ stale transactions без lost update;
- явные `Regular` и `Migration` transaction paths;
- runtime-bound state handle, который перестаёт работать после deactivate,
  unregister или crash, включая уже открытые transactions;
- recovery policy для частично неуспешных metadata transitions;
- удаление installation principal и его grants при успешном uninstall;
- отказ от неявного выбора installation при неоднозначности;
- fault-injection tests для conflict, commit failure, recovery failure и
  restart.

Exit criterion: два конкурентных stale writers не могут потерять update;
остановленный runtime не может ни читать state, ни завершить старый commit;
неуспешная migration или uninstall оставляет систему в явно восстанавливаемом
состоянии.

### M0.2 — Kernel boundary decision — Done

До стабилизации external API провести отдельный boundary review:

- перечислить инварианты, которые невозможно обеспечить вне privileged core;
- для registry, event transport, scheduler, persistence и UI host сравнить
  `kernel primitive` против `mandatory system plugin`;
- запретить попадание domain entities (`Tab`, `Message`, `Agent`, `Workspace`)
  в kernel независимо от результата;
- реализовать три игрушечных reference family — browser-like, agent-like и
  UI-like — на одном общем API;
- оформить ADR с rejected alternatives и условиями пересмотра.

Exit criterion: содержимое kernel объясняется необходимым инвариантом, а не
удобством текущей реализации или ранней блок-схемой. Evidence: ADR boundary
classification, `worldline-reference` acceptance suite и S0 restart proof.

### M0.3 — Plugin Runtime v1 — Done

- ввести настоящий `RuntimeId`, не представленный через `PluginId`;
- завершить модель
  `PluginDefinitionId -> InstallationId -> RuntimeId -> LifecycleScopeId`;
- разрешить несколько одновременно активных installations одной definition;
- формализовать lifecycle state machine и допустимые transitions;
- сделать activation/deactivation асинхронными без удержания глобального
  transition lock во время plugin code;
- добавить cancellation, deadlines и hung-plugin detection;
- определить restart/backoff/quarantine policy после crash;
- поддержать partial activation и degraded boot: failure необязательного plugin
  не блокирует shell или несвязанные capability subgraphs;
- добавить lazy activation и startup budgets;
- сделать provider selection явной policy с наблюдаемым объяснением выбора;
- добавить capability discovery и version negotiation;
- гарантировать очистку effects и authority во всех terminal paths.

Kill tests:

- зависший plugin не блокирует reconciliation остальных;
- старый runtime после replacement не наследует grants и state lease;
- две installations одного plugin одновременно активны и изолированы;
- provider replacement не требует перезапуска совместимого consumer.

Exit criterion: lifecycle, identity и replacement semantics стабильны и не
требуют специальных обходов от будущего browser runtime. Evidence:
[ADR-PLUGIN-RUNTIME-V1](docs/adr/ADR-PLUGIN-RUNTIME-V1.md), 20 новых runtime-v1
acceptance tests, сохранённые M0.1/M0.2 suites и S0 proving slice.

### M0.4 — Capability RPC and event transport — Done

Capability invocation plane:

- point-to-point `request -> result/error` через registry и broker;
- provider определяется до dispatch; event subscribers не участвуют в
  resolution;
- admission-time authority, resource scope и operation checks;
- request ID, deadline, cancellation и явная retry/idempotency classification;
- bounded concurrency и flow control между caller и provider;
- отсутствие provider даёт явный RPC error, а не ожидание подходящего event.

Event plane:

- typed envelope с `event_id`, producer principal, topic, sequence, schema
  version, correlation и causation IDs;
- publish/subscribe без обязательного consumer и без reply channel;
- bounded mailbox на subscriber, явные backpressure и droppable-event QoS;
- durable и ephemeral delivery являются разными opt-in режимами;
- получение event не передаёт subscriber authority producer/caller;
- follow-up action subscriber выполняет отдельным capability RPC под собственной
  authority.

Дополнительные границы M0.4:

- RPC queue и subscriber mailbox конечны; saturation одного provider или
  subscriber не создаёт unbounded memory growth и не блокирует unrelated RPC;
- authorization остаётся admission-time: revoke блокирует новые admissions,
  но не отменяет уже admitted in-flight invocation;
- Durable — это logical journal contract. Kernel поставляет только
  deterministic `InMemoryEventJournal` для tests и не делает production
  crash-safety claim;
- metadata-only `InvocationCompleted` observation публикуется независимо от
  уже установленного `RpcOutcome`; event delivery не является reply channel.

Kill tests:

- RPC возвращает result при нуле event subscribers;
- event subscribers не могут заменить отсутствующий capability provider;
- зависший subscriber не блокирует unrelated RPC;
- публикация события не считается успешным выполнением command;
- event payload не может подделать caller, grant или approval.

Exit criterion: invocation и event transport используют общую identity и
causality vocabulary, но имеют разные APIs, delivery contracts, failure modes и
acceptance suites. Evidence: RPC/event acceptance tests, S1 restart proof,
`ADR-CAPABILITY-RPC-EVENT-TRANSPORT-V1` и полный workspace verification.

### M0.5 — Persistence and recovery model — Done

Для каждого bounded domain отдельный ADR выбирает source of truth:

- transactional state store — для текущего installation/runtime/workspace
  state и других mutable entities;
- selective append-only audit/trajectory log — для security, approvals,
  lifecycle и тех causal facts, которым действительно нужен audit;
- content-addressed blob store — для screenshots, DOM snapshots, downloads и
  model artifacts;
- derived indexes/projections — для search, semantic retrieval и UI read models;
- telemetry store с собственной retention policy — для operational metrics.

Общие требования:

- production `StateBackend` с crash-safe atomicity и CAS;
- scheduler primitives с явно сохранённым current job state, deadlines,
  wakeups, cancellation и resource budgets;
- публикация event сама по себе ничего не сохраняет;
- если domain mutation и notification должны переживать crash согласованно,
  используется transactional outbox или другая отдельно доказанная схема;
- event sourcing включается только после ADR, доказывающего необходимость
  replay/fork/audit, приемлемую стоимость schema evolution и bounded retention;
- checkpoints/upcasters создаются только для выбранных event-sourced domains;
- schema/version, backup, retention, compaction, redaction и deletion policy
  определяются для каждого store отдельно;
- recovery tests останавливают process в каждой критической точке
  commit/migration/uninstall/outbox delivery.

Exit criterion: после crash каждый domain либо восстанавливает authoritative
state, либо явно отмечает незавершённую операцию; event bus не используется как
database; большие и секретные payloads не попадают в audit log по умолчанию.

Evidence: `ADR-PERSISTENCE-RECOVERY-V1`, generic kernel persistence contracts,
production `worldline-storage`, contract and hard-kill acceptance suites,
production-backed persistence S1, backup/restore validation и repository-owned
`All` suite. Локальный evidence не объявляет hosted CI run без именованного
GitHub Actions результата.

### M0.6 — Stable IPC and WASM Component Boundary — Done

- выбрать и закрепить поддерживаемую версию WASM Component Model/WASI;
- определить единый logical contract и adapters для builtin Rust, native IPC и
  WASM providers;
- описать WIT-контракты для plugin lifecycle и первых capability families;
- определить versioned IPC envelope, handshake, cancellation и flow control для
  trusted native processes;
- не выставлять plugin весь WASI world автоматически;
- передавать authority через host boundary только как непрозрачные,
  attenuated handles;
- ввести явные filesystem/network/clock/random permissions;
- обеспечить CPU, memory, wall-time и invocation quotas;
- изолировать trap, panic, crash и resource exhaustion;
- дать WASM plugin доступ к state только через installation contract;
- подготовить manifest, package identity и compatibility metadata;
- реализовать reference third-party plugin без special-case кода в kernel.

Главный kill test: вредоносный WASM plugin не может прочитать, вызвать или
сохранить ничего, что ему явно не делегировано, и не может повредить работу
других runtimes.

### M0.7 — Operability, compatibility and upgrade — Done

- stable/experimental capability lifecycle и правила major-version change;
- compatibility matrix: current kernel с SDK `N`, `N-1`, `N-2` и current SDK с
  поддерживаемыми kernels;
- staged install/update с migration на копии, health validation, atomic switch
  и rollback;
- safe mode, plugin quarantine и автоматизированный bisect;
- per-plugin activation time, CPU, memory, mailbox depth, crashes и denials;
- time-travel diagnostics по correlation/causation chain;
- resolver property tests, host-function negative security tests, fuzzing и
  process-kill chaos suite;
- deterministic replay для domains, явно выбравших event sourcing; state/outbox
  recovery для остальных;
- explicit `incomplete` для side effects, исход которых после crash невозможно
  доказать.

Exit criterion: намеренно несовместимый внутренний upgrade обнаруживается до
switch либо откатывается; один сломанный plugin локализуется без ручного
удаления всего профиля; пользователь видит degraded, но работающий shell.

### M0 exit criterion

M0 закрыт, когда:

- один host одновременно запускает несколько installations и providers;
- trusted native и sandboxed WASM plugins используют одинаковую capability
  model;
- provider можно заменить и runtime можно безопасно перезапустить;
- state, authority и effects корректно переживают lifecycle и crash paths;
- capability RPC и event transport имеют раздельные contracts и failure modes;
- event transport остаётся bounded, versioned и изолирует slow subscribers;
- каждый persisted domain явно называет source of truth; event sourcing нигде
  не появляется по умолчанию;
- boot продолжает работу при failure необязательного plugin;
- каждое kernel action наблюдаемо;
- proving slice S1 проходит boot, RPC, independent observation и restart;
- по одному reference plugin проходит builtin, native-process и WASM paths без
  изменения consumer;
- kernel всё ещё не импортирует browser, UI, inference или agent types.

## 7. M1 — Browser

### Цель

Поставить первый полезный vertical composition: browser plugins + UI plugins +
workspace plugin внутри одного runtime. На этом этапе agent plugins могут
отсутствовать; никакой browser component при этом не получает особого пути в
kernel.

### M1.1 — Browser contract and engine spike — Done

Рубеж закрыт:
- Разработан `worldline-browser-contract`: 8 versioned capability contracts
  (`browser.context`, `browser.page`, `browser.navigate`, `browser.observe`,
  `browser.query`, `browser.act`, `browser.download`, `browser.permission`).
- Строго разделены права: `ObservePage`, `QueryDocument`, `NavigatePage`,
  `ActOnPage`, `ControlDownload`, `ManagePermission`.
- Предотвращены confused-deputy атаки: провайдер строго проверяет совпадение
  admitted `ResourceId` из `InvocationContext` с целевым ресурсом полезной нагрузки.
- Введены бюджетные ограничения `QueryBounds` с флагами `is_truncated` и подсчетом отсеченных узлов.
- Ссылки `ElementRef` привязаны к `(PageId, DocumentRevision)`; устаревшие
  ссылки отклоняются с явной ошибкой.
- Логические идентификаторы профилей (`profile_id`) изолируют пользовательские данные
  без утечки путей файловой системы хоста в ABI.
- Реализована публикация типизированных событий через `InvocationContext::publish_event` в M0.4 транспорт ядра
  (`browser.page.created`, `browser.navigation.committed`, `browser.page.closed`, `browser.download.started`),
  проверенная через авторизованные pull-подписки `SubscriptionHandle`.
- Иерархические контекстные полномочия строго проверяют принадлежность страницы (`get_page_context`)
  без строковых префиксных байпассов.
- Реализован настоящий out-of-process спайк Chromium на Windows (`worldline-browser-spike/src/chromium.rs`):
  автоматическое обнаружение браузера, запуск headless-процесса (с политикой fail-closed в CI),
  управление по CDP через WebSocket, навигация по локальным HTML-фикстурам, извлечение дерева доступности Blink,
  точная адресация семантических элементов через `ElementRef.node_key` и исполнение действий в DOM,
  а также изоляция сбоя рендерера с гарантированным выживанием супервизора и хоста Worldline.
- Сохранен детерминированный in-memory эталон (`ReferenceBrowserSupervisor`) с полной
  поддержкой всех 8 контрактов для быстрых регрессионных проверок.
- Оформлен [ADR-BROWSER-ENGINE-V1](docs/adr/ADR-BROWSER-ENGINE-V1.md),
  в котором четко разделены измеренные эмпирические факты (холодный старт ~580 ms, RAM ~135 MB)
  и качественный анализ кандидатов, обосновавший выбор Chromium/CEF для M1.2.

Exit criterion: browser contracts компилируются без зависимостей от движков;
kernel не содержит типов браузера; спайк доказывает реальную изоляцию процессов и навигацию без
публичной сети; ADR обосновывает выбор движка на основе прямых измерений. Evidence: `worldline-browser-contract`
acceptance suite, `worldline-browser-spike` real Chromium and reference acceptance and measurement suites,
сохраненные M0 CI gates.

### M1.2 — Browser engine provider process — Done

- [ADR-BROWSER-ENGINE-PROVIDER-PROCESS-V1](docs/adr/ADR-BROWSER-ENGINE-PROVIDER-PROCESS-V1.md)
  зафиксировал топологию native provider process, FFI-границу, thread-affinity UI message loop,
  Windows Job Object containment и модель безопасности.
- `worldline-browser-contract` расширен experimental v0.1 контрактами (`browser.capture`,
  `browser.engine.cookies`, `browser.engine.storage`, `browser.engine.download-hook`)
  и аддитивными событиями с полной обратной совместимостью v1.0.
- `worldline-native-host` получил Job Object containment на Windows (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`)
  для автоматического завершения многопроцессных деревьев Chromium/CEF при выходе хоста,
  а также bounded blob streaming protocol (`BlobRequest`/`BlobResult`).
- `worldline-browser-provider` реализовал `BrowserProviderCore`, CAS-резервацию генераций страниц,
  строгую валидацию устаревания `ElementRef` и детерминированный `ReferenceBrowserBackend`.
- `worldline-browser-cef` реализовал C FFI-адаптер, ранний subprocess dispatch helper (`early_subprocess_dispatch`),
  потокобезопасный `CefLoopRunner` и Windows headful windowing.
- `worldline-browser-provider-process` скомпоновал автономный native бинарник для работы по `worldline-plugin-protocol`.
- Добавлен и подтвержден сквозной proving slice S2 (`worldline-reference/src/s2.rs`),
  доказывающий полный цикл создания контекста, страницы, навигации, семантических запросов,
  действий, визуального захвата и изоляции данных.

Exit criterion: CEF/reference provider исполняется в изолированном supervised process за stable IPC;
процессное дерево CEF гарантированно сворачивается при выходе хоста; действия со старыми `ElementRef`
отклоняются; visual capture стримится через контентно-адресуемые блобы; proving slice S2 постоянно зелёный.

### M1.3 — Browser service plugins

CEF provider предоставляет engine primitives, а не монолитный браузер. Поверх
контрактов отдельно подключаются:

- `tabs` — page grouping, selection и restore projection;
- `history` — durable navigation history;
- `downloads` — policy, progress, destination и artifacts;
- `cookies` — inspection, policy и workspace/profile boundaries;
- `devtools` — диагностическая surface;
- `adblock` — request/content policy;
- `search` — выбранные search providers;
- `crypto/trust compatibility` — опциональные trust anchors, client certificates,
  signing/verification и legacy-web bridges для enterprise/state web applications;
- позднее любые дополнительные browser capabilities без изменения CEF plugin.

Криптографическая compatibility family должна позволять подключать НУЦ,
корпоративные CA, client certificates через Windows CNG/CSP, PKCS#11 или
КриптоПро, а также CMS/CAdES/XML-signing и аналогичные операции как отдельные
plugins/capabilities, а не как policy в kernel. Private key material не должен
экспортироваться в renderer или передаваться как обычный payload: browser/web
surface получает только типизированный результат разрешённой операции.

Если совместимость требует ГОСТ непосредственно в TLS/network stack, это не
обычный service plugin, а отдельный browser-engine provider/profile за тем же
`BrowserEngine` contract (например, `cef.gost`). Стандартный CEF provider остаётся
обычным default; установка криптографического профиля не должна загрязнять или
ослаблять его security model.

WebApp/origin compatibility profile может выбирать trust/client-cert/crypto и
engine providers для конкретного приложения или origin, но такая UX/config
группировка не объединяет origins, cookie jars, BrowserContexts, permissions или
capability authority. Legacy Native Messaging/CryptoPro-style bridges допустимы
только как явно установленные high-risk native plugins с минимальными grants и
без специального пути в kernel.

Failure history или adblock plugin не должен мешать navigation. Удаление tabs
plugin не удаляет underlying workspace/page facts без отдельной операции.

### M1.4 — UI composition and Worldline Shell

Desktop host лишь загружает bootstrap composition; продуктовые UI surfaces
поставляются plugins.

- renderer/compositor provider; wgpu является начальным кандидатом для spike;
- workspace switcher и workspace home вместо пустого окна с адресной строкой;
- `ui.page-surface` и `ui.tab-bar` как заменяемые presentation plugins;
- `ui.command-palette`, omnibox и extensible commands;
- downloads, history и permission prompts;
- `ui.overlay` и extensible context actions для selection/page/link;
- sidebar допускается как plugin surface, но не является обязательным домом AI;
- task/activity timeline surface, пока ещё без обязательного agent runtime;
- restore последнего workspace после restart;
- accessibility, keyboard navigation и базовая диагностика renderer/engine.

#### Visual DNA — system glass over content

Glass в Worldline является семантическим материалом управляющего слоя, а не
глобальным skin. Базовая карта поверхностей:

| Surface | Default material intent |
| --- | --- |
| Web page, document, media и прочий provider content | В основном opaque; исходная читаемость и визуальная идентичность контента сохраняются |
| Workspace notes, evidence, results и другие долгоживущие content surfaces | В основном opaque или с плотным substrate; не становятся стеклом только потому, что принадлежат Worldline |
| Workspace switcher, tab/navigation chrome и системный status chrome | Glass как устойчивый слой навигации над контентом |
| Floating omnibox, command palette и controls over content | Glass; здесь появляется узнаваемая Worldline lens geometry |
| Selection tools, contextual panels, transient menus и notifications | Glass, пока поверхность действительно временная и расположена над контентом |
| Agent progress, action preview и approval shell | Glass как видимая граница agency; точный payload подтверждения получает плотный readable substrate |
| Dense settings, permissions, diagnostics и длинный текст | Dense/opaque fallback; оптический эффект никогда не важнее точности и читаемости |

Перенос из SignalWeave означает reuse проверенных material contracts, а не
копирование Android UI или пикселей:

- semantic material role отделён от optical geometry, intensity и readability;
- readability pressure не превращает один semantic material в другой, а
  независимо усиливает substrate, contrast, blur floor или opacity;
- optical material bounded собственным mask и не читает backdrop за пределами
  разрешённого composition region;
- stationary chrome остаётся stationary: никаких постоянных waves, swimming,
  глобального RGB ghosting или shimmer в default path;
- nested glass использует явную composition policy и один backdrop sample на
  chrome group, без накопления blur, refraction, rim и opacity;
- shader/pipeline identity остаётся структурно стабильной, а динамические
  параметры меняются uniforms без per-frame shader compilation, pipeline
  rebuild, bitmap allocation или I/O;
- reduced motion, reduced transparency/high contrast и GPU fallback являются
  частью material contract, а не поздним косметическим патчем.

Текущая реализация SignalWeave уже даёт полезные evidence-backed patterns:
semantic material levels, orthogonal readability policy, SDF-bounded
refraction, Fresnel/specular/inner-depth stack, API fallbacks и grouped backdrop
composition. Проектируемые там `FlatPanel`, `ConvexLens` и `CapsuleLens` с
единым thickness field пока рассматриваются Worldline именно как сильная
гипотеза для spike, а не как доказанный готовый renderer.

Worldline-specific signature candidate — floating omnibox или command surface в
форме сфероцилиндрической линзы: цилиндрическая середина почти не искажает
контент вдоль длинной оси, а выпуклые endcaps дают более сильную двумерную
кривизну. Один thickness field `h(x, y)` должен порождать и displacement, и
surface normal; этот же normal используют refraction, Fresnel, specular и
highlights. Так линза остаётся одним оптическим телом, а не набором
несогласованных blur/gradient эффектов.

Для `wgpu` spike проверяются:

1. единый WGSL material core и backdrop texture contract внутри renderer plugin,
   без glass-specific знания в kernel;
2. `FlatPanel`, `ConvexLens` и `CapsuleLens` как geometry profiles одного
   материала, а не отдельные shaders для каждого widget;
3. bounded sampling, clip/mask correctness и clean transmission center;
4. локальная dispersion только там, где optical power достаточно велика, off
   by default и без цветных призраков на тексте;
5. single-backdrop composition для нескольких chrome bounds и явное правило
   nested lens-inside-chrome;
6. frame-time, GPU memory, resize, multi-window, scroll и input-to-frame budgets;
7. полная keyboard, hit-test и accessibility semantics, не выводимая из
   отрисованных пикселей;
8. deterministic dense/opaque fallback при недостаточном GPU capability,
   reduced transparency или невозможности безопасно получить backdrop.

Kill tests визуального направления:

- bright, dark, saturated и rapidly changing page content не разрушает
  читаемость omnibox, approvals и navigation;
- два вложенных glass surfaces не удваивают blur/refraction и не создают bright
  rim stacking;
- default stationary capture остаётся pixel-stable без пользовательского input;
- выключение optical effects не меняет layout, focus order, hit targets или
  capability semantics;
- renderer degradation оставляет Worldline usable, а не блокирует browser boot;
- glass work не задерживает headful browser path: ранний M1 может использовать
  плотный fallback, пока optical compositor ещё проходит spike.

### M1.5 — Minimal durable workspace plugin

- stable `WorkspaceId` и явное создание/переименование/архивация;
- membership pages, downloads, notes и artifacts;
- сохранение layout и открытых/закрытых page references;
- activity facts: open, close, navigate, cite, download, annotate;
- ручное добавление и удаление объектов;
- экспорт/удаление workspace и понятная data ownership policy.

На M1 workspace ещё не обязан понимать смысл исследования. Он уже обязан
переживать закрытие приложения и быть лучшей единицей восстановления, чем
глобальная browser history.

### M1 exit criterion

- Worldline пригоден для ежедневного базового browsing;
- запуск открывает осмысленный workspace, а не пустую AI-панель;
- страницы, profiles, downloads и workspace восстанавливаются после restart;
- engine работает как plugin provider, а kernel не знает browser types;
- один диагностический экран показывает active plugins, selected providers,
  permissions и failures;
- смена browser или renderer provider не меняет shell/workspace consumer
  contracts;
- opaque page/content layer и glass system layer визуально различимы, при этом
  shell сохраняет читаемость, keyboard/accessibility semantics и dense fallback;
- failure необязательного browser/UI plugin приводит к degraded mode, а не к
  bootstrap deadlock.

## 8. M2 — Agentic Workspace

### Цель

Агент становится частью рабочей среды: понимает текущую activity, действует в
нескольких страницах, показывает процесс, сохраняет результаты и продолжает
trajectory после перерыва. Обязательного отдельного «AI sidebar» нет.

### M2.1 — Inference and agent runtime plugins

Начальные inference seams:

```text
inference.text
inference.vision
inference.embeddings
inference.tools
```

- отдельные provider plugins для GLM, OpenAI и local models;
- взаимозаменяемые remote и local providers;
- credentials как references, а не значения в trajectory;
- model routing, budgets, timeouts и cancellation;
- agent loop, planner и memory как независимые заменяемые plugins;
- GitHub и MCP как обычные capability/integration plugins, а не встроенные
  привилегии агента;
- цикл plan -> action -> observation над capability graph;
- interruption, steering, pause, resume и bounded retry;
- causal tree от user intent до каждого capability invocation;
- selective durable run/audit log и projections для UI без требования хранить
  весь agent/workspace state как один event-sourced aggregate.

Exit criterion: provider и agent loop можно заменить независимо; длинная
multitab-задача переживает interruption/restart и не теряет причинную связь
действий.

### M2.2 — Human control and visible agency

- общий approval broker для browser и integration mutations;
- policy различает read, reversible mutation и irreversible external effect;
- browser mutation адресует `PageId + DocumentRevision + ElementId`; stale
  target требует нового observation;
- preview точного текста/полей/получателей перед publish/send/purchase;
- approval действует на конкретный action digest, target revision и TTL и
  истекает при любом их изменении;
- UI показывает текущую страницу, действие, источник authority и результат;
- agent/action overlays используют system glass как видимую границу agency, но
  exact approval payload остаётся на dense readable substrate и не скрывается
  за refraction;
- stop отменяет pending work и отзывает task-scoped authority;
- rollback/undo там, где provider способен его гарантировать.

Kill test: после подтверждения черновика агент не может незаметно изменить
получателя или содержимое и использовать старое approval.

### M2.3 — Semantic observation and in-context interaction

- объяснение selection без copy/paste;
- page summary с цитатами и provenance;
- сравнение текущей страницы с соседними страницами workspace;
- поиск первоисточника и фиксация evidence chain;
- связь issue, diff, source files, specs и тестов;
- vision fallback для поверхностей без надёжной semantics;
- agent-created pages, searches и artifacts автоматически попадают в activity.

### M2.4 — Activity model and research memory

Workspace использует гибридную persistence model, а не автоматически
восстанавливается из универсального event stream:

```text
Workspace: Worldline
├── Intent and open questions
├── GitHub, source and documentation
├── Pages and extracted evidence
├── Notes and generated artifacts
├── Agent runs and approvals
└── Decisions, rejected hypotheses and follow-ups
```

- transactional entity state для workspace metadata, membership, intent,
  goals, open questions и completion state;
- selective trajectory/audit records для agent actions, approvals, decisions и
  rejected hypotheses, где causality действительно нужна;
- blob references для captured content и generated artifacts;
- typed relations между sources, excerpts, claims, decisions и actions;
- provenance для generated summaries;
- retrieval по workspace, времени, entity, decision и trajectory;
- resume/fork agent trajectory без изменения исходного audit log; fork всего
  workspace получает новую identity и explicit lineage;
- понятное различие user-authored facts, observed facts и model inference;
- semantic index является перестраиваемым derived index, а не source of truth;
- выбор event sourcing для отдельного aggregate требует собственного ADR и не
  наследуется от наличия event bus.

### M2.5 — Workspace hygiene

- объяснимое предложение сгруппировать страницы в workspace;
- обнаружение дублей, abandoned branches и низкоценного tab noise;
- команда очистки использует trajectory: cited/used pages сохраняются;
- закрытие страниц сначала обратимо;
- агент формулирует, что уже известно и какие вопросы остались открыты.

### M2 exit criterion

Один end-to-end acceptance journey должен доказать все свойства сразу:

1. Пользователь открывает workspace проекта после restart.
2. Агент читает diff, связанные решения, docs и tests в нескольких surfaces.
3. UI в реальном времени показывает navigation, observations и capability
   actions.
4. Агент выдаёт evidence-backed риски и создаёт отдельный artifact/change draft.
5. Внешняя публикация останавливается на точном approval preview.
6. Через повторный restart пользователь спрашивает, почему было принято старое
   решение, и получает ответ с исходным trajectory/provenance.
7. Команда очистки удаляет tab noise, не теряя использованные evidence.

M2 закрыт, когда этот journey работает без ручного переноса текста в чат и без
скрытых действий агента.

## 9. M3 — Internet Activity OS

### Цель

Workspace продолжает жить, когда страницы и даже UI закрыты. Интернет,
сообщения и внешние системы становятся потоками событий, на которые могут
безопасно реагировать persistent activities.

### M3.1 — Internet event-source plugins

- использовать общий event transport без второй интеграционной шины;
- domain events добавляют source, subject, observed-at, provenance и integrity;
- plugin-provided event sources для web changes, releases, feeds, mail,
  calendars, messages и repository activity;
- durable subscriptions и cursors хранятся в явно выбранном subscription store;
- deduplication, ordering model, retry/backoff и dead-letter handling;
- capability-scoped access к подпискам и содержимому events;
- handler, которому нужно действие, выполняет отдельный capability RPC;
- replay delivery не повторяет необратимые side effects благодаря idempotency и
  committed-action records, а не предположению, что event log является общей
  базой продукта.

### M3.2 — Persistent agent tasks

- schedule, conditional wakeup и explicit stop conditions;
- сохранение task state между process restarts;
- budgets по времени, модели, сети и количеству действий;
- semantic filtering до notification;
- escalation к человеку при изменении scope или authority;
- audit trail: почему task проснулся, что проверил и почему сообщил.

### M3.3 — Integrations

- GitHub как первый двусторонний integration provider;
- затем mail/calendar/feed providers по реальным product journeys;
- read и write capabilities разделены;
- secrets хранятся вне plugin state;
- минимальные resource scopes и per-workspace grants;
- form fill/draft отделены от submit/send;
- connector failure не блокирует остальные activities.

### M3.4 — Semantic notifications

- уведомлять по выполненному условию, а не по каждому новому событию;
- объединять дубли и связанные updates;
- показывать evidence, condition match и использованный provider;
- поддерживать snooze, mute, revise condition и stop monitoring;
- notification chrome может использовать system glass, но evidence и condition
  text остаются opaque/dense и читаемыми без backdrop effects;
- измерять precision полезных уведомлений и стоимость false positives.

### M3 exit criterion

Пользователь создаёт правило «следи за релизами GLM-5.3-Flash и сообщи только,
если исправлена проблема длинных agent runs», закрывает Worldline и позже
получает одно релевантное уведомление с changelog evidence. Task переживает
restart, не превышает budget, не получает лишних permissions и может быть
полностью остановлен.

После этого Worldline перестаёт быть только браузером: он хранит и продолжает
деятельность пользователя в информационной среде.

## 10. Planning envelope

Оценка ниже — способ проверить порядок и параллельность, а не обещание даты.
Предпосылки: два опытных Rust-разработчика, Windows-first architectural MVP,
существующий kernel bootstrap, отсутствие Chromium fork и сознательный отказ
от product polish на этом этапе.

| Относительный период | Основной critical path | Допустимая параллельная работа |
| --- | --- | --- |
| Каждая неделя | Proving slice остаётся runnable и получает следующий самый тонкий end-to-end outcome | Subsystem work не считается интегрированным до попадания в slice |
| Недели 1–2 | Завершить M0.1, вернуть green baseline, довести S0 до host boot/RPC/result | Boundary review, CEF и UI feasibility spikes без production coupling |
| Недели 3–5 | Runtime identity, async lifecycle, partial activation; S1 с restart | Раздельные event-transport и persistence prototypes, versioned IPC |
| Недели 6–9 | WIT/Wasmtime boundary, domain storage/recovery decisions | S2: CEF provider process и минимальная UI composition |
| Недели 10–12 | Upgrade/rollback, safe mode, compatibility/chaos gates | S3: tabs/workspace, один model provider и один видимый agent action |
| Недели 13–14 | Сквозной security, recovery и failure acceptance S0–S3 | Расширение breadth только при зелёном slice |

Результат этого окна — **architectural MVP**, а не завершённые M1 или M2:
один reference provider каждой основной family, один sandboxed plugin, один
browser page, один UI surface и один agent action проходят общие contracts.

При тех же предпосылках product-grade M1 + первый цельный M2 journey следует
считать работой масштаба как минимум 4–6 месяцев от старта, пока engine/UI
spikes не дадут более точные данные. Cross-platform polish, public plugin ABI,
ecosystem и M3 в эту оценку не входят. После каждого двухнедельного gate
диапазон пересчитывается по фактической скорости и обнаруженным рискам.

## 11. После M3: ecosystem и network userland

Эти направления не должны опережать стабилизацию plugin protocol:

- stable manifest, SDK, WIT versioning и conformance testkit;
- signing, provenance, permission review и reproducible packages;
- install/update/rollback/quarantine;
- plugin inspector и debugger;
- registry/marketplace только после compatibility и supply-chain gates;
- multi-device identity и encrypted workspace synchronization;
- portable state там, где это допускает provider contract;
- desktop capabilities и computer-use за пределами browser surface;
- external application surfaces без размывания approval boundary.

Marketplace не является критерием раннего product-market fit. Сначала Worldline
должен доказать собственные M1–M3 journeys на тех же публичных contracts,
которые затем получит ecosystem.

## 12. Сквозная проверка исходных сценариев

| Сценарий | Первый рубеж, где он должен работать полностью |
| --- | --- |
| Проверка diff против архитектурных инвариантов и поиск старого решения | M2 |
| Объяснить selection, сравнить вкладки, найти первоисточник | M2 |
| Вернуться к сравнению телефонов и найти пробелы исследования | M2 |
| Следить за релизом в фоне по смысловому условию | M3 |
| Подготовить ответ/issue и остановиться перед отправкой | M2 |
| Очистить вкладки по фактическому использованию | M2 |
| Продолжить деятельность через месяц, а не открыть список URL | M2 |

## 13. Метрики, которые защищают продуктовый замысел

Метрики вводятся по мере появления соответствующего рубежа. Они не заменяют
acceptance journeys.

- **Context recovery:** доля возобновлённых workspaces, где пользователь не
  вынужден заново собирать исходные страницы и объяснять задачу.
- **Provenance coverage:** доля утверждений и agent actions, для которых можно
  открыть источник и причинную цепочку.
- **Visible agency:** доля действий, отражённых в timeline до или во время
  выполнения, а не обнаруженных постфактум.
- **Approval integrity:** ноль external effects, выполненных с отсутствующим,
  просроченным или не соответствующим action digest подтверждением.
- **Workspace signal:** доля сохранённых объектов, реально использованных при
  последующем resume; доля безопасно убранного tab noise.
- **Resume success:** доля persistent tasks, корректно продолженных после
  restart без дублирования side effects.
- **Plugin replaceability:** conformance suite проходит минимум на двух
  providers ключевого capability без изменения consumer.
- **Notification precision:** доля фоновых уведомлений, которые пользователь
  считает соответствующими заданному условию.
- **Visual control clarity:** пользователь отличает content от Worldline
  controls/agency, а readability, accessibility и task completion не ухудшаются
  при glass on/off или degraded renderer fallback.

## 14. Правила изменения roadmap

1. Каждый implementation change ссылается на один рубеж и один проверяемый
   outcome.
2. Новый capability сначала описывает contract, authority и lifecycle, затем
   получает provider и consumer.
3. Event bus никогда не используется для реализации RPC, command dispatch или
   provider response.
4. Для каждого bounded domain явно фиксируется source of truth; event sourcing
   требует отдельного ADR и не является platform default.
5. Каждый planning cycle сохраняет зелёный end-to-end proving slice. Новый
   subsystem не считается прогрессом roadmap, пока не использован этим slice.
6. Milestone не закрывается демонстрацией happy path: обязательны crash,
   revocation, restart и denial tests, соответствующие риску.
7. Любой browser-, model- или provider-specific обход в kernel требует
   отдельного архитектурного решения и по умолчанию отклоняется.
8. Product spike может быть выброшен. Production code не опирается на spike,
   пока системный gate не закрыт.
9. Календарные обещания добавляются только после декомпозиции milestone на
   changes, определения владельцев и измерения фактической скорости.
10. Если новый feature улучшает чат, но не улучшает деятельность внутри
    workspace, он не является приоритетом Worldline.
11. Glass обозначает system control plane над контентом. Он не применяется к
    каждому panel, не расширяет kernel и не принимается без readable fallback,
    accessibility semantics, performance evidence и nested-composition tests.

Короткая формула roadmap:

> С первого дня держать зелёный end-to-end slice. Вокруг него доказать
> безопасную композицию plugins, затем расширить browser, наблюдаемого агента с
> памятью деятельности и, только после этого, постоянную событийную среду
> интернета. RPC, events и storage при этом остаются разными planes.
