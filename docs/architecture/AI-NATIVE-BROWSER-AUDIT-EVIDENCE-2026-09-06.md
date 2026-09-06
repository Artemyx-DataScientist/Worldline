# Worldline: границы и доказательства аудита

> Repository copy of the dated evidence packet. It records the audit at `dfc644dfd6f6aa4335231b15bf0d5019a3c073fd`; the audit documents were subsequently imported into this repository without changing the audited implementation.

Дата: 2026-09-06. Операция: `AUDIT-WORLDLINE-AI-NATIVE-20260906`, `codex_led`. Operation root и владелец заключения: текущая задача Codex. Это исследование и архитектурное предложение, не исполнение изменения production. Change ID и T-* для реализации: не назначались.

Репозиторий: `Worldline`. Ветка на момент исследования: `codex/browser-search-providers-spec-20260904`. HEAD: `dfc644dfd6f6aa4335231b15bf0d5019a3c073fd`. Рабочее дерево было чистым перед исследованием и при проверке после чтения/тестов. Worktree не создавался: исходники исследуются без записи. Результаты аудита ниже теперь versioned как repository documents; активный approved change `C-BROWSER-SEARCH-PROVIDERS-20260904` не изменялся и не принимался этим аудитом.

Scope: все workspace crates в части архитектуры, representative execution paths, `.grace`, ADR, roadmap, CI scripts. Это не полный security scan каждой строки, не penetration test и не оценка upstream Chromium/Wasmtime на отсутствие уязвимостей. Один независимый fresh-context reviewer исследовал границы доверия по процедуре threat-model; существенные выводы сверены основным аудитором с кодом. GLM не участвовал: в доступной конфигурации routing не было ZCode target.

Во время исследования разрешёнными записями были только внешний Markdown-пакет и per-audit threat-model. В текущем commit пакет импортируется в `docs/architecture` и проект ADR — в `docs/adr`; production, тесты, CI, `.grace` и принятые ADR не менялись. Откат кода не требуется.

## Свежие локальные проверки

| Команда | Результат | Что именно доказано |
| --- | --- | --- |
| `cargo test --offline -p worldline-kernel --test security_acceptance --test negative_security --test upgrade_acceptance --test compatibility_acceptance` | Exit 0; 19 + 3 + 11 + 5 = **38 passed**, 0 ignored | Проверенные свойства generic broker, revocation, contract compatibility и отдельных recovery models |
| `cargo test --offline -p worldline-browser-contract -p worldline-browser-services-contract -p worldline-wasm-host` | Exit 0; 19 + 23 + 12 = **54 passed**, 0 ignored | Browser DTO/authority contracts; WASM import denial, fuel exhaustion, memory limits, instance trap containment. Сборка fixtures выдала `unused_unsafe` warnings в memory-hog и trapper |
| `grace lint --path . --assertions current` | Exit 0; 188 files, 60 XML artifacts, 0 errors, 0 warnings, **0 governed files** | Синтаксическая/структурная целостность проверенных GRACE артефактов |
| `grace status --path . --with modules --json --fail-on errors` | **Exit 1**; 10 blocked modules; 0 integrity errors/warnings | Все 10 модулей получили `health.missing-implementation-files`: tool не обнаружил linked non-test governed files. Это проблема semantic linkage, а не свидетельство отсутствия Rust-кода |
| `pwsh -NoProfile -File scripts/ci/Test-WorldlineArchitecture.ps1` | Exit 0, Architecture guard passed | Проверяемые dependency/layout/anti-corruption ограничения |
| `git status --short`; `git rev-parse HEAD` | Чисто; HEAD совпал | Стабильность проверенного checkout на момент проверки |

Итого: **92 локальных теста прошли**. Список test binaries с нулём тестов и doc-tests не включён в число 92. Во второй Cargo-команде сначала ожидалась блокировка общего artifact directory, затем сборка и тесты завершились успешно. Внешняя сеть для Cargo не разрешалась (`--offline`).

GRACE CLI: 4.0.4. Для уточнения неуспешного status его JSON повторно прочитан и сведён к summary/blockers; повторный вызов подтвердил те же 10 linkage blockers. Результат status нельзя представлять как зелёный gate.

Не запускались: полный `Suite All`, реальные headful CEF/S3B/S3C/S3D/S3E, live Chromium spike, hosted CI, upgrade chaos, malicious-WASM cross-mode suite, fuzzing/sanitizers, эксплуатационные PoC, Firefox/Gecko прототип. Код и существующие тесты этих путей использованы как статические доказательства с соответствующей оговоркой. Именованного CI run у этого аудита нет.

## Метод и критерии доказательности

- **Факт кода**: прочитана реализация/вызывающий путь, ссылки проверены по текущему checkout.
- **wired**: прослежен путь к исполнителю; это не означает свежий запуск этого пути.
- **verified**: конкретное свойство поддержано одной из свежих проверок выше.
- **scaffolded/model-tested**: тип/алгоритм/fixture существует и может быть протестирован, но продуктовая интеграция не установлена.
- **Архитектурный риск**: следствие будущего подключения недоверенного caller или роста масштаба; не объявляется текущим exploit.
- **Гипотеза угрозы**: указаны actor, prerequisites, asset и недоказанный участок.

Названия tests и комментарии о production не принимались за самостоятельное доказательство. Например, тест safe-mode, вызывающий boolean filter, не доказывает загрузку recovery shell; `S2::run` с ReferenceBrowserBackend не доказывает CEF semantic input.

## Изученные артефакты

Прочитаны AGENTS.md; относящиеся к kernel/browser/UI/recovery/AI разделы ROADMAP.md; README.md; docs/CI-CD.md; GRACE context requirements/technology/principles/deployment/ux, graph и verification indexes/main; approved spec/plan активного search change; ADR о kernel boundary, browser engine, native provider, external plugin boundary, operability/compatibility/upgrade и связанные декларации.

Из implementation изучены: kernel identity/security/invocation/runtime/plugin/upgrade/quarantine/safe_mode/bisect и их integration usages; native supervisor/containment/connection/codec; WASM adapter/limits/WASI/WIT; browser contract/backend/core/CEF bootstrap and callbacks; reference S0/S2/S3A/S3B/S3D и external echo bridge; services tabs/history/downloads/cookies/search/adblock/devtools; SQLite и blob boundary; CEF manifest и CI initialization/architecture scripts. Точные anchors существенных утверждений перечислены в ссылках отчёта и threat-model. Это перечень исследованных областей, не заявление о полном построчном coverage каждого файла.

Модули: `M-KERNEL-CAPABILITY-RUNTIME`, `M-REFERENCE-PROVING-SLICE`, `M-PERSISTENCE-RECOVERY`, `M-EXTERNAL-PLUGIN-BOUNDARY`, `M-OPERABILITY-COMPATIBILITY-UPGRADE`, `M-BROWSER-CONTRACT-ENGINE-SPIKE`, `M-BROWSER-ENGINE-PROVIDER-PROCESS`, `M-BROWSER-SERVICE-PLUGINS`, `M-CI-BASELINE`, `M-GRACE-CONTROL-LAYER`; соответствующие `V-M-*` просмотрены по verification routing. Scope delta реализации: **нет**.

Внешние сведения проверены по первичным источникам Chromium/CEF, Mozilla/MDN, W3C, Wasmtime и Microsoft. Они описывают upstream contracts; сами по себе не доказывают, что Worldline правильно включил соответствующую защиту. Экономические пороги в roadmap являются предлагаемыми критериями решения, не измерением стоимости команды.


## Выпущенный пакет и финальная сверка

Пакет аудита содержит следующие versioned документы репозитория:

- [ARCHITECTURE-AUDIT.md](<AUDIT-AI-NATIVE-BROWSER-2026-09-06.md>) — фактическая карта, 12 findings, A/B/C, target architecture, инварианты, migration и backlog, покрытие 18 вопросов.
- [THREAT-MODEL.md](<THREAT-MODEL-AI-NATIVE-BROWSER-2026-09-06.md>) — 16 effective-resource rows, actors/boundaries, 16 attacker/failure stories, severity calibration.
- [ADR-WORLDLINE-VNEXT.md](<../adr/ADR-WORLDLINE-VNEXT.md>) — proposed решение C; не принято и не помещено в действующий ADR registry.
- [EVIDENCE.md](<AI-NATIVE-BROWSER-AUDIT-EVIDENCE-2026-09-06.md>) — этот completion/evidence packet.

При финальной сверке обнаружена арифметическая ошибка первоначальной сводки: 19 + 3 + 11 + 5 = **38**, а не 39. Обе указанные Cargo-команды повторно выполнены для проверки итогов: exit 0, соответственно 38 и 54 passed. **Уникальных успешно проверенных тестов 92**; повторный прогон не прибавлен к этому числу. Новых production или test изменений не было.

Все локальные ссылки и anchors пакета проверены на существование файла/номера строки; для source anchor проверено, что строка непустая. Это проверка навигации, не дополнительный runtime test. Markdown содержит три Mermaid-схемы: фактическую архитектуру, target architecture и жизненный цикл активации.

Оставшиеся риски и незапущенные проверки перечислены выше и в threat model. Реализация vNext остаётся предложением; существующий approved search change не изменён. Принятие ADR и будущих GRACE bundles — отдельные решения владельца проекта.
