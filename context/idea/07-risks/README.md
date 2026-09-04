# Premortem: почему проект провалился

Представим август 2027 года: personal-first версия поглотила год работы, но так и не стала ежедневным способом ввода. Ниже — 10 наиболее правдоподобных причин, выведенных из [границ MVP](../04-mvp/scope.md), [исследования аналогов](../05-research/README.md) и [принципов продукта](../06-principles/README.md).

## Топ-5

| Риск | Уровень | Почему опасен сейчас |
|---|---|---|
| **R-01: результат теряется при crash** | critical, **BLOCKER** | Прямо нарушает главный инвариант — ни одна завершённая диктовка не теряется. |
| **R-02: вставка ломается в целевых приложениях** | critical, **BLOCKER** | Без доставки текста продукт остаётся демонстрацией ASR, а не рабочим инструментом. |
| **R-03: clean install и runtime ненадёжны** | critical, **BLOCKER** | Продукт нельзя использовать или распространять, если модель работает только в окружении разработчика. |
| **R-04: RU/EN code-switching портит технический смысл** | high | Правдоподобная ошибка в API, имени или команде опаснее явно плохой транскрипции. |
| **R-05: release-to-insert latency разрушает привычку** | high | Пользователь начинает повторять фразу или возвращается к клавиатуре. |

## Три блокирующих риска

### R-01 — атомарное сохранение результата

До любой попытки cleanup или вставки должна существовать долговечная запись с raw transcript и состоянием pipeline. Ближайший gate: серия из 100 завершённых диктовок с fault injection между стадиями даёт **0 безвозвратно потерянных результатов**, включая перезапуск приложения. Полное описание: [operational.md](operational.md#r-01--результат-теряется-при-сбое).

### R-02 — совместимость Windows-вставки

Нужно доказать работу на золотой матрице VS Code, Codex, терминалов, браузерных полей, стандартных Windows controls и разных уровней привилегий. Ближайший gate: матрица пройдена без silent loss; каждый неуспех переводит результат в recovery. Полное описание: [technical.md](technical.md#r-02--вставка-ломается-в-целевых-приложениях).

### R-03 — воспроизводимая установка

Нужно выбрать одну поддерживаемую runtime-цепочку и доказать её на чистой Windows, прежде чем добавлять второй движок и множество ускорителей. Ближайший gate: installer, модель и диагностика поднимают золотую сцену без Python, ручного поиска DLL и скрытых шагов. Полное описание: [operational.md](operational.md#r-03--установка-модели-и-runtime-ненадёжна).

## Все 10 рисков и ближайшие gates

| ID | Категория | Уровень | Владелец проверки | Ближайший gate |
|---|---|---|---|---|
| R-01 | operational | critical / blocker | session & recovery | crash/fault-injection тест, 0 потерь на 100 завершённых диктовок |
| R-02 | technical | critical / blocker | Windows integration | app matrix без silent loss, все отказы recoverable |
| R-03 | operational | critical / blocker | packaging & runtime | golden flow на чистой Windows без ручной ML-настройки |
| R-04 | technical | high | ASR benchmark | отдельная technical-token error rate на RU/EN-корпусе |
| R-05 | technical | high | performance | измерены cold/warm p50/p95 release-to-insert на целевой машине |
| R-06 | technical | high | cleanup QA | raw/cleaned regression corpus и учёт смысловых расхождений |
| R-07 | product | high | recovery UX | unresolved queue имеет статусы, retry/copy/resolve и возраст записей |
| R-08 | operational | high | dependency governance | pinned dependencies, лицензии, rollback и fallback pipeline проверены релизом |
| R-09 | strategic | medium | product owner | roadmap до gates не содержит account/billing/enterprise scope |
| R-10 | product | high | input UX | измеряются unintended sessions и пропущенные release; есть watchdog/cancel |

## Вердикт

**Continue, но только через risk-first vertical slice.** Три critical blocker’а имеют конкретные обходы и проверяемые gates, поэтому убивать идею сейчас не нужно. Нельзя начинать широкий UI или коммерческие функции, пока не доказаны atomic recovery, Windows insertion matrix и clean-install runtime.

Если R-01 не удаётся закрыть, разработку следует остановить: продукт нарушает собственное обещание доверия. Если R-02 закрывается лишь для части приложений, нужно честно сузить support matrix. Если R-03 остаётся ручным ML-проектом, разумный pivot — распространять минимальный shell поверх одного встроенного runtime вместо мульти-модельной платформы.

## Навигация

- [Технические риски](technical.md)
- [Продуктовые риски](product.md)
- [Стратегические риски](strategic.md)
- [Операционные риски](operational.md)
