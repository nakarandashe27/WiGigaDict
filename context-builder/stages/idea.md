# idea.md — stages for context-builder skill

Stages that populate `context/idea/`. Stages 1-4 normally execute in sequence as part of the skill's main flow (Stage 4 dispatches a subagent via Task tool for isolated context). Stage 5 is post-build:

- Stage 5: wikilinks graph across files

Mode B (existing project) note: when the skill detects existing code, the SKILL.md router first ingests README, source structure, package metadata. Each stage below then uses that context in addition to its own upstream reads, and skips interview questions for facts already extracted.

---

## Stage 1: structure + 01-idea through 04-mvp via interview

Create `context/idea/` with this structure:

```
context/idea/
├── README.md
├── 01-idea/{README.md, why-now.md, inspiration.md}
├── 02-vision/{README.md, day-in-the-life.md, success-criteria.md}
├── 03-audience/{README.md, jobs-to-be-done.md, segments.md}
└── 04-mvp/{README.md, one-killer-feature.md, scope.md, success-metrics.md}
```

Each file: H1 with section name + empty section «Черновик».

`context/idea/README.md` — one-pager со ссылками на все разделы (wikilinks).

**Walk through interview in order:** `01-idea` → `02-vision` → `03-audience` → `04-mvp`. Один раздел за раз, вопросы — по одному (не списком). Пользователь отвечает голосом или текстом, часто сумбурно — структурируй сам и записывай в нужный файл.

**Ask (one question at a time, with 3-4 options where applicable):**
- `01-idea/README.md`: супер-идея в 1 предложении + 3 абзаца контекста
- `01-idea/why-now.md`: что изменилось в мире, что делает это возможным именно сейчас
- `01-idea/inspiration.md`: откуда пришло, какие референсы, кто уже решал
- `02-vision/day-in-the-life.md`: типичный день пользователя с продуктом
- `02-vision/success-criteria.md`: критерии успеха для пользователя
- `03-audience/README.md`: целевая аудитория в одном абзаце
- `03-audience/jobs-to-be-done.md`: одна боль = одна фича = одно решение
- `03-audience/segments.md`: 2-4 сегмента ЦА с различающимися болями
- `04-mvp/one-killer-feature.md`: золотая сцена — первый момент, когда агент приносит реальную пользу
- `04-mvp/scope.md`: что входит в MVP, что осознанно НЕ входит
- `04-mvp/success-metrics.md`: метрики успеха MVP

If any data is missing — ask a clarifying question.

When done: show tree of `context/idea/` (01-04 filled) and say «Готов к следующему этапу».

---

## Stage 2: deep research → 05-research/

Read `context/idea/01-idea/`, `02-vision/`, `03-audience/`, `04-mvp/` for context.

Create `context/idea/05-research/` with files (each: H1 + empty «Черновик»):
- `README.md`, `analogs.md`, `methodologies.md`, `what-to-steal.md`, `what-to-avoid.md`, `delta.md`

**Action:** launch 5 parallel deep-research subagents, each with its own angle:

1. **LinkedIn** — кто из основателей публично делает похожее; кто привлекает деньги под эту проблему
2. **Соцсети** (X, Reddit, продуктовые каналы) — какие open source стрельнули за 6 месяцев
3. **Web-поиск** — какие SaaS уже решают эту проблему, чем отличаются
4. **Y Combinator + акселераторы** (Techstars, 500 Global) — какие стартапы по теме сидят сейчас, какие провалились
5. **Open source** — какие готовые решения можно forkнуть и не делать с нуля

Then launch a 6th synthesizing subagent. Fill (на русском):
- `05-research/README.md` — TL;DR на 1 страницу: что нашли, главная дельта моей идеи
- `05-research/analogs.md` — таблица аналогов (фичи, цена, модель, кто делает)
- `05-research/methodologies.md` — архитектурные паттерны аналогов (agent loops, ReAct, OODA, multi-agent)
- `05-research/what-to-steal.md` — конкретные решения, ложащиеся в мой проект
- `05-research/what-to-avoid.md` — где у них болит, почему провалились
- `05-research/delta.md` — чем моя идея уникальна

**Citation rules — обязательны:**
- Каждый факт о конкретной компании, продукте или проекте — с источником: URL + дата обращения (`источник: https://… (проверено YYYY-MM-DD)`).
- Цифры (выручка, привлечённые раунды, размер команды) — только из named sources, не «по слухам».
- Если факт нельзя подтвердить источником — помечай его `[непроверено]` или вообще не включай.
- Не пиши «компания X работает над тем-то» без ссылки. Это галлюцинация.

**Fallback если deep research недоступен:**
- Если subagent'ы не могут выйти в web (ограничения окружения, нет инструментов) — НЕ выдумывай. Создай `05-research/` файлы со скелетом и пометкой `[требует ручного исследования — web недоступен]` в каждом разделе. Сообщи пользователю и предложи: (а) дать ссылки на источники вручную, (б) запустить ресёрч в окружении с web-доступом, (в) пропустить фазу research и идти на 06-principles с пометкой что delta не проверена.
- Никогда не заполняй analogs.md / competitors информацией «из памяти модели». Память на актуальные компании ненадёжна и устаревает.

Каждый файл — 1-2 страницы. Только применимое + проверяемое.

When done: show tree of `context/idea/05-research/` and ask what to refine.

---

## Stage 3: principles → 06-principles/

Read `context/idea/03-audience/` and `context/idea/05-research/` (особенно `methodologies.md` и `delta.md`).

Create `context/idea/06-principles/` with files (H1 + empty «Черновик»):
- `README.md`, `local-first.md`, `proactive.md`, `action-oriented.md`, `one-person-company.md`

**Derive from upstream — не выдумывай принципы с нуля:**
- Паттерны, повторяющиеся у успешных аналогов (`05-research/methodologies.md`)
- Боли пользователей из `03-audience/`
- Что отличает от аналогов (`05-research/delta.md`)

**Fill (на русском):**
- `06-principles/README.md` — 5-8 ключевых принципов, одной строкой каждый
- `06-principles/local-first.md` — где данные живут локально, что уходит в облако и почему
- `06-principles/proactive.md` — где агент инициирует сам, не дожидаясь команды
- `06-principles/action-oriented.md` — какие действия агент имеет право делать без подтверждения
- `06-principles/one-person-company.md` — что значит «компания из одного человека» для моего проекта

Каждый принцип — не «хотелка», а правило, которое потом повлияет на архитектуру. Принцип, не вытекающий из ресёрча или болей аудитории, — отбрасывай.

If any data is missing — ask a clarifying question.

When done: show tree of `context/idea/06-principles/` and ask what to refine.

---

## Stage 4: 10 reasons it failed → 07-risks/

**Dispatch as subagent.** Этот стейдж исполняется в изолированном контексте через Task tool (субагент). В основной сессии агент уже вложен в идею и будет выгораживать; в свежей — критикует жёстче. Не инструктируй пользователя открывать новый чат — оркестратор сам запускает субагента.

Read all of `context/idea/`.

Act as if a year passed and the idea failed. Пользователь потратил кучу времени, энергии и денег — и проиграл.

Create `context/idea/07-risks/` with files (H1 + empty «Черновик»):
- `README.md`, `technical.md`, `product.md`, `strategic.md`, `operational.md`

**Action:** назови 10 причин, почему это произошло. Не жалей, будь максимально жёстким.

For each reason:
- Что пошло не так (1 предложение)
- Почему это пошло не так (2-3 предложения)
- Что должен был сделать сейчас, чтобы этого избежать (1-2 предложения)

Then assign each reason to one of 4 risk files:
- `technical.md` — контекст, цена токенов, надёжность tool use
- `product.md` — агент делает ерунду, доверие сломается
- `strategic.md` — OpenAI/Anthropic выкатят то же бесплатно
- `operational.md` — данные утекут, агент сделает что-то критичное не то

In `07-risks/README.md` — топ-5 рисков с уровнем (low/medium/high/critical) и вердикт: какие 3 реально блокирующие, какие решаемые.

If 3+ risks are **critical** без понятного обхода — recommend killing the idea or pivoting at the core.

When done: show tree of `context/idea/07-risks/` and ask what to refine.

---

## Stage 5: ✨ wikilinks graph

Read all of `context/idea/`.

Stitch wikilinks between files (knowledge-graph principle):
- `context/idea/README.md` and each subfolder's `README.md` — link to all sections and files inside
- Each file mentioning another topic — link via a relative Markdown link `[label](relative/path)` (path relative to the linking file)
- Add any other meaningful missing links

When done: count links. Files with 0 links — flag as orphans, explain whether they're needed.
