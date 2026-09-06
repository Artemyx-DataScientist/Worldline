# Worldline — threat model для AI-native browser platform

> Repository copy of the dated audit model. It describes the checkout at `dfc644dfd6f6aa4335231b15bf0d5019a3c073fd` and the proposed vNext boundary; it is not a claim that every target control already exists.

Дата: 2026-09-06. Scope: текущий checkout и proposed vNext, с явным разделением.
Baseline: `dfc644dfd6f6aa4335231b15bf0d5019a3c073fd`.
Target ID: `sha256:0587591cd50e48a6810a0165a872db00d5581cc37b7b3b195c52450500af1cc2`.
Режим: standalone, fresh per-audit model; shared threat-model cache не использован.
Основания: локальный код, свежие ограниченные тесты, независимое fresh-context review; это не полный vulnerability scan.

## 1. Обзор системы и фактические ресурсы

Worldline содержит generic capability kernel; in-process Rust providers; external native-process и WASM adapters; browser contracts/reference backend; Windows CEF provider; browser domain services и SQLite/blob storage. Production shell, полный generated-feature browser path и durable package updater ещё не установлены.

Малый Worldline kernel — часть TCB. Host runtime, native providers, CEF browser process/FFI, credential/file/network brokers, Wasmtime host imports, updater и protected UI также влияют на безопасность. Вынесение компонента в plugin не выводит его из TCB.

### Effective resources и режимы запуска

Пути ниже — результаты конкретных constructors/entrypoints, а не обещание общей production configuration. `cwd`, `TEMP`, env и explicit args выбирает host. Контроль host environment не приписывается веб-странице или WASM guest.

| ID / режим | Фактический ресурс, выбор и consumer | Authority / isolation и доказательство |
| --- | --- | --- |
| R01 Native Windows | `NativeChildSpec.program/args` → обычный `Command`; stdin/stdout/stderr pipes; env и cwd не очищаются/не заменяются этим launcher | Аккаунт host, без AppContainer/restricted-token FS/network sandbox в этом пути. Job kill-on-close управляет жизнью дерева. [spawn](<../../crates/worldline-native-host/src/supervisor.rs#L85>), [Job limits](<../../crates/worldline-native-host/src/containment.rs#L35>) |
| R02 Native non-Windows | Тот же native supervisor; containment wrapper создаётся успешно | `ProcessTreeJob` create/assign — no-op. Нельзя переносить обещание tree cleanup из module comment на эти платформы. [platform branch](<../../crates/worldline-native-host/src/containment.rs#L77>) |
| R03 WASM | Component binary ≤8 MiB; guest memory 64 MiB; table 10 000; fuel 1 000 000 000 на call/instantiation; host-call payload 1 MiB по defaults | In-process Wasmtime; imports только state-access/event-publish; host-supplied broker. Нет ambient WASI. Manifest hints могут лишь уменьшать baseline limits. Это не общий OS sandbox host. [host](<../../crates/worldline-wasm-host/src/adapter.rs#L247>), [imports](<../../crates/worldline-wasm-host/src/adapter.rs#L310>), [limits](<../../crates/worldline-wasm-host/src/limits.rs#L22>), [WASI policy](<../../crates/worldline-wasm-host/src/wasi.rs#L9>) |
| R04 Real CEF Windows | Bootstrap executable загружает provider client DLL; CEF multi-process dispatcher получает sandbox_info | Nonzero Windows sandbox pointer required; `no_sandbox=0`; sandbox-disabling switches rejected. Это upstream renderer sandbox, а не sandbox всего native provider account. [CEF init](<../../crates/worldline-browser-cef/src/ffi.rs#L167>), [flags](<../../crates/worldline-browser-provider-process/src/lib.rs#L212>), [real launch](<../../crates/worldline-reference/src/s3b.rs#L1004>) |
| R05 CEF profile default | Provider `--cache-root` → иначе `WORLDLINE_CEF_CACHE_ROOT` → иначе `(cwd, fallback TEMP)/target/worldline-cef-profiles`; persistent context → root / SHA256(profile_id либо context_id) | CEF request-context consumer получает cache_path; incognito оставляет его отсутствующим. Logical context ID не изолирует весь engine browser process. [arg](<../../crates/worldline-browser-provider-process/src/lib.rs#L176>), [precedence](<../../crates/worldline-browser-provider-process/src/lib.rs#L240>), [default root](<../../crates/worldline-browser-cef/src/backend.rs#L1628>), [profile hash](<../../crates/worldline-browser-cef/src/backend.rs#L1797>), [context](<../../crates/worldline-browser-cef/src/backend.rs#L1944>) |
| R06 CEF S3B real profile | Explicit temporary root / `cef-runtime`; затем тот же hashed context/profile directory | Изолированный proving root, не доказательство настоящего user-profile deployment. [roots](<../../crates/worldline-reference/src/s3b.rs#L989>), [launch](<../../crates/worldline-reference/src/s3b.rs#L1004>) |
| R07 CEF download writer | Default: cache_root / `downloads` / `<download-id>-<sanitized-name>`; explicit destination принимается отдельной проверкой | Relative destination проверяется как `root.join(path)`, но callback получает исходный relative path. Consumer normalization расходится; фактическое CEF разрешение пути и attacker reachability не проверены. [admission](<../../crates/worldline-browser-cef/src/backend.rs#L2417>), [CEF sink](<../../crates/worldline-browser-cef/src/backend.rs#L818>) |
| R08 SQLite state | `SqliteStorage::open(profile_root)` → canonical root / `worldline.sqlite3`; WAL, foreign keys и FULL synchronous | Host выбирает root; broker выдаёт runtime-bound state handles. Default S1 использует in-memory, `run_production` — SQLite. [open](<../../crates/worldline-storage/src/sqlite.rs#L76>), [consumer](<../../crates/worldline-storage/src/sqlite.rs#L307>), [entrypoints](<../../crates/worldline-reference/src/s1.rs#L278>) |
| R09 Default artifact blobs | `WORLDLINE_DOWNLOAD_BLOB_ROOT` → иначе `(cwd, fallback TEMP)/target/worldline-download-blobs`; FilesystemBlobStore добавляет `blobs/sha256-v1-<digest>` | Canonical root/path checks есть; ArtifactStore::new — development default, hosted caller должен передавать explicit root. Hash — identity, не authority. [root](<../../crates/worldline-browser-downloads/src/artifact.rs#L43>), [store](<../../crates/worldline-storage/src/blob.rs#L27>), [path](<../../crates/worldline-storage/src/blob.rs#L87>) |
| R10 S3B host blobs | TempRoot / `host-blobs/blobs/<blob-id>` | Reference host blob sink разрешает Put/Verify и запрещает Get. Нельзя объявлять provider владельцем произвольного blob-read. [root](<../../crates/worldline-reference/src/s3b.rs#L989>), [sink](<../../crates/worldline-reference/src/s3b.rs#L630>) |
| R11 Downloads metadata | `DownloadsService::open`: staging_root / `downloads.snapshot.json`; `open_persistent`: explicit state_path; S3B: TempRoot / `downloads-state/records.json`, отдельный `service-staging` | Service сохраняет snapshot; выбор location делает host. Это настоящая domain persistence, отличная от in-memory upgrade catalog. [persistent service](<../../crates/worldline-browser-downloads/src/service.rs#L62>), [default](<../../crates/worldline-browser-downloads/src/service.rs#L100>), [S3B paths](<../../crates/worldline-reference/src/s3b.rs#L989>) |
| R12 Backup | Host-selected destination SQLite file; существующий destination отвергается; backup/fsync/schema validation | Копирует SQLite metadata, не внешний blob inventory. Не является автоматически полным browser backup. [backup contract](<../../crates/worldline-storage/src/sqlite.rs#L94>) |
| R13 Restore | Host-selected backup source → новый target profile / worldline.sqlite3; source проверяется, существующий target DB не перезаписывается | Контроль source не равен праву восстановить произвольный профиль: restore API остаётся host administration. [restore](<../../crates/worldline-storage/src/sqlite.rs#L187>) |
| R14 Upgrade/quarantine | Revision maps, staging maps и quarantined revisions хранятся в process memory | Нет связанного durable package activation pointer. Chaos child открывает SQLite отдельно и не сохраняет результат commit_switch как active code revision. [manager](<../../crates/worldline-kernel/src/upgrade.rs#L220>), [quarantine](<../../crates/worldline-kernel/src/quarantine.rs#L63>), [consumer](<../../crates/worldline-storage/src/bin/upgrade-chaos-child.rs#L85>) |
| R15 CEF CI staging | `-CacheRoot` → env WORLDLINE_CEF_CACHE_ROOT → RUNNER_TEMP/worldline-cef-runtime → repo/target/cef-runtime-cache; staged `cef-staged-windows64` с bootstrapc.exe/libcef.dll/client DLL | Exact archive SHA256 проверяется; required files/archive identity проверяются; client DLL строится и копируется отдельно. Это build staging, не полный signed installed-release catalog и не защита от уже контролирующего staging host account. [precedence](<../../scripts/ci/Initialize-WorldlineCef.ps1#L40>), [hash](<../../scripts/ci/Initialize-WorldlineCef.ps1#L61>), [build/load artifacts](<../../scripts/ci/Initialize-WorldlineCef.ps1#L153>) |
| R16 Chromium CDP spike | `CHROME_BIN` или known Windows Chrome/Edge; temporary profile `TEMP/worldline_chromium_spike_<pid>`; debugging port выбирается через bind 127.0.0.1:0 | Headless development/proving path, отдельный от CEF. Loopback automation endpoint не является общедоступным feature API; не обнаружен как production network service. [binary selection](<../../crates/worldline-browser-spike/src/chromium.rs#L277>), [launch](<../../crates/worldline-browser-spike/src/chromium.rs#L342>) |

Положительные boundaries: kernel-issued principals/grants, provider caller binding, operation/resource grant validation, attenuation, lifecycle-bound handles; plugin activation получает ограниченные context handles. Это полезная основа, и наличие незамкнутых domain paths её не отменяет. [principal registration](<../../crates/worldline-kernel/src/kernel.rs#L973>), [host root grants](<../../crates/worldline-kernel/src/kernel.rs#L1173>), [caller binding](<../../crates/worldline-kernel/src/invocation.rs#L190>), [admission](<../../crates/worldline-kernel/src/invocation.rs#L620>), [activation context](<../../crates/worldline-kernel/src/plugin.rs#L193>)

## 2. Trust boundaries, assets, actors и предположения

### Assets

Credentials/session cookies/passkeys; browsing history и page content; private/incognito data; host filesystem; package code/state and trusted active revision; grant authority и revocation; update signing/verification policy; UI identity/consent; browsing availability и recovery; correctness of externally dispatched actions.

### Actors и исходные полномочия

| Actor | Что контролирует | Чего ему не приписывается |
| --- | --- | --- |
| Malicious website | DOM/scripts, navigation/redirects, frames, response/download bytes, инструкции в странице | Host args/env, native binaries, kernel grants, user approval |
| Malicious или ошибочный feature package | Guest code, manifest requests, его own state/UI/messages | Ambient filesystem/network, other principals, grant issuance; эти ограничения должны enforce-иться |
| AI/model output | Предложенный код, interpretation страницы, предложенный tool call | Способность выдавать себе permissions или считать webpage instruction consent |
| Compromised feature dependency/publisher | Код внутри допущенного package revision | Подпись не даёт дополнительных runtime прав |
| Trusted native/system provider | Своё исполнение и выданные host privileges; фактически account-level OS access в текущем launcher | Его компрометация рассматривается как TCB compromise, а не обычный guest capability use |
| Local same-account attacker / OS admin | Может менять доступные ему host files/processes | Не используется как доказательство web-to-host privilege escalation; ОС сама по себе не обещает изолировать владельца от собственного account |
| Crash, power loss, incorrect generated code | Нарушение progress, partial operations, corrupt staged data | Не требует злого умысла; те же containment/recovery invariants должны действовать |

### Границы и обязательные проверки

1. **Web → engine:** upstream origin/site/process isolation и web policy. Worldline не отключает sandbox/CSP/SOP ради automation. Site isolation и renderer sandbox не означают маленький или недоверенный browser process. [Chromium site isolation](https://www.chromium.org/Home/chromium-security/site-isolation/)
2. **Page/model → planner authority:** page data остаются данными. Model не может преобразовать instruction в grant. Sensitive effect привязан к task/user intent и повторно проверяется broker.
3. **Package → runtime:** immutable digest, import/ABI validation, limits, trust class, отдельный build sandbox. Manifest validation сама не доказывает безопасность исполняемого кода.
4. **Guest → host broker:** authenticated principal/session, operation/resource scope, input schema/size, recipient identity. Caller identity из payload недопустима.
5. **Broker → browser/OS sink:** текущие origin/frame/document/profile/epoch и фактический filesystem/network target. Admission другого resource не переносится на payload.
6. **Runtime → durable state:** InstallationId переживает RuntimeId, но runtime authority не переживает revoke/replacement. State ownership и concurrent writer generation проверяются независимо.
7. **Feature UI → security UI/input:** untrusted slot не перекрывает consent/origin/stop, не крадёт protected focus и не становится recovery owner.
8. **Feature data → другие features/model/network/log/UI:** отдельная egress/declassification policy. Данные, уже раскрытые unrestricted коду с разрешённым внешним каналом, нельзя вернуть обратно revocation.
9. **Update → boot:** аутентифицированный bundle, совместимость с security floor, durable selection, independent recovery; никакой live patch доверенного кода из feature.

Текущие сильные стороны WASM не являются доказательством полной host containment. Host imports и сам runtime остаются доверенными; fuel не ограничивает автоматически compilation/блокирующий host syscall. [Wasmtime security](https://docs.wasmtime.dev/security.html)

### Предположения и явные неизвестные

- Текущие host-only domain APIs не считаются уже опубликованными любому сайту/guest. Их безопасная публикация — ещё работа.
- В текущем checkout не установлен production AI planner/executor с полным browser capability path; prompt injection рассматривается для target, не как найденный действующий RCE.
- У Worldline нет проверенной здесь desktop Gecko runtime assembly; два настоящих backend на одинаковых сценариях не сравнивались.
- Upstream CEF sandbox initialization прослежен статически, но actual OS token/AppContainer/renderer filesystem denial в этом аудите не измерялись.
- Не проверены все browser egress пути, hostile reparse-point races, CEF relative destination resolution, compile-time resource exhaustion, malicious native descendant timing.
- User/AI-generated native code запрещено считать untrusted только на основании IPC protocol. Если такой формат станет requirement, требуется иная OS containment model и новая threat model.
- Стабильность сигнатур/ABI не гарантирует одинаковую semantic behavior после engine update.
- Incognito partition требует отдельного policy для AI memory, logging и feature persistence; engine incognito flag не решает их автоматически.

## 3. Приоритетные attacker/failure stories

P0/P1 ниже — приоритет архитектурного gate перед допуском недоверенных features, а не CVSS текущей уязвимости.

| ID / priority | Story: источник → boundary → sink / asset | Статус по коду и недоказанные prerequisites | Требуемая защита / проверка |
| --- | --- | --- | --- |
| T01 P0 | Website содержит инструкцию прочитать другую вкладку/секрет и отправить модели; planner вызывает более широкую capability | Future confused-deputy risk. Current production planner path не установлен; F02 показывает незамкнутые domain boundaries | Page instructions не authority; selected-resource lease; independent model egress; adversarial page test |
| T02 P0 | Feature подставляет другой context/page либо вызывает cookie value library без caller grant | Методы host-only; их прямая будущая публикация нарушит isolation. [cookie read](<../../crates/worldline-browser-cookies/src/service.rs#L448>), [dispatch](<../../crates/worldline-browser-provider/src/core.rs#L147>) | Gateway обязан сопоставлять admitted target и payload; raw cookie values не обычный generated API |
| T03 P0 | Generated native provider читает profile/files, создаёт subprocess/socket без RPC | R01 подтверждает ambient native authority. Это не «обход» WASM: prerequisite — допуск native code в trusted class | Не разрешать ordinary generated native packages; sandboxed WASM и независимая OS boundary |
| T04 P0 | После Stop/revoke уже admitted action всё ещё отправляет request/записывает файл | Admission-only семантика явно проверяется existing test; поздний effect как конкретный exploit не воспроизведён. [revocation test](<../../crates/worldline-kernel/tests/security_acceptance.rs#L1435>) | Effect fence, leases/epochs, cancellation; dispatched/unknown outcomes не обозначать отменёнными |
| T05 P0 | UI feature скрывает origin, подделывает consent, перехватывает ввод и блокирует выключение | Target UI hazard; нынешний UiLike — string fixture, product compositor не реализован. [UI fixture](<../../crates/worldline-reference/src/ui_like.rs#L87>) | Protected compositor/input surface; trusted user gesture token; crash/layout recovery |
| T06 P0 | Bad revision активируется; crash host забывает quarantine и запускает её снова либо смешивает old code/new state | Upgrade/quarantine in-memory R14; current tests не доказывают persisted active package | Durable generation pointer, one writer, old epoch invalidation, fresh-process kill/restart tests |
| T07 P0 | Feature получает WASM capability import, а host broker реализует его через unrestricted engine/debugger/native API | Пока этого import в exported world нет; отсутствие пути — gap, а не безопасность будущей реализации. [world](<../../crates/worldline-plugin-protocol/wit/worldline-plugin.wit#L66>) | Curated operations, caller-bound handles, parser limits, executor-side target validation |
| T08 P1 | Download response/файл достаточно велик, CEF host читает целиком, сериализует в one-frame blob RPC | Source path R07/R10: whole-file allocation и frame risk установлены; OOM/DoS динамически не воспроизведён. [read](<../../crates/worldline-browser-cef/src/backend.rs#L878>), [transfer](<../../crates/worldline-browser-provider-process/src/lib.rs#L730>) | Streaming/backpressure/total quotas; failure не должен блокировать core/recovery |
| T09 P1 | Download destination проходит lexical admission, но filesystem consumer использует иной base или reparse resolution | Relative-path discrepancy R07 подтверждён; attacker-controlled relative path reachability и actual CEF write location не доказаны | Host-owned staging и handles; canonical/open-time containment; targeted integration test до публикации destination API |
| T10 P1 | Optional network policy fails open, а продукт считает его обязательным запретом утечки | Контракт допускает optional FailOpen и синхронный callback; security-wide coverage не доказана. [callback](<../../crates/worldline-browser-cef/src/backend.rs#L542>), [budget](<../../crates/worldline-browser-contract/src/request_policy.rs#L34>) | Mandatory floor отдельно; compiled rules; coverage redirects/workers/cache/other transports; feature/model egress отдельно |
| T11 P1 | Новый engine adapter принимает default no-op policy/cookie downgrade и рекламирует прежние permissions | Latent backend-contract risk; CEF overrides cookie v0.2. [defaults](<../../crates/worldline-browser-provider/src/backend.rs#L35>), [handshake](<../../crates/worldline-browser-provider-process/src/lib.rs#L446>) | Operation-level support, Unsupported instead of false success; contract/engine semantic acceptance |
| T12 P1 | Guest flood/много инстансов/host call stall расходует aggregate CPU/memory и лишает браузер responsiveness | Per-instance fuel/memory denial прошли тесты; aggregate/compile/OS denial не проверены этим аудитом | Process resource accounting, wall-clock deadlines, compilation isolation, bounded queues; independent watchdog |
| T13 P1 | Package/dependency или подменённый update расширяет authority либо навсегда удерживает старый engine | Future supply-chain/update risk; R15 pin/hash полезен, но не installed update workflow | Immutable bundle + verified metadata/security floor; rights diff отдельно от compatibility; optional feature quarantine |
| T14 P1 | Разрешённый read-cookie/DOM сочетается с fill и egress; секрет попадает в model/log/другую feature | Composition hazard; redacted Debug не предотвращает передачу bytes; точный текущий chain не установлен | Raw secrets withheld; несовместимые grants контролируются совместно; declassification and output recipients explicit |
| T15 P1 | Backup/restore сохраняет DB без blobs либо feature state выносит incognito data из engine partition | Backup metadata-only R12 подтверждён; полный user backup и AI-memory pipeline не установлены | Inventory/retention/privacy contracts; backup completeness test; incognito namespace/persistence denial |
| T16 P1 | Новый host повторяет неопределённый сетевой action после rollback/restart | Target transactional hazard; нельзя вывести externally exactly-once из локального WAL | Effect IDs, dispatch journal, remote idempotency при поддержке сервиса; unknown outcome reconciliation без blind retry |

### Материальные выводы, сохранённые после независимого review

Реальный privileged boundary нельзя вывести из названия crate или manifest capability. В частности, BlobReadBroker принимает host-issued principal/capability data как отдельный helper; это не доказательство единой browser authority chain. [blob grant issuer](<../../crates/worldline-storage/src/blob.rs#L265>)

CEF sandbox и NativeHost containment — разные механизмы. Внешний provider может быть supervised и одновременно оставаться trusted account-level code. Browser profile, download staging, blob root и SQLite root — разные ресурсы с разными constructors, их нельзя объединять в один «профиль» без анализа consumers.

Durability тоже неоднородна: SQLite и DownloadsService действительно имеют persistent storage paths, а UpgradeManager/QuarantineManager пока сохраняют только runtime maps. Называть всю систему stateless или всю recovery систему durable было бы одинаково неверно.

## 4. Severity calibration и границы заключения

Этот аудит **не подтвердил remotely exploitable RCE**. P0 findings означают, что до открытия недоверенного programming runtime архитектурная гарантия отсутствует или не подтверждена. Они не означают, что сайт сегодня может выполнить произвольный Win32-код.

- **Critical/High**, если будет доказан путь от website или ordinary WASM feature к arbitrary native execution, raw credentials другого context, подмене trusted consent/update, либо запись за пределами явно разрешённого resource. Нужно показать реальный entrypoint, attacker control, пересечённую boundary и sink.
- **High/Medium**, если показан cross-origin/profile read/write или необратимый effect после установленного revocation fence; оценка зависит от реальных grant prerequisites и объёма данных.
- **Medium**, если один недоверенный package/document валит общий browser host или приводит к воспроизводимой потере user state. Отказ только своего sandbox с работающим recovery обычно ниже.
- **Low/Informational**, если проблема лишь в readiness claims без текущего runtime exposure, либо требует уже полного контроля same-account native host. Claims важны как архитектурный gate, но сами не RCE.

Проверено свежими тестами: выбранные generic authority/compatibility/recovery models и WASM resource/import containment. Не проверены эксплуатационные chains, actual CEF sandbox token, real UI spoof resistance, production activation crash recovery или полный egress coverage. Детали: [EVIDENCE.md](<AI-NATIVE-BROWSER-AUDIT-EVIDENCE-2026-09-06.md>).

Residual risk остаётся даже после vNext: ошибки engine/OS/Wasmtime/brokers, malicious authorized action, сознательно разрешённое раскрытие данных и site changes. Достижимая гарантия — ограниченные полномочия, принудительные boundaries и recoverable local feature failures; не доказательство полной корректности произвольного AI-кода.
