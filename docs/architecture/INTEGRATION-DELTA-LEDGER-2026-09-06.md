# Integration delta ledger: branch snapshot 2026-09-06

Статус: фактический ledger и предложение по интеграции; это не approval merge и не разрешение на реализацию vNext.

Документ фиксирует состав опубликованной ветки codex/browser-search-providers-spec-20260904 на baseline commit d1f334d, до добавления этого reconciliation ledger. Он нужен потому, что ветка стала составной integration-веткой, хотя её имя указывает только на search providers.

## Срез состояния

- Рабочее дерево на момент подготовки baseline ledger было чистым.
- Baseline d1f334d опережал origin/master на 27 коммитов.
- Сравнение origin/master...d1f334d: 62 files changed, 10 329 insertions, 305 deletions.
- Перед добавлением ledger локальная ветка и origin/codex/browser-search-providers-spec-20260904 совпадали на d1f334d.
- Ветка не переписывается: ledger не требует force-push, reset или массового rebase.

Эти 10 329 строк не являются 10 329 строками production runtime. По верхним каталогам delta состоит из:

| Область | Файлов | Добавлено | Удалено | Интерпретация |
| --- | ---: | ---: | ---: | --- |
| crates | 35 | 5 947 | 92 | Реальные Rust-контракты, runtime slices и acceptance tests |
| .grace | 13 | 2 590 | 5 | Specs, plans, design context и архив control plane |
| docs | 8 | 1 231 | 0 | ADR, audit, threat model и evidence |
| ROADMAP.md | 1 | 395 | 207 | Product/architecture direction |
| scripts и .github | 3 | 139 | 1 | Architecture/CI gates |
| Cargo manifests | 2 | 27 | 0 | Workspace/package wiring |

## Пакеты delta

### P0 — kernel capability provider targeting

Коммиты: f26c68d, 92588c9, 488025a, 849be8b, 22dd32c, ec72f76, e0327b1, 1e1458d.

Scope: CapabilityTarget, targeted broker invocation, provider handles, installation targeting, acceptance tests и завершение GRACE bundle C-KERNEL-CAPABILITY-PROVIDER-TARGETING-20260905.

Архитектурный смысл: усиливает generic kernel authority model и является потенциальной предпосылкой для адресации capability к конкретной installation/provider.

Gate перед интеграцией: проверить, что targeting не добавляет browser/domain policy в kernel, сохраняет default-deny, revocation и отсутствие confused-deputy fallback. Нужны свежие workspace tests, clippy, architecture guard и GRACE status без новых linkage blockers.

### P1 — browser devtools diagnostics

Коммиты: 3a60d07, fec318b, 0f89dc2.

Scope: devtools contract, diagnostics provider/service wiring и отдельный browser-devtools crate.

Архитектурный смысл: proving slice для browser service contract. Название engine-neutral пока не является доказательством одинакового runtime поведения Chromium и Firefox.

Gate перед интеграцией: зафиксировать operation profile каждого backend, отделить contract/model-tested от real-engine evidence и проверить, что diagnostics не получает debugger/native authority через обход общего browser gateway.

### P2 — browser search providers и CEF proving

Коммиты: 108d293, 940bbaa, 01d07d7, f73e47b, 46312df, e4203d9, e623c9c, 0cfc34c, bf7e92e, 5326635, 76fe8d7, 32807df, dff9ccd, 9dee92b, dfc644d.

Scope: search contract/service/plugin, provider installation wiring, reference S3D/S3E slices, CEF backend changes, architecture guards и Windows CI job. e4203d9 — adjacent request-policy archive, попавший в ту же линейную историю.

Архитектурный смысл: самый близкий к product browser vertical, но он одновременно содержит ReferenceBrowserBackend и real-CEF proving path. S2r не закрывает S2c.

Gate перед интеграцией: отдельно предъявить evidence для Reference и CEF; закрыть реальные операции query/act/capture/permission либо явно оставить их unsupported; проверить authenticated gateway, resource-target validation, failure/degradation semantics и отсутствие service-to-storage обходов.

### P3 — AI-native audit и roadmap correction

Коммит: d1f334d.

Scope: фактический audit, threat model, evidence packet, proposed ADR и обновление roadmap.

Архитектурный смысл: это decision input, а не implementation. ADR остаётся Proposed; он не заменяет approval отдельного GRACE change.

Gate: ссылки и evidence должны быть доступны в репозитории; runtime claims должны оставаться маркированными contract/model-tested, wired или verified. См. [audit](AUDIT-AI-NATIVE-BROWSER-2026-09-06.md), [threat model](THREAT-MODEL-AI-NATIVE-BROWSER-2026-09-06.md), [evidence](AI-NATIVE-BROWSER-AUDIT-EVIDENCE-2026-09-06.md) и [proposed ADR](../adr/ADR-WORLDLINE-VNEXT.md).

## Рекомендуемый порядок интеграции

1. P0 — generic kernel targeting.
2. P1 — devtools service slice; допускается отдельная проверка от P0, но интеграция должна идти после проверки kernel boundary.
3. P2 — search/CEF vertical с отдельными labels для reference и real CEF.
4. P3 — audit/roadmap/ADR как decision layer после сверки runtime evidence.

Порядок является review/integration order, а не утверждением, что каждый commit должен быть механически cherry-picked без clean build. Перед merge каждого пакета нужна проверка на чистом основании и фиксация зависимостей.

## Общий integration gate

Пакет не считается готовым к merge, пока не предъявлены:

- scope и список затронутых модулей;
- TCB delta и capability delta;
- engine profile, если затронут browser path;
- named tests на contract, real consumer и failure scenario;
- rollback boundary и совместимость с текущим persisted state;
- cargo fmt, workspace tests, clippy и architecture guard;
- grace lint и сверка grace status;
- отдельная отметка scaffolded, wired и verified;
- отсутствие прямого вызова engine, filesystem, credentials или storage из untrusted feature path.

Большой diff сам по себе не является дефектом. Он становится stop signal, если пакет одновременно меняет kernel authority, engine boundary, capability vocabulary и recovery semantics.

## Admission contract для agent package

Каждая следующая agent-задача получает bounded package с:

- одним владельцем технической проверки;
- одной основной boundary или capability family;
- точным allowed/forbidden write scope;
- обязательными assertions и verification commands;
- запретом изменять protected tests и approved GRACE plan;
- независимой проверкой generated diff;
- явным stop condition при изменении TCB или публичного API.

Line budget не вводится: агенты могут писать быстро. Ограничивается authority surface и количество непросмотренных границ.

## Что это меняет в roadmap

До дальнейшего расширения M1 нужен отдельный M0.8 Integration delta reconciliation gate. Его exit criterion:

- текущая composite delta разложена на P0–P3;
- каждый пакет имеет clean-build evidence и rollback boundary;
- engine-neutral claims сокращены до реально общего уровня;
- active GRACE change и archived changes не смешаны с утверждением production readiness;
- integration branch не содержит неизвестного TCB/capability drift.

После прохождения M0.8 можно продолжать M1.2/M1.3 bounded work. До него не следует открывать marketplace, arbitrary generated packages или второй browser engine.

## DO NOT

- не удалять 10k строк по line count;
- не сливать 27 коммитов одним неразличимым merge;
- не переписывать уже опубликованную ветку без отдельного решения;
- не принимать proposed ADR как действующий architecture contract;
- не считать ReferenceBrowserBackend доказательством CEF/Firefox parity;
- не расширять common browser API, чтобы скрыть unsupported engine operations;
- не запускать user/AI code внутри kernel или engine privileged process.

Следующий рабочий decision gate — M0.8 и clean integration ledger. Сам ledger является docs-only артефактом и не меняет runtime.
