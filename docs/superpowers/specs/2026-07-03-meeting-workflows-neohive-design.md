# Meeting Workflows + NeoHive Export — Design

- **Date:** 2026-07-03
- **Status:** Approved (design); pending implementation plan
- **Author:** Nader Awad (with Claude)
- **Scope:** Frontend (Next.js) + Rust/Tauri core under `frontend/src-tauri/src`. No changes to the archived Python/FastAPI backend. Transcription is untouched and stays fully local.

> **Implementation update (2026-07-08) — as-built deltas vs. this design.** Built and merged into local `main` (personal fork). Notable changes discovered during implementation:
> - **NeoHive auth (supersedes §6/§7/§11):** `neohive.logilica.com/.../mcp` is behind **Cloudflare Access**, so auth uses a **service token**, not a single token. Settings store `neohiveEndpoint` + `neohiveAccessClientId` + `neohiveAccessClientSecret` + `neohiveEnabled` (migrations `20260703000000` + `20260703000001`); the client sends `CF-Access-Client-Id` / `CF-Access-Client-Secret` headers.
> - **Transport:** MCP-over-HTTP (`initialize` → `tools/call memory_store`); the client tolerates JSON *and* SSE responses, sends `notifications/initialized`, has a 30 s timeout, and detects tool-level failures via `result.isError` (not just top-level JSON-RPC `error`).
> - **Run status vocabulary:** `queued | running | completed | error | cancelled` (the earlier "done" wording is superseded). Note: `running` is defined but the current runner transitions `queued`→terminal without emitting `running`.
> - **v1 scope trims:** per-workflow NeoHive **section→type overrides and importance are stored but not yet editable in the UI** (defaults: Decisions→`decision`, Action Items→`insight`, else `narrative`); per-workflow LLM params (max_tokens/temperature/top_p) are plumbed through types but have no editor UI yet. Both are backlog, not blockers.

## 1. Problem & motivation

Meetily can already summarize a meeting, choose from templates, use a custom prompt, and route to several LLM providers (Ollama, Claude, OpenAI, Groq, OpenRouter, custom OpenAI). The user wants a **TypeWhisper-style "workflows"** capability: saved, named, one-click tasks that summarize a meeting *in different ways*, each pinned to a chosen model (typically an OpenRouter model), with the results **kept side-by-side** rather than overwriting a single summary. At the end of a run, the produced elements should be **exportable to a NeoHive instance** for reuse in the user's other work.

### What already exists (baseline — do NOT rebuild)

- **OpenRouter provider:** `ModelConfig.provider` includes `'openrouter'`; `openRouterApiKey` column (migration `20250920155811_add_openrouter_api_key.sql`); live model fetch in `ModelSettingsModal.tsx`; Rust routing in `summary/llm_client.rs` (`LLMProvider::OpenRouter`, line ~160).
- **"Different ways" via templates:** `summary/templates/` module (`types.rs`, `loader.rs`, `defaults.rs`, `mod.rs`) + built-in JSON in `frontend/src-tauri/templates/` (`standard_meeting`, `daily_standup`, `retrospective`, `project_sync`, `sales_marketing_client_call`, `psychatric_session`) + user custom templates in the app data dir. Template shape: `{name, description, sections:[{title, instruction, format: paragraph|list|string, item_format?}]}`. Surfaced via `api_list_templates` (`summary/template_commands.rs`) and `useTemplates`.
- **Summary pipeline:** `useSummaryGeneration.ts` → `api_process_transcript(text, model, modelName, meetingId, chunkSize, overlap, customPrompt, templateId, summaryLanguage)` → `summary/commands.rs` → `SummaryService` (`summary/service.rs`) → `summary/processor.rs` (chunk → per-chunk summary → combine → fill template section-by-section) → `generate_summary()` (`summary/llm_client.rs`). Includes fingerprint caching, cancellation tokens, and language detection/translation.
- **Local transcription:** Whisper/Parakeet in the Rust core; not touched by this feature.

### The one structural constraint driving the design

The `summary_processes` table holds **exactly one row per meeting**, updated in place (`database/repositories/summary.rs`: `UPDATE summary_processes SET result = ? WHERE meeting_id = ?`). That 1:1 assumption is *why* regeneration overwrites. Keeping each workflow run as its own artifact means **breaking that assumption with a new runs table** — so the heart of this feature is data modeling + reuse of the existing generation core, not new LLM code.

## 2. Goals / non-goals

**Goals**
1. Define, save, edit, and delete named **workflows** (recipe = template/prompt + pinned provider/model + params + optional export config).
2. Run a workflow on a meeting on demand; keep every run as its own retained artifact.
3. Display accumulated runs alongside the existing summary; copy/export each.
4. Export a completed run's **elements (template sections)** to a NeoHive instance — explicit and opt-in, never silent.

**Non-goals (YAGNI — explicitly out of scope)**
- Multi-step / chained workflows (one step feeding the next).
- Scheduling or auto-running a workflow when a recording finishes.
- Two-way NeoHive sync or reading memories back into the app.
- Replacing or removing the existing single-summary panel — workflows are additive.

## 3. Mental model

A **Workflow** is a reusable recipe. Running it produces a **Workflow Run** (an artifact tied to a meeting). Runs accumulate; nothing is overwritten. A run's sections are the "elements." Optionally, a run's elements are pushed to NeoHive. Transcription and the primary summary flow are unchanged.

## 4. Data model

Two new SQLite tables via one migration `frontend/src-tauri/migrations/<timestamp>_add_workflows.sql` (follow existing `YYYYMMDDHHMMSS_description.sql` naming).

### `workflows` (saved recipes)
| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | uuid |
| `name` | TEXT | user-facing, required |
| `description` | TEXT | optional |
| `template_id` | TEXT | reuses existing template system |
| `custom_prompt` | TEXT | optional, appended like today's `customPrompt` |
| `provider` | TEXT | `ollama`/`openrouter`/`claude`/`openai`/`groq`/`custom-openai` |
| `model` | TEXT | model name for the provider |
| `max_tokens` | INTEGER NULL | reuse existing param plumbing |
| `temperature` | REAL NULL | |
| `top_p` | REAL NULL | |
| `neohive_export` | TEXT (JSON) | export config (see §6); null/disabled by default |
| `created_at` / `updated_at` | TEXT | ISO timestamps |

A workflow is *defined in terms of* the existing template + custom-prompt + provider/model/params, so no generation logic is duplicated.

### `workflow_runs` (retained artifacts)
| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | uuid; the poll handle |
| `workflow_id` | TEXT FK → `workflows.id` | |
| `meeting_id` | TEXT FK → `meetings.id` | |
| `status` | TEXT | `queued`/`running`/`done`/`error`/`cancelled` |
| `result_markdown` | TEXT | full rendered output |
| `result_sections` | TEXT (JSON) | `[{title, format, content}]` — the persisted section breakdown that enables section-level export and per-section display |
| `error` | TEXT NULL | |
| `neohive_status` | TEXT | `none`/`pushed`/`failed` |
| `created_at` / `updated_at` | TEXT | |

**On delete:** deleting a meeting must also delete its `workflow_runs` (extend the existing cascade in `database/repositories/meeting.rs`). Deleting a workflow **keeps** its historical runs — runs are retained artifacts, not owned children. The *mechanism* (nullable `workflow_id` vs. a denormalized snapshot of the workflow name on the run) is a plan-time detail (§11); the decision to retain is settled.

## 5. Backend (Rust) — new module `summary/workflows/`

Mirror existing patterns (`summary/commands.rs`, `database/repositories/summary.rs`).

- `models.rs` — `Workflow`, `WorkflowRun`, serde with camelCase renames for the TS boundary.
- `repository.rs` — CRUD + run persistence (SQLx, same style as `summary.rs`).
- `commands.rs` — Tauri commands, registered in `lib.rs` `generate_handler!` (currently line ~526):
  - `api_list_workflows`
  - `api_save_workflow` (create/update)
  - `api_delete_workflow`
  - `api_run_workflow(workflow_id, meeting_id)` → returns `run_id`; frontend polls like summaries
  - `api_list_workflow_runs(meeting_id)`
  - `api_get_workflow_run(run_id)`
  - `api_export_run_to_neohive(run_id)`
- **Reuse of the generation core:** a workflow run is `api_process_transcript` with a pinned provider/model whose output is written to `workflow_runs` (with structured `result_sections`) instead of `summary_processes`. Refactor the shared body of the current summary generation into a function both call, so the pipeline is not forked. Reuse chunking, caching (keyed to include workflow/model/prompt), cancellation tokens, and language handling as-is.

## 6. NeoHive export subsystem — new module `neohive/`

### Transport
NeoHive is a **remote HTTPS service** (`neohive.logilica.com`), one endpoint per project: `.../projects/<project-uuid>/mcp`, authenticated with a header token. The Meetily project's endpoint is `.../projects/e95faa80-9092-478d-98b0-19ef8158efb8/mcp`. The Rust core already uses `reqwest` (for OpenRouter/Ollama), so the app calls NeoHive directly.

- `client.rs` — thin `reqwest` client exposing `memory_store(content, type, tags, importance)`.
- **OPEN IMPLEMENTATION QUESTION (resolve in plan/first task):** confirm whether `neohive.logilica.com` exposes a plain REST `memory_store` endpoint (preferred — simplest), or whether the client must speak MCP-over-HTTP JSON-RPC (`initialize` → `tools/call memory_store`, possibly SSE responses). Either is a self-contained ~150-line client; the rest of the design is unaffected by the choice.

### Config
New columns in `settings` (store the token like other API keys, never logged/echoed):
- `neohiveEndpoint` (defaults to the Meetily project endpoint above)
- `neohiveToken`
- an enable flag

Overridable so a workflow can target a **different** NeoHive instance for the user's other work.

### What gets sent (defaults, per §7 approval)
- **Granularity: section-by-section.** Each template section → one NeoHive memory.
- **Content:** the section body from `result_sections`.
- **Type mapping (editable per workflow):** `Decisions → decision`, `Action Items → insight`, all other/prose sections → `narrative`.
- **Tags:** `[meeting title, workflow name, section title, "meetily"]`.
- **Importance:** default 6.
- **Trigger:** manual **"Send to NeoHive"** button per run by default; optional per-workflow **"auto-export on completion"** toggle. Always surfaced to the user (toast/confirmation) — never silent. This upholds Meetily's privacy-first identity: meeting content only leaves the device on an explicit action, to the user's own infrastructure.

## 7. Approved defaults (confirmed with user 2026-07-03)

| Decision | Value |
|---|---|
| Export granularity | Section-by-section (each element its own memory) |
| When to export | Manual button per run; optional per-workflow auto-export toggle; never silent |
| Target hive | Meetily project's NeoHive by default; endpoint/token overridable in Settings |
| Section → memory type | Decisions→`decision`, Action Items→`insight`, prose→`narrative` (editable) |

## 8. Frontend (Next.js)

- **Workflows manager** (new settings surface): list / create / edit / delete. Fields: name, description, template picker (reuse `useTemplates`), provider+model picker (reuse `ModelSettingsModal` provider/model UI incl. OpenRouter fetch), params, and a **NeoHive export** panel (enable, granularity is fixed to section-by-section for v1, type-mapping overrides, auto-export toggle).
- **Run + results on the meeting view:** a "Run workflow ▾" control listing saved workflows; completed runs render as stacked, labeled cards next to the existing summary, each with copy/export and a **"Send to NeoHive"** button reflecting `neohive_status`.
- **Hooks:** `useWorkflows`, `useWorkflowRuns` (mirror `useTemplates` / `useSummaryGeneration`, including the poll loop).

## 9. Conventions to honor

- All new frontend-facing behavior goes through **Tauri commands/events in the Rust core** — not the archived FastAPI backend.
- Audio/device naming rules are irrelevant here, but general Meetily conventions apply: no hardcoded paths (use Tauri path APIs for any file access); `api_*` command naming; snake_case Rust with serde camelCase renames at the TS boundary; SQLx migrations named `YYYYMMDDHHMMSS_description.sql`.
- **Never log or echo** the NeoHive token or any provider API key.

## 10. End-to-end flow

1. User creates a workflow (e.g. "Exec summary via OpenRouter/claude-sonnet"), optionally enabling NeoHive export.
2. On a meeting, user clicks **Run workflow → Exec summary**. `api_run_workflow` returns a `run_id`; frontend polls.
3. Rust generates via the shared summary core with the workflow's pinned model, persists `result_markdown` + `result_sections` to `workflow_runs`.
4. The run appears as a card alongside the summary. User clicks **Send to NeoHive** (or auto-export fires if enabled) → each section is stored as a memory via the `neohive` client → `neohive_status = pushed`.

## 11. Open items to resolve during planning
- NeoHive transport: REST vs. MCP-JSON-RPC (§6).
- Caching key for workflow runs (must include workflow id, prompt, provider, model, params, template fingerprint).
- Exact retention semantics when a workflow is deleted (default: keep runs).
