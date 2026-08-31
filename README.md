# Worldline kernel bootstrap

Worldline строится как модульный информационный браузер: разбор web-страниц,
браузерная автоматизация и AI-провайдеры подключаются отдельными plugins, а
пользователь сам выбирает провайдера и условия его использования.

Worldline is a modular, user-controlled AI browser kernel for web parsing and
browser automation. Users bring their own AI provider; providers integrate as
isolated plugins behind capability-based authorization.

Текущее продуктовое направление, архитектурные гипотезы, milestones и
проверяемые критерии выхода описаны в [Worldline Roadmap](ROADMAP.md).

Это Rust workspace для Grace Changes `C-KERNEL-PLUGIN-RUNTIME-BOOTSTRAP-20260827`,
`C-KERNEL-CAPABILITY-SECURITY-20260828`,
`C-KERNEL-CAPABILITY-RPC-EVENT-TRANSPORT-20260828`,
`C-KERNEL-PERSISTENCE-RECOVERY-MODEL-20260828` и
`C-KERNEL-STABLE-IPC-WASM-COMPONENT-BOUNDARY-20260831`. Ядро намеренно не знает о
browser engine, LLM, UI, истории, вкладках или конкретном agent runtime.

В workspace входят:

- `worldline-kernel` — plugin contract, структурированные capability identities,
  dependency resolution, installation-scoped runtime lifecycle, opaque
  `RuntimeId`, split-phase activation/deactivation, lifecycle recovery,
  capability discovery/selection, lifecycle scopes, owned effects, principals,
  in-memory grants, resource attenuation, revocation, bounded capability RPC,
  typed event transport, logical event journal, invocation broker, opaque
  external handle table и append-only trajectory;
- `worldline-plugin-protocol` — transport-neutral vocabulary: package/plugin
  identities, manifest schema, WIT component definitions и versioned native
  IPC envelopes;
- `worldline-native-host` — supervised native process execution adapter over
  stdio IPC: handshake, framed envelopes, bounded in-flight requests/stderr и
  graceful shutdown timeouts;
- `worldline-wasm-host` — sandboxed WASM Component Model execution adapter
  (wasmtime, zero-ambient WASI authority, explicit quotas и trap isolation);
- `worldline-reference-external` — cross-mode reference plugin (`reference.echo/v1`),
  conformance suites (`EchoFixture`), malicious WASM containment, protocol
  robustness и external S1 proving paths;
- `worldline-demo` — host-level S0/S1 proving slices, показывающие разницу
  между capability availability и authorization, independent observation,
  restart continuity и compatible provider replacement;
- `worldline-reference` — browser-like, agent-like и UI-like reference
  families, host-local observation fixture и постоянный S0 proving slice;
- `worldline-storage` — production SQLite StateBackend, transactional outbox,
  durable EventJournal, metadata-only audit, content-addressed blobs,
  persistent jobs, backup/restore и hard-kill recovery fixtures.

Boundary decisions M0.2 зафиксированы в
[ADR-KERNEL-BOUNDARY-V1](docs/adr/ADR-KERNEL-BOUNDARY-V1.md). Reference
families используют только generic plugin/capability/state contracts; в
`worldline-kernel` нет family discriminator или product-specific domain types.
`worldline-demo` запускает S0 через два `Kernel` над общим backend: RPC result
и observation идут разными путями, installation state продолжается через
restart, а runtime identity и authority — нет.

`CapabilityHandle` является broker proxy. Наличие resolved dependency или
активного provider не создаёт grant: вызов проходит только при наличии
подходящего active grant. RPC имеет отдельные logical `RpcRequestId` и
per-attempt `InvocationId`, bounded provider flow control, monotonic deadline,
cooperative cancellation и provider-owned retry/idempotency contract. Provider
получает `InvocationContext` и может явно выбрать delegated authority либо
собственную authority; delegated invocation не имеет automatic self-authority
fallback.

Typed Event Transport — отдельный publish/subscribe plane. `EventContract` и
`EventEnvelope` ABI-neutral, producer metadata kernel-stamped, Publish/Subscribe
default-deny, а каждая live subscription получает finite pull mailbox с явным
`RejectForSubscriber`/`DropNewest`/`DropOldest` QoS. События могут быть
`Ephemeral` либо логически `Durable` через абстрактный `EventJournal`; текущий
`InMemoryEventJournal` предназначен для acceptance tests и не является
production crash-safe persistence. Event observation не является RPC reply,
не участвует в provider resolution и не передаёт subscriber producer
authority. S1 proving slice показывает независимые RPC result, observation,
follow-up RPC под authority observer-а и state continuity после restart.

Persistent state принадлежит `InstallationId`, а не `PluginId` и не
эфемерному runtime principal. Runtime получает только kernel-bound
`StateHandle` своего installation; записи меняются через atomic
`StateTransaction`. `unregister` сохраняет installation и его state, тогда как
`uninstall` является отдельной явной операцией удаления. При смене
`StateSchemaVersion` kernel строит directed migration plan и не активирует
новый runtime до успешного migration commit; migration context не имеет
capability-доступа.

Каждая запись также имеет монотонную `StateRevision`. Transaction фиксирует
свою исходную revision, а backend принимает commit только через optimistic
CAS; конфликтующие или stale transactions получают явную ошибку и не могут
затереть чужие изменения или откатить schema. Обычная transaction допускается
только в `Ready` и никогда не меняет schema metadata. Runtime получает
lease-bound `RuntimeStateHandle`: после deactivation/unregister сохранённый
clone handle и уже открытая transaction теряют право на state access.

Ошибки state preparation, migration и backend recovery возвращаются вызывающему
коду; migration path errors (`NoMigrationPath`/`AmbiguousMigrationPath`) не
маскируются generic activation failure. Запуск через custom `StateBackend`
также fallible: ошибки загрузки installation records не трактуются как пустое
хранилище.

Авторизация выполняется в момент допуска invocation: broker проверяет grant до
передачи вызова provider. Поэтому отзыв grant не отменяет уже допущенный
in-flight вызов; он блокирует последующие admissions. Вложенные вызовы также
ограничены максимальной глубиной, а `ProviderSelf` доступен только текущему
provider runtime и использует только его собственные grants.

Observation bus из reference crate остаётся минимальным host-local fixture для
старого S0; kernel Event Transport не смешивается с trajectory или
persistence. M0.3 runtime lifecycle primitives не притворяются production
scheduler: для native in-process plugin cancellation остаётся cooperative, а
`Hung` изолирует authority и publication логически, не убивая произвольный
thread. M0.5 теперь закрывает production SQLite state, transactional outbox,
durable journal, persistent jobs, CAS blobs, backup/restore и crash/restart
evidence; CEF/wgpu composition остаётся следующим отдельным рубежом.

## Проверка

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps
cargo tree -p worldline-kernel
cargo run -p worldline-demo
```

Toolchain проекта закреплён на последнем доступном стабильном Rust `1.98.0`,
edition — `2024`.

## Repository-owned CI

Все обязательные suites запускаются одинаково локально и в GitHub Actions через
repository-owned entrypoint:

```powershell
pwsh -NoProfile -File scripts/ci/Invoke-WorldlineCi.ps1 -Suite All
```

Стабильные required check names для pull request и `master`:

- `Source`
- `Correctness`
- `Architecture-Security`
- `Proving-Slice`

`Architecture-Security` проверяет GRACE layout, направление зависимостей kernel,
storage boundary и persistence acceptance evidence. `Proving-Slice` сохраняет
постоянные S0/S1 gates и production-backed persistence acceptance.
Workflow-файлы `.github/workflows/pr.yml` и `.github/workflows/master.yml`
остаются тонкой GitHub Actions orchestration; verification logic принадлежит
локальному PowerShell runner.

Успешный локальный запуск — только local evidence. Hosted CI считается
подтверждённым только по именованному GitHub Actions run; branch-protection
settings этим change не изменяются. Политика описана в
[docs/CI-CD.md](docs/CI-CD.md), а GRACE workflow — в
[docs/grace/README.md](docs/grace/README.md).

## Лицензия

Worldline распространяется на условиях двойной лицензии `MIT OR Apache-2.0`.
Подробности находятся в файлах [LICENSE-MIT](LICENSE-MIT) и
[LICENSE-APACHE](LICENSE-APACHE).
