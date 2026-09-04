---
name: context-builder
description: Use ONLY when the user explicitly asks to build, set up, refresh, or audit a `context/` folder for a project — never on a generic "let's start a new project" / "помоги мне с проектом" request. Skill writes many files, creates folder structure, and modifies project CLAUDE.md, so it must not trigger on ambiguous "starting" intents. Builds `context/idea/` (intent), `context/architecture/` (blueprint), `context/business/` (commercial reality). Two modes auto-detected with user confirmation: Mode A (interview from scratch) or Mode B (analyze existing code, then interview only the gaps). Triggers (verbatim or close paraphrase): "собери context", "обнови context", "настрой context для проекта", "проанализируй проект и собери context", "context-builder", "build project context", "refresh project context", "set up the second brain", «добавь LIVE-метрику», «аудит business».
---

# context-builder

Three-phase build of a project's "second brain" — the `context/` folder agents read before every task. Walks the user through `idea/` (intent), `architecture/` (blueprint), `business/` (commercial reality) one phase at a time.

## Target structure

After full run, project root contains:

```
project-root/
├── CLAUDE.md           ← skill adds/updates a «Project context» section here
└── context/
    ├── INDEX.md        ← navigation; skill creates at finalize
    ├── idea/           ← stages from stages/idea.md
    ├── architecture/   ← stages from stages/architecture.md
    └── business/       ← stages from stages/business.md
```

## Stage files (read on demand)

This skill orchestrates three stage files. Do not inline their content here — read each when its phase is active:

- `stages/idea.md` — 5 stages, populates `context/idea/`
- `stages/architecture.md` — 13 stages, populates `context/architecture/`
- `stages/business.md` — 11 stages, populates `context/business/`

---

## Mode detection (auto + confirm)

Before starting any phase, decide between Mode A and Mode B:

1. Scan project root. Signs of existing code:
   - `src/`, `lib/`, `app/`, `pkg/`, or similar source folder
   - `package.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`, `Gemfile`, `composer.json`
   - Non-empty `README.md` (>10 lines of real content)
   - Existing tests folder

2. If any sign is present:
   - Announce: «Вижу существующий код в проекте. Работаю в режиме анализа (Mode B) — сначала прочитаю код, заполню что выводится, потом интервью только на дыры. Подтверди или переключи на сборку с нуля (Mode A).»
   - Wait for user confirmation. On switch → Mode A.

3. If no signs → Mode A (from scratch), proceed immediately.

Also scan `context/` state:
- Folder doesn't exist → build all three phases (idea → architecture → business).
- Some subfolders exist → ask user: «Вижу частично собранный context. Что обновлять — конкретную подпапку или всё?»
- All three subfolders exist and look filled → ask: «Context уже собран. Хочешь обновить конкретную фазу, пересобрать с нуля или прогнать аудит?» Никогда не пересобирай молча.

**Idempotency policy.** Повторный запуск не должен портить уже собранный контекст. Принцип: подтверждение на уровне стейджа/фазы, не на уровне каждого файла. Различай canonical и out-of-band файлы.

- **Canonical files** — файлы в путях, которые стейдж канонически создаёт (например, для company Stage 1: `about.md`, `team.md`, `values.md`, `legal.md` внутри `context/business/company/`). Если пользователь явно запросил обновление этой фазы/стейджа — canonical files автоматически попадают в plan как «update candidate». Это файлы, которые стейдж и должен обновлять — иначе перезапуск был бы no-op.

- **Dynamic canonical files** — для стейджей с переменным числом файлов (business Stage 2 создаёт `product-1.md`, `product-2.md`, ...; architecture Stage 7 создаёт по файлу на каждую способность в `02-ядро/способности/`): файлы, чьи имена соответствуют naming pattern стейджа и лежат в канонической папке — тоже canonical, даже если их точный список не зафиксирован шаблоном. Файлы в той же папке с именами вне паттерна — out-of-band.

- **Out-of-band files** — файлы в папке стейджа, но НЕ в каноническом списке и НЕ соответствующие dynamic pattern (например, пользователь добавил `company/marketing-strategy.md`). Сохраняй нетронутыми, упомяни в plan как «не часть стейджа, оставляю как есть».

- **Semantic merge для canonical files — дефолт, а не full rewrite.** Каждый стейдж описывает структуру файла секциями (например, `about.md` стейджа company содержит «миссия», «год основания», «формат»). При обновлении canonical file:
  1. Идентифицируй секции, которые owns стейдж (те, что описаны в его шаблоне).
  2. Обнови ТОЛЬКО эти секции; пользовательские секции / добавления / комментарии оставь на месте.
  3. Full rewrite файла разрешён только если: (a) файл создаётся впервые, (b) в файле нет пользовательских добавлений (только stage-owned секции + маркеры), (c) пользователь явно подтвердил rewrite в plan.
  В plan для каждого canonical file указывай: «merge секций X, Y» или «full rewrite (новый файл)» — чтобы пользователь видел, что именно меняется.

- **Plan для запуска stage'а/фазы:** покажи одним сообщением: «создаю заново N файлов: ..., обновляю M canonical files: ..., оставляю K out-of-band файлов: ...». Спроси единое «да / нет / уточнить по файлу». Не показывай diff per file — wikilinks-стейдж меняет десятки файлов, бесконечные подтверждения убьют сессию.

- **Heuristic для canonical files в plan:** маркеры `[авто-выведено: ...]` и заглушки `[заполнить позже]` — сильный сигнал что файл ещё draft, обновляй смело. Файлы без таких маркеров (после первого подтверждённого прохода) считаются settled, но при explicit phase update это НЕ повод их пропускать — пользователь сам попросил. Просто пометь в plan: «обновляю settled canonical files: X, Y, Z — покажу diff по запросу». Дефолт — обновлять.

- **`CLAUDE.md` проекта:** если секция «Project context» уже есть — обнови её содержимое, не дублируй секцию. Если есть похожая по смыслу секция под другим названием — спроси, объединить или оставить рядом.

- **При перезапуске одной фазы** (например, «обнови business») — сохраняй файлы других фаз нетронутыми. Plan показывай только для затрагиваемой фазы.

---

## Mode A — from scratch

Walk through three phases sequentially. Between phases ask: «Продолжаем со следующей фазой или пауза?»

### Phase 1: idea/

Read `stages/idea.md` and execute its 5 stages in order:
1. Structure + 01-idea through 04-mvp via interview
2. Deep research → 05-research/
3. Principles → 06-principles/
4. **Premortem → 07-risks/** — dispatch as a subagent (Task tool) with a clean context window. Main session continues after subagent reports back. Не инструктируй пользователя открывать новый чат.
5. Wikilinks

### Phase 2: architecture/

Read `stages/architecture.md` and execute its 13 stages in order:
1. Press release
2. System map + boundaries
3. Character
4. Main brick (data abstraction)
5. **4 memory layers** — conditional, only if product has AI agent / LLM
6. Triggers
7. Capabilities + permissions
8. Entities + ER + invariants
9. Tech stack
10. **Premortem** — dispatch as a subagent (Task tool), same reason as idea Stage 4
11. Security + costs
12. Roadmap (also updates `context/architecture/README.md` and project `CLAUDE.md`)
13. Wikilinks

### Phase 3: business/

Read `stages/business.md` and execute Stages 1-9 in order:
1. company/
2. products/
3. audience/
4. goals/
5. economics/
6. marketing/
7. assets/
8. Finalize `context/business/INDEX.md`
9. Wikilinks

Stages 10 (add LIVE metric) and 11 (audit) are operational — run on demand, not as part of build.

---

## Mode B — analyze existing code, then interview gaps

### Pre-ingestion (before any phase)

Read in this order:
1. `README.md` at project root
2. Package metadata: `package.json` / `pyproject.toml` / `Cargo.toml` / `go.mod` / etc.
3. Source structure: walk top-level `src/` (or equivalent) — folder names, main file names, route/handler definitions if obvious
4. Tests folder — what is being tested tells you what's important
5. Existing `CLAUDE.md` in project root (previous decisions)
6. Any architecture docs (`docs/`, `ARCHITECTURE.md`)

**Ignore unconditionally** (не читай, не индексируй, не упоминай в выводах):
- `node_modules/`, `vendor/`, `.venv/`, `venv/`, `env/`, `__pycache__/`
- `.git/`, `.svn/`, `.hg/`
- `dist/`, `build/`, `out/`, `.next/`, `.nuxt/`, `.parcel-cache/`, `target/` (Rust/Java build), `bin/`, `obj/`
- `*.lock`, `*.lockfile`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `poetry.lock`, `Cargo.lock`, `go.sum`
- `*.min.js`, `*.min.css`, `*.bundle.js`, `*.map`
- `coverage/`, `.nyc_output/`, `.pytest_cache/`, `.mypy_cache/`, `.ruff_cache/`
- `.DS_Store`, `Thumbs.db`, `*.log`, `*.pyc`, `*.class`
- Любые сгенерированные файлы (`*.generated.*`, `*-generated.*`)

Если корень проекта больше 200 файлов после игнора — не читай файлы скопом; ограничься top-level структурой и main entrypoint'ом. Подробное чтение делегируй Explore-агенту узкой задачей по конкретной фазе.

Summarize internally what you extracted: idea hints, audience hints, tech stack, key entities. Don't write this summary as a file — it's working memory for the stages.

### Phase execution (Mode B)

Walk through the same three phases as Mode A (idea → architecture → business), but in each stage:

- **Pre-fill any field derivable from the pre-ingestion read.** Tag each auto-filled fact with `[авто-выведено: <источник>]` so user knows what to verify.
- **Skip interview questions for fields already pre-filled** — only ask the genuinely missing ones.
- For each stage's "Derive from upstream" section, extend it with: also derive from the pre-ingestion context where applicable.

If a field is partially filled (some facts from code, others missing) — keep the auto-filled parts and ask only about the rest.

**Lifecycle of `[авто-выведено: <источник>]` markers.** Маркер — временный, на draft-этап. Когда пользователь подтвердил факт (явно сказал «верно» / отредактировал / прокомментировал), убирай маркер из основного тела файла. Если хочется сохранить trace источника — переноси его в footnote/комментарий в конце файла (например, `<!-- источник: src/auth/login.ts:42 -->`), но не в основном повествовании. К концу полной фазы маркеры в основном тексте остаются только на полях, которые пользователь явно пометил как «вернёмся позже».

---

## Finalize (after all three phases, both modes)

1. **Create `context/INDEX.md`** with:
   - Project name (one line)
   - One sentence «о чём проект»
   - Navigation table: `idea/` → замысел, `architecture/` → чертёж, `business/` → бизнес-контекст
   - List of key files inside each subfolder (one-line description each)
   - Rule: «при работе читай только нужный файл, не загружай всё сразу»

2. **Update or create `CLAUDE.md`** in project root. Add (or merge) section:

   ```
   ## Project context

   Вся методологическая обвязка проекта — в `context/`. Точка входа — `context/INDEX.md`.

   Перед любой задачей:
   1. Прочитай `context/INDEX.md`, чтобы понять, где что лежит.
   2. Открой нужный файл из `context/idea/`, `context/architecture/` или `context/business/`. Не загружай всю папку.
   ```

3. **Show final tree** of `context/` and ask user what to refine.

---

## Individual stage triggers (for re-runs / updates)

User can update one phase without rebuilding everything:

- «собери idea» / «обнови idea» / «перезаполни idea» → run all stages from `stages/idea.md`
- «собери architecture» / «обнови architecture» → run all stages from `stages/architecture.md`
- «собери business» / «обнови business» → run all stages from `stages/business.md`

Or update a single stage within a phase:
- «обнови `context/business/economics/`» → run Stage 5 of `stages/business.md`
- «обнови `context/architecture/05-стек/`» → run Stage 9 of `stages/architecture.md`
- etc.

**Precondition checks before running an individual stage.** Многие стейджи читают upstream-папки (например, business Stage 5 читает `context/idea/07-risks/` и `context/idea/04-mvp/`):
- Если upstream существует и заполнен — запускай стейдж как обычно.
- Если upstream отсутствует или пуст — **поставь стейдж на паузу и спроси пользователя**, как поступить:
  1. Собрать недостающую upstream-фазу сначала (рекомендуется)
  2. Degraded run: запустить стейдж с пометкой `[заполнить позже]` на полях, требующих недостающего upstream
  3. Отменить операцию

Если пользователь явно выбрал degraded run (вариант 2) — запускай. Без выбора пользователя не действуй на догадках.

---

## Operational triggers (post-build, on demand)

These don't build new sections — they maintain existing content:

- «добавь LIVE-метрику» → run Stage 10 of `stages/business.md`
- «аудит business» / «прогон антипаттернов» → run Stage 11 of `stages/business.md`

---

## Rules across all stages

1. **One question at a time.** When interviewing, never ask multiple questions in one turn. Always offer 3-4 options as suggestions.

2. **Don't hallucinate.** If a field can't be filled from upstream or interview — write `[заполнить позже]`, not invented content.

3. **Wikilinks back to upstream.** Stages routinely reference `context/idea/` and `context/architecture/` files via relative Markdown links `[label](relative/path)` (path relative to the linking file). Wikilinks-finalization stage at the end of each phase wires this graph.

4. **`[LIVE: slug]` only for operational changing metrics.** Operational numbers that update faster than once a month (current revenue, DAU, period conversions, funnel snapshots) get marked `[LIVE: slug-name]`. Static values (tariff prices, period plans, quarterly/annual targets, unit-economics formulas) stay as plain numbers — no marker. Registry lives in `context/business/economics/live-metrics.md` — created on demand by Stage 10 of business. **Full criteria authoritative in Stage 10**; if other stages and Stage 10 ever disagree, Stage 10 wins.

5. **Subagents for premortems and any "fresh context" need.** Both Stage 4 of idea and Stage 10 of architecture must run in an isolated context — otherwise the main agent is attached to the work it just produced and softens criticism. Dispatch via Task tool (subagent), never instruct the user to open a new chat window. The same rule applies anywhere in the skill that requires uncontaminated context: spawn a subagent, don't bounce the user out.

6. **Russian for user-facing content, English for structural markers — но НЕ переводи существующие пути.** Interview questions, file content, summaries → Russian. Stage names и section headers внутри stage-файлов (Read / Create / Derive / Ask / When done) → English. Имена файлов и папок в `context/architecture/` исторически на русском (`01-обзор/`, `02-ядро/`, `пресс-релиз.md`, `безопасность.md` и т.д.) — оставляй как есть, никогда не переводи на английский на ходу. Ссылки и wikilinks тоже сохраняют исходный регистр и язык.

7. **Don't apply destructive changes without confirmation.** Audit (business Stage 11) finds problems but never auto-fixes — always show the report and ask which fixes to apply.
