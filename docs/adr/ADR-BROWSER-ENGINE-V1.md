# ADR: Browser Engine Provider Selection and Contract v1

Статус: accepted
Change: C-BROWSER-CONTRACT-ENGINE-SPIKE-20260831
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

Данный ADR фиксирует границу антикоррупции между Worldline и браузерными движками, методологию и результаты спайка встраивания, матрицу сравнения кандидатов и выбор первого провайдера для Milestone M1.2.

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
1. `BrowserContextId`: логический идентификатор изолированного контекста/профиля (куки, кэш, хранилище). Переживает перезапуск дочерних процессов движка.
2. `PageId`: логический идентификатор поверхности страницы. Управляется контрактом провайдера, а не сырыми указателями/дескрипторами ОС (`HWND`, `CefBrowser` pointers).
3. `NavigationId`: идентификатор попытки/акта навигации.
4. `DocumentRevision`: монотонный счетчик ревизий DOM/страницы.
5. `DownloadId`: идентификатор операции загрузки.
6. `ElementRef`: ссылка на семантический узел, жестко привязанная к кортежу `(PageId, DocumentRevision, NodeKey)`. Ссылка становится недействительной (`StaleElementReference`) при изменении ревизии документа после навигации или сброса DOM.

### 2.2 Разделение прав (Authority Separation)
- `ObservePage` -> разрешает только чтение метаданных страницы (URL, title, loading status, viewport).
- `QueryDocument` -> разрешает чтение семантического дерева доступности (AX Tree), извлечение текста и поиск элементов.
- `NavigatePage` -> разрешает инициацию навигации, перезагрузку, остановку, историю (back/forward).
- `ActOnPage` -> разрешает клик, ввод текста, установку фокуса, отправку форм и прокрутку.
- `ControlDownload` -> управление загрузками файлов.
- `ManagePermission` -> принятие решений по разрешениям сайтов (геолокация, медиа и т.д.).

**Инвариант:** наличие `QueryDocument` не удовлетворяет требование `ActOnPage`. Наличие `ObservePage` не удовлетворяет `NavigatePage`. Права привязаны к конкретному `ResourceScope` (`browser-page/{PageId}` или `browser-context/{ContextId}`).

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

Спайк был реализован в крейте `worldline-browser-spike` с исполняемым тестовым стендом, реализующим полный сквозной путь:
`Host Harness -> Capability Consumer -> Broker -> Provider Process Boundary -> Isolated Contexts -> Local Deterministic Navigation -> Committed Observation -> AX/DOM Query -> Authorized Form Action -> State Re-Query -> Child Crash Containment -> Host Survival Verification`.

Стенд использует локальные детерминированные страницы (in-memory/loopback HTML fixtures) и не требует подключения к публичной сети Интернет.

---

## 5. Embedding model

- **CEF**: Предоставляет отлаженную модель встраивания как в виде native child window (`CefWindowInfo::SetAsChild`), так и через Off-Screen Rendering (OSR) с получением пиксельного буфера (`OnPaint`).
- **WebView2**: Высокоуровневый COM-интерфейс встраивания в нативное окно Windows (`ICoreWebView2Controller`), поддержка Composition Controller для DirectComposition, но жесткая привязка к платформе Windows.
- **WPE WebKit**: Оптимизирован под OSR/Wayland/EGLStream, однако на Windows требует сложных прослоек эмуляции.
- **Gecko**: Встраивание (GeckoView / libxul) традиционно слабо документировано для десктопа и нестабильно вне Firefox.
- **Servo**: Отличный Rust-интерфейс, однако многие современные спецификации HTML5/JS/DOM находятся в стадии активной доработки.

---

## 6. Process isolation

- **CEF**: Полноценная многопроцессная архитектура Chromium: выделенный Browser Process, Renderer Processes (по одному на origin/site-instance), GPU Process, Utility Processes. Сбои рендерера изолированы в дочернем процессе.
- **WebView2**: Многопроцессная архитектура Chromium, изолированные runtime-процессы на пользователя/профиль.
- **WPE WebKit**: Двухпроцессная / многопроцессная модель (UIProcess + WebProcess + NetworkProcess).
- **Servo**: Модель на потоках и процессах Rust, изоляция паники на уровне потоков/каналов.

---

## 7. DOM and accessibility access

- **CEF**: Доступ через Chromium Accessibility API (`CefAccessibilityHandler`, AXTree), чтение DOM через Frame API / V8 context injection / CDP (Chrome DevTools Protocol). Позволяет строить легковесные структурированные снимки семантического дерева без передачи "живых" мутируемых DOM-объектов в Worldline.
- **WebView2**: Доступ через CDP (`ICoreWebView2DevToolsProtocolHelper`) и `ExecuteScript`.
- **WPE WebKit**: ATK / ATSPI на Linux, ограниченный доступ к AX на Windows.
- **Servo**: Прямой доступ к DOM структурам Rust, интеграция с `accesskit`.

---

## 8. Network interception

- **CEF**: Мощные перехватчики запросов: `CefRequestHandler`, `CefResourceRequestHandler`, `CefSchemeHandlerFactory`. Позволяют перехватывать запросы до выхода в сеть, фильтровать заголовки, подменять ответы и реализовывать защищенные приватные схемы (`worldline-internal://`).
- **WebView2**: `WebResourceRequested` фильтры.
- **WPE WebKit**: WebKitURIRequest / NetworkProcess custom schemes.

---

## 9. Downloads

- **CEF**: `CefDownloadHandler` предоставляет полный контроль: `OnBeforeDownload` (выбор пути, подтверждение пользователем), `OnDownloadUpdated` (прогресс, скорость, статус, отмена/пауза).
- **WebView2**: `DownloadStarting` и `DownloadOperation`.
- **WPE WebKit**: `WebKitDownload` lifecycle.

---

## 10. Profiles and contexts

- **CEF**: `CefRequestContext` позволяет создавать абсолютно изолированные контексты с собственными каталогами для кэша, cookies, IndexedDB, localStorage и сетевых сессий. Поддерживает как дисковые, так и полностью in-memory (incognito) профили.
- **WebView2**: `CoreWebView2Profile` с настраиваемым `UserDataFolder`.
- **WPE WebKit**: `WebKitWebsiteDataManager`.

---

## 11. Crash recovery

- При аварийном завершении (`crash` / `kill -9`) процесса рендеринга или сетевого процесса CEF:
  - Хост Worldline **остается полностью работоспособен**.
  - Провайдер ловит `OnRenderProcessTerminated` и транслирует типизированное событие `browser.engine.crashed`.
  - Все последующие операции над страницами упавшего процесса возвращают явную ошибку `BrowserError::EngineCrashed`.
  - Перезапуск движка создает **новый `RuntimeId`** и требует явной новой выдачи прав (никакого неявного наследования привилегий).

---

## 12. Packaging and distribution

- **CEF**: Бинарный дистрибутив включает `libcef.dll` (или `.so`/`.dylib`), вспомогательные ресурсы (`icudtl.dat`, `.pak` файлы) и субпроцесс (`cefclient` / `cef_helper`). Размер архива: ~80-120 МБ в сжатом виде, ~250 МБ на диске.
- **WebView2**: 0 МБ (используется Edge Evergreen Runtime, предустановленный на 99% машин Windows 10/11).
- **WPE WebKit**: ~40-60 МБ, но требует значительного числа системных библиотек.

---

## 13. Licensing

- **CEF / Chromium**: Лицензия BSD (3-clause) / MIT / Apache 2.0. Полностью совместима с коммерческим и открытым использованием Worldline, не накладывает copyleft-ограничений (в отличие от LGPL/GPL).
- **WebView2**: Проприетарная лицензия Microsoft Edge (бесплатное распространение на Windows, но зависимость от вендора).
- **WebKit**: LGPL v2.1 / BSD (требует строгого соблюдения правил динамической линковки).
- **Gecko**: MPL 2.0.

---

## 14. Upgrade cost

- **CEF**: Синхронизируется с релизами Chromium каждые 4 недели (Major versions) плюс LTS-ветки (каждые 8 недель). Наличие стабильного C API (`capi`) минимизирует поломку бинарного интерфейса при минорных обновлениях.
- **WebView2**: Обновляется автоматически службой Windows Update (zero-effort для хоста, но потенциальный риск необратимых изменений поведения).

---

## 15. OSR/windowed tradeoff

- **Решение**: Для первой функциональной реализации в Milestone M1.2 выбирается **Native Windowed (Headful)** интеграция:
  - Максимальная производительность и аппаратное ускорение видео/WebGL "из коробки".
  - Минимальная задержка ввода (input latency).
  - Идеальная совместимость со сложными веб-приложениями.
- **OSR + wgpu**: Выносится в отдельный измеряемый эксперимент:
  - Будет исследован в рамках M1.4 при реализации композиции поверхностей и оптического стекла (system glass).
  - Отсутствие готового OSR-композитора **не блокирует** запуск функционального браузера в M1.2.

---

## 16. Decision Matrix Summary

| Критерий | Вес | Chromium / CEF | Edge WebView2 | WPE WebKit | Gecko | Servo |
| :--- | :--- | :---: | :---: | :---: | :---: | :---: |
| **Embedding Model** | Required | **High** | High | Medium | Low | Medium |
| **Process Isolation** | Required | **High** | High | High | High | Medium |
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

## 17. Selected first provider

**Chromium Embedded Framework (CEF / Chromium)** выбран в качестве первого провайдера браузерного движка для **Milestone M1.2**.

Обоснование:
1. Наивысшая полнота поддержки современных веб-стандартов, accessibility APIs и медиа-кодеков.
2. Проверенная временем модель полной изоляции процессов рендеринга и аварийного восстановления.
3. Полный программный контроль над профилями, изолированными хранилищами, cookies и сетевыми перехватами.
4. Чистая разрешительная лицензия BSD-3-Clause без copyleft-рисков.
5. Кроссплатформенная однородность (Windows, Linux, macOS) без привязки к единственному вендору ОС.

---

## 18. Rejected alternatives

1. **Microsoft Edge WebView2**: Отклонен как основной движок из-за отсутствия кроссплатформенности и невозможности гарантировать независимость от политики Microsoft. Может быть рассмотрен позже как специализированный lightweight-провайдер для Windows.
2. **WPE WebKit / WebKitGTK**: Отклонен из-за высокой сложности качественной сборки и встраивания на Windows и меньшей зрелости Accessibility API на не-Linux платформах.
3. **Gecko**: Отклонен из-за отсутствия поддерживаемого официального standalone C/C++ embedding API для десктопа.
4. **Servo**: Отклонен для первой промышленной версии M1.2 из-за неполной поддержки современного веб-стека, но остается приоритетным кандидатом на долгосрочное исследование как чистый Rust-движок.

---

## 19. Reopen conditions

Данное решение может быть пересмотрено при наступлении одного из условий:
1. Servo достигнет зрелости ACID3 / Web Platform Tests > 95% и предоставит промышленную поддержку медиа/DRM/JS-фреймворков.
2. Изменение лицензионной политики или структуры проекта Chromium, препятствующее свободному встраиванию.
3. Появление высокостабильного кроссплатформенного WebKit порта со стабильным Rust C-FFI.
