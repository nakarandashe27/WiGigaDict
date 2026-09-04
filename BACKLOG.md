# Backlog

Статус: inventory задач, синхронизированный с research и architecture. Порядок выполнения и release gates определяет [architecture roadmap](context/architecture/08-дорожная-карта/roadmap.md); backlog сохраняет стабильные `BL-*` identifiers.

## P0 — Исследование и решения до реализации

- [ ] **BL-001:** Завершить UX-разбор Wispr Flow: публичный сайт и демонстрации изучены; отдельно проверить onboarding, оверлей, состояния, ошибки, настройки и микровзаимодействия в работающем приложении.
- [x] **BL-002:** Исследовать Voicy и Pipit как дополнительные продуктовые референсы.
- [x] **BL-003:** Провести аудит `dimastatz/whisper-flow`: лицензия, стек, архитектура, активность, Windows-интеграция, ввод текста, модельный слой, тесты и стоимость доработки.
  - Вердикт: полезный MIT streaming/benchmark backend или референс; не desktop foundation.
- [x] **BL-004:** Инвентаризировать локальное железо и существующие установки/модели Whisper и GigaAM.
  - Железо зафиксировано: Windows 10 Pro 19045; Ryzen 9 8940HX; 31.2 GiB RAM; RTX 5070 Laptop GPU с 8151 MiB VRAM.
  - Найдены кэши: `ai-sage/GigaAM-v3` с локальными весами около 449 МБ и `unslothai/whisper-large-v3-turbo-GGUF` около 1.62 ГБ.
  - Готовый `whisper` CLI и ASR-пакеты в проверенных базовых Python-окружениях не найдены; состояние runtime вынесено в отдельную задачу.
- [x] **BL-005:** Собрать personal-MVP набор русских технических диктовок владельца с английскими терминами и самоисправлениями.
  - `technical-token error rate` измерен для API, команд, путей, имён и англоязычных терминов; корпус не обобщается на другие голоса или публичную выборку.
- [x] **BL-006:** Сравнить Whisper и GigaAM по надёжности, точности, задержке и нагрузке; выбрать первый движок MVP.
  - Выбран Whisper large-v3-turbo Q5 Vulkan; тот же worker с `cpu-t16` — медленный явный recovery fallback (ADR-006).
  - Проверены 5/15/25/30/60 секунд, cold/warm и crash/restart; GigaAM не прошёл reliability/truncation gate.
- [x] **BL-007:** Выбрать Windows-стек приложения после исследования готовых решений и официальной документации.
  - Решение Stage 9: Tauri 2 + Rust shell, React/TypeScript UI, SQLite/rusqlite и отдельный управляемый Rust ASR worker; один engine выбирается benchmark.
- [x] **BL-008:** Определить безопасный локальный механизм очистки речи без изменения намерения пользователя.
  - Собрать raw/cleaned regression corpus и измерять cleanup disagreement rate (risk R-06).
- [x] **BL-009:** Выбрать и benchmark-воспроизводимо настроить runtime-цепочки Whisper/GigaAM для personal MVP.
  - Exact-pinned `transcribe-rs` 0.3.11 проверен; принят Whisper Q5 CPU/Vulkan path, GigaAM ONNX path отклонён текущим gate (ADR-006).
  - Найденный Whisper GGUF и исходный GigaAM PyTorch cache не объявлены runtime-ready и не изменялись.
  - Clean install и golden flow без Python/manual DLL search не отменены: это критерии Step 17 и Step 16 соответственно.
- [ ] **BL-023:** Провести глубокий fork/security/license-аудит AudioBud/Handy: lineage, зависимости, сетевые вызовы, updater, installer, model manager, insertion paths и воспроизводимость сборки.
- [ ] **BL-024:** Провести сравнительный аудит OpenWhispr как shortcut-форка: Electron footprint, scope удаления, лицензии, локальные runtime, updater и Windows insertion.
- [x] **BL-025:** Собрать decision matrix `узкий Tauri shell / AudioBud fork / OpenWhispr fork / Python prototype` и принять ADR до начала реализации.
  - Выбран узкий Tauri/Rust shell; AudioBud/Dictum — источники аудируемых модулей и тестовых идей, не foundation fork.
- [x] **BL-026:** Провести Win32 proof-of-concept no-focus overlay и многоступенчатой вставки на Windows 10.
  - Standard controls, реальное Tauri/WRY окно и browser fixture зелёные. VS Code завершился `transport_only → uncertain`; Windows Terminal / Claude Code 2.1.239 показал повреждённые glyphs вместо точного маркера и также завершён `uncertain`. Ни один неоднозначный путь не получил retry, fallback или compatibility rule. Windows 11 вынесена в BL-047 и не заявлена совместимой.
- [x] **BL-027:** Добавить offline-аудит, доказывающий отсутствие скрытых сетевых вызовов в основном цикле диктовки.
  - Step 15: deny-all harness запускает diagnostic/recovery flow с отключённым Cargo network и dead HTTP(S) proxy; статический boundary-аудит допускает сетевой клиент только в явном model manager.
- [x] **BL-028:** Реализовать preview и redaction для экспортируемого diagnostic bundle; исключить аудио и текст по умолчанию.
  - Step 15: bundle строится только из typed allowlist events, показывает manifest/count/size/SHA-256 preview и требует exact confirmation; audio/transcript/clipboard/window title/path/environment/secrets исключены схемой.
- [x] **BL-029:** Реализовать согласованную retention-политику для recovery/history и безопасное удаление локальных данных.
  - Step 13: confirmed delivered получает cutoff +15 дней; pinned и unresolved не получают auto-expiry; sweep блокируется active job. Удаление журналируется в `maintenance_run`, идемпотентно удаляет PCM/`.part`/quarantine, затем cascade SQLite с secure-delete/WAL checkpoint.
- [x] **BL-036:** Подготовить воспроизводимый Windows build bootstrap для выбранного стека.
  - Rust `1.97.1`, MSVC v143, Windows SDK 22621, Node/npm, Vulkan SDK и lockfiles зафиксированы; локальная release-сборка, worker и NSIS package воспроизводятся через `scripts/build.ps1`.
  - Проверка готового package на чистых Windows VM остаётся критерием M1 Step 17, а не незакрытой частью bootstrap.
- [x] **BL-037:** Доказать стабильный Tauri 2 no-focus overlay без fork runtime на Windows 10.
  - Primary: Tauri window + узкий `windows-rs` style shim; patched runtime допускается только после документированного failed gate.
  - ADR-004 принял узкий shim без fork; реальное Tauri/WRY WebView2 окно прошло 100 циклов с обязательными styles, без target mismatch и focus steal. Windows 11 regression вынесен в BL-047.
- [x] **BL-038:** Спроектировать и протестировать versioned NDJSON IPC и supervision для `wigigadict-asr.exe`.
  - Step 10: protocol `0.2.0`, bounded typed NDJSON, handshake, heartbeat/timeout/cancel, process supervision и crash recovery реализованы; UI не получает shell permission.
- [x] **BL-045:** Пройти security release gate personal alpha.
  - Проверить per-user ACL, managed path/reparse-point containment, offline deny-all, Tauri capability/CSP isolation, IPC fuzz/size limits, signature/downgrade rejection, log redaction, retention physical copies и Win32 privilege/evidence matrix.
  - Threat model MVP не обещает защиту от администратора, SYSTEM или malware текущего Windows-пользователя; прикладного шифрования SQLite/PCM нет.
  - Step 15: release-gate suites объединяют накопленные ACL/path, capability/CSP, IPC, model-signature, retention и Win32 evidence tests с новым offline/redaction/diagnostic-bundle audit; supply-chain policy и SBOM прошли без ослабления проверок.
- [ ] **BL-046:** Измерить реальную локальную стоимость и ресурсный профиль.
  - Step 4 зафиксировал personal-MVP RTF, RAM/VRAM и model/runtime benchmark evidence, достаточные для выбора engine.
  - Strict incremental watts/kWh, disk write/steady-state и packaged installer/runtime/model bytes проверяются на Step 16/17; до public beta также нужны актуальные предложения signing и российского artifact hosting.

## P0 — Вертикальный срез MVP

- [x] **BL-010:** Реализовать жизненный цикл фонового Windows-приложения.
  - Windows lock/session switch запрещает новые recordings/insertion, безопасно финализирует active capture и никогда не возобновляет микрофон/delivery автоматически после unlock.
- [x] **BL-011:** Реализовать настраиваемый global toggle hotkey.
  - Steps 7–8: hotkey работает через shell lifecycle и versioned settings с возвратом к last-known-good при ошибке применения.
- [x] **BL-012:** Реализовать захват аудио и выбор микрофона.
  - Steps 8/14: recoverable WASAPI capture, durable PCM commit, admission limits и выбор input device в настройках реализованы.
- [x] **BL-013:** Спроектировать интерфейс адаптера транскрибации.
  - Step 10: единый supervised adapter/worker contract отделён от очереди, хранения и UI.
- [x] **BL-014:** Подключить выбранный по бенчмарку первый движок.
  - Step 10: Whisper ggml подключён через `run-whisper`; основной профиль — large-v3-turbo Q5 Vulkan, CPU используется как явный fallback.
- [x] **BL-015:** Реализовать meaning-preserving очистку, пунктуацию и абзацы.
- [x] **BL-016:** Реализовать надёжную вставку в активное Windows-поле.
  - Release blocker: `delivered` допустим только для `target_ack` или versioned `certified_transport`; один return count `SendInput`/успех clipboard остаётся `transport_only` и ведёт в recoverable `uncertain`.
  - Step 12: production engine, immutable target/transcript/attempt ledger и no-replay crash handling реализованы; registry v1 намеренно пуст, неизвестный полный transport остаётся recoverable `uncertain`.
- [x] **BL-017:** Реализовать recovery buffer при ошибке вставки или потере фокуса.
  - Step 13: recovery view читает исходные session/transcript/delivery rows, показывает immutable attempts и не создаёт `RecoveryCopy`; explicit Retry захватывает новый target и предупреждает о duplicate risk, автоматический replay запрещён.
- [x] **BL-018:** Реализовать компактный оверлей состояний записи, обработки, успеха и ошибки.
  - Step 14: render-only overlay в стабильном viewport 184×44 не принимает focus, следует за foreground monitor и различает Recording/Processing/Delivered/Uncertain/Error без текста диктовки; success разрешён только delivery evidence.
- [x] **BL-019:** Реализовать минимальные настройки горячей клавиши, микрофона, модели и диагностики.
  - Step 14: immutable configuration snapshots с optimistic version и last-known-good управляют hotkey, input device, runtime, cleanup, user-level startup, warm-up opt-in и diagnostic opt-in; невалидное применение откатывает live shell state.
- [x] **BL-020:** Реализовать структурированные локальные логи без утечки содержимого диктовки по умолчанию.
  - Step 15: versioned NDJSON trace использует закрытые enums и bounded metadata, monotonic sequence, crash-tail recovery, rolling 30 дней/100 MiB/25 файлов и детерминированный content-free export.
- [ ] **BL-021:** Проверить золотую сцену end-to-end в Codex.
  - Step 16 in progress: frozen v1 thresholds, typed evidence validator и automated 100-session production-contract slice готовы; BL остаётся открытым до реальных 50 Codex + 50 VS Code диктовок и зелёного aggregate report.
- [x] **BL-022:** Проверить вставку в VS Code, Claude Code/терминал, браузерные поля и стандартные Windows-контролы.
  - Blocker gate R-02: versioned compatibility matrix включает focus change, destroyed HWND, elevated/UIPI, partial input, clipboard lock/restore failure; ни один исход без достаточного evidence не получает UI-success.
  - Windows 10 matrix и production policy regression зелёные; VS Code/terminal ambiguity остаётся `uncertain`, browser не получил broad rule. Windows 11 вынесена в BL-047 и fail-closed до отдельного evidence.
- [x] **BL-039:** Реализовать bounded durable ASR dispatcher и admission control.
  - Alpha defaults: 20 pending sessions, 256 MiB pending/reserved PCM, 32 MiB/session, 1 GiB свободного диска после reservation, один ASR lease.
  - Step 10: 21-я session или byte/disk overflow отклоняется до capture; restart сохраняет FIFO, expired lease возвращается без duplicate, backlog не загружается целиком в RAM.

## P1 — Сразу после доказательства MVP

- [ ] **BL-047:** Выполнить Windows 11 Win32 golden matrix до заявления совместимости или публичного release с Windows 11 support.
  - Повторить standard controls, реальное Tauri/WRY окно, VS Code/Codex, terminal/Claude Code и browser по `tests/win32-spike/windows-11-runbook.md`; до зелёного evidence Windows 11 не включать в supported OS matrix.
- [ ] **BL-030:** Добавить второй адаптер Whisper/GigaAM.
- [ ] **BL-031:** Добавить toggle-режим для длинной диктовки и доступности.
- [ ] **BL-032:** Добавить безопасный дословный режим.
- [x] **BL-033:** Добавить минимальную локальную историю/восстановление предыдущих результатов.
  - Step 13: main history/recovery projection показывает `pending / delivered / uncertain / copied / resolved`, raw/cleaned и immutable delivery evidence; Retry/Copy/Resolve/Pin/Delete — только явные version-checked действия.
- [ ] **BL-034:** Добавить пользовательский словарь технических терминов.
- [ ] **BL-035:** Добавить явную команду оптимизации текста в структурированный промпт.

## P2 — Продуктовое расширение

- [ ] **BL-040:** Подготовить самостоятельную визуальную систему и бренд при сохранении подтверждённых UX-паттернов.
- [ ] **BL-041:** Исследовать сценарии широких Windows knowledge workers.
- [ ] **BL-042:** Исследовать требования российских организаций к локальному развёртыванию и безопасности.
- [ ] **BL-043:** Провести отдельный accessibility-аудит.
- [ ] **BL-044:** Оценить macOS-порт после стабилизации Windows-архитектуры.
- [ ] **BL-048:** Оценить опциональное распознавание через ASR API в российском контуре (например, neuraldeep.ru с OpenAI-совместимым `/v1/audio/transcriptions`, Yandex SpeechKit) для пользователей, чей компьютер не тянет локальную модель или у кого нет места под веса.
  - Мотивация частично закрывается каталогом моделей: `ggml-base.bin` весит 141 МБ и работает на любом CPU. Задача остаётся только для случаев, где и этого мало.
  - Возврат «третьей ноги» опасной тройки: у приложения появляется возможность отправить аудио наружу. `безопасность.md` требует пересобрать threat model **до** реализации, а не после.
  - **Отложено до personal alpha** (решение владельца, 1 сентября 2026): каталог локальных моделей закрыл основную часть мотивации, а Step 16 и Step 17 ещё открыты. К задаче не возвращаться раньше.
  - Условия входа, каждое обязательно: новая threat model; ADR, отменяющий строку «cloud ASR/cleanup, аккаунты, billing» в [границах](context/architecture/01-обзор/границы.md); хранение ключа только через DPAPI; обновлённый [расходы.md](context/architecture/07-нефункциональные/расходы.md) с ненулевой предельной стоимостью; письменный ответ провайдера о хранении и логировании присланного аудио.
  - Решения владельца, принятые заранее (1 сентября 2026), чтобы к ним не возвращаться:
    - сетевой клиент живёт в shell, рядом с существующим клиентом загрузки моделей; сеть в ASR sidecar не добавляется, его изоляция остаётся нетронутой;
    - список провайдеров закрыт и задан в коде; поля «произвольный URL» нет — оно превратило бы приложение в прокси неизвестно куда и обнулило бы обещание про российский контур;
    - облачный профиль может быть активным, но переключение остаётся тем же явным действием, что и смена локальной модели, и текущая облачная модель всегда видна на экране;
    - в v1 нет счётчиков расхода и лимитов; отказ провайдера, включая исчерпание баланса, обрабатывается как обычная сетевая ошибка и даёт `uncertain` с сохранённым аудио.
    - порог приёмки ответа провайдера о хранении аудио, объявленный **до** письма провайдеру: принимается «аудио не сохраняется» либо «хранится не дольше 48 часов строго для защиты от злоупотреблений и отладки, без доступа людей и без использования в обучении». Отказ дают любое использование в обучении моделей, хранение без названного срока и ручной просмотр записей сотрудниками. Обоснование: владелец диктует рабочий материал — имена проектов, куски кода, внутренние термины, — поэтому 48 часов терпимы, а месяц или обучение нет.
    - ответ принимается только письменный, не в чате поддержки, и сохраняется с датой и дословной формулировкой: условия провайдеров меняются.
  - Независимо от ответа провайдера приложение обязано прямо показывать, что аудио уходит наружу и к кому. Политика провайдера не отменяет раскрытия.
  - Открытых решений не осталось: дальше только работа — threat model, ADR по границам, DPAPI-хранилище ключа, HTTP-клиент и письмо провайдеру.
  - Архитектурная развилка: ASR sidecar по проекту не имеет сети, поэтому облачный путь не может пройти по существующему тракту распознавания. Решение, где живёт сетевой клиент, принимается в той же threat model.
  - Инварианты, которые не ослабляются: выключено по умолчанию и включается явным действием владельца; тихого fallback ни в облако, ни обратно в локальную модель нет; отказ сети посреди диктовки даёт `uncertain` с сохранённым аудио, а не потерю; ограничение «обязательного облака и подписки нет» остаётся в силе — это опция, а не режим по умолчанию.
  - Realtime-модели Яндекса (`Speech Realtime`) — голосовые агенты поверх WebSocket, а не транскрипция файла; под push-to-talk диктовку профильное — SpeechKit STT.

- [ ] **BL-049:** Публиковать сборки через GitHub: лендинг на GitHub Pages, установщик — asset релиза (направление владельца, 2 сентября 2026). Ориентир по подаче — [handy.computer](https://handy.computer/): одна страница, кнопка загрузки, без регистрации.
  - Относится к M3 (Step 25 бренд/UX-polish, Step 26 публичная supply chain и distribution). До закрытия Step 17 и 18 не начинать: публиковать нечего, пока установщик не проверен на чистой машине.
  - Полезное следствие уже сейчас: адрес релиза закрывает открытый вопрос по кнопке «Обновить каталог» — `catalog.json` и `catalog.sig` становятся assets того же релиза, и приложение тянет их по стабильному URL, проверяя подпись вшитым ключом.
  - Главный нерешённый вопрос — **подпись установщика**. Разобран отдельно 2 сентября 2026: [подпись и SmartScreen](context/architecture/05-стек/исследования/2026-09-02-authenticode-и-smartscreen.md). Коротко: покупка сертификата предупреждение **не убирает** — EV перестал обходить SmartScreen в 2024 году, это написано в документации Microsoft; любой сертификат лишь позволяет репутации копиться между релизами. Ноль предупреждений даёт только Store с MSIX. Azure Artifact Signing частным лицам доступен лишь в США и Канаде; выдача OV российским заявителям ограничена западными CA — правовой вопрос юрисдикции владельца. Остаются два реальных пути, и оба требуют решения: открыть исходники и подписываться через SignPath Foundation либо сделать спайк по MSIX и Store, проверив, переживут ли контейнер глобальный хоткей, микрофон и вставка в чужие окна. Для личной alpha ничего не меняем: [Step 17](context/architecture/08-дорожная-карта/roadmap.md) прямо разрешает неподписанный пакет.
  - Хранилище весов остаётся отдельным вопросом: модели по 150–550 МБ в git не кладутся, каталог уже указывает на HuggingFace, и это не меняется.
  - Требует решения владельца перед началом: публичный репозиторий или приватный с публичными релизами; домен или `*.github.io`; язык лендинга.

## Принятые ограничения

- Personal-first, Windows-first.
- Local-first; обязательного облака и подписки нет.
- Один движок блокирует MVP, второй — нет.
- Надёжность важнее точности, точность важнее задержки, задержка важнее экономности ресурсов.
- Prompt optimization не выполняется скрыто.
- Бренд, тексты и визуальные активы Wispr Flow не копируются.
