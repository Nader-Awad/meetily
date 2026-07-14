# Workflow Templates + Optional "Save to Obsidian" Export — Design

- **Date:** 2026-07-15
- **Status:** Approved (design); pending implementation plan
- **Author:** Nader Awad (with Claude)
- **Scope:** Frontend (Next.js) + Rust/Tauri core under `frontend/src-tauri/src`. No changes to the archived Python/FastAPI backend. Transcription and the existing summary/NeoHive paths are untouched.
- **Builds on:** `2026-07-03` "Meeting Workflows + NeoHive Export" design (workflows, runs, NeoHive export).

## 1. Problem & motivation

The Workflows feature lets the user save named summary recipes (template + pinned model + optional NeoHive export) and run them on a meeting, keeping each run as a retained artifact. The user wants to:

1. Summarize meetings in **purpose-built ways at different technical levels** — specifically a **technical-decisions** extraction and an **action-items** extraction.
2. **Save a comprehensive meeting summary to a folder in their Obsidian vault**, so a separate Claude "cowork" workflow can read those notes and generate weekly summaries.

The key clarification driving this design: **"Save to Obsidian" is an optional export toggle available on _any_ workflow — not a dedicated workflow.** It is the direct sibling of the existing "Send to NeoHive" toggle. Different destinations (NeoHive pushes one memory per section over MCP; Obsidian writes one markdown file per run to a folder), same "optional, opt-in, per-workflow" shape.

### What already exists (baseline — do NOT rebuild)

- **Workflows + runs** data model and CRUD (`summary/workflows/`), the run background executor (`runner.rs`), section parsing (`sections.rs`), and the NeoHive export (`commands.rs::export_run` + pure `build_export_items`, transport in `crate::neohive`).
- **NeoHive export is the reference pattern:** a pure builder + an impure orchestrator, both in `commands.rs`, shared by a manual command (`api_export_run_to_neohive`) and an auto-hook in `runner.rs` (fires when `cfg.enabled && cfg.auto_export`). Global connection settings live in the settings table via `SettingsRepository::get/save_neohive_config` + `api_get/save_neohive_config`. Per-workflow config is the `neohive_export` JSON column parsed by `Workflow::neohive_config()`. Per-run status is the `neohive_status` column set by `set_run_neohive_status`.
- **Template system:** built-in templates are just JSON files in `frontend/src-tauri/templates/`. `loader.rs` resolves a template id with a three-tier fallback (custom dir → **bundled dir scan** → embedded `defaults.rs`), and `list_template_ids()` scans the bundled directory for any `*.json`. **Adding a built-in template = adding a JSON file; no Rust change and it auto-appears in the workflow editor's template picker** (this is how `project_sync`/`retrospective` already work without being in `defaults.rs`).
- **Meeting metadata:** `MeetingModel { id, title, created_at, updated_at, folder_path }` via `MeetingsRepository::get_meeting_metadata(pool, id) -> Result<Option<MeetingModel>, sqlx::Error>`. Attendees/duration are **not** columns — attendees are derived from distinct per-segment `speaker` values on transcripts.

## 2. Goals / non-goals

**Goals**
1. Ship three built-in **templates**, reusable by any workflow: `technical_decisions`, `action_items`, `comprehensive_meeting`.
2. Add an optional per-workflow **"Save to Obsidian"** export: writes the run's rendered markdown (with Obsidian-friendly YAML frontmatter) as a `.md` file into a user-configured vault folder. Manual button + optional auto-save on completion, exactly like NeoHive.
3. Keep it opt-in and never silent; independent of NeoHive (a workflow may enable neither, either, or both).

**Non-goals (YAGNI)**
- A generalized multi-destination "export registry." At N=2 destinations, keep NeoHive and Obsidian as parallel siblings; do not refactor the working NeoHive path.
- A dedicated `obsidian.rs` module. Filesystem writing is a few lines; the logic lives in `commands.rs` next to `export_run`.
- Two-way sync / reading Obsidian back into the app.
- Auto-saving *every* meeting summary regardless of workflow. Saving is a per-workflow opt-in only.
- A native folder-picker (nice-to-have follow-up); v1 uses a text path input.
- Duration in frontmatter (user chose the "rich" set without it).

## 3. Mental model

- A **template** is a content shape (sections). Three new ones are added.
- A **workflow** = template + pinned model + params + optional exports.
- **Exports are optional per-workflow toggles.** Today: "Send to NeoHive." Added: "Save to Obsidian (folder)." A run can be pushed to NeoHive and/or written to Obsidian, or neither.
- "Save to Obsidian" writes **one markdown file per run** (contrast NeoHive: one memory per section).

## 4. New templates (JSON only — no Rust changes)

Drop three files into `frontend/src-tauri/templates/`. They are auto-discovered by the bundled-dir scan and appear in the editor's template picker. Template schema: `{ name, description, sections: [{ title, instruction, format: "paragraph"|"list"|"string", item_format? }] }`.

### `technical_decisions.json`
- **name:** "Technical Decisions"
- **description:** "Extracts the technical/engineering decisions made in the meeting, with rationale, alternatives, and owners."
- **sections:**
  - `Technical Decisions` — list — instruction: "List every technical or engineering decision made in this meeting (architecture, tooling, libraries, APIs, data models, infra, process). For each: what was decided, why, what alternatives were considered, and who owns it. Only include decisions that are genuinely technical." — `item_format`: `"| **Decision** | Rationale | Alternatives Considered | Owner |\n| --- | --- | --- | --- |"`
  - `Open Technical Questions` — list — instruction: "List technical questions raised but not resolved, with any context needed to answer them later."

### `action_items.json`
- **name:** "Action Items"
- **description:** "Extracts a clean, assignable list of action items from the meeting."
- **sections:**
  - `Action Items` — list — instruction: "List all concrete action items agreed in the meeting. For each: the owner, the task, the due date if stated, and a reference to the transcript segment/timestamp." — `item_format`: `"| **Owner** | Task | Due | Reference |\n| --- | --- | --- | --- |"`
  - `Unassigned / Needs Owner` — list — instruction: "List tasks or follow-ups that were mentioned but have no clear owner yet."

### `comprehensive_meeting.json`
- **name:** "Comprehensive Meeting Note"
- **description:** "A full, well-structured meeting note suitable for archiving to Obsidian."
- **sections:**
  - `Overview` — paragraph — "A concise executive overview of the meeting: purpose, who was involved, and the outcome."
  - `Discussion & Key Topics` — paragraph — "Summarize the main topics discussed, key arguments, context, and insights, organized by topic."
  - `Key Decisions` — list — "List the important decisions made, with brief rationale."
  - `Action Items` — list — "List assigned tasks with owner and due date." — `item_format`: `"| **Owner** | Task | Due |\n| --- | --- | --- |"`
  - `Open Questions / Follow-ups` — list — "List unresolved questions and follow-ups."

**Note:** these are not embedded in `defaults.rs` (matching `project_sync`/`retrospective`); they are surfaced via the bundled-dir scan. A test asserts each is valid JSON that parses into `Template` and passes `Template::validate()`.

## 5. Data model (one migration)

New migration `frontend/src-tauri/migrations/<timestamp>_add_obsidian_export.sql` (follow `YYYYMMDDHHMMSS_description.sql` naming; timestamp must sort after existing migrations).

```sql
-- Global Obsidian connection settings (mirrors the neohive_* settings columns).
ALTER TABLE settings ADD COLUMN obsidian_vault_path TEXT;
ALTER TABLE settings ADD COLUMN obsidian_enabled INTEGER NOT NULL DEFAULT 0;

-- Per-workflow Obsidian export config (JSON; null/absent = disabled). Sibling of neohive_export.
ALTER TABLE workflows ADD COLUMN obsidian_export TEXT;

-- Per-run Obsidian result, mirroring neohive_status.
ALTER TABLE workflow_runs ADD COLUMN obsidian_status TEXT NOT NULL DEFAULT 'none'; -- none | saved | failed
ALTER TABLE workflow_runs ADD COLUMN obsidian_path TEXT;                            -- absolute path last written
```

> Implementer note: confirm the exact `settings` table shape / how neohive columns were added (migrations `20260703000000` + `20260703000001`) and mirror it. If `settings` is single-row keyed differently, follow whatever `save_neohive_config` does.

### `ObsidianExportConfig` (per-workflow, JSON in `workflows.obsidian_export`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianExportConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub auto_export: bool,
    /// Optional relative subfolder under the vault path (e.g. "Meeting Notes").
    #[serde(default)]
    pub subfolder: Option<String>,
    /// Extra frontmatter tags added on top of the defaults ["meeting","meetily"].
    #[serde(default)]
    pub tags: Vec<String>,
}
```
`Default` = all-off/empty. Add `Workflow::obsidian_config()` mirroring `neohive_config()` (parse the JSON string, fall back to default). Extend `Workflow`, `WorkflowInput`, `WorkflowRun` structs with the new fields and the repository upsert/select to read/write them.

## 6. Backend — folded into `summary/workflows/commands.rs`

Add next to the NeoHive export, mirroring its pure/impure split. **No new module.**

### Pure helpers (unit-tested)

```rust
/// Strips filesystem-hostile characters and trims; never returns empty.
pub(crate) fn sanitize_filename(title: &str) -> String { /* strip / \ : * ? " < > | and controls; collapse ws; trim; clamp len; fallback "meeting" */ }

/// Builds (filename, file_contents) for a completed run.
pub(crate) fn build_obsidian_note(
    run: &WorkflowRun,
    meeting: &MeetingModel,
    attendees: &[String],
    cfg: &ObsidianExportConfig,
) -> (String, String) { /* YAML frontmatter + "# {title}\n\n" + run.result_markdown */ }
```

**Frontmatter (the "rich" contract):**
```yaml
---
title: <meeting.title>
createdAt: <meeting.created_at, YYYY-MM-DD>      # matches the standup-note convention the weekly-summary skill parses
date: <meeting.created_at, RFC3339>
attendees: [<named speakers>]                    # omitted entirely if none are named
tags: [meeting, meetily, <cfg.tags...>]
meetingId: <run.meeting_id>
---
```
Filename: `"{YYYY-MM-DD} - {sanitize_filename(title)}.md"`.

### Impure orchestrator (mirrors `export_run`)

```rust
pub(crate) async fn save_run_to_obsidian(pool: &SqlitePool, run_id: &str) -> Result<ObsidianSaveResult, String>
```
Steps:
1. Load run; require `status == COMPLETED` (else error, same as NeoHive).
2. Load global Obsidian config (`SettingsRepository::get_obsidian_config`); require `enabled` and a non-empty `vault_path`; require the path exists and is a directory (clear error otherwise).
3. Load per-workflow `obsidian_config()` (subfolder, tags) via `run.workflow_id` (default if the workflow was deleted).
4. Load meeting metadata (`get_meeting_metadata`) and derive attendees (§7).
5. `build_obsidian_note(...)`.
6. Resolve target dir = `vault_path` [+ `subfolder`]; `create_dir_all`; **canonicalize and assert the final file path is inside the canonicalized vault_path** (reject traversal). Write the file (best-effort atomic: write temp + rename within the same dir).
7. Set `obsidian_status` (`saved`/`failed`) + `obsidian_path` on the run (`set_run_obsidian_result`).

Return `ObsidianSaveResult { path: String }` (or `{ saved: bool, path }`).

### Command + settings + registration

- `api_save_run_to_obsidian(run_id) -> ObsidianSaveResult` — manual trigger (mirrors `api_export_run_to_neohive`).
- `api_get_obsidian_config() -> { vaultPath: Option<String>, enabled: bool }` and `api_save_obsidian_config(vault_path, enabled)` — mirror the NeoHive config commands. Repo methods `SettingsRepository::get_obsidian_config` / `save_obsidian_config`.
- Register all three new commands in `lib.rs` `generate_handler!`.

### Auto-hook (`runner.rs`)

In `run_workflow_background`, after the existing NeoHive auto-export block, add an **independent** block:
```rust
let obs = workflow.obsidian_config();
if obs.enabled && obs.auto_export {
    if let Err(e) = crate::summary::workflows::commands::save_run_to_obsidian(&pool, &run_id).await {
        tracing::warn!("Auto-save to Obsidian failed for run {}: {}", run_id, e);
    }
}
```
Failure is logged, never fatal to the run (same posture as NeoHive auto-export).

### Collision policy (decision)

**Overwrite** an existing same-day-same-title file (`YYYY-MM-DD - Title.md`). One clean note per meeting keeps the vault tidy for the downstream weekly-summary reader; in-app runs still accumulate independently. Re-running the workflow refreshes the note in place.

## 7. Attendees derivation

Add `MeetingsRepository::get_distinct_named_speakers(pool, meeting_id) -> Result<Vec<String>, sqlx::Error>`:
`SELECT DISTINCT speaker FROM transcripts WHERE meeting_id = ? AND speaker IS NOT NULL AND speaker != '' AND speaker NOT GLOB 'Speaker *'` (exclude the `Speaker N` placeholders; keep insertion/alpha order stable). If empty, the `attendees` key is omitted from frontmatter.

> Implementer note: confirm the transcripts table/column names (`transcripts.speaker`, `meeting_id`) and the placeholder pattern against the diarization code; `Speaker \d+` is the placeholder format used elsewhere.

## 8. Frontend

### `types/workflow.ts`
- Add `ObsidianRunStatus = 'none' | 'saved' | 'failed'`.
- Add `ObsidianExportConfig { enabled; autoExport; subfolder?: string | null; tags: string[] }` + `DEFAULT_OBSIDIAN_EXPORT`.
- Add `ObsidianSettings { vaultPath: string | null; enabled: boolean }`.
- Extend `Workflow` (`obsidianExport?: string | null`), `WorkflowInput` (`obsidianExport?: ObsidianExportConfig | null`), `WorkflowRun` (`obsidianStatus: ObsidianRunStatus`, `obsidianPath?: string | null`).

### `components/workflows/WorkflowEditor.tsx`
- Add a **"Save summary to folder (Obsidian)"** toggle block below the NeoHive block, structurally identical: `enabled` switch → reveals `autoExport` switch + an optional `subfolder` text input. Persist as `obsidianExport` in the `WorkflowInput`. Seed default off.

### `components/MeetingDetails/WorkflowRunCard.tsx`
- Add a **"Save to Obsidian"** button next to "Send to NeoHive" for completed runs (calls a new `onSaveToObsidian(run.id)` handler). Show an `obsidianStatus` badge (`Obsidian: saved/failed`) mirroring the NeoHive badge, and surface the path on hover/title when saved.

### `hooks/meeting-details/useWorkflowRuns.ts`
- Add `saveToObsidian(runId)` invoking `api_save_run_to_obsidian`, with toast success/error (success shows the written path). Wire into `WorkflowRunCard`.

### Settings page (`app/settings/page.tsx` / the workflows/NeoHive settings area)
- Add an **Obsidian** section mirroring the NeoHive section: a vault folder **text path** input + an **enable** switch, wired to `api_get/save_obsidian_config`. Helper text: "Completed workflow runs with 'Save to Obsidian' enabled write a markdown note here."

## 9. Testing

**Rust (unit, no I/O for the pure parts):**
- `sanitize_filename`: strips illegal chars, trims, never empty, clamps length.
- `build_obsidian_note`: frontmatter carries title/createdAt(YYYY-MM-DD)/date/tags/meetingId; `attendees` present when non-empty and **omitted when empty**; body is `# title` + markdown; filename is `YYYY-MM-DD - <sanitized>.md`.
- Path-traversal guard: a title like `../../etc/x` cannot escape the vault (test the resolve+canonicalize check, or that sanitize removes separators).
- `save_run_to_obsidian` against a temp dir + in-memory sqlite: writes the file, sets `obsidian_status='saved'` + `obsidian_path`; errors cleanly when disabled / vault missing / run not completed.
- Template JSON: the three new files parse into `Template` and pass `validate()`.

**Frontend:** `cd frontend && npx tsc --noEmit && pnpm lint` clean.

**Manual E2E (verify skill):** configure a temp vault path in Settings; create a workflow using `comprehensive_meeting` with "Save to Obsidian" + auto-save on; run it on a meeting; confirm a correctly-named `.md` with valid frontmatter appears; re-run and confirm it overwrites; use the manual "Save to Obsidian" button on a `technical_decisions` run and confirm it also writes.

## 10. Conventions
- All new behavior goes through Tauri commands in the Rust core; no changes to the archived Python backend.
- `anyhow`/`Result<_, String>` error style as in existing workflow commands; user-facing errors via toast on the frontend. Never log secrets (Obsidian has none; vault path is fine to log).
- Rust JSON boundary uses serde `camelCase` renames, matching existing structs.
- `api_`-prefixed command names; commands registered in `lib.rs`.
- Git: gitmoji conventional commits, no AI attribution.

## 11. Open items for the plan
- Exact `settings` table shape and how neohive columns were persisted (confirm before writing the migration).
- Transcript table/column names for the attendees query and the placeholder pattern (`Speaker N`).
- Whether to expose the per-workflow `tags`/`subfolder` inputs in v1 or ship just the `enabled`/`autoExport` toggles and default subfolder/tags (leaning: ship `enabled`/`autoExport` + a single optional subfolder input; `tags` config-supported but not surfaced in the editor initially — parity with how NeoHive shipped section-overrides as backend-only in v1).
