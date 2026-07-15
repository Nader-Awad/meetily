# Workflow Templates + Optional Obsidian Export — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three purpose-built summary templates (technical decisions, action items, comprehensive) and an optional per-workflow "Save to Obsidian" export that writes a run's rendered markdown (with rich YAML frontmatter) to a configured vault folder.

**Architecture:** Obsidian is a second, independent export *destination* mirroring the existing NeoHive pattern exactly (pure builder + impure orchestrator in `commands.rs`, a manual command, an auto-hook in `runner.rs`, a per-workflow JSON config column, a per-run status column, and global settings columns). Templates are pure JSON files auto-discovered by the bundled-dir scan — no Rust registration.

**Tech Stack:** Rust/Tauri core (`sqlx` SQLite, `serde`, `chrono`, `tokio`), Next.js/TypeScript frontend (`@tauri-apps/api` invoke, `sonner` toasts, shadcn/ui).

## Global Constraints

- All new behavior goes through Tauri commands in the Rust core under `frontend/src-tauri/src`. Do NOT touch the archived Python/FastAPI backend under `backend/`.
- Rust↔TS boundary uses serde `#[serde(rename_all = "camelCase")]`. Command names are `api_`-prefixed and MUST be registered in `lib.rs` `generate_handler!` (block opens at `lib.rs:528`; workflow/NeoHive commands are at `lib.rs:676-694`).
- **Settings table columns are camelCase** (`neohiveEndpoint`, `neohiveEnabled`, …). New Obsidian settings columns: `obsidianVaultPath`, `obsidianEnabled`. The table is **single-row keyed `id='1'`**; upserts bootstrap the row with `provider='openai', model='gpt-4o-2024-11-20', whisperModel='large-v3'`.
- **`workflows` / `workflow_runs` columns are snake_case** (`neohive_export`, `neohive_status`). New columns: `workflows.obsidian_export`, `workflow_runs.obsidian_status`, `workflow_runs.obsidian_path`.
- `list_workflows`/`get_workflow`/`get_run`/`list_runs_for_meeting` use `SELECT *` + `sqlx::FromRow`, so **every new DB column MUST get a matching field on the `Workflow`/`WorkflowRun` struct in `models.rs`** or `FromRow` fails at runtime.
- New migration timestamp MUST sort after `20260709000002`. Use `20260715000000_add_obsidian_export.sql`.
- Rust errors: commands return `Result<_, String>`; internal helpers use `Result<_, String>` per existing workflow code. Frontend: try/catch + `sonner` toast. Never log secrets (Obsidian has none; vault path is fine to log).
- Git: gitmoji conventional commits, **no AI attribution / no `Co-Authored-By`**.
- **Rust test/build:** `cd frontend/src-tauri && cargo test --lib <module> 2>&1 | tail -20` and `cargo build 2>&1 | tail -6`. Pre-existing failures `audio::device_detection::{test_builtin_mic_detection, test_calculate_buffer_timeout_bluetooth}` are NOT yours — ignore them.
- **Frontend gate:** `next lint` is broken repo-wide — do NOT use it. Gate is `cd frontend && npx tsc --noEmit 2>&1 | tail -15` with **no NEW errors** (a pre-existing `bun:test` tsc error is not yours). **`node_modules` is missing** — run `cd frontend && pnpm install` once before the first `tsc` run.
- Product decisions (from the spec): overwrite an existing same-day-same-title note on re-run; attendees = distinct transcript speakers excluding the `Speaker N` placeholder, and the `attendees` key is omitted entirely when none are named.

---

### Task 1: Three built-in templates (JSON only)

**Files:**
- Create: `frontend/src-tauri/templates/technical_decisions.json`
- Create: `frontend/src-tauri/templates/action_items.json`
- Create: `frontend/src-tauri/templates/comprehensive_meeting.json`
- Modify: `frontend/src-tauri/src/summary/templates/loader.rs` (add a test)

**Interfaces:**
- Produces: three template ids (`technical_decisions`, `action_items`, `comprehensive_meeting`) discoverable via the existing bundled-dir scan and `api_list_templates`. No Rust API change; templates are surfaced by `list_template_ids()` reading the templates directory.

- [ ] **Step 1: Write `technical_decisions.json`**

```json
{
  "name": "Technical Decisions",
  "description": "Extracts the technical and engineering decisions made in the meeting, with rationale, alternatives, and owners.",
  "sections": [
    {
      "title": "Technical Decisions",
      "instruction": "List every technical or engineering decision made in this meeting (architecture, tooling, libraries, APIs, data models, infrastructure, or engineering process). For each decision state what was decided, why, what alternatives were considered, and who owns it. Only include decisions that are genuinely technical; ignore purely organizational or scheduling decisions.",
      "format": "list",
      "item_format": "| **Decision** | Rationale | Alternatives Considered | Owner |\n| --- | --- | --- | --- |"
    },
    {
      "title": "Open Technical Questions",
      "instruction": "List technical questions that were raised but not resolved during the meeting, with enough context to answer them later.",
      "format": "list"
    }
  ]
}
```

- [ ] **Step 2: Write `action_items.json`**

```json
{
  "name": "Action Items",
  "description": "Extracts a clean, assignable list of action items from the meeting.",
  "sections": [
    {
      "title": "Action Items",
      "instruction": "List all concrete action items agreed during the meeting. For each item give the owner, the task, the due date if one was stated, and a reference to the transcript segment or timestamp where it was agreed.",
      "format": "list",
      "item_format": "| **Owner** | Task | Due | Reference |\n| --- | --- | --- | --- |"
    },
    {
      "title": "Unassigned / Needs Owner",
      "instruction": "List tasks or follow-ups that were mentioned but do not yet have a clear owner.",
      "format": "list"
    }
  ]
}
```

- [ ] **Step 3: Write `comprehensive_meeting.json`**

```json
{
  "name": "Comprehensive Meeting Note",
  "description": "A full, well-structured meeting note suitable for archiving to Obsidian.",
  "sections": [
    {
      "title": "Overview",
      "instruction": "Provide a concise executive overview of the meeting: its purpose, who was involved, and the overall outcome.",
      "format": "paragraph"
    },
    {
      "title": "Discussion & Key Topics",
      "instruction": "Summarize the main topics discussed, including key arguments, context, and insights, organized topic by topic.",
      "format": "paragraph"
    },
    {
      "title": "Key Decisions",
      "instruction": "List the important decisions made during the meeting, each with brief rationale.",
      "format": "list"
    },
    {
      "title": "Action Items",
      "instruction": "List assigned tasks with their owner and due date.",
      "format": "list",
      "item_format": "| **Owner** | Task | Due |\n| --- | --- | --- |"
    },
    {
      "title": "Open Questions / Follow-ups",
      "instruction": "List unresolved questions and follow-ups from the meeting.",
      "format": "list"
    }
  ]
}
```

- [ ] **Step 4: Add a validity test in `loader.rs`**

In `frontend/src-tauri/src/summary/templates/loader.rs`, inside the existing `#[cfg(test)] mod tests { ... }` block (which already has `use super::*;`), add:

```rust
    #[test]
    fn test_new_builtin_templates_are_valid() {
        for content in [
            include_str!("../../../templates/technical_decisions.json"),
            include_str!("../../../templates/action_items.json"),
            include_str!("../../../templates/comprehensive_meeting.json"),
        ] {
            let template = validate_and_parse_template(content).expect("template should be valid");
            assert!(!template.name.is_empty());
            assert!(!template.sections.is_empty());
        }
    }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd frontend/src-tauri && cargo test --lib summary::templates::loader::tests::test_new_builtin_templates_are_valid 2>&1 | tail -20`
Expected: PASS. If it fails on parsing, the JSON has a schema mismatch — fix the JSON (check `types.rs` `Template`/`TemplateSection` field names: `name`, `description`, `sections[].title/instruction/format/item_format`).

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/templates/technical_decisions.json frontend/src-tauri/templates/action_items.json frontend/src-tauri/templates/comprehensive_meeting.json frontend/src-tauri/src/summary/templates/loader.rs
git commit -m "feat(workflows): :sparkles: add technical-decisions, action-items, and comprehensive templates"
```

---

### Task 2: Migration + Rust models + workflow repository

**Files:**
- Create: `frontend/src-tauri/migrations/20260715000000_add_obsidian_export.sql`
- Modify: `frontend/src-tauri/src/summary/workflows/models.rs`
- Modify: `frontend/src-tauri/src/summary/workflows/repository.rs`
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs` (test-only `WorkflowInput` literal)

**Interfaces:**
- Produces: `ObsidianExportConfig` struct; `Workflow::obsidian_config() -> ObsidianExportConfig`; `Workflow.obsidian_export: Option<String>`; `WorkflowInput.obsidian_export: Option<ObsidianExportConfig>`; `WorkflowRun.obsidian_status: String` + `WorkflowRun.obsidian_path: Option<String>`; `WorkflowsRepository::set_run_obsidian_result(pool, run_id, status, path) -> Result<(), sqlx::Error>`.
- Consumes: nothing new.

- [ ] **Step 1: Write the migration**

Create `frontend/src-tauri/migrations/20260715000000_add_obsidian_export.sql`:

```sql
-- Obsidian export: write completed workflow runs as markdown files to a vault folder.

-- Global connection settings (single-row settings table, id = '1'; camelCase columns).
ALTER TABLE settings ADD COLUMN obsidianVaultPath TEXT;
ALTER TABLE settings ADD COLUMN obsidianEnabled INTEGER NOT NULL DEFAULT 0;

-- Per-workflow Obsidian export config (JSON; null/absent = disabled). Sibling of neohive_export.
ALTER TABLE workflows ADD COLUMN obsidian_export TEXT;

-- Per-run Obsidian result, mirroring neohive_status.
ALTER TABLE workflow_runs ADD COLUMN obsidian_status TEXT NOT NULL DEFAULT 'none'; -- none | saved | failed
ALTER TABLE workflow_runs ADD COLUMN obsidian_path TEXT;                            -- absolute path last written
```

- [ ] **Step 2: Add `ObsidianExportConfig` + fields to `models.rs`**

In `frontend/src-tauri/src/summary/workflows/models.rs`, after the `NeoHiveExportConfig` block (before the `Workflow` struct), add:

```rust
/// Per-workflow config for writing a completed run to an Obsidian vault folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianExportConfig {
    /// Whether this workflow may write to Obsidian at all.
    #[serde(default)]
    pub enabled: bool,
    /// If true, write automatically when a run completes; else manual button only.
    #[serde(default)]
    pub auto_export: bool,
    /// Optional relative subfolder under the configured vault path (e.g. "Meeting Notes").
    #[serde(default)]
    pub subfolder: Option<String>,
    /// Extra frontmatter tags added on top of the defaults ["meeting", "meetily"].
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Default for ObsidianExportConfig {
    fn default() -> Self {
        Self { enabled: false, auto_export: false, subfolder: None, tags: Vec::new() }
    }
}
```

In the `Workflow` struct, add after `pub neohive_export: Option<String>,`:

```rust
    /// Raw JSON string of ObsidianExportConfig; parse with `obsidian_config()`.
    pub obsidian_export: Option<String>,
```

In the `impl Workflow` block, after `neohive_config()`, add:

```rust
    /// Parses the stored Obsidian export config, falling back to a disabled default.
    pub fn obsidian_config(&self) -> ObsidianExportConfig {
        self.obsidian_export
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
```

In the `WorkflowInput` struct, add after `pub neohive_export: Option<NeoHiveExportConfig>,`:

```rust
    pub obsidian_export: Option<ObsidianExportConfig>,
```

In the `WorkflowRun` struct, add after `pub neohive_status: String,`:

```rust
    pub obsidian_status: String,
    pub obsidian_path: Option<String>,
```

- [ ] **Step 3: Thread `obsidian_export` through `upsert_workflow` + add `set_run_obsidian_result` in `repository.rs`**

In `frontend/src-tauri/src/summary/workflows/repository.rs`, in `upsert_workflow`, after the `export_json` block add:

```rust
        let obsidian_json = match &input.obsidian_export {
            Some(cfg) => Some(
                serde_json::to_string(cfg)
                    .map_err(|e| sqlx::Error::Protocol(format!("serialize obsidian cfg: {}", e).into()))?,
            ),
            None => None,
        };
```

Update the INSERT column list, VALUES placeholders, ON CONFLICT SET, and binds. The INSERT column list becomes (add `obsidian_export` right after `neohive_export`):

```rust
            INSERT INTO workflows
                (id, name, description, template_id, custom_prompt, provider, model,
                 max_tokens, temperature, top_p, neohive_export, obsidian_export, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                template_id = excluded.template_id,
                custom_prompt = excluded.custom_prompt,
                provider = excluded.provider,
                model = excluded.model,
                max_tokens = excluded.max_tokens,
                temperature = excluded.temperature,
                top_p = excluded.top_p,
                neohive_export = excluded.neohive_export,
                obsidian_export = excluded.obsidian_export,
                updated_at = excluded.updated_at
```

And add the bind for `obsidian_json` immediately after `.bind(&export_json)`:

```rust
        .bind(&export_json)
        .bind(&obsidian_json)
        .bind(now)
        .bind(now)
```

At the end of the `impl WorkflowsRepository` block, after `set_run_neohive_status`, add:

```rust
    pub async fn set_run_obsidian_result(
        pool: &SqlitePool,
        run_id: &str,
        status: &str,
        path: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query("UPDATE workflow_runs SET obsidian_status = ?, obsidian_path = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(path)
            .bind(now)
            .bind(run_id)
            .execute(pool)
            .await?;
        Ok(())
    }
```

- [ ] **Step 4: Fix all `WorkflowInput` literals so the crate compiles**

`WorkflowInput` gained a required field. Update every struct literal:
- In `repository.rs` test module (`sample_input()` builder, ~line 190+): add `obsidian_export: None,`.
- In `commands.rs` test `save_then_list_then_delete_roundtrip` (the `WorkflowInput { ... }` at ~line 347): add `obsidian_export: None,`.

- [ ] **Step 5: Extend the repository roundtrip test**

In `repository.rs` test module, add a test proving `obsidian_export` round-trips and a new run defaults `obsidian_status='none'`:

```rust
    #[tokio::test]
    async fn obsidian_export_roundtrips_and_run_defaults_none() {
        let pool = test_pool().await;
        let mut input = sample_input();
        input.obsidian_export = Some(crate::summary::workflows::models::ObsidianExportConfig {
            enabled: true, auto_export: true, subfolder: Some("Meeting Notes".into()), tags: vec!["x".into()],
        });
        let wf = WorkflowsRepository::upsert_workflow(&pool, &input).await.unwrap();
        let cfg = wf.obsidian_config();
        assert!(cfg.enabled && cfg.auto_export);
        assert_eq!(cfg.subfolder.as_deref(), Some("Meeting Notes"));

        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", Some(&wf.id), &wf.name, "m1").await.unwrap();
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.obsidian_status, "none");
        assert!(run.obsidian_path.is_none());

        WorkflowsRepository::set_run_obsidian_result(&pool, "r1", "saved", Some("/tmp/x.md")).await.unwrap();
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.obsidian_status, "saved");
        assert_eq!(run.obsidian_path.as_deref(), Some("/tmp/x.md"));
    }
```

> Note: if `sample_input()` doesn't exist under that exact name, mirror the existing `WorkflowInput` literal used in the file's other tests and add `obsidian_export: None` to it, then set it in this test as above.

- [ ] **Step 6: Run tests**

Run: `cd frontend/src-tauri && cargo test --lib summary::workflows 2>&1 | tail -25`
Expected: PASS (including the new `obsidian_export_roundtrips_and_run_defaults_none`). The migration runs automatically via `sqlx::migrate!("./migrations")` in `test_pool()`.

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/migrations/20260715000000_add_obsidian_export.sql frontend/src-tauri/src/summary/workflows/models.rs frontend/src-tauri/src/summary/workflows/repository.rs frontend/src-tauri/src/summary/workflows/commands.rs
git commit -m "feat(workflows): :sparkles: add obsidian_export config + run status to data model"
```

---

### Task 3: Global Obsidian settings (repository + commands)

**Files:**
- Modify: `frontend/src-tauri/src/database/repositories/setting.rs`
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (register two commands)

**Interfaces:**
- Produces: `ObsidianSettings { vault_path: Option<String>, enabled: bool }`; `SettingsRepository::get_obsidian_config(pool) -> Result<ObsidianSettings, sqlx::Error>`; `SettingsRepository::save_obsidian_config(pool, vault_path: Option<&str>, enabled: bool) -> Result<(), sqlx::Error>`; commands `api_get_obsidian_config`, `api_save_obsidian_config`.

- [ ] **Step 1: Add `ObsidianSettings` + repo methods in `setting.rs`**

After the `NeoHiveSettings` struct add:

```rust
#[derive(Debug, Clone, Default)]
pub struct ObsidianSettings {
    pub vault_path: Option<String>,
    pub enabled: bool,
}
```

After `save_neohive_config`, add:

```rust
    pub async fn get_obsidian_config(
        pool: &SqlitePool,
    ) -> std::result::Result<ObsidianSettings, sqlx::Error> {
        let row: Option<(Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT obsidianVaultPath, obsidianEnabled FROM settings WHERE id = '1' LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;
        Ok(match row {
            Some((vault_path, enabled)) => ObsidianSettings {
                vault_path,
                enabled: enabled.unwrap_or(0) != 0,
            },
            None => ObsidianSettings::default(),
        })
    }

    pub async fn save_obsidian_config(
        pool: &SqlitePool,
        vault_path: Option<&str>,
        enabled: bool,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, obsidianVaultPath, obsidianEnabled)
            VALUES ('1', 'openai', 'gpt-4o-2024-11-20', 'large-v3', ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                obsidianVaultPath = excluded.obsidianVaultPath,
                obsidianEnabled = excluded.obsidianEnabled
            "#,
        )
        .bind(vault_path)
        .bind(if enabled { 1_i64 } else { 0_i64 })
        .execute(pool)
        .await?;
        Ok(())
    }
```

- [ ] **Step 2: Write a repo test (mirror `neohive_settings_tests`)**

Add to `setting.rs` a test module (or extend the existing one):

```rust
#[cfg(test)]
mod obsidian_settings_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn defaults_then_save_then_read() {
        let pool = pool().await;
        let cfg = SettingsRepository::get_obsidian_config(&pool).await.unwrap();
        assert!(cfg.vault_path.is_none());
        assert!(!cfg.enabled);

        SettingsRepository::save_obsidian_config(&pool, Some("/vault/Meetings"), true).await.unwrap();
        let cfg = SettingsRepository::get_obsidian_config(&pool).await.unwrap();
        assert_eq!(cfg.vault_path.as_deref(), Some("/vault/Meetings"));
        assert!(cfg.enabled);
    }
}
```

- [ ] **Step 3: Add the config commands in `commands.rs`**

Add near `api_get_neohive_config` (import `ObsidianSettings` is not needed — call the repo path directly):

```rust
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianConfigResponse {
    pub vault_path: Option<String>,
    pub enabled: bool,
}

#[tauri::command]
pub async fn api_get_obsidian_config(
    state: tauri::State<'_, AppState>,
) -> Result<ObsidianConfigResponse, String> {
    log_info!("api_get_obsidian_config called");
    let cfg = SettingsRepository::get_obsidian_config(state.db_manager.pool())
        .await
        .map_err(|e| { log_error!("api_get_obsidian_config failed: {}", e); e.to_string() })?;
    Ok(ObsidianConfigResponse { vault_path: cfg.vault_path, enabled: cfg.enabled })
}

#[tauri::command]
pub async fn api_save_obsidian_config(
    state: tauri::State<'_, AppState>,
    vault_path: Option<String>,
    enabled: bool,
) -> Result<(), String> {
    log_info!("api_save_obsidian_config called (enabled={})", enabled);
    SettingsRepository::save_obsidian_config(state.db_manager.pool(), vault_path.as_deref(), enabled)
        .await
        .map_err(|e| { log_error!("api_save_obsidian_config failed: {}", e); e.to_string() })
}
```

- [ ] **Step 4: Register the commands in `lib.rs`**

In `frontend/src-tauri/src/lib.rs`, immediately after `summary::workflows::commands::api_save_neohive_config,` (line ~694), add:

```rust
            summary::workflows::commands::api_get_obsidian_config,
            summary::workflows::commands::api_save_obsidian_config,
```

- [ ] **Step 5: Run tests + build**

Run: `cd frontend/src-tauri && cargo test --lib database::repositories::setting 2>&1 | tail -20`
Expected: PASS.
Run: `cd frontend/src-tauri && cargo build 2>&1 | tail -6`
Expected: builds (warnings OK). If `api_save_obsidian_config`'s `vault_path` arg-name mismatches the frontend later, note Tauri auto-converts camelCase JS args to snake_case Rust params — the JS side will pass `{ vaultPath, enabled }`.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/database/repositories/setting.rs frontend/src-tauri/src/summary/workflows/commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(workflows): :sparkles: add global Obsidian vault settings + config commands"
```

---

### Task 4: Pure Obsidian note builder (`sanitize_filename` + `build_obsidian_note`)

**Files:**
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs`

**Interfaces:**
- Consumes: `WorkflowRun`, `MeetingModel` (`crate::database::models::MeetingModel` — fields `title: String`, `created_at: DateTimeUtc` where `DateTimeUtc(pub DateTime<Utc>)`), `ObsidianExportConfig`.
- Produces: `pub(crate) fn sanitize_filename(title: &str) -> String`; `pub(crate) fn build_obsidian_note(run: &WorkflowRun, meeting: &MeetingModel, attendees: &[String], cfg: &ObsidianExportConfig) -> (String, String)` returning `(filename, contents)`.

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)] mod tests` in `commands.rs` (and add imports at the top of the test module: `use crate::database::models::{MeetingModel, DateTimeUtc};`, `use crate::summary::workflows::models::ObsidianExportConfig;`, `use chrono::TimeZone;`):

```rust
    fn sample_meeting() -> MeetingModel {
        MeetingModel {
            id: "m1".into(),
            title: "Sprint Planning: Q3".into(),
            created_at: DateTimeUtc(chrono::Utc.with_ymd_and_hms(2026, 7, 15, 14, 30, 0).unwrap()),
            updated_at: DateTimeUtc(chrono::Utc.with_ymd_and_hms(2026, 7, 15, 15, 0, 0).unwrap()),
            folder_path: None,
        }
    }

    fn sample_run() -> WorkflowRun {
        WorkflowRun {
            id: "r1".into(), workflow_id: Some("w1".into()), workflow_name: "Comprehensive".into(),
            meeting_id: "m1".into(), status: "completed".into(),
            result_markdown: Some("## Overview\nWe planned Q3.".into()),
            result_sections: None, error: None,
            neohive_status: "none".into(), obsidian_status: "none".into(), obsidian_path: None,
            created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn sanitize_filename_strips_illegal_and_never_empty() {
        assert_eq!(super::sanitize_filename("a/b:c*?\"<>|d"), "abcd");
        assert_eq!(super::sanitize_filename("../../etc/passwd"), "etcpasswd");
        assert_eq!(super::sanitize_filename("   "), "meeting");
        assert!(!super::sanitize_filename("normal title").is_empty());
    }

    #[test]
    fn build_obsidian_note_has_frontmatter_and_filename() {
        let cfg = ObsidianExportConfig { tags: vec!["planning".into()], ..Default::default() };
        let (filename, contents) = super::build_obsidian_note(&sample_run(), &sample_meeting(), &["Alice".into(), "Bob".into()], &cfg);
        assert_eq!(filename, "2026-07-15 - Sprint Planning Q3.md");
        assert!(contents.contains("createdAt: 2026-07-15"));
        assert!(contents.contains("title:"));
        assert!(contents.contains("meetingId: m1"));
        assert!(contents.contains("attendees: [\"Alice\", \"Bob\"]"));
        assert!(contents.contains("\"meeting\""));
        assert!(contents.contains("\"meetily\""));
        assert!(contents.contains("\"planning\""));
        assert!(contents.contains("We planned Q3."));
        assert!(contents.starts_with("---\n"));
    }

    #[test]
    fn build_obsidian_note_omits_attendees_when_empty() {
        let (_f, contents) = super::build_obsidian_note(&sample_run(), &sample_meeting(), &[], &ObsidianExportConfig::default());
        assert!(!contents.contains("attendees:"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test --lib summary::workflows::commands 2>&1 | tail -20`
Expected: FAIL (functions not found / type errors).

- [ ] **Step 3: Implement the pure functions in `commands.rs`**

Add near the other pure helpers (`build_export_items`), and add `use crate::database::models::MeetingModel;` + `use crate::summary::workflows::models::ObsidianExportConfig;` to the module's top-level imports:

```rust
/// Double-quotes a string for a YAML scalar, escaping backslashes and quotes.
fn yaml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Strips filesystem-hostile characters; collapses whitespace; never empty.
pub(crate) fn sanitize_filename(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control())
        .collect();
    // collapse internal whitespace runs to single spaces, trim ends
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = collapsed.trim_matches('.').trim().chars().take(120).collect();
    if trimmed.is_empty() { "meeting".to_string() } else { trimmed }
}

/// Builds (filename, file_contents) for a completed run. Frontmatter uses the
/// meeting's created_at for both `createdAt` (YYYY-MM-DD, matching the vault's
/// standup convention) and `date` (RFC3339). `attendees` is omitted when empty.
pub(crate) fn build_obsidian_note(
    run: &WorkflowRun,
    meeting: &MeetingModel,
    attendees: &[String],
    cfg: &ObsidianExportConfig,
) -> (String, String) {
    let created = meeting.created_at.0;
    let day = created.format("%Y-%m-%d").to_string();
    let filename = format!("{} - {}.md", day, sanitize_filename(&meeting.title));

    let mut tags: Vec<String> = vec!["meeting".to_string(), "meetily".to_string()];
    for t in &cfg.tags {
        if !t.trim().is_empty() && !tags.contains(t) { tags.push(t.clone()); }
    }
    let tags_yaml = tags.iter().map(|t| yaml_str(t)).collect::<Vec<_>>().join(", ");

    let mut fm = String::from("---\n");
    fm.push_str(&format!("title: {}\n", yaml_str(&meeting.title)));
    fm.push_str(&format!("createdAt: {}\n", day));
    fm.push_str(&format!("date: {}\n", created.to_rfc3339()));
    if !attendees.is_empty() {
        let att = attendees.iter().map(|a| yaml_str(a)).collect::<Vec<_>>().join(", ");
        fm.push_str(&format!("attendees: [{}]\n", att));
    }
    fm.push_str(&format!("tags: [{}]\n", tags_yaml));
    fm.push_str(&format!("meetingId: {}\n", run.meeting_id));
    fm.push_str("---\n\n");

    let body = run.result_markdown.clone().unwrap_or_default();
    let contents = format!("{}# {}\n\n{}\n", fm, meeting.title, body);
    (filename, contents)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test --lib summary::workflows::commands 2>&1 | tail -20`
Expected: PASS (all three new tests + existing ones).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/commands.rs
git commit -m "feat(workflows): :sparkles: add pure Obsidian note builder + filename sanitizer"
```

---

### Task 5: Attendees query + `save_run_to_obsidian` orchestrator + command + auto-hook

**Files:**
- Modify: `frontend/src-tauri/src/database/repositories/meeting.rs`
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs`
- Modify: `frontend/src-tauri/src/summary/workflows/runner.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (register one command)

**Interfaces:**
- Consumes: `sanitize_filename`, `build_obsidian_note` (Task 4); `SettingsRepository::get_obsidian_config` (Task 3); `WorkflowsRepository::{get_run, get_workflow, set_run_obsidian_result}`; `Workflow::obsidian_config()` (Task 2); `MeetingsRepository::get_meeting_metadata`.
- Produces: `MeetingsRepository::get_distinct_named_speakers(pool, meeting_id) -> Result<Vec<String>, sqlx::Error>`; `pub(crate) async fn save_run_to_obsidian(pool, run_id) -> Result<ObsidianSaveResult, String>`; `ObsidianSaveResult { path: String }`; command `api_save_run_to_obsidian`.

- [ ] **Step 1: Add the attendees query in `meeting.rs`**

Add a method to `MeetingsRepository`:

```rust
    /// Distinct named speakers for a meeting, excluding the "Speaker N" placeholder.
    pub async fn get_distinct_named_speakers(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT speaker FROM transcripts \
             WHERE meeting_id = ? AND speaker IS NOT NULL AND TRIM(speaker) != '' \
             AND speaker NOT GLOB 'Speaker *' ORDER BY speaker",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|(s,)| s).collect())
    }
```

> Note: `GLOB 'Speaker *'` excludes labels like `Speaker 1`. A rare real name literally starting "Speaker " would be filtered; acceptable. Confirm `SqlitePool` is imported in this file (it is used elsewhere in the repo).

- [ ] **Step 2: Write a failing test for the orchestrator**

Add to `commands.rs` test module (imports: `use tempfile::tempdir;` — add `tempfile` as a dev-dependency in Step 3 if missing):

```rust
    #[tokio::test]
    async fn save_run_to_obsidian_writes_file_and_sets_status() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().to_str().unwrap();

        SettingsRepository::save_obsidian_config(&pool, Some(vault), true).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','Sync','2026-07-15T10:00:00Z','2026-07-15T10:00:00Z')")
            .execute(&pool).await.unwrap();

        let input = crate::summary::workflows::models::WorkflowInput {
            id: None, name: "Comp".into(), description: None,
            template_id: "comprehensive_meeting".into(), custom_prompt: None,
            provider: "openrouter".into(), model: "x/y".into(),
            max_tokens: None, temperature: None, top_p: None,
            neohive_export: None,
            obsidian_export: Some(crate::summary::workflows::models::ObsidianExportConfig {
                enabled: true, auto_export: true, subfolder: None, tags: vec![],
            }),
        };
        let wf = WorkflowsRepository::upsert_workflow(&pool, &input).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", Some(&wf.id), &wf.name, "m1").await.unwrap();
        WorkflowsRepository::complete_run(&pool, "r1", "## Overview\nhi", "[]", "completed").await.unwrap();

        let res = super::save_run_to_obsidian(&pool, "r1").await.unwrap();
        assert!(res.path.ends_with(".md"));
        assert!(std::path::Path::new(&res.path).exists());
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.obsidian_status, "saved");
        assert_eq!(run.obsidian_path.as_deref(), Some(res.path.as_str()));
    }

    #[tokio::test]
    async fn save_run_to_obsidian_errors_when_disabled() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','Sync','2026-07-15T10:00:00Z','2026-07-15T10:00:00Z')")
            .execute(&pool).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", None, "W", "m1").await.unwrap();
        WorkflowsRepository::complete_run(&pool, "r1", "x", "[]", "completed").await.unwrap();
        let err = super::save_run_to_obsidian(&pool, "r1").await.unwrap_err();
        assert!(err.to_lowercase().contains("obsidian"));
    }
```

- [ ] **Step 3: Ensure `tempfile` dev-dependency exists**

Run: `cd frontend/src-tauri && grep -q '^tempfile' Cargo.toml && echo present || echo MISSING`
If MISSING, add under `[dev-dependencies]` in `frontend/src-tauri/Cargo.toml`: `tempfile = "3"`. (If a `[dev-dependencies]` section doesn't exist, create it.)

- [ ] **Step 4: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test --lib summary::workflows::commands::tests::save_run_to_obsidian 2>&1 | tail -20`
Expected: FAIL (function not found).

- [ ] **Step 5: Implement `save_run_to_obsidian` + `ObsidianSaveResult` + command in `commands.rs`**

Add imports if missing: `use crate::summary::workflows::models::ObsidianExportConfig;` (already added in Task 4). Add:

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianSaveResult {
    pub path: String,
}

/// Writes a completed run's markdown to the configured Obsidian vault folder.
/// Shared by the manual command and the auto-save hook. Overwrites an existing
/// same-day-same-title note by design (one clean note per meeting).
pub(crate) async fn save_run_to_obsidian(
    pool: &SqlitePool,
    run_id: &str,
) -> Result<ObsidianSaveResult, String> {
    let run = WorkflowsRepository::get_run(pool, run_id)
        .await.map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Run '{}' not found", run_id))?;
    if run.status != WorkflowRunStatus::COMPLETED {
        return Err("Only completed runs can be saved to Obsidian".to_string());
    }

    let obs = SettingsRepository::get_obsidian_config(pool).await.map_err(|e| e.to_string())?;
    if !obs.enabled {
        return Err("Obsidian export is disabled in Settings".to_string());
    }
    let vault = obs.vault_path.filter(|p| !p.trim().is_empty())
        .ok_or("Obsidian vault path is not configured")?;
    let vault_path = std::path::PathBuf::from(&vault);
    if !vault_path.is_dir() {
        return Err(format!("Obsidian vault path does not exist or is not a directory: {}", vault));
    }

    // Per-workflow config (subfolder, tags); default if workflow was deleted.
    let cfg: ObsidianExportConfig = match &run.workflow_id {
        Some(id) => WorkflowsRepository::get_workflow(pool, id).await.ok().flatten()
            .map(|w| w.obsidian_config()).unwrap_or_default(),
        None => ObsidianExportConfig::default(),
    };

    let meeting = MeetingsRepository::get_meeting_metadata(pool, &run.meeting_id)
        .await.map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Meeting '{}' not found", run.meeting_id))?;
    let attendees = MeetingsRepository::get_distinct_named_speakers(pool, &run.meeting_id)
        .await.unwrap_or_default();

    let (filename, contents) = build_obsidian_note(&run, &meeting, &attendees, &cfg);

    // Resolve target dir (vault [+ subfolder]); create; assert it stays inside vault.
    let target_dir = match cfg.subfolder.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(sub) => vault_path.join(sub),
        None => vault_path.clone(),
    };
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("Failed to create folder: {}", e))?;
    let vault_canon = std::fs::canonicalize(&vault_path).map_err(|e| e.to_string())?;
    let target_canon = std::fs::canonicalize(&target_dir).map_err(|e| e.to_string())?;
    if !target_canon.starts_with(&vault_canon) {
        return Err("Resolved Obsidian path escapes the vault folder".to_string());
    }

    let file_path = target_canon.join(&filename);
    std::fs::write(&file_path, contents).map_err(|e| format!("Failed to write note: {}", e))?;
    let path_str = file_path.to_string_lossy().to_string();

    if let Err(e) = WorkflowsRepository::set_run_obsidian_result(pool, run_id, "saved", Some(&path_str)).await {
        log_error!("Failed to update obsidian_status for run {}: {}", run_id, e);
    }
    Ok(ObsidianSaveResult { path: path_str })
}

#[tauri::command]
pub async fn api_save_run_to_obsidian(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<ObsidianSaveResult, String> {
    log_info!("api_save_run_to_obsidian called (run {})", run_id);
    let res = save_run_to_obsidian(state.db_manager.pool(), &run_id).await;
    if res.is_err() {
        // best-effort mark failed so the UI badge reflects it
        let _ = WorkflowsRepository::set_run_obsidian_result(state.db_manager.pool(), &run_id, "failed", None).await;
    }
    res
}
```

> Ensure `MeetingsRepository` is imported in `commands.rs` (it already is — used by `export_run`).

- [ ] **Step 6: Add the auto-save hook in `runner.rs`**

In `run_workflow_background`, inside the `Ok(()) => { ... }` arm after the NeoHive auto-export `if` block (after line ~80), add:

```rust
                    let obs = workflow.obsidian_config();
                    if obs.enabled && obs.auto_export {
                        if let Err(e) = crate::summary::workflows::commands::save_run_to_obsidian(&pool, &run_id).await {
                            tracing::warn!("Auto-save to Obsidian failed for run {}: {}", run_id, e);
                            let _ = crate::summary::workflows::repository::WorkflowsRepository::set_run_obsidian_result(&pool, &run_id, "failed", None).await;
                        }
                    }
```

- [ ] **Step 7: Register the command in `lib.rs`**

After the two Obsidian config commands added in Task 3 (after `api_save_obsidian_config,`), add:

```rust
            summary::workflows::commands::api_save_run_to_obsidian,
```

- [ ] **Step 8: Run tests + build**

Run: `cd frontend/src-tauri && cargo test --lib summary::workflows 2>&1 | tail -25`
Expected: PASS (both new orchestrator tests + all prior).
Run: `cd frontend/src-tauri && cargo build 2>&1 | tail -6`
Expected: builds.

- [ ] **Step 9: Commit**

```bash
git add frontend/src-tauri/src/database/repositories/meeting.rs frontend/src-tauri/src/summary/workflows/commands.rs frontend/src-tauri/src/summary/workflows/runner.rs frontend/src-tauri/src/lib.rs frontend/src-tauri/Cargo.toml
git commit -m "feat(workflows): :sparkles: write completed runs to Obsidian vault (manual + auto)"
```

---

### Task 6: Frontend types

**Files:**
- Modify: `frontend/src/types/workflow.ts`

**Interfaces:**
- Produces: `ObsidianRunStatus`, `ObsidianExportConfig`, `ObsidianSettings`, `ObsidianSaveResult`, `DEFAULT_OBSIDIAN_EXPORT`; extended `Workflow`/`WorkflowInput`/`WorkflowRun`.

- [ ] **Step 1: Install deps (node_modules is missing)**

Run: `cd frontend && pnpm install 2>&1 | tail -5`
Expected: completes; `node_modules` now present.

- [ ] **Step 2: Add the types**

In `frontend/src/types/workflow.ts` add after the NeoHive types:

```ts
export type ObsidianRunStatus = 'none' | 'saved' | 'failed';

export interface ObsidianExportConfig {
  enabled: boolean;
  autoExport: boolean;
  /** Relative subfolder under the configured vault path. */
  subfolder?: string | null;
  /** Extra frontmatter tags on top of the defaults ["meeting","meetily"]. */
  tags: string[];
}

export interface ObsidianSettings {
  vaultPath: string | null;
  enabled: boolean;
}

export interface ObsidianSaveResult {
  path: string;
}

export const DEFAULT_OBSIDIAN_EXPORT: ObsidianExportConfig = {
  enabled: false,
  autoExport: false,
  subfolder: null,
  tags: [],
};
```

Extend the interfaces:
- `Workflow`: add `obsidianExport?: string | null;` (raw JSON string, like `neohiveExport`).
- `WorkflowInput`: add `obsidianExport?: ObsidianExportConfig | null;`.
- `WorkflowRun`: add `obsidianStatus: ObsidianRunStatus;` and `obsidianPath?: string | null;`.

- [ ] **Step 3: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors (the only allowed pre-existing error involves `bun:test`).

- [ ] **Step 4: Commit**

```bash
git add frontend/src/types/workflow.ts
git commit -m "feat(workflows): :sparkles: add Obsidian export TypeScript types"
```

---

### Task 7: "Save to Obsidian" toggle in the workflow editor

**Files:**
- Modify: `frontend/src/components/workflows/WorkflowEditor.tsx`

**Interfaces:**
- Consumes: `ObsidianExportConfig`, `DEFAULT_OBSIDIAN_EXPORT` from `@/types/workflow`.
- Produces: `WorkflowInput.obsidianExport` populated on save.

- [ ] **Step 1: Wire state + save**

In `WorkflowEditor.tsx`:
- Update the import: `import { NeoHiveExportConfig, ObsidianExportConfig, DEFAULT_OBSIDIAN_EXPORT, Workflow, WorkflowInput } from '@/types/workflow';`
- Add state after `exportCfg`:

```tsx
  const [obsidianCfg, setObsidianCfg] = useState<ObsidianExportConfig>(() => {
    if (initial?.obsidianExport) {
      try { return JSON.parse(initial.obsidianExport) as ObsidianExportConfig; } catch { /* fall through */ }
    }
    return DEFAULT_OBSIDIAN_EXPORT;
  });
```

- In `handleSave`, add `obsidianExport: obsidianCfg,` to the `input` object (after `neohiveExport: exportCfg,`).

- [ ] **Step 2: Add the JSX block**

After the NeoHive block (after the `{exportCfg.enabled && (...)}` auto-export section, before the buttons `<div className="flex justify-end ...">`), add:

```tsx
      <div className="flex items-center justify-between border-t pt-3">
        <div>
          <Label>Save summary to folder (Obsidian)</Label>
          <p className="text-xs text-muted-foreground">Writes the run as a markdown note to your configured vault folder.</p>
        </div>
        <Switch
          checked={obsidianCfg.enabled}
          onCheckedChange={(v) => setObsidianCfg((c) => ({ ...c, enabled: v }))}
        />
      </div>

      {obsidianCfg.enabled && (
        <div className="space-y-3 pl-2">
          <div className="flex items-center justify-between">
            <Label className="text-sm">Auto-save when a run completes</Label>
            <Switch
              checked={obsidianCfg.autoExport}
              onCheckedChange={(v) => setObsidianCfg((c) => ({ ...c, autoExport: v }))}
            />
          </div>
          <div className="space-y-1">
            <Label className="text-sm">Subfolder (optional)</Label>
            <Input
              value={obsidianCfg.subfolder ?? ''}
              onChange={(e) => setObsidianCfg((c) => ({ ...c, subfolder: e.target.value || null }))}
              placeholder="e.g. Meeting Notes"
            />
          </div>
        </div>
      )}
```

- [ ] **Step 3: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/workflows/WorkflowEditor.tsx
git commit -m "feat(workflows): :sparkles: add Save-to-Obsidian toggle to workflow editor"
```

---

### Task 8: Run-card "Save to Obsidian" button + status badge + hook handler

**Files:**
- Modify: `frontend/src/hooks/meeting-details/useWorkflowRuns.ts`
- Modify: `frontend/src/components/MeetingDetails/WorkflowRunSection.tsx`
- Modify: `frontend/src/components/MeetingDetails/WorkflowRunCard.tsx`

**Interfaces:**
- Consumes: `ObsidianSaveResult` from `@/types/workflow`; command `api_save_run_to_obsidian`.
- Produces: `useWorkflowRuns().saveToObsidian(runId)`; `WorkflowRunCard` prop `onSaveToObsidian`.

- [ ] **Step 1: Add `saveToObsidian` to the hook**

In `useWorkflowRuns.ts`, add the import `import { ..., ObsidianSaveResult } from '@/types/workflow';` (extend the existing type import), then add after `exportRun`:

```ts
  const saveToObsidian = useCallback(async (runId: string): Promise<ObsidianSaveResult | null> => {
    try {
      const result = await invokeTauri<ObsidianSaveResult>('api_save_run_to_obsidian', { runId });
      toast.success(`Saved to Obsidian: ${result.path}`);
      await refresh();
      return result;
    } catch (err) {
      console.error('Failed to save run to Obsidian:', err);
      toast.error(`Obsidian save failed: ${err instanceof Error ? err.message : String(err)}`);
      return null;
    }
  }, [refresh]);
```

Add `saveToObsidian` to the returned object (the `return { runs, ..., exportRun, activeRunId }` at the end):

```ts
  return { runs, isLoading, refresh, runWorkflow, cancelRun, exportRun, saveToObsidian, activeRunId };
```

- [ ] **Step 2: Pass it through `WorkflowRunSection.tsx`**

Update the destructure: `const { runs, runWorkflow, cancelRun, exportRun, saveToObsidian } = useWorkflowRuns(meetingId);`
Update the card render:

```tsx
          <WorkflowRunCard key={run.id} run={run} onExport={exportRun} onCancel={cancelRun} onSaveToObsidian={saveToObsidian} />
```

- [ ] **Step 3: Add the button + badge in `WorkflowRunCard.tsx`**

- Extend imports: `import { Copy, Send, Loader2, CheckCircle2, XCircle, FolderDown } from 'lucide-react';`
- Extend the props interface:

```tsx
interface WorkflowRunCardProps {
  run: WorkflowRun;
  onExport: (runId: string) => Promise<unknown>;
  onCancel: (runId: string) => Promise<unknown>;
  onSaveToObsidian: (runId: string) => Promise<unknown>;
}
```

- Update the destructure: `export function WorkflowRunCard({ run, onExport, onCancel, onSaveToObsidian }: WorkflowRunCardProps) {`
- Add state + handler after `exporting`:

```tsx
  const [savingObsidian, setSavingObsidian] = useState(false);
  const doSaveObsidian = async () => {
    setSavingObsidian(true);
    try { await onSaveToObsidian(run.id); } finally { setSavingObsidian(false); }
  };
```

- After the NeoHive status badge block (`{run.neohiveStatus !== 'none' && (...)}`), add:

```tsx
          {run.obsidianStatus !== 'none' && (
            <span className="text-xs rounded px-1.5 py-0.5 bg-muted" title={run.obsidianPath ?? ''}>Obsidian: {run.obsidianStatus}</span>
          )}
```

- Inside the `{run.status === 'completed' && (<>...</>)}` button group, after the "Send to NeoHive" button, add:

```tsx
              <Button variant="outline" size="sm" onClick={doSaveObsidian} disabled={savingObsidian}>
                {savingObsidian ? <Loader2 className="h-4 w-4 animate-spin mr-1" /> : <FolderDown className="h-4 w-4 mr-1" />}
                Save to Obsidian
              </Button>
```

- [ ] **Step 4: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/hooks/meeting-details/useWorkflowRuns.ts frontend/src/components/MeetingDetails/WorkflowRunSection.tsx frontend/src/components/MeetingDetails/WorkflowRunCard.tsx
git commit -m "feat(workflows): :sparkles: add Save-to-Obsidian action + status to run cards"
```

---

### Task 9: Obsidian settings section in Settings

**Files:**
- Modify: `frontend/src/components/workflows/WorkflowsSettings.tsx`

**Interfaces:**
- Consumes: `ObsidianSettings` from `@/types/workflow`; commands `api_get_obsidian_config`, `api_save_obsidian_config`.

- [ ] **Step 1: Add state + load/save**

In `WorkflowsSettings.tsx`:
- Extend the type import to include `ObsidianSettings`.
- Add state after the `neo` state:

```tsx
  const [obs, setObs] = useState<ObsidianSettings>({ vaultPath: null, enabled: false });
```

- In the existing `useEffect` (or a new one), load the config:

```tsx
  useEffect(() => {
    invoke<ObsidianSettings>('api_get_obsidian_config')
      .then((cfg) => setObs({ vaultPath: cfg.vaultPath ?? null, enabled: cfg.enabled }))
      .catch((e) => console.error('Failed to load Obsidian config:', e));
  }, []);
```

- Add a save handler after `saveNeo`:

```tsx
  const saveObs = async () => {
    try {
      await invoke('api_save_obsidian_config', { vaultPath: obs.vaultPath || null, enabled: obs.enabled });
      toast.success('Obsidian settings saved');
    } catch (e) {
      console.error('Failed to save Obsidian settings:', e);
      toast.error('Failed to save Obsidian settings');
    }
  };
```

- [ ] **Step 2: Add the settings `<section>`**

Immediately after the NeoHive `</section>` (line ~157) and before the Workflows list section, add:

```tsx
      {/* Obsidian export */}
      <section className="space-y-3 border rounded-lg p-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="font-medium">Obsidian export</h3>
            <p className="text-xs text-muted-foreground">
              Workflow runs with &quot;Save to Obsidian&quot; enabled write a markdown note into this folder.
            </p>
          </div>
          <Switch checked={obs.enabled} onCheckedChange={(v) => setObs((o) => ({ ...o, enabled: v }))} />
        </div>

        <div className="space-y-1">
          <Label>Vault folder path</Label>
          <Input
            value={obs.vaultPath ?? ''}
            onChange={(e) => setObs((o) => ({ ...o, vaultPath: e.target.value }))}
            placeholder="/Users/you/Obsidian/Vault/Meeting Notes"
          />
        </div>

        <div className="flex justify-end">
          <Button size="sm" onClick={saveObs}>Save Obsidian settings</Button>
        </div>
      </section>
```

- [ ] **Step 3: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/workflows/WorkflowsSettings.tsx
git commit -m "feat(workflows): :sparkles: add Obsidian vault settings section"
```

---

## Final verification (after all tasks)

- [ ] Rust: `cd frontend/src-tauri && cargo test --lib summary::workflows 2>&1 | tail -25` — all pass.
- [ ] Rust: `cd frontend/src-tauri && cargo test --lib database::repositories::setting 2>&1 | tail -20` — all pass.
- [ ] Rust: `cd frontend/src-tauri && cargo build 2>&1 | tail -6` — builds.
- [ ] Frontend: `cd frontend && npx tsc --noEmit 2>&1 | tail -15` — no NEW errors.
- [ ] Manual E2E (verify skill / `pnpm tauri:dev`): set a temp vault path in Settings → Workflows; create a workflow using `comprehensive_meeting` with "Save to Obsidian" + auto-save on; run it on a meeting; confirm a correctly-named `.md` with valid frontmatter appears; re-run and confirm it overwrites; use the manual "Save to Obsidian" button on a `technical_decisions` run and confirm it also writes.

## Self-Review (plan vs. spec)

**Spec coverage:**
- §4 three templates → Task 1. ✅
- §5 migration (settings + workflows + runs columns) → Task 2 (workflow/run) + settings columns in Task 2 migration; settings repo in Task 3. ✅
- §5 `ObsidianExportConfig` + `obsidian_config()` + struct fields → Task 2. ✅
- §6 pure builder + sanitizer → Task 4; impure orchestrator + command + auto-hook → Task 5. Folded into `commands.rs` (no new module), per user feedback. ✅
- §6 collision=overwrite (std::fs::write on a stable filename) → Task 5. ✅
- §6 path-traversal guard (canonicalize + starts_with) → Task 5. ✅
- §7 attendees derivation (distinct named speakers, exclude `Speaker N`, omit when empty) → Task 5 query + Task 4 builder omission + test. ✅
- §8 frontend types → Task 6; editor toggle → Task 7; run card button+badge+hook → Task 8; settings section → Task 9. ✅
- §9 tests (sanitize, build_obsidian_note incl. empty-attendees, save roundtrip, disabled error, template validity, settings roundtrip, tsc gate) → Tasks 1–9. ✅
- §10 conventions (Tauri commands, camelCase, api_ naming, register in lib.rs, no secrets, gitmoji no-attribution) → Global Constraints + per-task. ✅

**Placeholder scan:** No TBD/TODO. The two "Note:" callouts (sample_input name; SqlitePool import) are verification prompts against real code with explicit fallbacks, not placeholders. ✅

**Type consistency:** `ObsidianExportConfig` (Rust snake_case fields / TS camelCase) consistent across Tasks 2/4/5/6/7. `set_run_obsidian_result(pool, run_id, status, path)` signature matches all call sites (Tasks 2 def, 5 use, 8 n/a). `save_run_to_obsidian(pool, run_id) -> ObsidianSaveResult{path}` consistent (Task 5 def, hook, command; Task 8 TS `ObsidianSaveResult{path}`). `obsidian_status`/`obsidian_path` (DB, snake) ↔ `obsidianStatus`/`obsidianPath` (TS, camel) via serde rename. `api_save_run_to_obsidian` arg `runId` (JS) → `run_id` (Rust). ✅
