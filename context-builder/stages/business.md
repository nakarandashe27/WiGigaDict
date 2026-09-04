# business.md — stages for context-builder skill

Stages that populate `context/business/`. Stages 1-8 normally execute in sequence as part of the skill's main flow (or individually if updating one block). Stages 9-11 are post-build:

- Stage 9: wikilinks graph across files
- Stage 10: add a LIVE metric (run on demand)
- Stage 11: audit `context/business/` for antipatterns (run on demand)

---

## Stage 1: company/

Read `context/idea/01-idea/` and `context/idea/06-principles/`.

Create `context/business/` with `context/business/INDEX.md` (heading «Бизнес-контекст проекта», body empty — filled in Stage 8).

Create `context/business/company/` with four files:
- `about.md` — кто мы, миссия в одно предложение, год основания, формат (соло / команда)
- `team.md` — кто чем занимается, зоны ответственности, контакты
- `values.md` — 3-5 ценностей, которыми руководствуемся при принятии решений
- `legal.md` — юр. форма, реквизиты, страна, режим налогообложения

**Derive from upstream:**
- `about.md`: миссия из `context/idea/01-idea/` (одно предложение). Формат «соло / команда» — из `context/idea/06-principles/` (если упомянут `one-person-company`).
- `values.md`: переформулируй принципы из `context/idea/06-principles/` как 3-5 ценностей.

**Ask (one question at a time, with 3-4 options):**
- `about.md`: год основания / запуска проекта
- `team.md`: состав команды и зоны ответственности (или «соло»)
- `legal.md`: юр. форма, страна, режим налогообложения (если не оформлено — пометь «не оформлено»)

**Privacy guardrails for `legal.md`** — не запрашивай и НЕ записывай:
- Паспортные данные, СНИЛС, ИНН физлица (только ИНН юрлица допустим, если пользователь явно его даёт)
- Полные банковские реквизиты (счёт, корсчёт, БИК) — кроме случая, когда пользователь явно настаивает и понимает, что файл может попасть в git
- Личные домашние адреса учредителей (только юр. адрес компании, если есть)
- Любые ключи, токены, пароли
- Полные ФИО частных лиц без согласия

Записывай только то, что нужно для понимания юр.формы и налогового режима (например: «ИП на УСН 6% в РФ», «Ltd в Делавэре, B-corp», «не оформлено»). Если пользователь начал диктовать чувствительные данные — остановись и спроси, точно ли он хочет их в `context/business/`.

If any data is missing — ask a clarifying question.

When done: show tree of `context/business/company/` and ask what to refine.

---

## Stage 2: products/

Read `context/idea/04-mvp/`, `context/architecture/01-обзор/пресс-релиз.md`, `context/architecture/02-ядро/способности/` (if exists).

Create `context/business/products/` with files:
- `overview.md` — что продаём в принципе, одной картиной (1-2 абзаца)
- `pricing.md` — все тарифы, что входит в каждый, цены, скидки
- `product-1.md`, `product-2.md`, ... — описание конкретного продукта: задача, фичи, как работает, на каком этапе. Создай столько файлов, сколько продуктов.

**Derive from upstream:**
- `overview.md`: перепиши пресс-релиз короче для внутреннего использования — что и для кого, без маркетинговой обёртки.
- `product-1.md`: главная фича — из `context/idea/04-mvp/` (one-killer-feature); список фич — из `context/architecture/02-ядро/способности/`. Этап продукта (идея / прототип / MVP / продакшен) — выведи из контекста.

**Ask (one question at a time, with 3-4 options):**
- Сколько продуктов (если непонятно из `context/idea/`)
- `pricing.md`: тарифы и цены. Если продукт не монетизируется — пометь «определить позже».

If any data is missing — ask a clarifying question.

When done: show tree of `context/business/products/` and ask what to refine.

---

## Stage 3: audience/

This block is almost a 1:1 port from `context/idea/03-audience/`. If all four cuts are there — just reformat. If gaps — fill via interview.

Read `context/idea/03-audience/` in full.

Create `context/business/audience/` with four files:
- `avatar.md` — портрет идеального клиента: возраст, город, доход, занятие
- `segments.md` — какие сегменты ЦА есть и чем они отличаются
- `objections.md` — главные возражения и страхи при покупке + как закрываем
- `journey.md` — путь клиента: как узнал → как пришёл → как купил → как остался

**Derive from upstream:**
- `avatar.md` и `segments.md`: должны быть полностью или почти полностью в `context/idea/03-audience/`. Перенеси с минимальной редакцией под формат «бизнес-карточка»: маркированный список вместо нарратива.
- `objections.md`: возьми из jobs-to-be-done или pain-points в `context/idea/`, если есть.
- `journey.md`: если в `context/idea/` есть карта пути или сценарии — перенеси. Если нет — через интервью.

**Ask (one question at a time, with 3-4 options; only if upstream is missing):**
- `objections.md`: 3-5 главных возражений и закрытие каждого
- `journey.md`: типичный путь клиента от первого касания до покупки и удержания

If any data is missing — ask a clarifying question.

When done: show tree of `context/business/audience/` and ask what to refine.

---

## Stage 4: goals/

Read `context/idea/02-vision/` and `context/idea/04-mvp/`.

Create `context/business/goals/` with four files:
- `annual.md` — большая цель на год (выручка, аудитория, продукт)
- `quarterly.md` — три-четыре цели на квартал
- `monthly.md` — фокус месяца, 1-3 пункта максимум
- `kpi.md` — какие метрики измеряем еженедельно или ежемесячно

**Derive from upstream:**
- `annual.md`: большая цель — из `context/idea/02-vision/` (success-criteria) и `context/idea/04-mvp/` (success-metrics). Сформулируй конкретно: «к концу года — выручка $X / DAU Y / N клиентов» — то, что применимо к проекту.
- `kpi.md`: список метрик из `context/idea/02-vision/` и `context/idea/04-mvp/`. Операционные значения (которые меняются чаще раза в месяц) маркируй `[LIVE: slug-метрики]` — подтянем через Stage 10. Плановые годовые/квартальные цели и формулы — без маркера. Критерии — в Stage 10.

**Ask (one question at a time, with 3-4 options):**
- `quarterly.md`: 3-4 квартальные цели (тактика — что закрываем за квартал)
- `monthly.md`: фокус текущего месяца (1-3 пункта)

If any data is missing — ask a clarifying question.

When done: show tree of `context/business/goals/` and ask what to refine.

---

## Stage 5: economics/

This block is almost entirely interview — `context/idea/` / `context/architecture/` rarely have economics.

Read `context/idea/07-risks/` (for strategic risk context) and `context/idea/04-mvp/` (if monetization model is mentioned).

Create `context/business/economics/` with four files:
- `unit-economics.md` — стоимость привлечения клиента (CAC), доход с клиента (LTV), маржа
- `revenue.md` — структура доходов (откуда деньги)
- `costs.md` — структура расходов (куда уходят деньги)
- `forecast.md` — прогноз на 3-6 месяцев

**Derive from upstream:**
- `revenue.md`: модель монетизации — если упомянута в `context/idea/04-mvp/`, перенеси.
- Остальное — интервью.

**Ask (one question at a time, with 3-4 options):**
- `unit-economics.md`: CAC, LTV, маржа. Если цифр нет — пометь каждое поле «определить позже».
- `revenue.md`: основные источники дохода и доля каждого
- `costs.md`: фиксированные расходы (зарплаты, инфра, маркетинг) и переменные
- `forecast.md`: цель по выручке / расходам на 3-6 месяцев вперёд

Операционные числа (текущая выручка/расходы за период, актуальная маржа) маркируй `[LIVE: slug-метрики]` — подтянем через Stage 10. Формулы (определение CAC/LTV) и плановые величины — без маркера. Критерии — в Stage 10.

If any data is missing — ask a clarifying question.

When done: show tree of `context/business/economics/` and ask what to refine.

---

## Stage 6: marketing/

Read `context/idea/05-research/` and `context/architecture/02-ядро/характер.md`.

Create `context/business/marketing/` with four files:
- `channels.md` — каналы привлечения и их статус (работает / тестируем / умер)
- `funnel.md` — воронка: показы → клики → лиды → продажи → удержание
- `competitors.md` — 3-5 главных конкурентов, чем отличаемся
- `content.md` — контент-план и тон коммуникации

**Derive from upstream:**
- `competitors.md`: из `context/idea/05-research/` (аналоги + delta — чем отличаемся). Почти 1:1 перенос.
- `content.md` (раздел «тон»): из `context/architecture/02-ядро/характер.md` (стиль, обращение, табу).

**Ask (one question at a time, with 3-4 options):**
- `channels.md`: какие каналы используешь и их статус
- `funnel.md`: метрики каждого этапа воронки. Текущие значения (snapshot за последний период) маркируй `[LIVE: ...]`; целевые плановые значения этапов — без маркера.
- `content.md` (раздел «контент-план»): какие форматы и темы публикуешь, частота

If any data is missing — ask a clarifying question.

When done: show tree of `context/business/marketing/` and ask what to refine.

---

## Stage 7: assets/

Read `context/architecture/02-ядро/характер.md`.

Create `context/business/assets/` with two files and one subfolder:
- `brand-guidelines.md` — цвета, шрифты, тон
- `logo/` — пустая папка для файлов лого (svg, png)
- `testimonials.md` — отзывы клиентов, кейсы, цифры

**Derive from upstream:**
- `brand-guidelines.md` (раздел «тон»): из `context/architecture/02-ядро/характер.md`.

**Ask (one question at a time, with 3-4 options):**
- `brand-guidelines.md` (цвета + шрифты): основные цвета (HEX) и шрифты (заголовочный + текстовый). Если ничего ещё нет — пометь «определить позже».
- `testimonials.md`: есть ли отзывы / кейсы / цифры? Если есть — добавь 3-5. Если нет — пометь «собрать после первых клиентов».

If any data is missing — ask a clarifying question.

When done: show tree of `context/business/assets/` and ask what to refine.

---

## Stage 8: finalize INDEX.md

Update `context/business/INDEX.md` to contain:
- название проекта одной строкой
- одно предложение «о чём проект»
- список всех файлов внутри `context/business/` с одной строкой описания каждого
- правило: «при работе читай только нужный файл, не загружай всё сразу»
- ссылку на `context/business/economics/live-metrics.md` (если файл существует) — реестр живых метрик

When done: show final INDEX.md and tree of all `context/business/`.

---

## Stage 9: ✨ wikilinks graph

Read all of `context/business/`.

Stitch wikilinks between files (knowledge-graph principle):
- `context/business/INDEX.md` links to all subfolders and key files inside.
- Each file mentioning another topic — link via a relative Markdown link `[label](relative/path)` (path relative to the linking file).
- Mandatory cross-references (skip if target doesn't exist):
  - `company/about.md` → `[01-idea](../../idea/01-idea/)`
  - `company/values.md` → `[06-principles](../../idea/06-principles/)`
  - `products/overview.md` → `[пресс-релиз.md](../../architecture/01-обзор/пресс-релиз.md)`
  - `products/product-N.md` → `[способности/](../../architecture/02-ядро/способности/)`
  - `audience/avatar.md` and `audience/segments.md` → `[03-audience](../../idea/03-audience/)`
  - `audience/avatar.md` ↔ `marketing/content.md` (на кого пишем)
  - `audience/objections.md` ↔ `marketing/content.md` (что отвечаем на возражения)
  - `goals/kpi.md` ↔ `economics/unit-economics.md`, `economics/revenue.md` (метрики экономики)
  - `goals/kpi.md` ↔ `marketing/funnel.md` (метрики воронки)
  - `marketing/competitors.md` → `[05-research](../../idea/05-research/)`
  - `marketing/content.md` → `[характер.md](../../architecture/02-ядро/характер.md)`, `[brand-guidelines.md](../assets/brand-guidelines.md)`
  - `assets/brand-guidelines.md` → `[характер.md](../../architecture/02-ядро/характер.md)`
- If `context/business/economics/live-metrics.md` exists — every `[LIVE: slug]` occurrence in business files links to section `<slug>` inside `live-metrics.md`.
- Add any other meaningful missing links.

When done: count links. Files with 0 links — flag as orphans, explain whether they're needed.

---

## Stage 10: add a LIVE metric

Problem: некоторые числа меняются часто и быстро устаревают в файле. Решение — маркер `[LIVE: slug]` + отдельный файл с запросами, который агент дёргает при необходимости.

**Когда LIVE уместен (операционные меняющиеся метрики):**
- Текущая выручка / расходы / маржа за период
- DAU / MAU / активные подписки сейчас
- Метрики воронки за последний период (CTR, конверсия, drop-off)
- Любые «снимки на сейчас», которые завтра уже другие

**Когда LIVE НЕ нужен (статические или плановые величины):**
- Цены тарифов (`pricing.md`) — статичные до следующего пересмотра тарифа; не подтягивай из БД
- Плановые цели (`goals/annual.md`, `quarterly.md`, `monthly.md`) — фиксируются на период, не пересчитываются
- Юнит-экономика как формула (CAC = X, LTV = Y) — обновляется руками раз в квартал, не онлайн
- Любые числа, которые меняются реже раза в месяц

Не превращай каждое число в `[LIVE: ...]`. Сначала спроси себя: «это снимок реальности на сейчас или зафиксированная величина?». Если зафиксированная — пиши как есть, без маркера.

Run on demand whenever a new live (operational, changing) number is needed in `context/business/`.

**Metric parameters (collect from user):**
- Slug (short snake-case name, e.g., `revenue-current-month`)
- Where to display (file path, e.g., `context/business/economics/revenue.md`)
- Section heading in human language (e.g., «Выручка за текущий месяц»)
- How to fetch data (SQL / HTTP / CLI — query description, where to send)

**Action:**

1. Insert into the target file:

   ```
   ## [Section heading]

   [LIVE: <slug>]

   См. `context/business/economics/live-metrics.md` для определения запроса.
   ```

2. Open `context/business/economics/live-metrics.md` (create if it doesn't exist). Add section:

   ```
   ## <slug>
   [SQL / HTTP / CLI]: <query>
   Источник: <where to fetch from>
   Credentials: <DO NOT put real keys — name the env variable>
   ```

3. Show the user:
   - where the LIVE marker was inserted,
   - what was written to `live-metrics.md`,
   - which env variables need to be set for the query to work.

Important:
- Do NOT write real credentials (API keys, DB passwords) into `context/business/` files. Only env variable names.
- Reply to user in Russian.

---

## Stage 11: audit for antipatterns

Run on demand. Finds problems in `context/business/` before the agent stumbles on them.

**Antipatterns:**

1. **Кладовка.** В `context/business/` валится всё без структуры (переписки, идеи, бекапы); файлы отвечают сразу на несколько тем.
   Check: каждый файл — одна тема? Никаких сваленных «разное.md» / «misc.md» / «notes.md»?
   Note: стандартные подпапки скилла (`company/`, `products/`, `audience/`, `goals/`, `economics/`, `marketing/`, `assets/`) с 3-4 каноническими файлами — это НЕ кладовка, локальный INDEX.md в них не нужен. Верхний `context/business/INDEX.md` уже всё навигирует.

2. **Дубль кода.** В `context/business/` описана структура таблиц БД, имена эндпоинтов, переменные окружения — то, что уже есть в коде. Должно быть только ЗАЧЕМ, не ЧТО.
   Check: есть ли описания, дублирующие код?
   Note: `context/business/economics/live-metrics.md` — допустимое исключение. Там нужны имена эндпоинтов / SQL / env-переменных, чтобы агент мог дёрнуть живую цифру. Но даже там запрещены боевые credentials — только имена env-переменных.

3. **Хардкод операционных метрик.** Меняющиеся операционные числа (текущая выручка, DAU, конверсии за период) без маркера `[LIVE: ...]`.
   Check: какие операционные цифры устареют через 1-3 месяца и должны стать LIVE?
   Note: статические величины (тарифные цены, плановые цели на год / квартал, юнит-экономика как формула) — НЕ требуют LIVE и не должны на это ругаться. См. Stage 10 для критериев.

4. **Бизнес отдельно от кода.** Папка `context/business/` лежит вне репозитория проекта (в Notion / Drive / отдельной директории).
   Check: `context/business/` в корне проекта рядом с кодом?

**Action:**

1. Read `context/business/INDEX.md` and walk the structure of `context/business/`.

2. For each antipattern find concrete violations with paths:
   - Antipattern N: [found / not found]
   - Concrete violations: `<file or path>` — `<what's wrong>`
   - Fix: `<concrete change>`

3. Show summary report and ask which fixes to apply.

Important:
- Do NOT apply fixes without explicit user confirmation.
- Reply to user in Russian.
