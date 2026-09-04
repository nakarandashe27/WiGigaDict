# Roadmap WiGigaDict

Состояние на 3 сентября 2026 года: M0 и M1 Steps 5–15 реализованы; Step 16 имеет готовый автоматизированный durability slice, но ждёт реального owner run, Step 17 имеет готовые NSIS/clean-install инструменты, но ждёт прогона на чистых Windows VM. Инженерные этапы продолжаются с первого `[ ]` и не обходят эти personal-alpha gates.

## Правило: архитектура описывает целевую систему

Файлы архитектуры (`01-обзор` … `08-дорожная-карта`) — единый источник правды о том, КАКОЙ система должна быть по итогам текущего замысла (целевой чертёж), а не только снимок сегодняшнего кода. Каждый компонент / способность / сущность помечается статусом: ✅ реализовано (есть в коде) · 🔜 запланировано (решено, кода ещё нет) · 💡 гипотеза (требует решения/исследования).

Для частей со статусом ✅ источник истины — код: если документация расходится с уже написанным кодом, прав КОД — чините документацию. Для 🔜/💡 документация — это проектное решение, по которому код будет писаться.

Решение поменялось — правь сам файл, не оставляй устаревшее: документация, расходящаяся с замыслом, хуже её отсутствия.

Значимое решение (смена стека, изменение модели данных, сдвиг границ системы, новый или убранный модуль) дополнительно фиксируй короткой ADR-записью с датой и причиной в `context/architecture/06-решения/журнал-решений/`. Журнал отвечает на «почему и когда поменялось», файлы архитектуры — на «какой система должна быть по замыслу».

Мелкие уточнения вноси прямо в файлы — журнал для них не нужен, их история и так лежит в git. Не перекраивай архитектуру без причины на ходу, но реальное новое решение всегда отражай в документации, а не прячь его «чтобы не нарушать правило».

## Как выполнять roadmap

- Брать первый незакрытый этап `[ ]`; более поздний этап начинать раньше можно только для независимого research/spike без изменения production-кода.
- Перед реализацией читать только перечисленный у этапа контекст, а не всю папку.
- После выполнения критерия готовности менять `[ ]` на `[x]`, обновлять статусы компонентов и синхронизировать затронутые backlog items.
- Personal alpha выпускается сразу после своих gates; дата public launch не оправдывает пропуск проверки и не задерживает личное использование.
- Если gate провален, фиксировать причину/решение ADR и исправлять текущий этап, а не маскировать дефект следующим слоем UI.

## Milestones

| Milestone | Результат | Условие выхода |
|---|---|---|
| M0 — Risk retirement | Воспроизводимый skeleton и доказаны два самых рискованных внешних контура | Win32 insertion/overlay spike и ASR benchmark дают принятые ADR |
| M1 — Personal alpha | Полный локальный цикл `нажать → сказать → нажать → получить/восстановить текст` | Три blocker-gates, 0 безвозвратно потерянных результатов из 100, clean install |
| M2 — Post-MVP | Второй движок, длинная диктовка, Notetaker v1, verbatim/history/glossary | Каждый модуль включается отдельно и не ломает M1; Notetaker ждёт personal alpha и long-form/FFmpeg gates |
| M3 — Public Windows launch | Собственный бренд, подписанный installer/update channel, support и публичная матрица качества | Release gates пройдены; целевая дата — 31 октября 2026 года |
| M4 — Future expansion | Российский B2B-контур и оценка macOS | Отдельные research/legal/security/architecture решения после стабильной Windows-версии |

## M0 — Инженерная основа и снятие главных неизвестных

- [x] **0. Зафиксировать целевой архитектурный baseline**
  - **Что делаем:** Завершить интервью, research, модель данных, стек, premortem, security/cost contracts и порядок реализации. Подготовить ADR-журнал для будущих изменений.
  - **Контекст из архитектуры:** [README архитектуры](../README.md), [карта системы](../01-обзор/карта-системы.md), [premortem](../07-нефункциональные/риски-архитектуры.md), [журнал решений](../06-решения/журнал-решений/README.md)
  - **Критерий готовности:** Stages 1–13 context-builder завершены; roadmap и cross-links существуют; кода приложения нет и это явно отмечено.

- [x] **1. Поднять воспроизводимый Windows toolchain и repository skeleton**
  - **Что делаем:** Выполнить BL-036: установить/pin Rust, MSVC/Windows SDK и необходимые build tools; создать Tauri 2 + React/TypeScript/Vite shell, Cargo workspace и пустой bundled ASR sidecar без бизнес-логики.
  - **Контекст из архитектуры:** [выбранный стек](../05-стек/технологии.md), [отвергнутые варианты](../05-стек/что-не-выбрали.md), [границы](../01-обзор/границы.md), [расходы](../07-нефункциональные/расходы.md)
  - **Критерий готовности:** С чистого checkout одной документированной командой собирается и запускается `v0.0.1-dev`; toolchain/lockfiles pinned; sidecar включён в dev bundle и отвечает только version handshake.

- [x] **2. Ввести quality gates и контрактные test harnesses**
  - **Что делаем:** Настроить format/lint/unit/integration jobs для Rust и TypeScript, fixture-driven NDJSON tests, fault-injection hooks, dependency/license/SBOM checks и Windows CI. Production logic ещё не добавлять.
  - **Контекст из архитектуры:** [безопасность](../07-нефункциональные/безопасность.md), [стек — Build/CI](../05-стек/технологии.md#таблица-технологий), [нерушимые правила](../03-данные/правила-нерушимые.md)
  - **Критерий готовности:** CI на fresh Windows runner собирает shell/sidecar, прогоняет пустые contract suites и публикует неподписанный internal artifact вместе с SBOM/license report.

- [x] **3. Закрыть Win32 spike: hotkey, no-focus overlay и insertion evidence**
  - **Что делаем:** Выполнить BL-026/037: доказать key-down/key-up, `WS_EX_NOACTIVATE`, сохранение foreground target и лестницу Unicode/`SendInput`/clipboard на Windows 10. Windows 11 matrix отложена в BL-047 без заявления совместимости (ADR-005). Не строить полный UI до результата spike.
  - **Контекст из архитектуры:** [Windows insertion capability](../02-ядро/способности/вставить-текст.md), [delivery evidence](../03-данные/правила-нерушимые.md#контракт-доказательства-доставки), [триггеры](../04-потоки/триггеры.md), [граница привилегий](../01-обзор/границы.md#privilege-boundary)
  - **Критерий готовности:** Автоматизированная/ручная Windows 10 golden matrix для VS Code/Codex, terminal/Claude Code, browser и standard controls фиксирует evidence; focus/UIPI/clipboard failures дают `uncertain`, overlay ни разу не крадёт focus; выбранный path и граница OS support оформлены ADR.

- [x] **4. Выбрать первый ASR engine измерением, а не ожиданием**
  - **Что сделано:** Для personal MVP владельца выполнены BL-005/006/009 и benchmark-часть BL-046: RU/EN technical corpus, exact-pinned Whisper/GigaAM runtime probes, 5/15/25/30/60-секундные samples, CPU/GPU, cold/warm и crash/restart evidence. Выбраны Whisper large-v3-turbo Q5 Vulkan и явный `cpu-t16` recovery fallback ([ADR-006](../06-решения/журнал-решений/2026-08-23-006-whisper-personal-mvp.md)).
  - **Контекст из архитектуры:** [ASR gate и runtime stack](../05-стек/технологии.md#asr-gate-что-именно-сравниваем), [способность ASR](../02-ядро/способности/распознать-речь.md), [cost/resource contract](../07-нефункциональные/расходы.md), [MVP metrics](../../idea/04-mvp/success-metrics.md)
  - **Evidence scope:** Sanitized reports фиксируют WER/CER, technical-token errors, p50/p95, RTF, RAM/VRAM и truncation/boundary checks. Выбор не заявляет multi-speaker/public quality; owner golden flow остаётся Step 16, а clean-install, strict watts/kWh, disk и packaged runtime/model bytes — Step 17 и открытая BL-046.

### Research track Notetaker до M2

Эти spikes можно выполнять независимо, не меняя первый незакрытый production step. Их документы не означают готовую функцию.

- [x] **R1. Доказать воспроизводимую LGPL-сборку FFmpeg**
  - **Что делаем:** На основе завершённого feasibility research собрать exact-pinned shared Windows build без GPL/nonfree, подготовить source/recipe/licenses/SHA-256/SBOM и проверить реальную media matrix. Ничего не бандлить в M1 installer.
  - **Контекст из архитектуры:** [FFmpeg spike](../05-стек/исследования/2026-08-21-ffmpeg-lgpl-build-spike.md), [ADR-003](../06-решения/журнал-решений/2026-08-21-003-bundled-lgpl-ffmpeg.md), [security gates](../07-нефункциональные/безопасность.md#release-gates-безопасности)
  - **Критерий готовности:** Clean rebuild и dependency/license audit воспроизводимы; `ffmpeg -buildconf` не содержит GPL/nonfree; source/binary correspondence и probe/decode/cancel/corrupt matrix зелёные.
  - **Выполнено 2 сентября 2026:** tag `n8.1.2`, подпись тарбола проверена ключом FFmpeg с независимого keyserver, изменений апстрима 0. Профиль shared LGPL 2.1 без GPL/nonfree/version3 и вовсе без внешних библиотек (`--disable-autodetect`): нужные форматы покрывают родные декодеры. Две чистые сборки в разных каталогах **побайтово идентичны** после снятия трёх недетерминированных величин — путь сборки в строке configure, штампы времени в таблице экспорта и в PE-заголовке. Поставка 29,8 МБ, PE-зависимости только системные. Media matrix 11/11, compliance bundle собран. Рецепт: `scripts/build-ffmpeg-lgpl.sh`, матрица: `scripts/ffmpeg-media-matrix.sh`, bundle: `scripts/ffmpeg-compliance-bundle.sh`; подробности в [spike](../05-стек/исследования/2026-08-21-ffmpeg-lgpl-build-spike.md).

- [x] **R2. Пройти long-form benchmark и 4-hour soak**
  - **Что делаем:** После Step 4 прогнать model-independent artifact/segment contract, window/overlap/VAD candidates, seam fixtures, restart и Dictation preemption. Production Notetaker module не создавать.
  - **Контекст из архитектуры:** [long-form spike](../05-стек/исследования/2026-08-21-long-form-asr-spike.md), [ASR gate](../05-стек/технологии.md#asr-gate-что-именно-сравниваем), [Notetaker invariants](../03-данные/правила-нерушимые.md#notetakerjob-и-long-form)
  - **Критерий готовности:** 30/60-second markers не теряются/не дублируются, final drift ≤1 s; 4-hour soak имеет zero lost chunks/truncation, bounded resources и lossless crash/preemption resume; accepted profile/config зафиксирован evidence.
  - **Выполнено 2 сентября 2026:** harness `scripts/long-form-benchmark.ps1`, фикстуры собираются синтезом, поэтому время каждого маркера известно по построению. Принятый профиль: окно 30 с, перекрытие 2 с, whisper turbo/vulkan, seam policy v2 (чанк отдаёт только свой шаг плюс снятие дубля по повтору слов на пересечении). Soak: 14 403 с медиа, 1552 маркера, 515 чанков, RTF 0,0408, пик 627 МБ без тренда роста, 0 дублей, дрейф финального маркера 0 мс при максимуме 554 мс, 515 из 515 чанков зафиксированы, 10 вытеснений диктовкой на границах, экспорт TXT/SRT/VTT стабилен. Два настоящих убийства процесса и одна имитация продолжились с checkpoint без пересчёта и дублей. Найдено и закрыто: политика «перекрытие за ранним чанком» теряла хвост, дубли на границе владения, отказ движка на монотонном звуке (24 чанка из 515, все приняты одним повтором с укороченным окном). Подробности в [spike](../05-стек/исследования/2026-08-21-long-form-asr-spike.md).

## M1 — Вертикальный срез и personal alpha

- [x] **5. Реализовать SQLite schema и миграции доменной модели**
  - **Что делаем:** Создать bundled SQLite schema для 18 entities, FK/UNIQUE/CHECK constraints, immutable versions, timestamps, WAL+FULL settings и versioned migrations. UI/ASR пока используют repository contracts, а не прямой SQL.
  - **Контекст из архитектуры:** [сущности](../03-данные/сущности.md), [ER-схема](../03-данные/схема-связей.md), [главный кирпич](../03-данные/главный-кирпич.md), [инварианты](../03-данные/правила-нерушимые.md)
  - **Критерий готовности:** Fresh DB и upgrade fixture накатываются автоматически; все 18 entities/relations представлены; negative tests доказывают ключевые FK/XOR/state constraints.

- [x] **6. Реализовать durable session journal и двуххранилищный PCM commit**
  - **Что сделано:** Добавлены technical commit ledger, prepare row, bounded PCM writer, `.part`, hash/size, `FlushFileBuffers`, same-volume write-through rename без перезаписи, CAS SQLite final marker и идемпотентный startup reconciliation по `commit_id`.
  - **Контекст из архитектуры:** [commit-протокол](../03-данные/правила-нерушимые.md#commit-протокол-sqlite--pcm), [audio/session entities](../03-данные/сущности.md#основной-aggregate-диктовки), [основной поток](../01-обзор/карта-системы.md#основной-поток), [killer risk 2](../07-нефункциональные/риски-архитектуры.md#2-sqlite-и-pcm-переживали-crash-как-две-несогласованные-истины)
  - **Evidence:** Шесть checkpoint restart-cases (prepare, part write, flush, rename, SQLite commit window и post-commit), повторная reconciliation и negative cases дают только `continue`/`recovery`/`corrupt`; queued ASR остаётся не более одной, принятый artifact не удаляется. Storage tests: 20/20; полный quality gate зелёный.
  - **Критерий готовности:** Выполнен в personal-MVP scope без микрофона, прослушивания, моделей или новой ASR-матрицы.

- [x] **7. Собрать безопасный lifecycle Rust/Tauri shell**
  - **Что сделано:** Реализованы non-elevated bootstrap, per-user single-instance mutex до writer-capable setup (повторный запуск показывает окно живого экземпляра через broadcast-активацию, а не завершается молча), tray close/show/quit, UUID startup generation, WTS lock/disconnect/logoff и shutdown handling, protected `%LOCALAPPDATA%\WiGigaDict\`, strict CSP и отдельные main/overlay capabilities с Rust-side caller authorization.
  - **Контекст из архитектуры:** [границы](../01-обзор/границы.md), [права](../02-ядро/права-доступа.md), [безопасность](../07-нефункциональные/безопасность.md), [триггеры](../04-потоки/триггеры.md)
  - **Evidence:** 13/13 desktop tests доказывают actual non-elevated token, kernel mutex exclusivity/reacquire, unique generation, lock → recovery без auto-resume, shutdown idempotency, protected inheritable DACL, reparse/path rejection, strict CSP и render-only overlay; полный workspace quality gate зелёный.
  - **Критерий готовности:** Выполнен в personal-MVP scope. Реальный hotkey/capture подключается к готовому safety seam только в Step 8.

- [x] **8. Реализовать global toggle hotkey и recoverable audio capture**
  - **Что сделано:** Реализованы configurable Rust-only global PTT, выбор/health input device через CPAL/WASAPI, bounded non-blocking callback channel, worker-side downmix/resampling в mono 16 kHz S16, durable PCM writer/recovery, lost-keyup watchdog, `Esc`, system/device/overflow и 32 MiB safety stops, а также live recording/finalizing/recovery state в UI.
  - **Контекст из архитектуры:** [способность записи](../02-ядро/способности/записать-диктовку.md), [trigger table](../04-потоки/триггеры.md#таблица-триггеров), [audio stack](../05-стек/технологии.md#таблица-технологий), [лимиты](../07-нефункциональные/безопасность.md#лимиты-и-защита-от-злоупотребления)
  - **Evidence:** 20/20 desktop tests, 22/22 storage tests и 3/3 frontend unit tests доказывают PTT edge/debounce, bounded overflow signal, формат/лимит/watchdog, fake 48 kHz stereo → durable 16 kHz mono PCM, explicit recovery без ложного ASR, live-status/cancel policy и lifecycle safety; полный workspace quality gate зелёный.
  - **Критерий готовности:** Выполнен в personal-MVP scope без микрофона, прослушивания, ASR-матрицы или моделей. Runtime profile намеренно инжектируется менеджером Step 9; при его отсутствии capture отклоняется до открытия микрофона.

- [x] **9. Реализовать безопасный model/runtime manager**
  - **Что сделано:** Реализован signed manifest manager с license/size preview, explicit offline import, resumable HTTPS Range download, Ed25519 strict verification, per-file SHA-256, bounded managed materialization без универсального archive extractor, path/reparse checks, disk preflight, ABI/probe health gate, versioned active config и last-known-good rollback.
  - **Контекст из архитектуры:** [model capability](../02-ядро/способности/управлять-моделями.md), [model entities](../03-данные/сущности.md#модели-и-runtime), [supply chain](../07-нефункциональные/безопасность.md#цепочка-поставки-и-обновление), [stack compatibility](../05-стек/технологии.md#совместимость-найденных-локальных-моделей)
  - **Evidence:** 7/7 model-manager и 2/2 catalog integration tests доказывают offline commit, online resume, отменённую и возобновляемую загрузку, освобождение байтов при удалении без потери истории, invalid/revoked/unknown signature, traversal, incompatible ABI, insufficient disk, corrupted artifact, failed probe и downgrade без замены active profile; storage 26/26 unit и полный workspace quality gate зелёные.
  - **Критерий готовности:** Выполнен в personal-MVP scope на локальных byte fixtures без загрузки/запуска моделей и ASR. Trusted public keys инжектируются release/desktop boundary; private signing keys не входят в repository/runtime.

- [x] **10. Реализовать supervised ASR sidecar, durable dispatcher и первый engine**
  - **Что сделано:** Реализованы protocol `0.2.0`, bounded typed NDJSON, strict Whisper profiles, SHA-bound managed paths, supervised `run-whisper` adapter, heartbeat/timeout/cancel, SQLite FIFO CAS lease/reclaim и immutable raw commit. Capture admission атомарно применяет лимиты 20 sessions / 256 MiB / 32 MiB / 1 GiB до CPAL.
  - **Контекст из архитектуры:** [ASR capability](../02-ядро/способности/распознать-речь.md), [AsrAttempt](../03-данные/сущности.md#распознавание-и-очистка), [bounded dispatcher](../04-потоки/триггеры.md#bounded-durable-dispatcher), [ASR stack](../05-стек/технологии.md#asr-gate-что-именно-сравниваем)
  - **Evidence:** 9 protocol tests, 9 sidecar tests (включая process-level positive fixture), 6 dispatcher/admission tests, 23 desktop tests, 22 core storage + 5 model-manager tests и полный workspace quality gate зелёные. Expiry/restart возвращает тот же attempt с новой generation без duplicate; stale completion не проходит; raw UPDATE блокируется trigger.
  - **Критерий готовности:** Выполнен в personal-MVP scope без новой ASR-матрицы, записи, прослушивания или загрузки модели. Step 4 уже доказал реальный selected engine; Step 10 доказывает production orchestration на byte/process fixtures. Owner golden flow остаётся Step 16, clean-install packaging — Step 17.

- [x] **11. Реализовать immutable raw и meaning-preserving cleanup**
  - **Что сделано:** Выполнены BL-008/015: immutable raw остаётся источником истины; pure Rust policy v1 детерминированно применяет punctuation/whitespace, явно изолированные fillers и соседние точные повторы. Cleaned сохраняется отдельной immutable version; policy/hash/glossary зафиксированы; prompt optimization отсутствует.
  - **Контекст из архитектуры:** [cleanup capability](../02-ядро/способности/очистить-текст.md), [transcript entities](../03-данные/сущности.md#распознавание-и-очистка), [характер](../02-ядро/характер.md), [invariants](../03-данные/правила-нерушимые.md#аудио-и-транскрипты)
  - **Evidence:** Versioned RU/EN corpus и 6/6 targeted tests сохраняют отрицания, требования, числа, paths/CLI и API/SQL/Rust/TypeScript tokens; доказывают raw immutability, отдельную cleaned version, deterministic hash/output, error/timeout raw fallback, duplicate suppression, mismatch rejection и content-free disagreement metric. Desktop 23/23, core storage 22/22, dispatcher 6/6, model manager 5/5 и полный workspace quality gate зелёные.
  - **Критерий готовности:** Выполнен в personal-MVP scope без LLM, prompt optimization, микрофона, модели или delivery/insertion. Policy `v1 / 86ae0836c57a6d166de97deba6356283a9285cdb581c114b5f5430cb96ff4d68 / glossary 0`; transcript `logs/quality-20260824-162348.transcript`.

- [x] **12. Реализовать Windows insertion engine и evidence registry**
  - **Что делаем:** Перенести доказанный spike в production module: immutable target snapshot, integrity/focus revalidation, method ladder, clipboard restoration, versioned compatibility rules и безусловный запрет auto-Enter/auto-retry uncertain.
  - **Контекст из архитектуры:** [insertion capability](../02-ядро/способности/вставить-текст.md), [delivery entities](../03-данные/сущности.md#доставка-и-recovery), [evidence contract](../03-данные/правила-нерушимые.md#контракт-доказательства-доставки), [permissions](../02-ядро/права-доступа.md)
  - **Критерий готовности:** Выполнен: production policy воспроизводит принятую Windows 10 BL-016/022 matrix; только `target_ack/certified_transport` получает `delivered`, а focus/UIPI/partial input/clipboard errors и interrupted operation дают durable `uncertain` без автоматического дубля. Registry `v1 / a9bb8488b04d6d9c93f582e29a129d396ff988415af835e4c7037873b0e6db8e` намеренно пуст до отдельной сертификации; Windows 11 fail-closed по ADR-005. Transcript: `logs/quality-20260824-173755.transcript`.

- [x] **13. Реализовать recovery, history и retention без второй истины**
  - **Что делаем:** Выполнить BL-017/029/033 минимального MVP: recovery view из session aggregate, Retry/Copy/Resolve/Pin/Delete, delivered 15 days, unresolved indefinitely, physical-copy deletion journal.
  - **Контекст из архитектуры:** [recovery capability](../02-ядро/способности/восстановить-результат.md), [history capability](../02-ядро/способности/управлять-историей.md), [retention invariants](../03-данные/правила-нерушимые.md#recovery-и-retention), [data inventory](../07-нефункциональные/безопасность.md#активы-и-личные-данные)
  - **Критерий готовности:** Выполнен: restart projection строится только из session aggregate и возвращает все non-terminal sessions с immutable operations/attempts; явный Retry создаёт новый target и user action без replay; Copy/Resolve/Pin/Delete используют expected state version. Delivered получает cutoff +15 дней, sweep сохраняет pinned/unresolved/active, а resumable `maintenance_run` journal удаляет SQLite/WAL/SHM/PCM/`.part`/quarantine без второй копии текста. Transcript: `logs/quality-20260825-232336.transcript`.

- [x] **14. Собрать no-focus overlay, tray, settings и recovery UX**
  - **Что делаем:** Выполнить BL-018/019: ambient overlay states Recording/Processing/Delivered/Uncertain/Error, tray shell, hotkey/mic/model/cleanup/startup/warm-up settings и понятные recovery actions с формальным обращением «вы».
  - **Контекст из архитектуры:** [карта и UX-стиль](../01-обзор/карта-системы.md#ux-стиль), [характер](../02-ядро/характер.md), [config capability](../02-ядро/способности/настроить-приложение.md), [trigger failures](../04-потоки/триггеры.md#таблица-триггеров)
  - **Критерий готовности:** Выполнен: render-only overlay получает только content-free состояния и нативно закреплён как `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST` с полностью снятым caption/system-menu frame и отключённой DWM-тенью, размером, выведенным из DPI целевого монитора ([ADR-007](../06-решения/журнал-решений/2026-08-27-007-overlay-frame-и-single-instance-activation.md)); `Delivered` показывается только после подтверждённого delivery evidence, а `Uncertain` и `Error` остаются отдельными исходами. Tray, versioned settings и русскоязычный recovery/history UI применяют одно явное действие, предупреждают о duplicate/delete risk, поддерживают focus trap/Escape/visible focus и сохраняют last-known-good при ошибке конфигурации. Transcript: `logs/quality-20260826-003734.transcript`.

- [x] **15. Завершить content-free diagnostics и security/offline hardening**
  - **Что делаем:** Выполнить BL-020/027/028/045: structured trace, rolling limits, bundle manifest/preview/redaction, deny-all network audit, IPC fuzz, capability/ACL/supply-chain/retention tests.
  - **Контекст из архитектуры:** [diagnostic capability](../02-ядро/способности/собрать-диагностику.md), [security release gates](../07-нефункциональные/безопасность.md#release-gates-безопасности), [cost limits](../07-нефункциональные/расходы.md#cost-guardrails), [premortem](../07-нефункциональные/риски-архитектуры.md)
  - **Критерий готовности:** Golden flow проходит с заблокированной сетью; marker secrets отсутствуют в logs/bundle; malformed IPC/package/UI requests не выходят из boundary; support trace восстанавливает порядок crash/focus/commit failure без content.

- [ ] **16. Доказать вертикальный golden flow и zero-loss gate**
  - **Что делаем:** Выполнить BL-021 и интегрировать все компоненты в главный сценарий Codex/VS Code; заморозить benchmark-derived latency/quality/resource thresholds и regression baselines.
  - **Контекст из архитектуры:** [press release](../01-обзор/пресс-релиз.md), [system flow](../01-обзор/карта-системы.md#основной-поток), [killer feature](../../idea/04-mvp/one-killer-feature.md), [success metrics](../../idea/04-mvp/success-metrics.md)
  - **Критерий готовности:** Серия 100 завершённых диктовок имеет 0 irrecoverable results; target matrix, cleanup corpus, crash/load/offline tests и measured p50/p95/RTF/resources проходят frozen thresholds.
  - **Статус:** 🔄 In progress. Frozen gate/evaluator и automated 100-session durability slice готовы; `[ ]` сохраняется до реального owner run 50 Codex + 50 VS Code и aggregate `passed=true`.

- [ ] **17. Собрать clean-install NSIS package personal alpha**
  - **Что делаем:** Упаковать per-user x64 installer с shell/sidecar/runtime prerequisites, explicit model install/offline import, uninstall/data choice и диагностируемым preflight. Silent updater и обязательная подпись не блокируют только личную alpha.
  - **Контекст из архитектуры:** [installer/update stack](../05-стек/технологии.md#таблица-технологий), [security supply chain](../07-нефункциональные/безопасность.md#цепочка-поставки-и-обновление), [cost contract](../07-нефункциональные/расходы.md), [clean-install risk](../07-нефункциональные/риски-архитектуры.md#9-чистая-установка-работала-только-на-машине-разработчика)
  - **Критерий готовности:** Clean Windows 10 22H2 и Windows 11 x64 VM устанавливают package без Python/manual DLL search и выполняют первую CPU-диктовку; uninstall/repair/rollback paths протестированы.
  - **Статус:** 🔄 In progress. Часть «explicit model install/offline import» реализована: экран «Модели», Tauri-команды поверх `ModelManager`, подписанный формат каталога и тул подписи ([ADR-011](../06-решения/журнал-решений/2026-08-31-011-подписанный-каталог-моделей.md)). Каталог из четырёх ggml-моделей подписывается и отображается: worker вынесен из model package в поставку приложения ([ADR-012](../06-решения/журнал-решений/2026-08-31-012-worker-в-приложении.md)). Worker с Vulkan воспроизводимо собирается скриптом `scripts/prepare-worker.ps1`: ggml собирает свой shader generator вложенным CMake ExternalProject, и тот падает с `No CMAKE_C_COMPILER could be found`, если в пути сборки есть пробел, поэтому при пробеле в пути репозитория крейт стейджится в директорию без пробелов. Проверено: 2180 `.spv`, `ggml-vulkan.lib`, probe установленной модели на GPU даёт `compatible_ggml`. Worker собирается в pipeline и входит в установщик: `scripts/install-vulkan-sdk.ps1` ставит SDK, запиненный по версии `1.4.357.0`, размеру и SHA-256 из манифеста LunarG с fail-closed проверкой; `build.ps1` собирает worker перед `tauri:build`; в CI добавлен соответствующий шаг. Проверено локально: `WiGigaDict_0.0.1_x64-setup.exe` содержит desktop 13,3 МБ, sidecar 0,57 МБ и worker 56,48 МБ — 73 875 838 байт сжаты в 9 242 815. Подписанный каталог входит в бандл: ключ владельца создан вне репозитория, `catalog.json`+`catalog.sig` объявлены `bundle.resources` и подтверждены внутри установщика; публичная половина инжектируется сборкой через `WIGIGADICT_CATALOG_PUBLIC_KEY`, сгенерированный каталог не версионируется. Установщик содержит desktop 15,71 МБ, worker 56,48 МБ, sidecar 0,57 МБ и каталог — 76 407 832 байта сжаты в 9 962 181. Остаются: clean-install на Windows 10 22H2 и Windows 11 VM с первой CPU-диктовкой и проверка uninstall/repair/rollback. Прогон подготовлен: `scripts/verify-clean-install.ps1` запускается внутри VM без репозитория и автоматизирует снимок среды до установки, тихую установку, состав пакета, первый запуск с soak, CPU-распознавание воркером на синтезированной через SAPI фразе (запись речи не нужна), деинсталляцию с проверкой сохранности данных, repair и rollback, оставляя микрофонную диктовку и вставку ручными; порядок и критерий закрытия — [runbook](../../../tests/clean-install/runbook.md). Связка «синтез SAPI → воркер `cpu-t16` → сверка слов» прогнана вживую: голос Irina, `ggml-base.bin`, узнано 0,89 слов фразы за 6 с inference. Записи каталога приведены в соответствие с моделями: whisper многоязычный, поэтому `languages: ["multi"]`, а не `["ru","en"]`, и отбора по языку в UI больше нет; у `whisper-small-cpu` `min_ram_mb` исправлен с 4096 на 2048 как оценка по соседям каталога. Каталог пересобран тем же ключом, публичная половина не менялась. Сборка на CI получает `WIGIGADICT_CATALOG_PUBLIC_KEY` из секретов репозитория; сам секрет добавляет владелец. Остаток этапа — только прогон на двух чистых VM: [handoff](../../../logs/2026-09-02-m1-step17-clean-install-kit.md).

  - **Актуализация 2026-09-04:** [ADR-014](../06-решения/журнал-решений/2026-09-04-014-публичный-release-каталог-и-agent-install.md) и [ADR-015](../06-решения/журнал-решений/2026-09-04-015-короткий-worker-build-root.md) заменяют устаревшие детали выше: подписанные `catalog.json`/`catalog.sig` версионируются, публичный ключ закреплён в workflow, а worker всегда собирается через Ninja в коротком staging root до компиляции Tauri.

- [ ] **18. Выпустить personal alpha и зафиксировать фактический baseline**
  - **Что делаем:** Использовать WiGigaDict в ежедневной постановке задач AI-агентам, собирать только локальные diagnostics и исправлять blocker regressions. Не добавлять P1 scope до стабильного golden flow.
  - **Контекст из архитектуры:** [press release milestone](../01-обзор/пресс-релиз.md), [character/error UX](../02-ядро/характер.md), [cost measurements](../07-нефункциональные/расходы.md#измерения-обязательные-по-roadmap), [risk verdict](../07-нефункциональные/риски-архитектуры.md#вердикт)
  - **Критерий готовности:** Personal alpha используется владельцем как основной dictation path минимум в Codex/VS Code; найденные P0 дефекты закрыты, architecture/status/backlog отражают реальный код, baseline опубликован локально.

## M2 — Расширение после доказанного MVP

- [ ] **19. Добавить второй ASR adapter без изменения домена**
  - **Что делаем:** Выполнить BL-030: подключить проигравший/второй Whisper или GigaAM profile через тот же adapter/manifest/worker contract, не дублируя queue/recovery/model manager.
  - **Контекст из архитектуры:** [ASR adapter stack](../05-стек/технологии.md#таблица-технологий), [ASR entities](../03-данные/сущности.md#распознавание-и-очистка), [model permissions](../02-ядро/права-доступа.md)
  - **Критерий готовности:** Оба engine проходят один contract suite; profile switch явный и versioned; active/default/rollback routes зарегистрированы, retired path не входит в package.
  - **Уточнение после ADR-011:** несколько ggml Whisper моделей разного размера — это уже не второй engine: они идут через тот же worker и `run-whisper`, а `EngineKind::WhisperGgml` добавлен без смены `PROTOCOL_VERSION`. Этот шаг остаётся про движок на **другом** runtime (GigaAM/NeMo-ONNX, T-one, Vosk), который тянет ONNX Runtime в бандл, отдельный лицензионный аудит и собственный ADR; возврат GigaAM после отказа [ADR-006](../06-решения/журнал-решений/2026-08-23-006-whisper-personal-mvp.md) п.3 — самостоятельное решение. **Решение владельца 1 сентября 2026: не возвращать.** Отказ ADR-006 п.3 был не по точности, а по truncation — терялся конец фразы, что противоречит zero-loss премисе; предлагать такую модель пользователю не будем. Следствие: каталог остаётся whisper-only, локальных русских моделей в нём нет, пока какая-нибудь не пройдёт тот же gate. Проверено при обсуждении: `transcribe-rs 0.3.11` — последняя версия, её ONNX-движки это `canary`, `cohere`, `gigaam`, `moonshine`, `parakeet`, `sense_voice`; модуль `gigaam` декодирует только CTC, поэтому GigaAM v3 E2E-RNN-T не поддержан, а Qwen3-ASR отсутствует полностью. Vosk и T-one требуют собственных рантаймов.

- [ ] **20. Добавить toggle mode и длинную диктовку**
  - **Что делаем:** Выполнить BL-031: явный toggle start/stop, доступный визуальный контроль, chunk/VAD strategy и bounded streaming/storage без always-listening.
  - **Контекст из архитектуры:** [recording capability](../02-ядро/способности/записать-диктовку.md), [ASR duration gate](../05-стек/технологии.md#asr-gate-что-именно-сравниваем), [limits](../07-нефункциональные/безопасность.md#лимиты-и-защита-от-злоупотребления), [boundaries](../01-обзор/границы.md)
  - **Критерий готовности:** Toggle никогда не стартует без явного action, lock/device/crash сохраняет chunks, long-form не truncates после 25 секунд и memory/disk остаются bounded.

- [ ] **21. Реализовать Notetaker v1 для локального файла**
  - **Что делаем:** Только после personal alpha и зелёных R1/R2 создать отдельный `NotetakerJob`, local read-only ingress, FFmpeg probe/decode, durable chunks/checkpoints, shared ASR priority, transcript view и deterministic TXT/SRT/VTT export. Не добавлять editor/diarization/summary.
  - **Контекст из архитектуры:** [режим и компоненты](../01-обзор/карта-системы.md#m2-отдельный-поток-notetaker), [capabilities](../02-ядро/способности/транскрибировать-запись.md), [Notetaker entities](../03-данные/сущности.md#aggregate-notetaker), [invariants](../03-данные/правила-нерушимые.md#notetakerjob-и-long-form), [ADR-001](../06-решения/журнал-решений/2026-08-21-001-notetaker-как-отдельный-режим.md)
  - **Критерий готовности:** Local import создаёт completed/incomplete immutable transcript без сети/изменения source; pause/crash/Dictation resume lossless; 2 GiB/4h/1+2/disk limits и cleanup/export/delete matrices зелёные; first-use notice/copy приняты.

- [ ] **22. Завершить Notetaker v1 опциональным Yandex ingress**
  - **Что делаем:** Добавить выключенную по умолчанию capability только для public single downloadable file: disclosure, официальный API, strict URL/redirect/address validation, dual size cap, durable resumable download и deletion после PCM. ASR остаётся локальным.
  - **Контекст из архитектуры:** [import capability](../02-ядро/способности/импортировать-запись.md), [Yandex security](../07-нефункциональные/безопасность.md#опциональный-yandex-ingress), [network triggers](../04-потоки/триггеры.md#notetaker-m2), [ADR-002](../06-решения/журнал-решений/2026-08-21-002-опциональный-yandex-ingress.md)
  - **Критерий готовности:** No background traffic; capability revoke/expiry/redirect/SSRF/no-download/Range/oversize/low-disk tests зелёные; URL/href/container удаляются после durable PCM; UI всегда показывает network stage и локальный ASR.

- [ ] **23. Добавить verbatim, расширенную history и glossary**
  - **Что делаем:** Выполнить BL-032/033/034: безопасный verbatim toggle, searchable local metadata/history и versioned glossary/app profiles без обучения на прошлых текстах.
  - **Контекст из архитектуры:** [glossary capability](../02-ядро/способности/управлять-словарём-и-профилями.md), [history capability](../02-ядро/способности/управлять-историей.md), [configuration entities](../03-данные/сущности.md#конфигурация-и-персонализация), [permissions](../02-ядро/права-доступа.md)
  - **Критерий готовности:** Verbatim обходит cleanup; glossary changes создают version/preview/rollback; history соблюдает retention и не отправляет/обучает содержимое автоматически.

- [ ] **24. Исследовать и отдельно добавить explicit prompt optimization**
  - **Что делаем:** Выполнить BL-035 только после отдельного product/security/cost decision: явная команда над выбранным transcript, локальная модель, preview и запрет silent intent changes/tools.
  - **Контекст из архитектуры:** [cleanup boundary](../02-ядро/способности/очистить-текст.md), [permissions](../02-ядро/права-доступа.md), [dangerous-triad reassessment](../07-нефункциональные/безопасность.md#проверка-опасной-тройки), [model costs](../07-нефункциональные/расходы.md#распределение-локальных-моделей)
  - **Критерий готовности:** Новый threat model и regression corpus приняты; функция выключена по умолчанию, показывает исходный/оптимизированный diff и никогда не выполняет внешнее действие.

## M3 — Публичный Windows launch

- [ ] **25. Завершить самостоятельный бренд и UX-polish**
  - **Что делаем:** Выполнить BL-001/040: проверить работающий Wispr Flow и другие референсы, затем реализовать собственные бренд, тексты, визуальные assets, onboarding и микровзаимодействия без копирования чужих материалов.
  - **Контекст из архитектуры:** [UX character](../02-ядро/характер.md), [system UX style](../01-обзор/карта-системы.md#ux-стиль), [press release](../01-обзор/пресс-релиз.md), [boundaries](../01-обзор/границы.md)
  - **Критерий готовности:** UX benchmark против референсов пройден; все states/errors/settings/onboarding имеют собственные assets/copy и не ухудшают no-focus/keyboard flow.
  - **Статус:** 🔄 Первый фирменный проход выполнен досрочно и принят в [ADR-013](../06-решения/журнал-решений/2026-09-03-013-фирменный-стиль-и-язык-взаимодействия.md): главный UI переведён на самостоятельную палитру, локальные OFL-шрифты и fluid interaction tokens. Шаг остаётся открытым до UX-разбора работающих референсов, собственного onboarding/assets/copy и финальной физической приёмки overlay transition из `design-qa.md`.

- [ ] **26. Поднять публичную supply chain, distribution и support readiness**
  - **Что делаем:** Получить Windows code signing, настроить signed Tauri updates и российский artifact/model mirror, provenance/SBOM/key rotation/rollback, support channel/privacy/license notices и актуальную cost/legal модель.
  - **Контекст из архитектуры:** [security supply chain](../07-нефункциональные/безопасность.md#цепочка-поставки-и-обновление), [public costs](../07-нефункциональные/расходы.md#стоимость-распространения-публичного-продукта), [public product constraints](../05-стек/технологии.md#российский-публичный-продукт), [press release questions](../01-обзор/пресс-релиз.md#внешние-вопросы)
  - **Критерий готовности:** Подписанный app/model manifest проходит install/update/rollback/downgrade/revocation tests; support/privacy/license channels опубликованы; provider estimate для 100/1 000/10 000 MAU утверждён.

- [ ] **27. Пройти public release matrix и выпустить Windows 1.0**
  - **Что делаем:** Проверить Windows 10/11, поддерживаемые IDE/terminal/browser controls, CPU/GPU profiles, чистую установку/обновление/удаление, offline mode, accessibility baseline и полный regression/security suite.
  - **Контекст из архитектуры:** [release gates](../07-нефункциональные/безопасность.md#release-gates-безопасности), [triggers](../04-потоки/триггеры.md), [premortem](../07-нефункциональные/риски-архитектуры.md), [launch contract](../01-обзор/пресс-релиз.md)
  - **Критерий готовности:** Все declared support cells зелёные либо честно исключены с recovery; signed Windows 1.0 опубликован не позднее целевой даты 31 октября 2026 года, но дата не отменяет blocker gates.

## M4 — После стабильного Windows launch

- [ ] **28. Отдельно спроектировать B2B, accessibility и macOS expansion**
  - **Что делаем:** Выполнить BL-041/042/043/044: исследовать широких knowledge workers, российские организации, полноценную accessibility и стоимость/архитектуру macOS. Не переносить personal owner permissions в tenant roles.
  - **Контекст из архитектуры:** [B2B security gate](../07-нефункциональные/безопасность.md#b2b-gate--не-часть-personal-mvp), [boundaries](../01-обзор/границы.md), [cost guardrails](../07-нефункциональные/расходы.md#cost-guardrails), [system map](../01-обзор/карта-системы.md)
  - **Критерий готовности:** Для каждого направления есть отдельный evidence-backed go/no-go, threat model, economics и ADR; Windows personal core остаётся local/offline и не получает обязательный control plane.
