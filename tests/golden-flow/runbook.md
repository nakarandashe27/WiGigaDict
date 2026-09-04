# M1 Step 16 golden-flow и zero-loss runbook

## Статус

Этот runbook готовит и проверяет evidence, но сам по себе не закрывает Step 16. Этап закрывается только после реальных 100 завершённых диктовок владельца на Windows 10 build 19045 с 0 irrecoverable results и зелёным итоговым report.

Автоматический тест `golden_flow_tests::one_hundred_completed_sessions_have_zero_irrecoverable_results` использует production storage/cleanup/insertion contracts и fake target evidence. Он доказывает durability и fail-closed policy, но не сертифицирует Codex или VS Code.

## Frozen gate v1

Источник истины: `tests/golden-flow/thresholds-v1.json`. Не менять пороги после начала серии. Провал означает исследование причины и новый versioned gate/ADR, а не ослабление v1 задним числом.

- ровно 100 sessions: 50 Codex и 50 VS Code;
- минимум 10 short, 30 medium и 50 long samples; оставшиеся 10 распределяются заранее до старта;
- 0 irrecoverable results, 0 intent changes, 0 corrupt audio;
- минимум 51 результат «готов после быстрой проверки» — буквальная строгая реализация обещания «в большинстве случаев»;
- release-to-terminal p50 ≤1500 ms, p95 ≤2500 ms;
- ASR inference p95 ≤1163 ms, RTF p95 ≤0.0341;
- peak RAM ≤727843636 bytes, peak incremental VRAM ≤1019635303 bytes;
- offline deny-all, crash/restart, load admission, cleanup corpus и marker redaction gates обязательны.

ASR ceilings равны boundary-safe owner take-04 cold p95 с 10% regression budget: 1057 ms → 1163 ms и 0.030933 RTF → 0.0341. RAM — 661676032 bytes +10%. VRAM — measured 926941184 bytes +10%. End-to-end p95 добавляет к ASR ceiling 250 ms cleanup hard limit и примерно 1 s bounded supervision/delivery budget; p50 1500 ms фиксирует интерактивную цель. Эти два end-to-end числа являются Step 16 engineering budget и впервые проверяются реальной серией, а не переименовываются в уже измеренный baseline.

## Что записывает evidence

Только технические поля: ordinal, machine session id, target family, duration class, terminal outcome/evidence, recoverability flags, manual quick-review/intent-change flags, timings и resource counters. Запрещены transcript, audio, clipboard, window title, полный path, имя пользователя и environment.

`delivered` допустим только при `target_ack` или существующем `certified_transport`. Текущий built-in registry пуст. Полный transport без сильного evidence записывается как `uncertain`, даже если текст визуально появился; это не zero-loss failure, когда audio/text доступен в recovery, но такой прогон не создаёт compatibility rule автоматически.

## Preflight

1. Работать из обычной non-elevated Windows 10 build 19045 session.
2. Убедиться, что выбран exact `whisper-large-v3-turbo-q5-vulkan`, модель/runtime healthy, microphone выбран явно, сеть отключена или заблокирована.
3. Запустить полный quality gate и сохранить transcript.
4. Проверить frozen threshold fixture:

```powershell
pwsh -NoProfile -File .\scripts\golden-flow.ps1 -CheckThresholdsOnly
```

5. Создать новый ignored evidence-файл вне versioned fixtures, например `artifacts/golden-flow/run-01.json`, на основе `evidence-template.incomplete.json`. Не перезаписывать предыдущий run.
6. До первого sample зафиксировать порядок 100 строк: 50/50 targets и duration coverage. Не заменять неудачные строки удачными; повтор после failure получает следующий ordinal.

## Выполнение одной строки

1. Сфокусировать заранее объявленный target и удержать PTT.
2. Произнести заранее выбранную owner-corpus задачу, отпустить PTT и дождаться terminal overlay state.
3. Убедиться, что session появилась в History/Recovery.
4. Если outcome `uncertain`/`failed`, открыть recovery и доказать доступность audio или selected text; не запускать automatic retry.
5. Записать content-free session fields. `release_to_terminal_ms` измеряется от durable finalize/release marker до terminal `delivered/uncertain/failed`; `inference_ms` берётся из immutable ASR metrics, а RTF вычисляет validator.
6. Для quick-review отметить `true` только если текст требует проверки, но не существенного переписывания. Любое изменение требования/отрицания/technical token намерения — `intent_changed=true` и немедленный gate failure.

## Target matrix

- Codex: 50 sessions в текущем non-elevated desktop target.
- VS Code: 50 sessions в обычном editor/input target.
- Focus change, destroyed target и elevated/UIPI входят в отдельные negative rows/tests и никогда не получают `delivered` без evidence.
- Windows 11 не входит в v1 и остаётся BL-047.

## Обязательные fault/regression gates

Перед финальной оценкой должны быть зелёными:

```powershell
cargo test -p wigigadict-desktop --lib one_hundred_completed_sessions_have_zero_irrecoverable_results --locked --offline
cargo test -p wigigadict-storage --test recovery --test cleanup --test delivery --test diagnostics --locked --offline
pwsh -NoProfile -File .\scripts\offline-audit.ps1
```

В evidence booleans становятся `true` только после соответствующего завершённого gate текущего code state. Они не являются заменой transcript/report: handoff связывает run с canonical quality transcript.

## Оценка

Validator читает максимум 4 MiB, отклоняет неизвестные поля, duplicate/out-of-range ordinal, duplicate/invalid session id, false-delivered, неверный OS/runtime/gate id и неполную матрицу. Report агрегирован и не содержит пользовательского content.

```powershell
pwsh -NoProfile -File .\scripts\golden-flow.ps1 `
  -Evidence .\artifacts\golden-flow\run-01.json `
  -Output .\artifacts\golden-flow\report-01.json
```

Output обязан быть новым `.json`; запись идёт через `.part → final`. Exit code 2 означает валидный, но проваленный gate. Не удалять failed evidence: исправление получает новый run id.

## Условие закрытия Step 16

- итоговый report имеет `passed=true` и 100 sessions;
- target matrix, duration coverage, quality/performance/resources и все regression gates зелёные;
- raw evidence остаётся локальным ignored artifact, а versioned handoff содержит только агрегаты, hashes и ограничения;
- BL-021 и roadmap Step 16 закрываются только после проверки report и canonical full quality transcript.
