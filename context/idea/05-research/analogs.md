# Аналоги

## Коммерческие и системные продукты

| Продукт | Что важно для сравнения | Вердикт |
|---|---|---|
| Wispr Flow | Диктовка в активное поле, cleanup, история и словарь; транскрибация всегда облачная. Позиционирование выходит за пределы ASR к voice-native computing. [Сайт](https://wisprflow.ai/about), [privacy](https://wisprflow.ai/privacy), [pricing](https://wisprflow.ai/pricing) (проверено 2026-08-20). | **REFERENCE.** Брать продуктовую планку «сказал — получил готовый текст», app-aware режимы и ясный feedback. Не копировать cloud-only архитектуру. |
| Superwhisper | Поддерживает macOS, Windows и iOS, локальные и облачные модели, позиционируется как ввод polished text в разных приложениях. [Официальный сайт](https://superwhisper.com/) (проверено 2026-08-20). | **REFERENCE.** Гибрид Local/Cloud и выбор модели полезны позже; для MVP облако не обязательно. |
| Typeless | Продвигает voice-first ввод для сообщений, документов и AI, а не технические характеристики ASR. [About](https://www.typeless.com/about) (проверено 2026-08-20). | **REFERENCE.** Формулировать ценность через готовый текст и меньше действий. |
| Speechify Windows | Заявляет Windows voice typing и полностью локальную обработку на совместимых устройствах. [Анонс](https://www.prweb.com/releases/speechify-launches-with-on-device-voice-ai-for-1b-windows-users-worldwide-302728335.html), [TechCrunch](https://techcrunch.com/2026/03/31/speechifys-windows-app-uses-local-models-for-transcription-and-dictation/) (проверено 2026-08-20). | **REFERENCE.** Подтверждает самостоятельную ценность Windows и on-device режима. |
| Keynap | Windows-only: hold-to-talk, floating HUD, offline-обработка, бесплатный basic и lifetime premium. [Официальный сайт](https://keynap.monosma.com/) (проверено 2026-08-20). | **REFERENCE.** Брать простоту основного цикла, waveform и отсутствие обязательного аккаунта. |
| OpenVerba | Windows-first open-source продукт с hotkey/chord, локальным CPU/GPU ASR, восстановлением clipboard и локальным AI-редактированием. [Официальный сайт](https://openverba.com/) (проверено 2026-08-20). | **REFERENCE.** Особенно ценны clipboard preservation, конфликт-чек hotkey и разделение диктовки/редактирования. |
| Ondula | Явно разделяет Cloud и Local; локальный вариант продаётся как lifetime-лицензия. [Официальный сайт](https://ondula.ai/en/) (проверено 2026-08-20). | **REFERENCE.** Хороший образец честной индикации режима обработки и понятной цены. |
| Windows Voice Access / Voice Typing | Voice Access работает on-device после загрузки языковой модели; Voice Typing использует online speech recognition. [Voice Access](https://support.microsoft.com/en-us/accessibility/windows/voice-access/set-up-voice-access), [Voice Typing](https://support.microsoft.com/en-us/windows/privacy/speech-voice-activation-inking-typing-and-privacy) (проверено 2026-08-20). | **COMPETE / DO NOT CLONE.** Продукт обязан превосходить встроенные функции в смешанном RU/EN, словаре, cleanup, вставке и восстановлении. |

## Open-source кандидаты

| Репозиторий | Сильные стороны и ограничения | Решение |
|---|---|---|
| AudioBud | Windows-first Tauri/Rust/React, локальный model manager, hotkey, overlay, insertion, history и diagnostics. Репозиторий молод; на дату проверки — 3 stars и 97 issues. [GitHub](https://github.com/jamditis/audiobud) (проверено 2026-08-20). | **FORK / USE.** Первый кандидат на аудит и выборочное переиспользование. Форк оправдан, если model lifecycle и Windows insertion проходят локальные тесты. |
| OpenWhispr | Активный Electron/React/TypeScript full product с локальными и облачными провайдерами; 5597 stars и 789 forks на дату проверки. [GitHub](https://github.com/OpenWhispr/openwhispr) (проверено 2026-08-20). | **FORK, если важна скорость выхода.** Самый зрелый full-product кандидат. Минусы — Electron, широкий scope и риск унаследовать лишнюю сложность. |
| Dictum | Windows-only Tauri 2/Rust: shortcut, tray/HUD, clipboard injection с Unicode fallback, Credential Manager и SQLite history; локальные веса самостоятельно не управляются. [GitHub](https://github.com/Vkandil/dictum) (проверено 2026-08-20). | **USE / REFERENCE.** Сильный источник компонентов Windows shell; зрелость недостаточна для слепого форка. |
| VoiceFlow | Python-приложение для Windows/Linux с faster-whisper, tray, hold/toggle hotkey, popup, paste, CPU/CUDA и model manager; включает более широкий meeting workflow. [GitHub](https://github.com/infiniV/VoiceFlow) (проверено 2026-08-20). | **FORK для Python-прототипа; REFERENCE для Tauri.** Удалить meeting scope, если выбран форк. |
| local-wisprflow-windows | Windows proof-of-concept: faster-whisper, hotkey, Ollama cleanup, Win32 SendInput/clipboard fallback, no-focus overlay, logs и tests; проект новый и без подтверждённой зрелости. [GitHub](https://github.com/darian-gajgic/local-wisprflow-windows) (проверено 2026-08-20). | **REFERENCE.** Проверить сценарии и тестовые идеи, но не принимать wholesale без security review и локальных испытаний. |
| Echo | Архивированный Rust/Tauri/React проект с RU/EN-фокусом, адаптерами Whisper/Parakeet/GigaAM и native Unicode input. [GitHub](https://github.com/master5d/Echo) (проверено 2026-08-20). | **REFERENCE / REJECT AS FOUNDATION.** Полезен для model-adapter и RU/EN архитектуры; архивный статус исключает его как активную основу. |
| Parrot | Cross-platform Tauri/Rust/TypeScript, локальные whisper.cpp и llama.cpp cleanup; 8 stars на дату проверки. [GitHub](https://github.com/basic-intelligence/parrot) (проверено 2026-08-20). | **REFERENCE.** Заимствовать разделение ASR/cleanup; не ставить зрелость проекта в критический путь. |
| dimastatz/whisper-flow | MIT, Python/FastAPI/PyAudio, WebSocket и HTTP STT, chunked streaming, benchmarks и tests; 920 stars и 126 forks. Desktop shell и Windows insertion отсутствуют. [GitHub](https://github.com/dimastatz/whisper-flow) (проверено 2026-08-20). | **USE AS STREAMING COMPONENT / REFERENCE; REJECT AS DESKTOP FOUNDATION.** Полезный backend, но не готовое Windows-приложение. |

## Решение между стратегиями

1. **Узкая Tauri/Rust shell + компоненты AudioBud/Dictum — рекомендуется.** Лучший контроль над размером, startup latency, Windows API, overlay, безопасностью и scope.
2. **Форк AudioBud/Handy — условно рекомендуется.** Выбирать после аудита lineage, лицензий, открытых issues, model manager и insertion paths.
3. **Форк OpenWhispr — рациональный shortcut.** Выбирать, если time-to-market важнее минимального runtime и узкой архитектуры.
4. **VoiceFlow/local-wisprflow-windows — референсы или временный прототип.** Первый подходит для быстрого Python-пилота, второй — для проверки Windows workflow; ни один не должен автоматически определять production-архитектуру.

## Как использовать сравнение

- Конкретные компоненты и паттерны: [что переиспользовать](what-to-steal.md).
- Ограничения кандидатов: [чего избегать](what-to-avoid.md).
- Критерий выбора основы: [защищаемая дельта](delta.md).
- Операционные риски форка: [premortem](../07-risks/operational.md#r-08--зависимости-и-форки-становятся-неподъёмными).
