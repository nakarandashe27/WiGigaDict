# ADR-006: Whisper large-v3-turbo Q5 для personal MVP

- **Дата:** 2026-08-23
- **Статус:** принято для personal alpha
- **Заменяет:** —

## Контекст

M0 / Step 4 сравнил локальные Whisper и GigaAM runtime-пути на синтетическом и приватном RU/EN technical corpus, CPU/GPU, cold/warm и crash/restart сценариях. Отдельный preregistered boundary experiment подтвердил, что выбранный evidence take не содержит устойчивой speech-like активности в последней секунде: frozen WebRTC VAD + RMS classifier дал 0 FP / 0 FN на held-out controls, а `speaker-b/take-02` дал 0/10 primary detections. Пороги после human takes не менялись.

Это personal MVP одного владельца. Другого доступного диктора нет, поэтому Step 4 не должен блокировать рабочий вертикальный срез требованием статистической генерализации на другие голоса. Одновременно текущий человеческий корпус показывает высокую ошибку и слабое прохождение scripted final markers; эти результаты нельзя превращать в обещание общего качества.

## Решение

1. Первым ASR candidate для personal alpha выбран exact-pinned Whisper large-v3-turbo Q5 (`ggml-large-v3-turbo-q5_0.bin`) через проверенный Vulkan-capable Rust worker, профиль `gpu` на NVIDIA Vulkan.
2. CPU fallback — тот же model/worker с явным профилем `cpu-t16`. Он функционален и пригоден для recovery/диагностики, но его p95 RTF выше real-time; скрытое переключение внутри session запрещено.
3. GigaAM CTC не принимается первым engine: текущая цепочка не прошла reliability/truncation gate. Whisper turbo Q8 и non-turbo large-v3 Q5 не улучшили заранее объявленный набор quality gates и не заменяют Q5 turbo.
4. Выбор ограничен голосом владельца, текущим Windows 10 development host и personal-alpha задачей. Он не доказывает качество на других дикторах, Windows 11 или публичной пользовательской выборке.
5. Новая запись и второй диктор не требуются для закрытия Step 4. Реальная пригодность подтверждается позже вертикальным golden flow владельца на Step 16; найденная регрессия возвращает выбор в ADR, а не ослабляет benchmark classifier.
6. Clean-install runtime/model packaging, первая CPU-диктовка на чистой VM и installer/runtime bytes остаются обязательным Step 17. Это не считается выполненным данным ADR.
7. Private WAV, raw NDJSON, diagnostic manifests, models и build artifacts не становятся versioned evidence; решения ссылаются только на санитизированные отчёты и хэши.

## Evidence и ограничения

- Boundary classifier: calibration и held-out controls — 0 FP / 0 FN; `speaker-b/take-02` — 0/10 primary speech detections; take-03 остаётся diagnostic.
- Boundary-safe owner take-04: Vulkan p50/p95 inference 415/1 057 ms cold и 265/823 ms warm; CPU p50/p95 12 849/56 782 ms cold и 17 714/54 117 ms warm.
- На этом scripted take mean WER остаётся примерно 0.52–0.58, а final marker найден только в 2/20 строках каждого backend. Это известное ограничение personal-alpha candidate, а не доказательство production-grade общей точности.
- Q8 и non-turbo large-v3 preflight были остановлены по заранее объявленным gates; дополнительный подбор модели/порогов по тому же human take не требуется.
- Строгие watts/kWh, disk steady-state и packaged installer/runtime bytes остаются в BL-046 и проверяются вместе с Step 16/17; они не блокируют начало доменной реализации.

Основные санитизированные источники: `tests/asr-benchmark/reports/2026-08-22-local-smoke.md`, `tests/asr-benchmark/reports/2026-08-23-boundary-classifier.md`, `tests/asr-benchmark/reports/2026-08-23-boundary-classifier.json`, `logs/2026-08-23-step4-independent-review.md`, `logs/2026-08-23-step4-followup.md`.

## Последствия

M0 research gate закрыт на достаточном для personal MVP evidence и работа переходит к M1 / Step 5. ASR production adapter не добавляется сейчас: его реализация остаётся Step 9. Любые будущие публичные claims требуют отдельной multi-speaker/OS/clean-install evidence, но отсутствие другого голоса не блокирует владельца.

## Затронутые документы

- `context/architecture/05-стек/технологии.md`
- `context/architecture/08-дорожная-карта/roadmap.md`
- `BACKLOG.md`
- `tests/asr-benchmark/runbook.md`