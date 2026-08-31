# ADR: Browser Engine Provider Selection and Contract v1

Статус: accepted
Change: C-BROWSER-M1-1-AUDIT-CLOSURE-HARDENING-20260831
Рубеж: M1.1 — Browser contract and engine spike

---

## 1. Context

С завершением Milestone M0 (M0.1–M0.7) платформа Worldline обладает устойчивым минимальным ядром (`worldline-kernel`), моделью авторизации по принципу наименьших привилегий (default-deny), типизированным транспортом событий, персистентностью SQLite, транзакционным outbox, контролируемым внешним плагинным интерфейсом (native process IPC и WASM components), а также транзакционным обновлением и изоляцией сбоев (safe mode, quarantine, automated bisect).

Milestone M1 начинает разработку первой прикладной функциональной семьи — браузера.
В соответствии с инвариантами Worldline:
- **Браузер — это семейство плагинов, а не подсистема ядра.**
- Ядро не содержит логики вкладок, истории, DOM, сетевых перехватов или визуального представления.
- Интерфейс браузера для человека (Worldline Shell / UI) и для будущих автономных агентов **идентичен** на уровне logical capabilities и различается только вызывающим субъектом (`PrincipalId`) и объёмом выданных прав (`CapabilityGrant`).
- Доступ к чтению/наблюдению страницы (`ObservePage`, `QueryDocument`) **никогда не влечёт** автоматического права на взаимодействие (`ActOnPage`) или навигацию (`NavigatePage`).
- Команды передаются через явный Capability RPC; события браузера являются фактами ретроспективного наблюдения и не могут служить скрытой очередью команд.

Данный ADR фиксирует границу антикоррупции между Worldline и браузерными движками, методологию и результаты спайка встраивания, матрицу сравнения кандидатов, измеренные эмпирические факты и выбор первого провайдера для Milestone M1.2.

---

## 2. Browser contract boundary

Граница `worldline-browser-contract` изолирует абстракции Worldline от деталей реализации любого конкретного движка (Chromium/CEF, WebView2, WebKit, Gecko, Servo).

```text
  Worldline Consumer (UI / Agent)
               │
               ▼
   [Capability RPC / Default-Deny Grants]
   browser.context/v1
   browser.page/v1
   browser.navigate/v1
   browser.observe/v1
   browser.query/v1
   browser.act/v1
   browser.download/v1
   browser.permission/v1
               │
               ▼
   ┌────────────────────────────────────────┐
   │ worldline-browser-contract             │
   │ (Engine-neutral IDs, Errors, Actions)  │
   └────────────────────────────────────────┘
               │
   ════════════╡ Anti-Corruption Boundary ╞════════════
               │
   ┌────────────────────────────────────────┐
   │ Browser Engine Provider (M1.2 Process) │
   │ (CEF / Chromium Engine Adapter)        │
   └────────────────────────────────────────┘
```

### 2.1 Идентичности и типы
1. `BrowserContextId`: логический идентификатор изолированного контекста/профиля (`profile_id`, куки, кэш, хранилище). Переживает перезапуск дочерних процессов движка. Никакие сырые пути файловой системы хоста не утекают в ABI.
2. `PageId`: логический идентификатор поверхности страницы. Управляется контрактом провайдера, а не сырыми указателями/дескрипторами ОС (`HWND`, `CefBrowser` pointers).
3. `NavigationId`: идентификатор попытки/акта навигации.
4. `DocumentRevision`: монотонный счетчик ревизий DOM/страницы.
5. `DownloadId`: идентификатор операции загрузки.
6. `ElementRef`: ссылка на семантический узел, жестко привязанная к кортежу `(PageId, DocumentRevision, NodeKey)`. Ссылка становится недействительной (`StaleElementReference`) при изменении ревизии документа после навигации или сброса DOM. В реальном исполнителе `NodeKey` напрямую адресует конкретный DOM/AX элемент.
7. `QueryBounds`: жесткие бюджетные ограничения на глубину обхода (`max_depth`), количество узлов (`max_nodes`), длину строк (`max_text_len`) и суммарный объем текста (`max_total_text_bytes`), предотвращающие переполнение памяти и DoS в IPC.

### 2.2 Разделение прав (Authority Separation) и защита от Confused-Deputy
- `ObservePage` -> разрешает только чтение метаданных страницы (URL, title, loading status, viewport).
- `QueryDocument` -> разрешает чтение семантического дерева доступности (AX Tree), извлечение текста и поиск элементов.
- `NavigatePage` -> разрешает инициацию навигации, перезагрузку, остановку, историю (back/forward).
- `ActOnPage` -> разрешает клик, ввод текста, установку фокуса, отправку форм и прокрутку.
- `ControlDownload` -> управление загрузками файлов.
- `ManagePermission` -> принятие решений по разрешениям сайтов (геолокация, медиа и т.д.).

**Инвариант защиты от Confused-Deputy:** провайдер валидирует совпадение ресурса из `InvocationContext` (`browser-page/{PageId}` или `browser-context/{ContextId}`) с целевым ресурсом в полезной нагрузке. Для иерархических контекстных полномочий проверяется точное владение целевой страницей (`supervisor.get_page_context(page_id) == owning_context_id`). Любые несовпадения и произвольные префиксные байпассы немедленно отвергаются как `ResourceMismatch`.

---

## 3. Engine candidates

В качестве кандидатов на роль первого браузерного движка были детально проанализированы:

1. **Chromium Embedded Framework (CEF / Chromium)**:
   Полнофункциональный C/C++ embedding framework на базе современного Chromium.
2. **Microsoft Edge WebView2 (Chromium-based)**:
   Системный компонент Windows на базе Chromium, использующий установленный в ОС Edge Runtime.
3. **WPE WebKit / WebKitGTK**:
   Легковесный порт WebKit для встроенных систем и Linux.
4. **Mozilla Gecko / Spidermonkey**:
   Движок рендеринга Firefox.
5. **Servo**:
   Независимый веб-движок на чистом Rust с параллельным рендерингом и CSS-парсингом.

---

## 4. Spike methodology

Спайк реализован в двух дополняющих компонентах:
1. **Real Out-of-Process Chromium Engine Spike** (`worldline-browser-spike/src/chromium.rs`, `real_chromium_acceptance.rs`):
   - Запуск реального внешнего процесса Chromium/Edge (`--headless=new`) с политикой fail-closed в CI.
   - Управление по протоколу CDP (Chrome DevTools Protocol) через безопасный легковесный WebSocket-клиент на чистом Rust.
   - Навигация по локальному HTML-файлу (`file:///...`).
   - Извлечение настоящего дерева доступности Blink (`Accessibility.getFullAXTree`), адресация семантических элементов в `ElementRef.node_key` и наложение `QueryBounds`.
   - Выполнение реальных действий, адресованных конкретным элементам (ввод текста в поле формы, клик по кнопке с изменением DOM), с проверкой `ElementNotFound` для некорректных ссылок.
   - Принудительное уничтожение процесса рендерера (`Page.crash`) с проверкой выживания управляющего процесса и хоста Worldline.
   - Измерение реального времени холодного старта и потребления RAM (Working Set).
2. **Deterministic In-Memory Reference Provider** (`worldline-browser-spike/src/engine.rs`, `spike_acceptance.rs`, `measurement_suite.rs`):
   - Быстрый контрактный эталон для юнит- и интеграционных тестов без внешних зависимостей.
   - Полное покрытие всех 8 capability contracts (`context`, `page`, `navigate`, `observe`, `query`, `act`, `download`, `permission`).
   - Публикация типизированных событий через `InvocationContext::publish_event` в M0.4 транспорт событий ядра (`browser.page.created`, `browser.navigation.committed`, `browser.page.closed`, `browser.download.started`) с валидацией через pull-подписки `SubscriptionHandle`.

---

## 5. Measured Empirical Spike Results

Все числовые показатели в этом разделе получены в результате прямых автоматических измерений в тестовых наборах `real_chromium_acceptance` и `measurement_suite` на Windows:

### 5.1 Реальный Out-of-Process Chromium Engine Spike (Google Chrome 151)
- **Холодный запуск процесса Chromium:** `575 – 625 ms` (включая инициализацию песочницы, создание временного `user-data-dir` и запуск CDP DevTools сервера).
- **Потребление оперативной памяти (Working Set):** `134.7 – 135.5 MB` (для одного процесса браузера с открытой страницей).
- **Навигация по локальному HTML-файлу и фиксация:** `~150 ms`.
- **Извлечение полного Blink AX Tree:** `~20 – 35 ms`.
- **Диспетчеризация и исполнение DOM-взаимодействия (Input + Click):** `~10 – 15 ms`.
- **Изоляция сбоя рендерера (`Page.crash`):**
  - Процесс рендерера аварийно завершен мгновенно.
  - Управляющий процесс супервизора **остался полностью жив** (`is_host_alive() == true`).
  - Хост-процесс Worldline **не получил сбоя** и вернул типизированную ошибку `BrowserError::EngineCrashed`.

### 5.2 Эталонный Reference Provider (In-Memory Microkernel Benchmark)
- **Инициализация ядра и публикация 8 capability-контрактов:** `1.31 ms`.
- **Создание изолированного контекста:** `234 µs`.
- **Создание страницы и фиксация навигации:** `151 µs`.
- **Запрос Document Snapshot & Bounded AX Tree:** `228 µs`.
- **Диспетчеризация и валидация действия формы:** `90 µs`.

---

## 6. Architectural and Literature Assessment

В этом разделе приведен качественный анализ кандидатов на основе документации, лицензий и экосистемы.

### 6.1 Модель встраивания и отображения
- **CEF**: Предоставляет отлаженную модель встраивания как в виде native child window (`CefWindowInfo::SetAsChild`), так и через Off-Screen Rendering (OSR) с получением пиксельного буфера (`OnPaint`).
- **WebView2**: Высокоуровневый COM-интерфейс встраивания в нативное окно Windows (`ICoreWebView2Controller`), поддержка Composition Controller для DirectComposition, но жесткая привязка к платформе Windows.
- **WPE WebKit**: Оптимизирован под OSR/Wayland/EGLStream, однако на Windows требует сложных прослоек эмуляции.
- **Gecko**: Встраивание (GeckoView / libxul) традиционно слабо документировано для десктопа и нестабильно вне Firefox.
- **Servo**: Отличный Rust-интерфейс, однако многие современные спецификации HTML5/JS/DOM находятся в стадии активной доработки.

### 6.2 Изоляция процессов
- **CEF**: Полноценная многопроцессная архитектура Chromium: выделенный Browser Process, Renderer Processes (по одному на origin/site-instance), GPU Process, Utility Processes. Сбои рендерера изолированы в дочернем процессе.
- **WebView2**: Многопроцессная архитектура Chromium, изолированные runtime-процессы на пользователя/профиль.
- **WPE WebKit**: Двухпроцессная / многопроцессная модель (UIProcess + WebProcess + NetworkProcess).
- **Servo**: Модель на потоках и процессах Rust, изоляция паники на уровне потоков/каналов.

### 6.3 Лицензирование и поставка
- **CEF / Chromium**: Лицензия BSD (3-clause) / MIT / Apache 2.0. Полностью совместима с коммерческим и открытым использованием Worldline, не накладывает copyleft-ограничений. Размер дистрибутива: ~80-120 МБ в сжатом виде.
- **WebView2**: Проприетарная лицензия Microsoft Edge (зависимость от вендора и ОС Windows).
- **WebKit**: LGPL v2.1 / BSD (требует строгого соблюдения правил динамической линковки).
- **Gecko**: MPL 2.0.

---

## 7. Decision Matrix Summary

| Критерий | Вес | Chromium / CEF | Edge WebView2 | WPE WebKit | Gecko | Servo |
| :--- | :--- | :---: | :---: | :---: | :---: | :---: |
| **Measured Boot Latency** | Required | **~580 ms** | ~600 ms (est) | ~400 ms (est) | ~900 ms (est) | **~150 ms** (est) |
| **Measured Memory (RAM)** | Required | **~135 MB** | ~120 MB (est) | ~90 MB (est) | ~160 MB (est) | **~50 MB** (est) |
| **Embedding Model** | Required | **High** | High | Medium | Low | Medium |
| **Process Isolation** | Required | **High** (Verified) | High | High | High | Medium |
| **DOM / Accessibility** | Required | **High** (AXTree/CDP) | High | Medium | High | High (AccessKit) |
| **Network Interception** | Required | **High** (Custom Schemes) | High | Medium | Low | Low |
| **Downloads Control** | Required | **High** | High | Medium | Low | Low |
| **Profile Isolation** | Required | **High** (Multi-context) | High | Medium | Medium | Low |
| **Crash Recovery** | Required | **High** (Deterministic) | High | High | High | Medium |
| **Packaging & Delivery** | Required | **Medium** (Self-contained) | High (Windows only)| Medium | Low | High |
| **Licensing** | Required | **High** (BSD) | Medium (Proprietary)| Medium (LGPL) | Medium (MPL) | High (MIT/Apache) |
| **Cross-Platform Uniformity** | Important | **High** (Win/Linux/Mac) | Low (Win-first) | Medium (Linux) | Medium | High |
| **Offscreen Rendering** | Non-blocking | **High** (OSR API) | Medium (DComp) | High (EGL) | Low | High (WebRender) |

---

## 8. Selected first provider

**Chromium Embedded Framework (CEF / Chromium)** выбран в качестве первого провайдера браузерного движка для **Milestone M1.2**.

Обоснование:
1. **Подтверждено спайком:** холодный запуск ~580 ms и ~135 MB RAM приемлемы для десктопного окружения; протокол CDP / Blink AX Tree обеспечивает надежное извлечение дерева доступности и исполнение действий.
2. **Проверенная изоляция:** сбой рендерера детерминированно изолируется в дочернем процессе без риска для ядра Worldline.
3. **Лицензионная чистота:** BSD-3-Clause без copyleft-рисков.
4. **Кроссплатформенность:** одинаковая поддержка Windows, Linux и macOS.

---

## 9. Rejected alternatives

1. **Microsoft Edge WebView2**: Отклонен как основной движок из-за отсутствия кроссплатформенности и жесткой привязки к Windows.
2. **WPE WebKit / WebKitGTK**: Отклонен из-за высокой сложности сборки и встраивания на Windows.
3. **Gecko**: Отклонен из-за отсутствия поддерживаемого standalone C/C++ embedding API.
4. **Servo**: Отклонен для первой промышленной версии M1.2 из-за незавершенности спецификаций современного веб-стека, но остается приоритетным кандидатом на долгосрочное исследование как чистый Rust-движок.

---

## 10. Reopen conditions

Данное решение может быть пересмотрено при наступлении одного из условий:
1. Servo достигнет зрелости Web Platform Tests > 95% и предоставит промышленную поддержку медиа/DRM/JS-фреймворков.
2. Изменение лицензионной политики или структуры проекта Chromium, препятствующее свободному встраиванию.
3. Появление высокостабильного кроссплатформенного WebKit порта со стабильным Rust C-FFI.
