# Worldline CI/CD Policy v0.1

Статус документа: living engineering policy
Зафиксирован: 28 августа 2026 года

Этот документ определяет, как изменения Worldline проходят от локального
рабочего дерева до проверенного commit, release candidate и публикуемого
артефакта.

Он описывает не конкретную конфигурацию GitHub Actions, а долгоживущие
инварианты CI/CD проекта.

Конкретные workflows, scripts, required checks и release jobs могут меняться,
если сохраняются изложенные здесь свойства.

---

## 1. Зачем Worldline нужен отдельный CI/CD contract

Worldline строится как plugin-oriented userland operating environment, где
корректность определяется не только тем, компилируется ли код.

CI должен постоянно доказывать как минимум:

- корректность Rust-кода;
- сохранение архитектурных инвариантов;
- capability/security boundaries;
- lifecycle/recovery semantics;
- отсутствие скрытых product-specific зависимостей в kernel;
- работоспособность сквозных proving slices;
- воспроизводимость сборки;
- пригодность артефакта к выпуску.

Главный принцип:

> **CI PROVES A COMMIT. CD PUBLISHES AN EXPLICITLY SELECTED RELEASE.**

Зелёный `master` означает проверенное состояние исходного дерева.

Он НЕ означает автоматическую публикацию новой версии пользователям.

---

## 2. Основные инварианты

> **IF IT CANNOT RUN LOCALLY, IT SHOULD NOT EXIST ONLY IN CI.**

Основная verification logic должна быть доступна через repository scripts,
Cargo commands или отдельные test binaries.

GitHub Actions является orchestration layer, а не единственным местом, где
существует логика проверки.

---

> **PROVING SLICES ARE FIRST-CLASS CI GATES.**

S0, S1 и последующие proving slices являются постоянными архитектурными
тестами.

Поломка активного proving slice блокирует дальнейшее расширение соответствующего
roadmap milestone.

Slice не превращается в необязательный demo после появления следующего slice.

---

> **NO GREEN BY SKIPPING.**

Required check не может становиться зелёным потому, что критический тест:

- не нашёл нужный binary;
- не смог запуститься;
- был условно пропущен;
- потерял test discovery;
- столкнулся с неподдерживаемой платформой;
- завершился через `continue-on-error`.

Такой результат должен быть явным failure либо отдельным non-required
informational job.

---

> **FAST FAILURES FIRST, EXPENSIVE EVIDENCE LATER.**

Дешёвые deterministic проверки выполняются раньше дорогих integration,
sanitizer, fuzz и chaos suites.

---

> **ARTIFACT CREATION IS NOT RELEASE.**

CI может собирать binaries/packages для проверки.

Публичным release они становятся только через отдельный release workflow.

---

> **RELEASES ARE BUILT FROM AN IMMUTABLE COMMIT.**

Release artifact всегда связан с точным Git commit и version/tag.

Нельзя собирать публичный release из mutable working branch state.

---

> **CI HAS NO AMBIENT AUTHORITY.**

Workflow получает минимальные GitHub permissions и secrets только там, где они
реально требуются.

PR verification не получает release authority.

---

## 3. CI/CD planes

Worldline разделяет четыре режима automation.

### 3.1 Pull Request Gate

Цель:

быстро доказать, что change достоин merge и не нарушает текущий архитектурный
baseline.

PR gate является главным developer feedback loop.

Ориентир:

- быстрый;
- deterministic;
- reproducible локально;
- required для merge.

---

### 3.2 Master Gate

Цель:

доказать корректность интегрированного состояния default branch.

Master gate может быть шире PR gate:

- больше feature combinations;
- больше platform coverage;
- integration/recovery tests;
- build artifacts;
- documentation verification.

Failure master gate считается regression даже если отдельный PR ранее был
зелёным.

---

### 3.3 Nightly / Deep Verification

Цель:

искать ошибки, которые слишком дороги, вероятностны или длительны для каждого
PR.

Сюда постепенно входят:

- fuzzing;
- property testing;
- Miri;
- sanitizers;
- long-running concurrency;
- repeated restart/recovery;
- process-kill chaos;
- compatibility matrices;
- dependency/security deep scans.

Nightly failure не должен незаметно жить неделями.

Каждый failure либо:

- создаёт actionable issue;
- получает explicit triage;
- либо suite переводится в documented experimental status.

---

### 3.4 Release Gate

Цель:

из уже проверенного immutable commit получить публикуемый набор артефактов.

Release является отдельным событием.

Ни merge в `master`, ни успешный nightly не публикуют release автоматически.

---

## 4. Gate model

Проверки группируются по смыслу, а не только по названиям workflow jobs.

### G0 — Source Hygiene

Доказывает базовую чистоту дерева.

Минимальный baseline:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps
````

Также:

* committed `Cargo.lock` должен соответствовать workspace;
* generated source не должен неожиданно изменять repository tree;
* запрещены случайно закоммиченные binaries, credentials и build outputs.

---

### G1 — Correctness

Доказывает функциональные contracts.

Минимум:

```text
cargo test --workspace
```

По мере роста проекта сюда входят:

* unit tests;
* integration tests;
* acceptance tests;
* contract tests;
* property tests, достаточно быстрые для PR;
* compile-fail/type-level tests при необходимости.

Тест считается полезным только если failure сообщает конкретно нарушенный
contract.

Реальный Chromium/Edge engine spike не входит в `cargo test --workspace` и не
исполняется внутри обычного G1 Correctness suite. Он является отдельным
required platform gate, потому что требует Windows-hosted runner и запускает
внешний браузерный процесс:

```text
pwsh -NoProfile -File scripts/ci/Invoke-WorldlineCi.ps1 -Suite RealChromium
```

Этот gate выполняется один раз на pull request и на `master`. Он остаётся
fail-closed при отсутствии или невозможности запуска браузера; feature-gating
теста не превращает пропущенный real-engine evidence в success.

---

### G2 — Architecture

Доказывает, что код сохраняет архитектурную конституцию Worldline.

Примеры:

* kernel не получает browser/agent/UI domain dependencies;
* forbidden dependency directions отсутствуют;
* plugin/reference crates зависят от kernel, а не наоборот;
* capability RPC не начинает использовать event subscribers как responders;
* event transport не становится persistence layer;
* trusted native/WASM adapters не меняют logical capability contract.

Architecture checks должны по возможности быть executable.

Текст ADR или roadmap не заменяет test там, где invariant можно проверить
машиной.

---

### G3 — Security

Доказывает authority и isolation semantics.

В текущем M0 сюда входят:

* default-deny capability access;
* grant/revoke;
* delegation attenuation;
* runtime authority discontinuity;
* state lease revocation;
* ProviderSelf isolation;
* anti-spoofing trusted metadata;
* installation/runtime principal separation.

Позже:

* malicious WASM tests;
* filesystem/network permission denial;
* quota/resource exhaustion;
* hostile package tests;
* approval-integrity tests.

Security regression всегда является required failure.

Для неё нет категории "flaky, поэтому ignore".

---

### G4 — Lifecycle and Recovery

Доказывает поведение не только happy path.

Сюда входят:

* activation/deactivation failure;
* panic containment;
* cancellation;
* deadlines;
* runtime replacement;
* crash/restart;
* stale completion rejection;
* state recovery;
* migration failure;
* provider disappearance;
* degraded boot;
* quarantine;
* fault injection.

После появления production persistence этот gate расширяется process-kill
recovery matrix.

---

### G5 — Packaging and Supply Chain

Появляется постепенно по мере приближения к распространяемому application.

Содержит:

* reproducible build inputs;
* pinned toolchain;
* dependency policy;
* license policy;
* vulnerability scanning;
* package manifest validation;
* SBOM;
* artifact checksums;
* signatures / attestations;
* provenance;
* installer/package smoke tests.

---

## 5. Proving slice track

Roadmap proving slices являются частью CI contract.

Текущий track:

```text
S0 — Kernel
S1 — Durable runtime
S2 — Browser
S3 — Agentic
S4 — Background
```

Каждый slice является end-to-end acceptance path.

Общее правило:

```text
horizontal platform work
        ↓
integration into current proving slice
        ↓
only then broader feature expansion
```

Ни одна значимая horizontal platform branch не должна оставаться без integration
в proving slice дольше одного планового development cycle.

### S0

Проверяет базовую композицию:

```text
host
  -> capability resolution
  -> authorization
  -> provider
  -> result
  -> observation
```

### S1

Добавляет:

```text
boot
 -> installation/runtime
 -> RPC
 -> state mutation
 -> independent observation
 -> restart
 -> state recovery
 -> fresh runtime authority
```

### S2

Добавит реальный browser vertical slice:

```text
shell command
 -> browser provider boundary
 -> navigation
 -> rendered page
 -> state/history recovery
```

### S3

Добавит agent path:

```text
user intent
 -> agent
 -> inference
 -> browser capability
 -> visible action/result
 -> exact approval where required
```

### S4

Добавит background activity:

```text
event source
 -> persistent subscription
 -> task wakeup
 -> semantic evaluation
 -> notification
```

Каждый новый slice расширяет предыдущий contract, но не отменяет старые
lower-level suites.

---

## 6. Pull Request pipeline

Целевая структура PR pipeline:

```text
                ┌─ fmt
                ├─ check
PR ──> source ──┼─ clippy
                ├─ docs
                │
                ├─ unit / acceptance
                ├─ architecture
                ├─ security
                └─ proving slice
```

### Required PR checks

На текущем этапе:

```text
source
correctness
architecture-security
proving-slice
```

Jobs могут быть разбиты иначе для скорости, но required GitHub checks должны
иметь стабильные имена.

Branch protection не должна зависеть от десятков динамически меняющихся matrix
job names.

### PR rules

PR CI:

* не публикует release;
* не имеет signing/release secrets;
* не изменяет repository;
* не push'ит commit;
* не создаёт tag;
* не имеет write authority без отдельной доказанной необходимости.

Для untrusted/fork PR secrets недоступны.

---

## 7. Master pipeline

Push в `master` запускает полный integration baseline.

Минимальный master gate:

```text
G0 Source Hygiene
G1 Correctness
G2 Architecture
G3 Security
G4 Recovery
current proving slices
docs
artifact build
```

Master artifact может храниться как CI artifact для диагностики и smoke testing.

Он не считается публичным release.

---

## 8. Platform matrix

Matrix должна отражать реальную поддержку продукта, а не желание иметь побольше
зелёных клеточек.

На раннем kernel этапе:

* один Linux runner может быть быстрым primary verification environment;
* Windows должен стать required там, где появляется Windows-first runtime,
  browser host, IPC, filesystem или packaging behavior;
* macOS добавляется после появления реального supported product path.

Нельзя заявлять platform support только потому, что `cargo check` там прошёл.

### Platform-specific tests

После появления browser provider real Chromium acceptance запускается отдельным
Windows required gate (`Real-Chromium`), а Linux workspace/correctness gates
остаются детерминированными и не зависят от внешнего браузерного процесса.

Если поведение зависит от ОС, test должен явно исполняться на соответствующей
ОС.

`cfg(target_os)` не должен превращать важный required test в silently skipped
test на единственном CI runner.

---

## 9. Rust toolchain policy

Repository содержит authoritative `rust-toolchain.toml`.

CI использует repository-pinned toolchain и не выбирает случайный latest stable.

PR и release должны по умолчанию использовать один declared baseline toolchain.

Обновление Rust toolchain является отдельным reviewable change.

### Additional toolchains

Nightly Rust разрешается использовать для:

* Miri;
* sanitizers;
* experimental compiler checks;

но nightly-specific failure не должен менять production MSRV/toolchain
неявно.

---

## 10. Dependency caching

Caching является performance optimization, не частью correctness.

Допускается кэширование:

* Cargo registry;
* Cargo git checkout;
* compiled target data, если invalidation надёжна.

Cache key должен учитывать как минимум:

* OS;
* architecture;
* Rust toolchain;
* `Cargo.lock`;
* relevant feature/build configuration.

Cache miss обязан приводить только к более медленной сборке.

Cache hit не может быть необходим для успешной сборки.

Release reproducibility не должна зависеть от mutable CI cache.

---

## 11. Dependency and license policy

`Cargo.lock` коммитится и является частью reviewed build input.

CI должен постепенно получить автоматические проверки:

* known vulnerable dependencies;
* duplicate/forbidden dependency policy;
* allowed licenses;
* forbidden sources;
* git dependencies;
* unreviewed source origins.

Предпочтительный direction для Rust ecosystem:

```text
cargo-deny
cargo-audit or equivalent advisory check
```

Конкретный инструмент выбирается implementation change и может быть заменён,
если contract проверки сохраняется.

Security advisory finding должен иметь explicit policy:

* deny;
* temporary allow with reason;
* expiration/review date.

Бесконечный global ignore запрещён.

---

## 12. Workflow security

Каждый workflow начинает с минимального permission set.

Default:

```text
contents: read
```

Write permissions добавляются только конкретному job, которому они необходимы.

### Pull requests

PR jobs не получают:

* release token;
* signing key;
* package publishing credential;
* production credential.

### Third-party Actions

Используемые actions должны быть pinned на immutable revision.

Floating mutable tags вроде:

```text
some/action@main
```

для security-sensitive workflow запрещены.

Допускается комментарий с human-readable version рядом с pinned revision.

### Secrets

Secrets:

* не печатаются в log;
* не передаются build script без необходимости;
* не используются для обычного compile/test;
* отделены по environment.

Release environment должен иметь отдельную GitHub protection boundary.

---

## 13. Local CI parity

В repository должен существовать canonical local verification entrypoint.

Целевая форма:

```text
scripts/ci/verify
```

или эквивалентный cross-platform command.

Он должен запускать основной merge-blocking baseline.

Например:

```text
verify source
verify test
verify architecture
verify proving
verify all
```

GitHub Actions вызывает те же commands.

Нельзя поддерживать две независимые реализации:

```text
локально: одна логика
CI YAML: другая логика
```

### Windows

Если shell scripts становятся проблемой для Windows-first repository,
предпочтительно использовать:

* небольшой Rust `xtask`;
* либо другой repository-owned cross-platform runner.

Например:

```text
cargo xtask ci
cargo xtask ci --gate architecture
cargo xtask ci --gate proving
```

По мере усложнения Worldline `xtask` предпочтительнее большого набора
platform-specific shell scripts.

---

## 14. Test determinism

Required CI tests должны быть deterministic насколько это практически возможно.

Не допускается зависимость required kernel tests от:

* публичного интернета;
* случайно доступного remote API;
* live LLM;
* времени суток;
* внешнего GitHub repository state;
* arbitrary sleep как механизма синхронизации.

### Time

Deadline/backoff/scheduler tests используют injected/controlled time там, где это
возможно.

### Randomness

Property/fuzz failure обязан печатать seed/reproducer.

### Concurrency

Concurrency tests должны проверять состояние/coordination, а не полагаться на
"sleep 100 ms и, наверное, поток уже закончил".

---

## 15. Flaky test policy

Required test не может постоянно rerun'иться до зелёного результата.

Автоматический retry допустим только для диагностики и не должен скрывать
первоначальный failure.

Если test flaky:

1. failure считается настоящим;
2. сохраняется evidence;
3. открывается issue;
4. test либо исправляется;
5. либо временно переводится из required suite с явной причиной и сроком.

Правило:

> **FLAKINESS IS A BUG IN THE TEST OR SYSTEM, NOT A CI FEATURE.**

---

## 16. Time budgets

CI должен иметь performance budgets.

Начальные targets, а не contractual SLA:

```text
PR fast feedback:        < 10 min desirable
Full PR required gate:   < 20 min desirable
Master deep baseline:    < 30 min desirable
Nightly:                 may run significantly longer
```

Рост времени CI должен быть наблюдаемым.

Новый expensive suite должен объяснять, почему он:

* required per PR;
* master-only;
* либо nightly.

---

## 17. Artifacts

CI artifacts используются для:

* debugging;
* test reports;
* crash logs;
* coverage reports;
* binaries for smoke tests;
* release candidates.

Artifact должен иметь metadata:

```text
git commit
build target
toolchain
profile
workflow/run identity
```

Artifact из PR не является trusted release artifact.

Retention policy определяется стоимостью и диагностической ценностью.

---

## 18. Logs and diagnostics

Failure должен оставлять достаточно evidence, чтобы его можно было
воспроизвести без доступа к ephemeral runner.

Для сложных suites сохраняются:

* structured logs;
* failed test identity;
* seed;
* fault-injection point;
* platform/toolchain;
* relevant backtrace;
* test artifact/reproducer, если безопасно.

Logs не должны содержать:

* secrets;
* raw credentials;
* user private data;
* raw capability payloads, если они могут быть sensitive.

---

## 19. Nightly / deep suites

Nightly постепенно получает следующие группы.

### Concurrency

* repeated lifecycle races;
* RPC cancellation/deadline races;
* provider replacement races;
* state CAS contention.

### Recovery

* restart at critical transition points;
* fault injection;
* persistence recovery;
* outbox recovery after M0.5.

### Rust deep checks

* Miri where applicable;
* sanitizer builds where supported;
* additional compiler diagnostics.

### Fuzzing

Приоритетные поверхности:

* manifest parsing;
* capability/event envelopes;
* version negotiation;
* WIT/IPC codecs;
* state migration inputs;
* package metadata.

### Chaos

После появления process boundaries:

* kill provider;
* kill host;
* corrupt/interrupt state transition;
* exhaust quota;
* delay IPC;
* close channel unexpectedly.

---

## 20. Architecture CI

Architecture invariants должны иметь отдельный machine-readable gate.

Примеры будущих checks:

```text
kernel does not depend on browser crates
kernel does not depend on inference crates
kernel does not depend on renderer crates

reference -> kernel
kernel !-> reference

event transport !-> browser domain
RPC provider resolution !-> event subscriber registry
```

Для dependency rules предпочтительно проверять Cargo metadata/dependency graph,
а не grep, если возможно.

Grep допустим только для простых guardrails и не считается полноценным semantic
proof.

---

## 21. Security CI

Security suite разделяется на:

```text
authorization semantics
lifecycle authority cleanup
malicious-input tests
dependency/supply-chain checks
sandbox tests
```

После M0.6 hostile plugin suite становится required для relevant changes.

Пример будущего acceptance:

```text
untrusted plugin attempts:
  filesystem without grant       -> deny
  network without grant          -> deny
  forged principal metadata      -> deny
  stale capability handle        -> deny
  quota exhaustion               -> contained
  panic/trap                     -> isolated
```

---

## 22. Release model

Worldline использует explicit release model.

```text
verified master commit
        ↓
version/tag selection
        ↓
release workflow
        ↓
build
        ↓
release verification
        ↓
sign / attest
        ↓
publish
```

Никакой обычный push не публикует release.

### Release trigger

Допустимые варианты:

* signed/approved Git tag;
* explicit GitHub release workflow dispatch.

Окончательное решение фиксируется отдельным release-engineering change.

---

## 23. Release candidate

Release workflow сначала создаёт immutable candidate artifacts.

Candidate проходит:

* full test baseline;
* platform build;
* package validation;
* smoke tests;
* supply-chain checks;
* metadata/provenance validation.

Только после прохождения candidate становится публикуемым artifact set.

---

## 24. Versioning

До стабилизации public plugin/IPC ABI Worldline может использовать pre-1.0
versioning.

Release version является отдельным product version.

Она не должна автоматически совпадать с:

* kernel internal schema;
* WIT interface version;
* plugin package version;
* state schema version.

Эти version domains независимы.

---

## 25. Release provenance

Каждый публичный artifact должен быть однозначно связан с:

* Git repository;
* exact commit SHA;
* release version/tag;
* target platform/architecture;
* toolchain;
* workflow identity;
* dependency lockfile.

По мере появления production releases добавляются:

* SBOM;
* checksums;
* signing;
* build provenance/attestation.

Предпочтение отдаётся short-lived/OIDC-based signing и attestation там, где это
практически возможно, вместо долгоживущих repository signing secrets.

---

## 26. Rollback

CD design обязан учитывать rollback до появления automatic update system.

Минимальное правило:

> **A RELEASE NEVER DESTROYS THE ABILITY TO RETURN TO THE PREVIOUS WORKING
> ARTIFACT.**

Release pipeline сохраняет предыдущие immutable packages.

Когда появятся migrations/update mechanisms:

```text
stage
 -> migrate copy
 -> validate
 -> switch
 -> rollback if unhealthy
```

Новая версия не должна необратимо портить единственную рабочую state copy до
успешной validation.

---

## 27. Branch strategy

До появления реальной необходимости Worldline не использует сложную Git flow
модель.

Основная модель:

```text
short-lived branch
    ↓
pull request
    ↓
required CI
    ↓
master
```

`master` должен оставаться releasable по engineering quality, даже если release
из него не публикуется автоматически.

Long-lived release branches вводятся только после появления доказанной
необходимости поддерживать несколько published release lines.

---

## 28. Branch protection

Целевой `master` protection:

* required status checks;
* branch must be up-to-date before merge, если integration race становится
  практической проблемой;
* no force-push;
* no branch deletion;
* review requirements могут быть усилены при росте числа contributors.

Administrator bypass допустим только как аварийная операция.

Bypass не превращает failing commit в accepted baseline:
после emergency merge CI всё равно обязан пройти либо regression должен быть
явно зафиксирован.

---

## 29. Merge policy

Предпочтительный default до появления иной необходимости:

```text
squash merge
```

если один GRACE change представлен одним PR.

Это сохраняет связь:

```text
approved change
    ↕
review unit
    ↕
master commit
```

Если change намеренно состоит из нескольких independently valuable commits,
может использоваться другой merge mode.

CI/CD policy не требует механически уничтожать meaningful history.

---

## 30. GRACE integration

Каждый значимый implementation PR должен уметь ответить:

```text
какой GRACE change реализуется?
какой roadmap milestone затрагивается?
какой invariant проверяется?
какой proving slice подтверждает integration?
```

GRACE spec может добавлять более строгие checks, чем общий CI baseline.

Он не может ослабить repository-wide security/correctness gates без отдельного
изменения этой policy.

---

## 31. Required checks evolve with roadmap

CI не должен сразу выполнять весь будущий Worldline roadmap.

Gate усиливается, когда появляется соответствующий subsystem.

### Current M0

Обязательны:

```text
fmt
check
clippy
docs
workspace tests
kernel acceptance
security acceptance
state/recovery tests
architecture checks
S0
current S1 when introduced
```

### After production persistence

Добавляются:

```text
process-kill recovery
storage corruption/failure tests
backup/restore validation
outbox atomicity tests
```

### After WASM/native IPC

Добавляются:

```text
WIT conformance
malicious plugin suite
IPC protocol compatibility
resource quota tests
process crash isolation
```

### After browser provider

Добавляются:

```text
browser process startup
navigation smoke
process crash/restart
profile isolation
packaging
S2
```

### After agent runtime

Добавляются:

```text
provider contract tests
tool/approval integrity
deterministic fake-model journeys
S3
```

Live paid LLM API не является required CI dependency.

---

## 32. External services

Required CI не зависит от live third-party services, если можно использовать
contract-compatible fake/local provider.

Это относится к:

* LLM APIs;
* GitHub integration;
* mail;
* calendar;
* search providers;
* web services.

Fake provider должен находиться за тем же logical boundary, что и production
provider.

Test, который напрямую вызывает mock internals вместо public contract, не
является end-to-end evidence.

---

## 33. Coverage

Line coverage не является главным quality metric Worldline.

Coverage может использоваться как diagnostic signal.

Он не заменяет:

* invariant tests;
* negative tests;
* failure injection;
* proving slices.

Не вводится искусственный global percentage target ради цифры.

Для security-sensitive modules допустимы локальные coverage expectations, если
они действительно помогают обнаруживать непротестированные branches.

---

## 34. Performance regression

По мере появления runtime/browser workloads CI добавляет benchmark tracking.

Benchmarks должны разделять:

* correctness gate;
* performance observation;
* hard regression budget.

Нестабильный shared GitHub runner не должен использоваться для ложной
наносекундной точности.

Hard performance gate вводится только когда measurement environment достаточно
стабилен.

---

## 35. Repository-owned CI entrypoint

Целевая структура:

```text
.github/
  workflows/
    pr.yml
    master.yml
    nightly.yml
    release.yml

scripts/
  ci/
    ...

docs/
  CI-CD.md
```

При росте cross-platform logic предпочтителен:

```text
xtask/
```

с interface примерно:

```text
cargo xtask ci fast
cargo xtask ci full
cargo xtask ci architecture
cargo xtask ci security
cargo xtask ci proving
cargo xtask ci nightly
cargo xtask package
```

Workflow YAML остаётся коротким.

---

## 36. Workflow decomposition

Не следует создавать один гигантский `ci.yml` на сотни строк.

Также не следует плодить десятки workflows без semantics.

Целевое деление:

### `pr.yml`

Fast merge gate.

### `master.yml`

Integrated full baseline + test artifacts.

### `nightly.yml`

Expensive/deep verification.

### `release.yml`

Explicit artifact production and publication authority.

Reusable workflows допускаются, если они уменьшают duplication без сокрытия
security boundary.

---

## 37. Failure ownership

Каждый required CI failure является repository regression до доказательства
обратного.

Нельзя оставлять:

```text
"оно иногда красное"
```

как нормальное состояние master.

Если failure инфраструктурный, это всё равно CI defect и он получает triage.

---

## 38. CI observability

Следующие данные должны быть легко видны:

* duration каждого gate;
* cache effectiveness;
* flaky/retry count;
* test count;
* platform matrix;
* artifact creation;
* failure category.

Со временем полезно отслеживать:

```text
PR feedback latency
master failure frequency
nightly failure age
release lead time
```

Метрики не являются целью сами по себе.

Они показывают, когда CI начинает мешать development loop.

---

## 39. Cost control

CI minutes являются ресурсом.

Оптимизация выполняется в таком порядке:

1. не запускать бессмысленную работу;
2. early fail;
3. parallelize independent gates;
4. reuse compilation where safely possible;
5. cache;
6. перенести дорогие low-signal suites в nightly.

Нельзя снижать security/correctness evidence только ради уменьшения CI bill без
явного engineering decision.

---

## 40. Initial implementation envelope

Первая версия CI не обязана реализовать весь этот документ.

`C-INFRA-CI-BASELINE-*` должна реализовать минимальный useful subset.

### Phase A — baseline

```text
GitHub Actions
pinned Rust toolchain
fmt
check
clippy
docs
cargo test --workspace
existing acceptance suites
S0/S1 proving
dependency-direction sanity
artifact/log upload on failure where useful
```

### Phase B — repository policy

```text
stable required check names
master protection
minimal workflow permissions
dependency/license policy
local CI entrypoint
```

### Phase C — deep verification

```text
nightly
Miri/sanitizers where useful
fuzz/property suites
long recovery/concurrency tests
```

### Phase D — release engineering

Только когда появляется первый реально распространяемый application:

```text
platform packaging
SBOM
signing
attestation
release smoke tests
explicit publish
rollback/update verification
```

---

## 41. What CI must not become

CI/CD не должен превращаться в:

* вторую build system поверх Cargo;
* огромную бизнес-логику внутри YAML;
* hidden environment, который нельзя воспроизвести локально;
* способ автоматически "лечить" flaky tests rerun'ами;
* место хранения permanent production credentials;
* mandatory live LLM integration test;
* автоматический release любого commit в master;
* substitute для architecture review;
* substitute для GRACE acceptance criteria.

---

## 42. Current baseline commands

Пока repository-owned higher-level CI runner не создан, authoritative baseline
остаётся:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps
cargo tree -p worldline-kernel
cargo run -p worldline-demo
```

Milestone-specific proving/acceptance binaries добавляются в этот baseline через
CI implementation change.

---

## 43. Policy change rules

Изменение этой policy требует review, если оно:

* удаляет required gate;
* ослабляет security test;
* меняет release authority;
* разрешает новый source of secrets;
* меняет branch protection semantics;
* вводит автоматическую публикацию;
* меняет supported platform gate;
* ослабляет proving-slice requirement.

Обычная оптимизация implementation не требует изменения policy, если observable
contract остаётся прежним.

---

## 44. Короткая формула

> Сначала быстро доказать, что change чистый и корректный.
>
> Затем доказать, что он не разрушил архитектуру и security boundaries.
>
> Затем доказать сквозной сценарий.
>
> Дорогие и вероятностные проверки вынести в deep verification.
>
> И только явно выбранный immutable commit превращать в release.
