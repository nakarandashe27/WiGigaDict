# Чего избегать

## 1. Не строить продукт вокруг одной ASR-модели

Whisper и GigaAM доступны как открытые модельные основы; «мы запускаем локальное распознавание» — слабая самостоятельная защита. [Whisper](https://github.com/openai/whisper), [GigaAM](https://github.com/salute-developers/GigaAM) (проверено 2026-08-20).

Отвергнуть модельное имя как основное позиционирование, жёсткую привязку UI к одному runtime, обещания качества без собственного корпуса и предположение, что больший model size автоматически даёт лучший UX.

## 2. Не принимать dimastatz/whisper-flow за desktop foundation

Репозиторий предоставляет Python/FastAPI/PyAudio streaming STT, WebSocket/HTTP API, chunking и benchmarks. В нём нет Windows tray, global hotkey, overlay, focused-app capture, insertion, recovery buffer и cleanup UX. [dimastatz/whisper-flow](https://github.com/dimastatz/whisper-flow) (проверено 2026-08-20).

**Решение:** использовать streaming-код или идеи benchmarks; отвергнуть как готовое Windows-приложение и единственный фундамент продукта.

## 3. Не считать «private» синонимом «local»

Wispr Flow выполняет транскрибацию в облаке даже при наличии privacy controls; Keynap заявляет полностью локальную offline-обработку. [Wispr Flow Privacy](https://wisprflow.ai/privacy), [Keynap](https://keynap.monosma.com/) (проверено 2026-08-20).

В интерфейсе показывать буквально: «аудио остаётся на этом ПК», «аудио отправляется провайдеру X», «cleanup выполняется локально/в облаке».

## 4. Не делать clipboard единственным способом вставки

Issue WezTerm показывает, что системное распознавание может видеть текст, но не доставить его в несовместимое поле. [WezTerm #7791](https://github.com/wezterm/wezterm/issues/7791) (проверено 2026-08-20).

Clipboard-only создаёт риски повреждения пользовательского буфера, вставки не в то окно, потери текста при смене фокуса и невозможности установить факт успешной доставки. Нужны несколько путей вставки и обязательный recovery buffer.

## 5. Не делать cleanup необратимым

Публичная демонстрация Wispr Flow показывает очистку повторов, fillers и самоисправлений, но это маркетинговое представление, а не гарантия сохранения смысла. [Wispr Flow](https://wisprflow.ai/) (проверено 2026-08-20).

Избегать перезаписи raw transcript, генеративной переформулировки без индикации, облачного cleanup после локального ASR без согласия и одинакового cleanup для кода, сообщений и документов.

## 6. Не обещать бесшовный RU/EN без измерений

Официальное обсуждение Whisper отмечает, что модель не обучалась специально произвольному code-switching внутри одной записи. [Whisper Discussion #2285](https://github.com/openai/whisper/discussions/2285) (проверено 2026-08-20). Отдельные сообщения о галлюцинациях на русской речи являются анекдотическими failure reports, но требуют тестов пауз и тишины. [Whisper Discussion #1391](https://github.com/openai/whisper/discussions/1391) (проверено 2026-08-20).

До benchmark нельзя обещать идеальное переключение языка, правильность всех английских терминов, отсутствие hallucination и универсальность словаря.

## 7. Не игнорировать cold start и слабые CPU

Единичный issue OpenWhispr описывает 41 секунду локальной обработки для 6,6 секунды аудио на конкретном Intel Core Ultra 5 125U. Это не сравнительный benchmark. [OpenWhispr #989](https://github.com/OpenWhispr/openwhispr/issues/989) (проверено 2026-08-20).

Избегать загрузки модели только после отпускания hotkey, GPU-only happy path, скрытого медленного fallback и бесконечного spinner без стадии и прогноза.

## 8. Не превращать установку в ML-проект пользователя

Пользователи Whisper4Windows сообщали о проблемах CUDA/cuDNN DLL, Defender и автоустановки зависимостей. Это анекдотические отчёты. [Whisper4Windows discussion](https://www.reddit.com/r/LocalLLaMA/comments/1o0zxuu/i_built_a_local_whisperbased_dictation_app_for/) (проверено 2026-08-20).

MVP не должен требовать Python, ручной `pip install`, поиска CUDA DLL, копирования весов в скрытые каталоги и самостоятельного выбора несовместимых runtime/quantization.

## 9. Не форкать молодой репозиторий без аудита

AudioBud наиболее близок к требуемому Windows-продукту, но молод и имел 97 issues при 3 stars на дату проверки. [AudioBud](https://github.com/jamditis/audiobud) (проверено 2026-08-20). local-wisprflow-windows имел 0 stars/forks и должен рассматриваться как proof-of-concept. [local-wisprflow-windows](https://github.com/darian-gajgic/local-wisprflow-windows) (проверено 2026-08-20). Echo архивирован и не подходит как активная основа. [Echo](https://github.com/master5d/Echo) (проверено 2026-08-20).

Перед форком проверить лицензии и происхождение кода, updater/installer, сетевые вызовы, хранение ключей, subprocess/runtime boundary, подпись бинарников, clipboard/permissions и воспроизводимость сборки.

## 10. Не расширяться в «voice OS» до доказанного ядра

Wispr позиционируется как voice interface company с долгосрочной целью voice-native computing; Blue расширяет продукт от транскрипции к действиям в приложениях. [Wispr About](https://wisprflow.ai/about), [Blue YC](https://www.ycombinator.com/companies/blue) (проверено 2026-08-20).

Для текущего проекта это направления рынка, а не обязательный scope. Агентные действия, meeting assistant, TTS, автоматизация приложений и универсальные голосовые команды следует отклонить до достижения надёжного hold-to-talk workflow.

## Где запреты закреплены

- Архитектурные правила: [принципы продукта](../06-principles/README.md).
- Scope-control для одного разработчика: [one-person-company](../06-principles/one-person-company.md).
- Проверяемые последствия нарушения: [premortem](../07-risks/README.md).
