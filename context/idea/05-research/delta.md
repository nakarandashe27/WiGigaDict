# Дельта продукта

## Короткая формулировка

> Local-first Windows-диктовка для русской и смешанной RU/EN-речи, которая надёжно доставляет готовый текст в исходное приложение и никогда не теряет транскрипт при ошибке вставки.

Дельта находится не внутри ASR-модели, а на стыке:

`Windows integration × RU/EN personalization × reversible cleanup × predictable latency × recovery`

## Что действительно может отличать проект

### 1. Надёжность доставки текста

Системная ценность возникает только после появления текста в нужном поле. Issue WezTerm подтверждает случай, когда распознанные слова видны в системном flyout, но не попадают в терминал. Issue Codex описывает start/cancel-повторы и тихую неудачу вставки; это symptom reports, не статистика. [WezTerm #7791](https://github.com/wezterm/wezterm/issues/7791), [Codex #37593](https://github.com/openai/codex/issues/37593) (проверено 2026-08-20).

Защищаемый слой: capture исходного окна, несколько механизмов вставки, сохранение clipboard, детектирование неопределённого результата, локальная история, повторная вставка и диагностируемые коды отказа. Это измеримо через insertion success rate по матрице приложений.

### 2. RU/EN-персонализация для технической речи

Whisper code-switching является известной сложностью, поскольку произвольное переключение языков внутри записи не было специальной целью обучения. [Whisper Discussion #2285](https://github.com/openai/whisper/discussions/2285) (проверено 2026-08-20). Архивированный Echo был ориентирован на RU/EN и поддерживал Whisper, Parakeet и GigaAM — это сигнал отдельной архитектурной потребности, не benchmark. [Echo](https://github.com/master5d/Echo) (проверено 2026-08-20).

Защищаемый слой: персональный glossary, app/mode-scoped terms, алиасы/preferred spelling, deterministic replacements, корпус реальных смешанных фраз, router между Whisper/GigaAM-профилями и метрика ошибок по именам, API, брендам и терминам.

### 3. Обратимая локальная очистка

Wispr Flow и Typeless позиционируют продукт вокруг готового текста, а не сырой транскрипции; это продуктовые заявления, не независимое сравнение качества. [Wispr Flow](https://wisprflow.ai/about), [Typeless](https://www.typeless.com/about) (проверено 2026-08-20).

Дельтой может стать прозрачность: raw/cleaned версии, diff или быстрое переключение, deterministic словарь до генеративного этапа, локальный default, разные режимы и возможность отключить любую стадию.

### 4. Предсказуемая производительность на Windows-железе

Единичный OpenWhispr issue сообщает о крайне медленной локальной обработке на конкретной CPU-конфигурации. [OpenWhispr #989](https://github.com/OpenWhispr/openwhispr/issues/989) (проверено 2026-08-20). Это не доказывает общую медлительность, но показывает, что одной средней latency недостаточно.

Защищаемая инженерная дельта: заранее загруженная модель, cold/warm метрики, CPU/GPU profiles, time-to-first-feedback, release-to-final, last-word latency, видимый fallback и benchmark на целевых устройствах.

### 5. Честный privacy contract

Wispr Flow использует облачную транскрибацию, Keynap заявляет fully local offline, а Ondula явно разделяет Local и Cloud. [Wispr Flow Privacy](https://wisprflow.ai/privacy), [Keynap](https://keynap.monosma.com/), [Ondula](https://ondula.ai/en/) (проверено 2026-08-20).

Дельта — не слово «private», а проверяемое поведение: local-first default, отсутствие обязательного аккаунта и сети после загрузки модели, раздельные индикаторы ASR/cleanup, opt-in для cloud и локальный diagnostic log без аудио.

## Что не является дифференциацией

- **Локальный ASR сам по себе.** Whisper/GigaAM открыты, Keynap и OpenVerba уже заявляют offline/on-device workflow. [Whisper](https://github.com/openai/whisper), [GigaAM](https://github.com/salute-developers/GigaAM), [Keynap](https://keynap.monosma.com/), [OpenVerba](https://openverba.com/) (проверено 2026-08-20).
- **Global hotkey и overlay.** Их уже реализуют Pipit, Keynap и OpenVerba. [Pipit Voice](https://www.pipitvoice.com/), [Keynap](https://keynap.monosma.com/), [OpenVerba](https://openverba.com/) (проверено 2026-08-20).
- **«Текст в любом приложении».** Так позиционируются Wispr Flow, Voicy и Aqua Voice; обещание требует доказательства матрицей Windows-приложений. [Wispr Flow](https://wisprflow.ai/about), [Voicy](https://usevoicy.com/firefox-voice-to-text), [Aqua Voice YC](https://www.ycombinator.com/companies/aqua-voice) (проверено 2026-08-20).
- **Cleanup и polished text.** Это parity-функции Wispr Flow, Voicy, Pipit и Typeless; отличием могут стать локальность, обратимость и качество на RU/EN-корпусе. [Wispr Flow](https://wisprflow.ai/), [Voicy](https://usevoicy.com/firefox-voice-to-text), [Pipit Voice](https://www.pipitvoice.com/), [Typeless](https://www.typeless.com/about) (проверено 2026-08-20).
- **Отсутствие подписки.** Keynap предлагает free/lifetime, Ondula — lifetime Local, OpenVerba — бесплатный MIT-продукт. [Keynap](https://keynap.monosma.com/), [Ondula](https://ondula.ai/en/), [OpenVerba](https://openverba.com/) (проверено 2026-08-20).
- **Несколько моделей.** AudioBud, OpenWhispr и Echo уже поддерживают или заявляют несколько модельных семейств. [AudioBud](https://github.com/jamditis/audiobud), [OpenWhispr](https://github.com/OpenWhispr/openwhispr), [Echo](https://github.com/master5d/Echo) (проверено 2026-08-20).

## Итоговый defensible claim

До накопления собственных benchmark и retention-данных проект не должен заявлять уникальность ASR, превосходство над облаком или идеальный RU/EN.

Корректное обещание первой версии:

> Самая контролируемая локальная диктовка для Windows-разработчика или русскоязычного knowledge worker: быстрый hold-to-talk, персональный RU/EN-словарь, прозрачная очистка и восстановление текста при любом сбое вставки.

Защита возникает из накопленного app-compatibility слоя, пользовательских словарей, корпуса смешанной речи, latency-профилей и диагностики — данных и workflow, которые труднее скопировать, чем подключение очередной модели.

## Где дельта превращается в требования

- Первичный пользователь и его Job: [аудитория](../03-audience/README.md) и [Jobs to be Done](../03-audience/jobs-to-be-done.md).
- Архитектурные ограничения: [принципы продукта](../06-principles/README.md).
- Проверка на failure modes: [карта рисков](../07-risks/README.md).
