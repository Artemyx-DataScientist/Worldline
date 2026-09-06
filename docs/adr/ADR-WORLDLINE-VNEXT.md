# ADR-WORLDLINE-VNEXT: Portable product model, engine-specific runtime

Статус: **PROPOSED / не принят**. Дата: 2026-09-06.
Операция: `AUDIT-WORLDLINE-AI-NATIVE-20260906`.
Baseline: `dfc644dfd6f6aa4335231b15bf0d5019a3c073fd`.
Документ versioned в `docs/adr`, но имеет статус Proposed и не изменяет действующие инварианты или approved GRACE plan.

## Контекст и проблема

Worldline хочет позволить пользователю и AI создавать существенные browser features, сохраняя безопасность, обновляемость и аварийное восстановление. Сейчас generic Rust kernel имеет настоящие foundations authority/lifecycle/state, но один untrusted feature ещё не проходит весь путь до browser operation, controlled replacement и recovery.

Основные основания: CEF semantic query/action пока Unsupported; browser service libraries не замкнуты на production capability gateway; WASM world не импортирует capability invocation; upgrade/quarantine модели не образуют durable package loader; real UI composition отсутствует. Ссылки и степень подтверждения: [аудит F01–F07](<../architecture/AUDIT-AI-NATIVE-BROWSER-2026-09-06.md>).

Решение нельзя строить на двух допущениях: «всё, что называется plugin, одинаково недоверенно» и «общий Rust trait делает два engine взаимозаменяемыми». Движок и privileged brokers остаются частью TCB независимо от каталога/crate, в котором расположены.

## Решение

**Выбрать C: Portable product model, engine-specific runtime. Первую desktop runtime assembly строить на Chromium/CEF. Равноправный Firefox backend сейчас не обещать.**

1. Общим сделать product/feature/UI/capability model, portable domain data, typed outcomes и lifecycle. Внешний feature ABI не содержит CEF/CDP pointers, raw profile paths или engine database schemas.
2. Engine adapter реализует negotiated operation profiles и явные engine-specific extensions. Неподдерживаемая guarantee возвращает Unsupported; security-sensitive downgrade запрещён.
3. Small trusted control kernel владеет generic identity, handle/grant authority, invocation admission, epochs и installation-state binding. Browser domain policy живёт в обязательных trusted brokers вне kernel.
4. Mandatory boot composition включает проверенные browser/security brokers, trusted consent/origin/stop/recovery surface и updater/recovery launcher. Ordinary feature не может заменить их или изменить trust policy.
5. Native/system providers имеют отдельный trust admission. Обычный формат AI/user executable feature — WASM Component в OS-restricted feature host, без ambient WASI/network/filesystem/process execution.
6. Все browser/OS эффекты проходят authenticated broker и проверку фактического target. Grants учитывают origin/frame/document, короткий lifetime и activation epoch там, где это нужно.
7. AI planner и builder не получают административных прав. Generated package неизменяем; публикация, активация и изменение rights — разные операции.
8. Feature updates переключают durable code/state/policy generation. Recovery не зависит от исправности пользовательской composition. Engine security update может отключить несовместимую optional feature, сохранив её данные.
9. UI composition расширяема через bounded slots/commands/view schemas. Trusted security surface нельзя перекрыть или подменить.
10. Каждый promoted claim требует свежего evidence на реальном consumer. Reference models сохраняются для contract testing, но не заменяют engine/device acceptance.

## Предлагаемая корректировка трактовки действующего invariant

Действующий текст: «Nothing above the kernel is special; product capabilities come from plugins».

Сохранить его цель: продуктовая роль, бренд provider или имя capability не дают скрытой authority; product features не должны врастать в generic kernel.

Для vNext уточнить: **«Все продуктовые функции используют явные capability contracts. Trust class определяется проверенной boot/installation policy; security-critical system components могут быть обязательными. Их замена не относится к ordinary feature authority».**

Это предложение для отдельного approved architecture change. Оно не принято автоматически этим документом. Размещение security broker в plugin crate не делает его безопасным для произвольной пользовательской подмены.

## Альтернативы

| Вариант | Выгода | Цена/риск | Решение |
| --- | --- | --- | --- |
| A. Глубокая Chromium platform | Самый короткий путь к качественной desktop интеграции | Ecosystem lock-in, если internals попадут в внешний ABI | Допустимый fallback, если даже узкая переносимость мешает доказанным сценариям |
| B. Chromium и Firefox с общим полным API | Единый marketing promise и теоретический выбор engine | Два lifecycle/update/rendering/security пути и дорогое выравнивание различий | Отклонить сейчас; нет второго runtime и подтверждённого спроса |
| C. Общая product model, разные runtime | Сохраняет переносимость сценариев/данных и допускает глубокую интеграцию | Требует дисциплины operation profiles; feature может быть engine-specific | Принять как target, CEF-first implementation |
| Native user plugins с подписью/consent | Быстро открыть все возможности | Подпись идентифицирует publisher, но не ограничивает Win32/syscall authority | Не допускать как обычный generated feature format |
| Только декларативные recipes | Меньше execution surface | Ограниченная композиция алгоритмов и преобразований | Использовать там, где хватает; дополнить WASM |
| Полный browser fork со свободным AI patching | Максимальная формальная свобода | Нет стабильного TCB, обновления и recovery становятся задачей каждого fork | Отдельный developer workflow без обещания прежней trust guarantee |

CEF имеет desktop embedding инфраструктуру. Официальный GeckoView ориентирован на Android; из этого не следует техническая невозможность desktop Gecko, но сопоставимый desktop путь требует отдельного подтверждения. [CEF](https://chromiumembedded.github.io/cef/general_usage.html), [GeckoView](https://mozilla.github.io/geckoview/)

WebDriver BiDi — кандидат для части automation vocabulary/test harness, а не replacement embedding/security/UI boundary. [W3C BiDi](https://w3c.github.io/webdriver-bidi/)

## Последствия

Положительные: возможность обновлять engine без обещания ABI его внутренностей авторам features; bounded blast radius пользовательского кода; явные privileges; восстановление без запуска сломанной feature; переносимые данные и сценарии без обязательного parity.

Цена: дополнительные broker/gateway реализации, строгая модель principals/resources, sandbox launcher и trusted shell, durable package catalog, operation compatibility matrix. Маленький kernel не устраняет необходимость аудировать эту TCB. WASM и OS sandbox требуют отдельной платформенной проверки.

Ограничения: нельзя дать произвольному коду полный доступ к секретам и одновременно гарантировать, что он их не раскроет. Нельзя автоматически откатить уже отправленный запрос или внешнюю транзакцию. Нельзя гарантировать вечную работоспособность automation на изменившемся сайте.

Не измерено: размер команды, бюджет release engineering, business demand второго движка, memory/latency per-feature process, подходящий desktop Gecko embedding. Эти неизвестные не скрываются фиктивной оценкой человеко-месяцев.

## Acceptance и порядок реализации

1. **G0 — фактическая карта.** Operation support, TCB/resources и readiness claims соответствуют real CEF/source; GRACE linkage blockers разобраны.
2. **G1–G2 — проверка гипотезы.** Одна полезная generated feature через WASM + OS isolation + scoped browser broker; права не расширяются через DOM/model output; crash/loop не ломает browsing.
3. **G3 — проверка обновляемости.** Новая revision активируется вместе со state generation; kill/restart в каждом переходе возвращает единственное допустимое состояние; old runtime authority не воскресает.
4. **G4–G5 — продуктовая эксплуатация.** Trusted UI/a11y/IME, stream downloads, independent updater, recovery boot, candidate engine migration и bounded observability.
5. **G6 — отдельное решение о Gecko.** Только при подтверждённой product value и владельце сопровождения. Предлагаемый spike budget 10–15 инженерных дней — ограничение инвестиции, не оценка полной разработки.

Spike должен подтвердить на выбранной ОС embedding, input/IME/accessibility, profile isolation, crash containment и security update rehearsal. Stop criteria: требуется глубокий долговременный fork без владельца; нужно отключить sandbox; core API получает unsafe escape hatch; update train невозможно поддерживать.

## Когда пересмотреть решение

- Общий product layer систематически не выражает востребованные сценарии без потери качества — сузить portable promises или выбрать A для affected surface.
- Второй engine имеет оплачиваемый спрос, поддерживаемый embedding путь и владельца release/security — рассмотреть C с дополнительной runtime assembly; B всё ещё требует отдельного обоснования parity.
- Один untrusted feature нельзя реализовать без raw debugger/native полномочий — сузить product promise и пересмотреть capability vocabulary до расширения платформы.
- Изоляция/восстановление не выдерживают fault injection — не открывать marketplace и arbitrary generated packages.

Следующее действие — архитектурное принятие/корректировка этого предложения владельцем продукта и отдельный GRACE bundle. Массовый рефакторинг данным ADR не разрешён и не выполнялся.
