# Архитектурные методологии и паттерны

## 1. Desktop pipeline как конечный автомат

Основной workflow следует реализовать как явные состояния:

`Idle → Armed → Recording → Finalizing → Transcribing → Cleaning → Inserting → Done/Recovery`

Каждое состояние должно иметь тайм-аут, пользовательскую индикацию и структурированную причину отказа. Это предотвращает повторные start/cancel-сессии и «тихие» провалы, подобные описанным в конкретном issue Codex; issue является symptom report, а не доказательством массовости проблемы. [Codex #37593](https://github.com/openai/codex/issues/37593) (проверено 2026-08-20).

Сырой звук, транскрипт, очищенный текст и результат вставки — разные артефакты. Переход между ними должен быть наблюдаемым и повторяемым.

## 2. Hotkey: hold-to-talk как default

- key-down — захват целевого окна и старт записи;
- первое нажатие — запись;
- второе отдельное нажатие — остановка и запуск финального распознавания;
- короткое нажатие без речи — отмена;
- `Esc` — явная отмена;
- toggle mode — опциональная настройка.

Hold-to-talk повторяется в Pipit, Keynap и OpenVerba и снижает число действий до одной мышечной привычки. [Pipit Voice](https://www.pipitvoice.com/), [Keynap](https://keynap.monosma.com/), [OpenVerba](https://openverba.com/) (проверено 2026-08-20).

Регистрация shortcut должна проверять конфликт до сохранения. Для Tauri 2 официально документированы `TrayIconBuilder` и plugin глобальных shortcut; это подтверждает базовые API, но не доказывает корректность no-focus overlay на Windows. [Tauri 2 migration docs](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/start/migrate/from-tauri-1.mdx) (проверено 2026-08-20).

## 3. Overlay как интерфейс доверия

Overlay должен появляться сразу после key-down, не забирать фокус, показывать микрофон/recording, уровень сигнала, Local/Cloud, processing/error и целевое приложение. Исчезать он должен только после успешной вставки или перехода в recovery.

Keynap показывает waveform, Pipit — немедленный overlay, SabreVoice — target app. Это продуктовые реализации, а не независимое доказательство их надёжности. [Keynap](https://keynap.monosma.com/), [Pipit Voice](https://www.pipitvoice.com/), [SabreVoice](https://www.sabreproducts.com/voice/) (проверено 2026-08-20).

Tauri документирует окна без системных decorations и capability permissions, однако non-focus поведение необходимо отдельно валидировать на поддерживаемых версиях Windows. [Tauri window customization](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/learn/window-customization.mdx) (проверено 2026-08-20).

## 4. STT adapter, а не жёсткая привязка к модели

```text
prepare(model, device)
transcribe(audio, language_hint, glossary, partial_callback)
cancel()
health()
metrics()
```

Адаптер должен скрывать различия между Whisper, GigaAM и возможным OpenAI-compatible endpoint. Whisper является многоязычной MIT-моделью; GigaAM предоставляет открытые русские ASR-модели под MIT. [Whisper](https://github.com/openai/whisper), [GigaAM](https://github.com/salute-developers/GigaAM) (проверено 2026-08-20).

Для streaming можно переиспользовать идеи chunking, tumbling windows и WebSocket API из `dimastatz/whisper-flow`, но этот репозиторий не покрывает desktop workflow. [dimastatz/whisper-flow](https://github.com/dimastatz/whisper-flow) (проверено 2026-08-20).

Важно разделить streaming partials для feedback, финальную транскрипцию после отпускания, glossary/context bias, детектор тишины и защиту от hallucination на паузах. Повторяющийся выдуманный текст на русских паузах описан пользователями Whisper; это anecdote/failure report, а не количественный benchmark. [Whisper Discussion #1391](https://github.com/openai/whisper/discussions/1391) (проверено 2026-08-20).

## 5. Cleanup как отдельный обратимый слой

`raw transcript → deterministic glossary → punctuation → self-correction cleanup → mode formatter`

- raw transcript всегда доступен;
- deterministic replacements выполняются раньше генеративной обработки;
- cleanup можно отключить;
- режимы «сообщение», «заметка», «код», «промпт» имеют отдельные политики;
- облачный cleanup — только opt-in;
- если локальный cleanup не готов, MVP вставляет сырой текст, а не блокирует диктовку.

Wispr Flow публично демонстрирует удаление filler/repetition и обработку самоисправлений; это маркетинговая демонстрация, не независимая оценка. [Wispr Flow](https://wisprflow.ai/) (проверено 2026-08-20). Parrot показывает архитектурный вариант локальной связки whisper.cpp и llama.cpp cleanup, но зрелость решения ограничена. [Parrot](https://github.com/basic-intelligence/parrot) (проверено 2026-08-20).

## 6. Вставка и восстановление

1. Сохранить handle/identity целевого окна при key-down.
2. Перед вставкой проверить существование окна и ожидаемый target.
3. Использовать основной native Unicode path.
4. При отказе — `SendInput`.
5. Затем — clipboard + paste.
6. Восстановить прежний clipboard, включая нетекстовые данные.
7. Проверить хотя бы технический факт выполнения операции.
8. При неопределённом результате сохранить текст в history/recovery и показать «Вставить снова».

Проблема несовместимости с Windows Text Services Framework подтверждается issue WezTerm; OpenVerba заявляет сохранение и восстановление clipboard. [WezTerm #7791](https://github.com/wezterm/wezterm/issues/7791), [OpenVerba](https://openverba.com/) (проверено 2026-08-20).

Автоматический `Enter` должен быть выключен по умолчанию: ошибочная отправка сообщения опаснее дополнительного нажатия.

## 7. Model lifecycle

Model manager обязан поддерживать каталог совместимых моделей/устройств, размер до загрузки, resumable download, checksum, атомарную установку, удаление, warm-up, CPU/GPU fallback и диагностику несовместимости. Статусы «вес присутствует» и «runtime готов» должны быть раздельными.

Присутствие model weights на диске нельзя считать доказательством готового runtime. Пользовательские сообщения о CUDA/cuDNN DLL, Defender и автоустановке в Whisper4Windows показывают типичный installation risk; это анекдотические отчёты. [Whisper4Windows discussion](https://www.reddit.com/r/LocalLLaMA/comments/1o0zxuu/i_built_a_local_whisperbased_dictation_app_for/) (проверено 2026-08-20).

## 8. Observability без скрытой telemetry

Локально хранить длительность аудио, model load/warm-up, time-to-first-partial, release-to-final, cleanup/insertion latency, adapter/device, способ вставки, код отказа и recovery events.

По умолчанию не сохранять аудио. Экспорт diagnostic bundle — явное действие пользователя с предварительным просмотром. Удалённая telemetry — отдельный opt-in; локальная диагностика не должна зависеть от аккаунта или сети.

## Переход к решениям проекта

- Продуктовые ограничения этих паттернов: [принципы](../06-principles/README.md).
- Разрешённые автоматические действия: [action-oriented](../06-principles/action-oriented.md).
- Failure modes реализации: [технические](../07-risks/technical.md) и [операционные риски](../07-risks/operational.md).
