# Что переиспользовать

## Сейчас — критический путь

### 1. Windows shell из AudioBud и Dictum

Проверить и выборочно перенести tray lifecycle, global shortcut, компактный HUD, историю, clipboard/Unicode insertion, diagnostics и границу Rust shell/UI.

AudioBud — самый полный Windows-first кандидат с model manager, overlay, insertion и diagnostics, но молодость и 97 issues требуют аудита до форка. [AudioBud](https://github.com/jamditis/audiobud) (проверено 2026-08-20). Dictum предлагает более узкий Tauri 2/Rust образец с Credential Manager, SQLite history и Unicode fallback. [Dictum](https://github.com/Vkandil/dictum) (проверено 2026-08-20).

**Ранг: 1.** Это быстрее всего сокращает Windows-specific риск.

### 2. Многоступенчатую вставку и recovery

Переиспользовать паттерн: capture target на старте, Unicode/`SendInput`/clipboard fallback, сохранение прежнего clipboard, локальная история и повторная вставка одним действием.

OpenVerba заявляет сохранение и восстановление clipboard; local-wisprflow-windows уже содержит Win32 `SendInput`/clipboard fallbacks, хотя остаётся непроверенным новым проектом. [OpenVerba](https://openverba.com/), [local-wisprflow-windows](https://github.com/darian-gajgic/local-wisprflow-windows) (проверено 2026-08-20).

**Ранг: 2.** Надёжность вставки важнее ещё нескольких процентов ASR-качества.

### 3. STT adapter и model manager

Из AudioBud взять идеи управления несколькими семействами локальных моделей; из архивированного Echo — RU/EN-oriented adapter boundary и поддержку GigaAM как отдельного backend. [AudioBud](https://github.com/jamditis/audiobud), [Echo](https://github.com/master5d/Echo) (проверено 2026-08-20).

Не переносить модели напрямую без проверки лицензии конкретных весов, размера, runtime и качества на целевой машине.

**Ранг: 3.** Адаптер должен позволять заменить модель без переделки shell и UX.

### 4. Streaming и benchmarks из dimastatz/whisper-flow

Переиспользовать chunked streaming, tumbling-window experiments, разделение WebSocket/HTTP, benchmark harness и тесты последовательных аудиофрагментов. Это активная MIT-библиотека/сервер для streaming STT, но не desktop app. [GitHub](https://github.com/dimastatz/whisper-flow) (проверено 2026-08-20).

**Ранг: 4. Вердикт:** полезный backend-компонент или референс; не основа Windows-продукта.

### 5. Hold-to-talk и мгновенный overlay

Паттерн «удержать → говорить → отпустить → вставить» подтверждён реализациями Pipit, Keynap и OpenVerba. [Pipit Voice](https://www.pipitvoice.com/), [Keynap](https://keynap.monosma.com/), [OpenVerba](https://openverba.com/) (проверено 2026-08-20).

Переиспользовать свойства, а не внешний стиль: overlay появляется до инициализации модели, показывает микрофон и Local/Cloud, не получает фокус и остаётся видимым во время финализации.

**Ранг: 5.**

### 6. Локальный словарь как deterministic слой

```text
spoken form → preferred spelling
aliases → canonical form
scope → global / app / mode
case policy → exact / preserve / smart
```

AudioBud, stt-whispr и Dictum содержат personal dictionary, replacements или context-bias направления; это наблюдение по реализации, не benchmark качества. [AudioBud](https://github.com/jamditis/audiobud), [stt-whispr](https://github.com/uguremrah/stt-whispr), [Dictum](https://github.com/Vkandil/dictum) (проверено 2026-08-20).

**Ранг: 6.** Начать с детерминированных замен, а не с обучения персональной модели.

## Позже — после надёжного MVP

### 7. Режимы текста и app-aware policy

Режимы: сообщение, заметка, код, промпт и email — с разными политиками cleanup. Публичное позиционирование Wispr Flow и Typeless показывает, что рынок продаёт качество конечного текста, а не только транскрипцию. Это продуктовые заявления, не независимая оценка. [Wispr Flow](https://wisprflow.ai/about), [Typeless](https://www.typeless.com/about) (проверено 2026-08-20).

### 8. Локальная генеративная очистка

Parrot демонстрирует связку whisper.cpp и llama.cpp, local-wisprflow-windows — опциональный Ollama cleanup. Оба являются референсами, не доказанными production-компонентами. [Parrot](https://github.com/basic-intelligence/parrot), [local-wisprflow-windows](https://github.com/darian-gajgic/local-wisprflow-windows) (проверено 2026-08-20).

Добавлять после измерения дополнительной задержки, изменений смысла, потребления памяти, частоты полезных правок и доли отмен cleanup.

### 9. Явный гибрид Local/Cloud

Ondula и SabreVoice показывают понятную модель с раздельными локальным и облачным режимами. [Ondula](https://ondula.ai/en/), [SabreVoice](https://www.sabreproducts.com/voice/) (проверено 2026-08-20). Добавлять позже как opt-in для неподдерживаемого языка, слабого CPU, лучшей модели или командной политики.

### 10. Continuous dictation

SabreVoice заявляет continuous mode с нарезкой речи по паузам и поэтапной печатью. [SabreVoice](https://www.sabreproducts.com/voice/) (проверено 2026-08-20). Это отдельный workflow с более сложными отменой, пунктуацией и восстановлением; не смешивать с первым hold-to-talk MVP.

## Ограничители reuse

- Решения кандидатов сопоставляются в [таблице аналогов](analogs.md).
- Заимствование не должно расширять [scope MVP](../04-mvp/scope.md).
- Для solo maintainer действует порядок [build vs reuse](../06-principles/one-person-company.md#build-vs-reuse).
- Перед форком закрывается [R-08](../07-risks/operational.md#r-08--зависимости-и-форки-становятся-неподъёмными).
