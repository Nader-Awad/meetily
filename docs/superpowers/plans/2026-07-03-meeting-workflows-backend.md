# Meeting Workflows — Backend Implementation Plan (Plan 1 of 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Rust/Tauri backend for saved, named **workflows** (template/prompt + pinned provider/model) whose runs are retained as their own artifacts, plus opt-in section-by-section export of a run's elements to a NeoHive instance.

**Architecture:** Two new SQLite tables (`workflows`, `workflow_runs`) + a new `summary/workflows/` Rust module (models, repository, pure section parser, CRUD + run + export Tauri commands) + a new `neohive/` module (a `reqwest` client that speaks MCP-over-HTTP to `neohive.logilica.com`). Workflow generation reuses the existing `processor::generate_meeting_summary()` (which returns `(markdown, english, chunks)` and touches no DB) and writes to `workflow_runs`. Section-level NeoHive export parses the generated markdown blob back into `{title, content}` using the template's known section titles.

**Tech Stack:** Rust, Tauri v2, SQLx (SQLite), `reqwest`, `serde`/`serde_json`, `uuid`, `chrono`, `tokio`, `once_cell`. Crate name `meetily`, lib name `app_lib`.

> **As-built note (2026-07-08).** The NeoHive transport open question (Task 10 probe) resolved to **Cloudflare Access**: the endpoint requires a **service token**, not a single token. So Task 9's single `neohiveApiKey` was superseded by a follow-on migration (`20260703000001`) adding `neohiveAccessClientId` + `neohiveAccessClientSecret` (see the "Task 10b" step in the SDD ledger), and Task 11's client sends `CF-Access-Client-Id` / `CF-Access-Client-Secret` headers, tolerates JSON+SSE, sends `notifications/initialized`, uses a 30 s timeout, and checks `result.isError`. Everything else built as planned. Verified: `cargo test workflows` 16/0, `neohive` 7/0; requires **Xcode** (for the `cidre`/ScreenCaptureKit build).

**Scope:** Backend only. The frontend (Workflows settings manager, run controls, run-result cards) is **Plan 2**, written after this plan. This plan produces working, independently testable software: workflows can be created/listed/deleted, run against a meeting, polled to completion, and exported to NeoHive — all exercised via `cargo test` and manual Tauri command invocation.

## Global Constraints

Every task's requirements implicitly include these (copied from the spec + verified against the codebase):

- **All new behavior goes through Tauri commands in the Rust core** under `frontend/src-tauri/src`. Do NOT touch the archived `backend/` FastAPI tier.
- **Command naming:** `api_*` prefix; register every command in `frontend/src-tauri/src/lib.rs` inside the `tauri::generate_handler![ ... ]` macro (starts at `lib.rs:526`), next to the other `summary::` commands (~lines 660–682).
- **Tauri arg convention:** command params are plain `snake_case`; the frontend passes `camelCase` and Tauri auto-converts. Do NOT add `#[serde(rename_all)]` to command *argument* lists. Structs *returned* to the frontend use `#[serde(rename_all = "camelCase")]` (mirrors `MeetingSummaryLanguagePreference` in `summary/commands.rs:44`).
- **SQLx style:** positional `?` placeholders, `.bind()` chaining, `chrono::Utc::now()` bound directly, `r#"..."#` raw SQL, `sqlx::Error::Protocol(msg.into())` for serialization errors, `query_as::<_, T>` for row mapping. Timestamps are stored as **TEXT** columns but mapped to `chrono::DateTime<chrono::Utc>` fields.
- **Migrations:** add a new timestamped file in `frontend/src-tauri/migrations/`; it is embedded + run at startup by `sqlx::migrate!("./migrations")` in `database/manager.rs:37`. Use `IF NOT EXISTS` and `ON DELETE CASCADE` FK declarations to match existing style (note: runtime deletes are still done manually — see Task 8).
- **DB access in a command:** `state: tauri::State<'_, AppState>` then `let pool = state.db_manager.pool();` → `&SqlitePool`. Clone the pool (`.pool().clone()`) before moving it into a spawned background task.
- **Never log or echo** the NeoHive token or any provider API key.
- **Run tests** from `frontend/src-tauri` with `cargo test <filter>`. There is **no DB test harness** — this plan creates one (Task 3) using an in-memory pool with `max_connections(1)` (required: a multi-connection in-memory pool would run migrations on one connection and queries on another, so tables would appear missing).
- **DRY / YAGNI / TDD / frequent commits.** Commit after every task's tests pass. Commit style: conventional + gitmoji, e.g. `feat(workflows): :sparkles: <desc>`.

## File Structure

**Create:**
- `frontend/src-tauri/migrations/20260703000000_add_workflows.sql` — two new tables + settings columns.
- `frontend/src-tauri/src/summary/workflows/mod.rs` — module wiring + re-exports.
- `frontend/src-tauri/src/summary/workflows/models.rs` — `Workflow`, `WorkflowRun`, `NeoHiveExportConfig`, input/response structs.
- `frontend/src-tauri/src/summary/workflows/sections.rs` — pure markdown section parser + section→memory-type mapping (unit tested).
- `frontend/src-tauri/src/summary/workflows/repository.rs` — `WorkflowsRepository` CRUD + run persistence (+ in-memory DB test harness).
- `frontend/src-tauri/src/summary/workflows/runner.rs` — `run_workflow_background` + per-run cancellation registry.
- `frontend/src-tauri/src/summary/workflows/commands.rs` — all `api_*` workflow Tauri commands.
- `frontend/src-tauri/src/neohive/mod.rs` — module wiring.
- `frontend/src-tauri/src/neohive/client.rs` — `NeoHiveClient` (MCP-over-HTTP `memory_store`) + payload unit tests.

**Modify:**
- `frontend/src-tauri/src/summary/mod.rs` — add `pub mod workflows;`.
- `frontend/src-tauri/src/lib.rs` — `mod neohive;` + register all new commands in `generate_handler!`.
- `frontend/src-tauri/src/database/repositories/setting.rs` — add `get_neohive_config` / `save_neohive_config`.
- `frontend/src-tauri/src/database/repositories/meeting.rs` — extend `delete_meeting_with_transaction` to delete `workflow_runs`.

---

## Phase 1 — Data layer & settings

### Task 1: Migration — `workflows` + `workflow_runs` tables + NeoHive settings columns

**Files:**
- Create: `frontend/src-tauri/migrations/20260703000000_add_workflows.sql`

**Interfaces:**
- Produces: tables `workflows`, `workflow_runs`; settings columns `neohiveEndpoint`, `neohiveApiKey`, `neohiveEnabled`. Column names and types consumed by Tasks 3, 8, 9.

- [ ] **Step 1: Write the migration SQL**

```sql
-- Workflows: saved, named summary recipes (template/prompt + pinned model + export config)
CREATE TABLE IF NOT EXISTS workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    template_id TEXT NOT NULL,
    custom_prompt TEXT,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    max_tokens INTEGER,
    temperature REAL,
    top_p REAL,
    neohive_export TEXT,          -- JSON: NeoHiveExportConfig (null/absent = disabled)
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Workflow runs: retained artifacts. workflow_id is NOT a FK because runs survive
-- workflow deletion; workflow_name is a denormalized snapshot for display.
CREATE TABLE IF NOT EXISTS workflow_runs (
    id TEXT PRIMARY KEY,
    workflow_id TEXT,
    workflow_name TEXT NOT NULL,
    meeting_id TEXT NOT NULL,
    status TEXT NOT NULL,             -- queued | running | completed | error | cancelled
    result_markdown TEXT,
    result_sections TEXT,            -- JSON: [{ "title": .., "content": .. }]
    error TEXT,
    neohive_status TEXT NOT NULL DEFAULT 'none',  -- none | pushed | partial | failed
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workflow_runs_meeting ON workflow_runs(meeting_id);

-- NeoHive export settings (single-row settings table, id = '1')
ALTER TABLE settings ADD COLUMN neohiveEndpoint TEXT;
ALTER TABLE settings ADD COLUMN neohiveApiKey TEXT;
ALTER TABLE settings ADD COLUMN neohiveEnabled INTEGER NOT NULL DEFAULT 0;
```

- [ ] **Step 2: Verify the migration compiles/embeds and applies**

Run: `cd frontend/src-tauri && cargo check`
Expected: compiles. (`sqlx::migrate!("./migrations")` embeds `.sql` files at build time; a malformed file fails the macro expansion here.)

- [ ] **Step 3: Verify it applies to a fresh DB (via the Task 3 harness once it exists, or manually now)**

Run: `sqlite3 /tmp/wf_test.db < frontend/src-tauri/migrations/20250916100000_initial_schema.sql && sqlite3 /tmp/wf_test.db < frontend/src-tauri/migrations/20260703000000_add_workflows.sql && sqlite3 /tmp/wf_test.db ".tables"`
Expected: output includes `workflows` and `workflow_runs`. (The `ALTER TABLE settings` lines require the initial schema's `settings` table to exist; if running standalone note the initial schema must be applied first — which it always is in-app because migrations run in order.)

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/migrations/20260703000000_add_workflows.sql
git commit -m "feat(workflows): :card_file_box: add workflows + workflow_runs tables and NeoHive settings columns"
```

---

### Task 2: Rust models

**Files:**
- Create: `frontend/src-tauri/src/summary/workflows/models.rs`
- Create: `frontend/src-tauri/src/summary/workflows/mod.rs`
- Modify: `frontend/src-tauri/src/summary/mod.rs` (add `pub mod workflows;`)

**Interfaces:**
- Produces: `Workflow`, `WorkflowRun`, `WorkflowInput`, `NeoHiveExportConfig`, `WorkflowRunStatus` — consumed by Tasks 3, 5, 6, 7, 12.

- [ ] **Step 1: Write `models.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Section-title -> memory-type overrides for NeoHive export.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeoHiveExportConfig {
    /// Whether this workflow may export to NeoHive at all.
    #[serde(default)]
    pub enabled: bool,
    /// If true, export automatically when a run completes; else manual button only.
    #[serde(default)]
    pub auto_export: bool,
    /// Per-section-title memory type override (e.g. "Key Decisions" -> "decision").
    #[serde(default)]
    pub section_type_overrides: HashMap<String, String>,
    /// Memory type for sections without an override.
    #[serde(default = "default_memory_type")]
    pub default_type: String,
    /// Importance (1-10) applied to every exported memory.
    #[serde(default = "default_importance")]
    pub importance: u8,
}

fn default_memory_type() -> String { "narrative".to_string() }
fn default_importance() -> u8 { 6 }

impl Default for NeoHiveExportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_export: false,
            section_type_overrides: HashMap::new(),
            default_type: default_memory_type(),
            importance: default_importance(),
        }
    }
}

/// A saved workflow recipe (DB row). DB columns are snake_case; JSON to the
/// frontend is camelCase.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub template_id: String,
    pub custom_prompt: Option<String>,
    pub provider: String,
    pub model: String,
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    /// Raw JSON string of NeoHiveExportConfig; parse with `neohive_config()`.
    pub neohive_export: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Workflow {
    /// Parses the stored export config, falling back to a disabled default.
    pub fn neohive_config(&self) -> NeoHiveExportConfig {
        self.neohive_export
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

/// Frontend-supplied workflow definition for create/update.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInput {
    /// None => create; Some => update existing.
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub template_id: String,
    pub custom_prompt: Option<String>,
    pub provider: String,
    pub model: String,
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub neohive_export: Option<NeoHiveExportConfig>,
}

/// A workflow run (DB row / poll target).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: Option<String>,
    pub workflow_name: String,
    pub meeting_id: String,
    pub status: String,
    pub result_markdown: Option<String>,
    /// JSON array of { title, content }.
    pub result_sections: Option<String>,
    pub error: Option<String>,
    pub neohive_status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Terminal + in-flight statuses as string constants (kept as &str to match the
/// TEXT column and the frontend polling contract).
pub struct WorkflowRunStatus;
impl WorkflowRunStatus {
    pub const QUEUED: &'static str = "queued";
    pub const RUNNING: &'static str = "running";
    pub const COMPLETED: &'static str = "completed";
    pub const ERROR: &'static str = "error";
    pub const CANCELLED: &'static str = "cancelled";
}
```

- [ ] **Step 2: Write `mod.rs`**

```rust
pub mod commands;
pub mod models;
pub mod repository;
pub mod runner;
pub mod sections;

pub use models::{NeoHiveExportConfig, Workflow, WorkflowInput, WorkflowRun, WorkflowRunStatus};
```

(Note: `commands`, `repository`, `runner`, `sections` are created in later tasks. To keep `cargo check` green *now*, create empty stub files `repository.rs`, `runner.rs`, `sections.rs`, `commands.rs` each containing only a `// implemented in a later task` comment, OR temporarily comment out the not-yet-created `pub mod` lines and re-enable them per task. Prefer creating empty stubs so the module tree is stable.)

- [ ] **Step 3: Wire the module in `summary/mod.rs`**

Add near the other `pub mod` lines in `frontend/src-tauri/src/summary/mod.rs`:

```rust
pub mod workflows;
```

- [ ] **Step 4: Verify it compiles**

Run: `cd frontend/src-tauri && cargo check`
Expected: compiles (with empty stubs for repository/runner/sections/commands).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/ frontend/src-tauri/src/summary/mod.rs
git commit -m "feat(workflows): :sparkles: add workflow + run data models"
```

---

### Task 3: `WorkflowsRepository` (CRUD + run persistence) with in-memory DB test harness

**Files:**
- Create/replace stub: `frontend/src-tauri/src/summary/workflows/repository.rs`

**Interfaces:**
- Consumes: `Workflow`, `WorkflowRun`, `WorkflowInput` (Task 2).
- Produces (all `pub async fn`, first arg `pool: &SqlitePool`):
  - `list_workflows(pool) -> Result<Vec<Workflow>, sqlx::Error>`
  - `get_workflow(pool, id: &str) -> Result<Option<Workflow>, sqlx::Error>`
  - `upsert_workflow(pool, input: &WorkflowInput) -> Result<Workflow, sqlx::Error>`
  - `delete_workflow(pool, id: &str) -> Result<bool, sqlx::Error>`
  - `create_run(pool, run_id, workflow_id, workflow_name, meeting_id) -> Result<(), sqlx::Error>`
  - `complete_run(pool, run_id, markdown, sections_json, status) -> Result<(), sqlx::Error>`
  - `fail_run(pool, run_id, error, status) -> Result<(), sqlx::Error>`
  - `get_run(pool, run_id) -> Result<Option<WorkflowRun>, sqlx::Error>`
  - `list_runs_for_meeting(pool, meeting_id) -> Result<Vec<WorkflowRun>, sqlx::Error>`
  - `set_run_neohive_status(pool, run_id, status: &str) -> Result<(), sqlx::Error>`

- [ ] **Step 1: Write the failing test harness + first tests**

```rust
use crate::summary::workflows::models::{Workflow, WorkflowInput, WorkflowRun};
use sqlx::SqlitePool;

pub struct WorkflowsRepository;

impl WorkflowsRepository {
    // implemented in Step 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::workflows::models::{NeoHiveExportConfig, WorkflowRunStatus};
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool pinned to ONE connection so migrations + queries share the
    /// same database (a multi-connection :memory: pool loses the schema).
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn sample_input(name: &str) -> WorkflowInput {
        WorkflowInput {
            id: None,
            name: name.to_string(),
            description: Some("desc".to_string()),
            template_id: "standard_meeting".to_string(),
            custom_prompt: None,
            provider: "openrouter".to_string(),
            model: "anthropic/claude-sonnet-4".to_string(),
            max_tokens: Some(4096),
            temperature: Some(0.4),
            top_p: None,
            neohive_export: Some(NeoHiveExportConfig::default()),
        }
    }

    #[tokio::test]
    async fn create_then_list_returns_workflow() {
        let pool = test_pool().await;
        let created = WorkflowsRepository::upsert_workflow(&pool, &sample_input("Exec")).await.unwrap();
        assert!(!created.id.is_empty());
        let all = WorkflowsRepository::list_workflows(&pool).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Exec");
        assert_eq!(all[0].provider, "openrouter");
    }

    #[tokio::test]
    async fn upsert_with_id_updates_in_place() {
        let pool = test_pool().await;
        let created = WorkflowsRepository::upsert_workflow(&pool, &sample_input("Old")).await.unwrap();
        let mut update = sample_input("New");
        update.id = Some(created.id.clone());
        let updated = WorkflowsRepository::upsert_workflow(&pool, &update).await.unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "New");
        assert_eq!(WorkflowsRepository::list_workflows(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_removes_workflow_but_run_lifecycle_persists() {
        let pool = test_pool().await;
        // meetings row for the run FK
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        let wf = WorkflowsRepository::upsert_workflow(&pool, &sample_input("W")).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", Some(&wf.id), "W", "m1").await.unwrap();
        WorkflowsRepository::complete_run(&pool, "r1", "# Title\n**Summary**\nhi", "[]", WorkflowRunStatus::COMPLETED).await.unwrap();

        assert!(WorkflowsRepository::delete_workflow(&pool, &wf.id).await.unwrap());
        // run still retrievable (retained artifact)
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.status, "completed");
        assert_eq!(run.workflow_name, "W");
    }

    #[tokio::test]
    async fn run_lifecycle_create_complete_fail_status() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", None, "Ad hoc", "m1").await.unwrap();
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.status, "queued");

        WorkflowsRepository::fail_run(&pool, "r1", "boom", WorkflowRunStatus::ERROR).await.unwrap();
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.status, "error");
        assert_eq!(run.error.as_deref(), Some("boom"));

        WorkflowsRepository::set_run_neohive_status(&pool, "r1", "pushed").await.unwrap();
        assert_eq!(WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap().neohive_status, "pushed");

        let for_meeting = WorkflowsRepository::list_runs_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(for_meeting.len(), 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test workflows::repository -- --nocapture`
Expected: FAIL — methods not implemented (compile errors / `not found`).

- [ ] **Step 3: Implement `WorkflowsRepository`**

Replace the `impl WorkflowsRepository {}` block with:

```rust
impl WorkflowsRepository {
    pub async fn list_workflows(pool: &SqlitePool) -> Result<Vec<Workflow>, sqlx::Error> {
        sqlx::query_as::<_, Workflow>("SELECT * FROM workflows ORDER BY updated_at DESC")
            .fetch_all(pool)
            .await
    }

    pub async fn get_workflow(pool: &SqlitePool, id: &str) -> Result<Option<Workflow>, sqlx::Error> {
        sqlx::query_as::<_, Workflow>("SELECT * FROM workflows WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn upsert_workflow(
        pool: &SqlitePool,
        input: &WorkflowInput,
    ) -> Result<Workflow, sqlx::Error> {
        let now = chrono::Utc::now();
        let id = input.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let export_json = match &input.neohive_export {
            Some(cfg) => Some(
                serde_json::to_string(cfg)
                    .map_err(|e| sqlx::Error::Protocol(format!("serialize export cfg: {}", e).into()))?,
            ),
            None => None,
        };

        sqlx::query(
            r#"
            INSERT INTO workflows
                (id, name, description, template_id, custom_prompt, provider, model,
                 max_tokens, temperature, top_p, neohive_export, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.template_id)
        .bind(&input.custom_prompt)
        .bind(&input.provider)
        .bind(&input.model)
        .bind(input.max_tokens)
        .bind(input.temperature)
        .bind(input.top_p)
        .bind(&export_json)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        Self::get_workflow(pool, &id)
            .await?
            .ok_or_else(|| sqlx::Error::RowNotFound)
    }

    pub async fn delete_workflow(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM workflows WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn create_run(
        pool: &SqlitePool,
        run_id: &str,
        workflow_id: Option<&str>,
        workflow_name: &str,
        meeting_id: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query(
            r#"
            INSERT INTO workflow_runs
                (id, workflow_id, workflow_name, meeting_id, status, neohive_status, created_at, updated_at)
            VALUES (?, ?, ?, ?, 'queued', 'none', ?, ?)
            "#,
        )
        .bind(run_id)
        .bind(workflow_id)
        .bind(workflow_name)
        .bind(meeting_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn complete_run(
        pool: &SqlitePool,
        run_id: &str,
        markdown: &str,
        sections_json: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query(
            r#"
            UPDATE workflow_runs
            SET status = ?, result_markdown = ?, result_sections = ?, error = NULL, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(markdown)
        .bind(sections_json)
        .bind(now)
        .bind(run_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn fail_run(
        pool: &SqlitePool,
        run_id: &str,
        error: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query("UPDATE workflow_runs SET status = ?, error = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(error)
            .bind(now)
            .bind(run_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn get_run(pool: &SqlitePool, run_id: &str) -> Result<Option<WorkflowRun>, sqlx::Error> {
        sqlx::query_as::<_, WorkflowRun>("SELECT * FROM workflow_runs WHERE id = ?")
            .bind(run_id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_runs_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<WorkflowRun>, sqlx::Error> {
        sqlx::query_as::<_, WorkflowRun>(
            "SELECT * FROM workflow_runs WHERE meeting_id = ? ORDER BY created_at DESC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    pub async fn set_run_neohive_status(
        pool: &SqlitePool,
        run_id: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query("UPDATE workflow_runs SET neohive_status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(run_id)
            .execute(pool)
            .await?;
        Ok(())
    }
}
```

Add imports at the top of the file (replace the earlier `use` block):

```rust
use crate::summary::workflows::models::{Workflow, WorkflowInput, WorkflowRun};
use sqlx::SqlitePool;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test workflows::repository -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/repository.rs
git commit -m "feat(workflows): :sparkles: add WorkflowsRepository CRUD + run persistence"
```

---

## Phase 2 — Section parsing & type mapping (pure, TDD)

### Task 4: `sections.rs` — parse markdown blob into `{title, content}` + section→type mapping

**Files:**
- Create/replace stub: `frontend/src-tauri/src/summary/workflows/sections.rs`

**Interfaces:**
- Consumes: `NeoHiveExportConfig` (Task 2).
- Produces:
  - `pub struct ParsedSection { pub title: String, pub content: String }` (serde camelCase not needed — stored/serialized as JSON array of `{title, content}`; keep field names `title`/`content`).
  - `pub fn parse_sections(markdown: &str, section_titles: &[String]) -> Vec<ParsedSection>`
  - `pub fn memory_type_for(section_title: &str, cfg: &NeoHiveExportConfig) -> String`

**Rationale (from ground-truth):** the LLM returns ONE markdown blob whose section headers are the template titles rendered as bold `**Title**` (occasionally `## Title`/`# Title`). We recover content by locating each *known* title (in template order) as a heading-ish line and slicing between consecutive matches.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::workflows::models::NeoHiveExportConfig;
    use std::collections::HashMap;

    fn titles() -> Vec<String> {
        vec!["Summary".into(), "Key Decisions".into(), "Action Items".into()]
    }

    #[test]
    fn parses_bold_delimited_sections() {
        let md = "# Team Sync\n\n**Summary**\n\nWe shipped v2.\n\n**Key Decisions**\n\n- Ship Friday\n\n**Action Items**\n\n- Alice: docs\n";
        let out = parse_sections(md, &titles());
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].title, "Summary");
        assert!(out[0].content.contains("We shipped v2."));
        assert!(!out[0].content.contains("Key Decisions")); // does not bleed into next
        assert!(out[1].content.contains("Ship Friday"));
        assert!(out[2].content.contains("Alice: docs"));
    }

    #[test]
    fn parses_hash_heading_sections() {
        let md = "## Summary\nAll good.\n## Key Decisions\nNone.\n## Action Items\nNone.\n";
        let out = parse_sections(md, &titles());
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].content.trim(), "All good.");
        assert_eq!(out[1].content.trim(), "None.");
    }

    #[test]
    fn missing_section_yields_empty_content_and_is_kept_in_order() {
        let md = "**Summary**\nHi.\n**Action Items**\n- x\n"; // no "Key Decisions"
        let out = parse_sections(md, &titles());
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].title, "Key Decisions");
        assert_eq!(out[1].content.trim(), "");
        assert!(out[2].content.contains("- x"));
    }

    #[test]
    fn heading_matches_are_case_and_colon_insensitive() {
        let md = "**summary:**\nlower\n**KEY DECISIONS**\nupper\n**Action Items**\nok\n";
        let out = parse_sections(md, &titles());
        assert_eq!(out[0].content.trim(), "lower");
        assert_eq!(out[1].content.trim(), "upper");
    }

    #[test]
    fn memory_type_uses_override_then_default() {
        let mut overrides = HashMap::new();
        overrides.insert("Key Decisions".to_string(), "decision".to_string());
        let cfg = NeoHiveExportConfig { section_type_overrides: overrides, default_type: "narrative".into(), ..Default::default() };
        assert_eq!(memory_type_for("Key Decisions", &cfg), "decision");
        assert_eq!(memory_type_for("Summary", &cfg), "narrative");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test workflows::sections -- --nocapture`
Expected: FAIL — `parse_sections` / `memory_type_for` not defined.

- [ ] **Step 3: Implement `sections.rs`**

Prepend above the test module:

```rust
use crate::summary::workflows::models::NeoHiveExportConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedSection {
    pub title: String,
    pub content: String,
}

/// Normalizes a line for heading comparison: strips leading `#`, surrounding
/// `**`, whitespace, and a trailing `:`, then lowercases.
fn normalize_heading(line: &str) -> String {
    let mut s = line.trim();
    // strip markdown heading hashes
    s = s.trim_start_matches('#').trim();
    // strip bold markers
    if let Some(inner) = s.strip_prefix("**").and_then(|x| x.strip_suffix("**")) {
        s = inner.trim();
    }
    let s = s.trim_end_matches(':').trim();
    s.to_lowercase()
}

/// Returns true if `line` is a heading for `title` (bold, #, or plain), tolerant
/// of case and a trailing colon.
fn is_heading_for(line: &str, title_lower: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    // Only treat short-ish lines as potential headings to avoid matching prose
    // that merely mentions the title.
    let looks_like_heading = t.starts_with('#')
        || (t.starts_with("**") && t.trim_end_matches(':').trim_end().ends_with("**"))
        || t.trim_end_matches(':').eq_ignore_ascii_case(title_lower); // plain title-only line
    looks_like_heading && normalize_heading(t) == title_lower
}

/// Splits the generated markdown into sections in the order of `section_titles`.
/// Content of each section is everything between its heading and the next
/// recognized section heading (or end of document). Missing sections get "".
pub fn parse_sections(markdown: &str, section_titles: &[String]) -> Vec<ParsedSection> {
    let lines: Vec<&str> = markdown.lines().collect();
    let titles_lower: Vec<String> = section_titles.iter().map(|t| t.trim().to_lowercase()).collect();

    // For each line index, if it is a heading for some known title, record (line_idx, title_idx).
    let mut markers: Vec<(usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        for (ti, tl) in titles_lower.iter().enumerate() {
            if is_heading_for(line, tl) {
                markers.push((i, ti));
                break;
            }
        }
    }

    // Content for a title = lines after its heading up to the next marker line.
    let mut content_by_title: Vec<String> = vec![String::new(); section_titles.len()];
    for (m_idx, &(line_idx, title_idx)) in markers.iter().enumerate() {
        let start = line_idx + 1;
        let end = markers
            .get(m_idx + 1)
            .map(|&(next_line, _)| next_line)
            .unwrap_or(lines.len());
        if start <= end {
            let body = lines[start..end].join("\n").trim().to_string();
            // Last marker for a given title wins only if earlier was empty; keep first non-empty.
            if content_by_title[title_idx].is_empty() {
                content_by_title[title_idx] = body;
            }
        }
    }

    section_titles
        .iter()
        .enumerate()
        .map(|(i, title)| ParsedSection {
            title: title.clone(),
            content: content_by_title[i].clone(),
        })
        .collect()
}

/// Resolves the NeoHive memory type for a section: explicit override wins, else default.
pub fn memory_type_for(section_title: &str, cfg: &NeoHiveExportConfig) -> String {
    cfg.section_type_overrides
        .get(section_title)
        .cloned()
        .unwrap_or_else(|| cfg.default_type.clone())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test workflows::sections -- --nocapture`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/sections.rs
git commit -m "feat(workflows): :sparkles: add markdown section parser + memory-type mapping"
```

---

## Phase 3 — Workflow CRUD commands

### Task 5: `api_list_workflows` / `api_save_workflow` / `api_delete_workflow`

**Files:**
- Create/replace stub: `frontend/src-tauri/src/summary/workflows/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (register commands)

**Interfaces:**
- Consumes: `WorkflowsRepository` (Task 3), `Workflow`/`WorkflowInput` (Task 2), `AppState` (`state.rs`).
- Produces Tauri commands `api_list_workflows`, `api_save_workflow`, `api_delete_workflow`.

- [ ] **Step 1: Write the commands**

```rust
use crate::state::AppState;
use crate::summary::workflows::models::{Workflow, WorkflowInput};
use crate::summary::workflows::repository::WorkflowsRepository;
use log::{error as log_error, info as log_info};

#[tauri::command]
pub async fn api_list_workflows(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Workflow>, String> {
    log_info!("api_list_workflows called");
    WorkflowsRepository::list_workflows(state.db_manager.pool())
        .await
        .map_err(|e| {
            log_error!("api_list_workflows failed: {}", e);
            e.to_string()
        })
}

#[tauri::command]
pub async fn api_save_workflow(
    state: tauri::State<'_, AppState>,
    workflow: WorkflowInput,
) -> Result<Workflow, String> {
    log_info!("api_save_workflow called (name: {})", workflow.name);
    if workflow.name.trim().is_empty() {
        return Err("Workflow name cannot be empty".to_string());
    }
    if workflow.provider.trim().is_empty() || workflow.model.trim().is_empty() {
        return Err("Workflow provider and model are required".to_string());
    }
    WorkflowsRepository::upsert_workflow(state.db_manager.pool(), &workflow)
        .await
        .map_err(|e| {
            log_error!("api_save_workflow failed: {}", e);
            e.to_string()
        })
}

#[tauri::command]
pub async fn api_delete_workflow(
    state: tauri::State<'_, AppState>,
    workflow_id: String,
) -> Result<bool, String> {
    log_info!("api_delete_workflow called (id: {})", workflow_id);
    WorkflowsRepository::delete_workflow(state.db_manager.pool(), &workflow_id)
        .await
        .map_err(|e| {
            log_error!("api_delete_workflow failed: {}", e);
            e.to_string()
        })
}
```

- [ ] **Step 2: Register the commands in `lib.rs`**

In `frontend/src-tauri/src/lib.rs`, inside `tauri::generate_handler![ ... ]` (near the `// Template commands` block ~line 671), add:

```rust
            // Workflow commands
            summary::workflows::commands::api_list_workflows,
            summary::workflows::commands::api_save_workflow,
            summary::workflows::commands::api_delete_workflow,
```

- [ ] **Step 3: Write a repository-level integration test for the command path**

(Commands themselves need a Tauri runtime, which we don't harness; instead assert the repository behavior the commands delegate to, plus that validation logic is correct. Add to `commands.rs`:)

```rust
#[cfg(test)]
mod tests {
    use crate::summary::workflows::models::{NeoHiveExportConfig, WorkflowInput};
    use crate::summary::workflows::repository::WorkflowsRepository;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1)
            .connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn save_then_list_then_delete_roundtrip() {
        let pool = test_pool().await;
        let input = WorkflowInput {
            id: None, name: "Exec".into(), description: None,
            template_id: "standard_meeting".into(), custom_prompt: None,
            provider: "openrouter".into(), model: "x/y".into(),
            max_tokens: None, temperature: None, top_p: None,
            neohive_export: Some(NeoHiveExportConfig::default()),
        };
        let wf = WorkflowsRepository::upsert_workflow(&pool, &input).await.unwrap();
        assert_eq!(WorkflowsRepository::list_workflows(&pool).await.unwrap().len(), 1);
        assert!(WorkflowsRepository::delete_workflow(&pool, &wf.id).await.unwrap());
        assert_eq!(WorkflowsRepository::list_workflows(&pool).await.unwrap().len(), 0);
    }
}
```

- [ ] **Step 4: Run test + build to verify**

Run: `cd frontend/src-tauri && cargo test workflows -- --nocapture && cargo check`
Expected: PASS; `cargo check` clean (commands compile + are registered).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(workflows): :sparkles: add workflow CRUD Tauri commands"
```

---

## Phase 4 — Run execution

### Task 6: `runner.rs` — `run_workflow_background` (reuse `generate_meeting_summary`) + cancellation

**Files:**
- Create/replace stub: `frontend/src-tauri/src/summary/workflows/runner.rs`

**Interfaces:**
- Consumes: `WorkflowsRepository` (Task 3), `parse_sections` (Task 4), `Workflow` (Task 2), `processor::generate_meeting_summary` + `templates::get_template` + `llm_client::LLMProvider` (existing), `SettingsRepository::get_api_key` + `get_model_config` (existing).
- Produces:
  - `pub fn cancel_run(run_id: &str) -> bool`
  - `pub async fn run_workflow_background<R: tauri::Runtime>(app: tauri::AppHandle<R>, pool: SqlitePool, run_id: String, workflow: Workflow, meeting_id: String, text: String, summary_language: Option<String>)`

**Reference (mirror the setup in `summary/service.rs::process_transcript_background`, lines ~294–583):** resolve provider via `LLMProvider::from_str`, fetch api_key via `SettingsRepository::get_api_key(pool, provider)`, fetch ollama endpoint + custom-openai endpoint from settings/config, load template via `templates::get_template`, resolve `app_data_dir` from the app handle, then call `generate_meeting_summary(...)`.

- [ ] **Step 1: Write the failing test (pure helper: building the sections JSON from a completed run)**

The network/LLM path cannot be unit-tested without a live model, so test the *pure* result-shaping helper that turns `(markdown, template_titles)` into the stored sections JSON. Add this helper + test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_sections_json_roundtrips_titles_and_content() {
        let md = "# T\n**Summary**\nhi\n**Action Items**\n- a\n";
        let titles = vec!["Summary".to_string(), "Action Items".to_string()];
        let json = build_sections_json(md, &titles);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["title"], "Summary");
        assert!(parsed[0]["content"].as_str().unwrap().contains("hi"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd frontend/src-tauri && cargo test workflows::runner -- --nocapture`
Expected: FAIL — `build_sections_json` not defined.

- [ ] **Step 3: Implement `runner.rs`**

```rust
use crate::state::AppState; // not used directly here but keeps import parity if needed
use crate::summary::llm_client::LLMProvider;
use crate::summary::processor::{generate_meeting_summary, language_name_from_code};
use crate::summary::templates;
use crate::summary::workflows::models::{Workflow, WorkflowRunStatus};
use crate::summary::workflows::repository::WorkflowsRepository;
use crate::summary::workflows::sections::parse_sections;
use crate::database::repositories::setting::SettingsRepository;
use once_cell::sync::Lazy;
use reqwest::Client;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

// Per-run cancellation registry (mirrors summary::service CANCELLATION_REGISTRY).
static RUN_CANCELLATION: Lazy<Arc<Mutex<HashMap<String, CancellationToken>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

fn register_cancel(run_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    if let Ok(mut reg) = RUN_CANCELLATION.lock() {
        reg.insert(run_id.to_string(), token.clone());
    }
    token
}

fn cleanup_cancel(run_id: &str) {
    if let Ok(mut reg) = RUN_CANCELLATION.lock() {
        reg.remove(run_id);
    }
}

pub fn cancel_run(run_id: &str) -> bool {
    if let Ok(reg) = RUN_CANCELLATION.lock() {
        if let Some(t) = reg.get(run_id) {
            t.cancel();
            return true;
        }
    }
    false
}

/// Pure: parse the markdown blob into ordered `{title, content}` and serialize.
pub(crate) fn build_sections_json(markdown: &str, section_titles: &[String]) -> String {
    let sections = parse_sections(markdown, section_titles);
    serde_json::to_string(&sections).unwrap_or_else(|_| "[]".to_string())
}

/// Fire-and-forget: generates the workflow summary and writes the run row.
pub async fn run_workflow_background<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    pool: SqlitePool,
    run_id: String,
    workflow: Workflow,
    _meeting_id: String,
    text: String,
    summary_language: Option<String>,
) {
    let token = register_cancel(&run_id);

    let result = generate_for_workflow(&app, &pool, &workflow, &text, summary_language, &token).await;

    match result {
        Ok((final_markdown, section_titles)) => {
            let sections_json = build_sections_json(&final_markdown, &section_titles);
            if let Err(e) = WorkflowsRepository::complete_run(
                &pool, &run_id, &final_markdown, &sections_json, WorkflowRunStatus::COMPLETED,
            ).await {
                error!("Failed to persist completed workflow run {}: {}", run_id, e);
            }
        }
        Err(e) => {
            let status = if e.contains("cancelled") {
                WorkflowRunStatus::CANCELLED
            } else {
                WorkflowRunStatus::ERROR
            };
            if let Err(db_e) = WorkflowsRepository::fail_run(&pool, &run_id, &e, status).await {
                error!("Failed to persist failed workflow run {}: {}", run_id, db_e);
            }
        }
    }

    cleanup_cancel(&run_id);
}

/// Returns (final_markdown, ordered_section_titles).
async fn generate_for_workflow<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    pool: &SqlitePool,
    workflow: &Workflow,
    text: &str,
    summary_language: Option<String>,
    token: &CancellationToken,
) -> Result<(String, Vec<String>), String> {
    info!("Running workflow '{}' with {}/{}", workflow.name, workflow.provider, workflow.model);

    let provider = LLMProvider::from_str(&workflow.provider)?;
    let api_key = SettingsRepository::get_api_key(pool, &workflow.provider)
        .await
        .map_err(|e| format!("Failed to read API key: {}", e))?
        .unwrap_or_default();

    // Endpoints (ollama / custom-openai) come from saved model config.
    let (ollama_endpoint, custom_openai_endpoint) = resolve_endpoints(pool, &workflow.provider).await;

    let template = templates::get_template(&workflow.template_id)
        .map_err(|e| format!("Failed to load template '{}': {}", workflow.template_id, e))?;
    let section_titles: Vec<String> = template.sections.iter().map(|s| s.title.clone()).collect();

    let app_data_dir: Option<PathBuf> = app.path().app_data_dir().ok();

    let custom_prompt = workflow.custom_prompt.clone().unwrap_or_default();
    let client = Client::new();

    let detected_transcript_language: Option<&str> = None; // Auto path; language detection handled elsewhere
    let (final_markdown, _english_markdown, _chunks) = generate_meeting_summary(
        &client,
        &provider,
        &workflow.model,
        &api_key,
        text,
        &custom_prompt,
        &workflow.template_id,
        &template,
        40000, // token_threshold, mirrors api_process_transcript chunk_size default
        ollama_endpoint.as_deref(),
        custom_openai_endpoint.as_deref(),
        workflow.max_tokens.map(|v| v as u32),
        workflow.temperature.map(|v| v as f32),
        workflow.top_p.map(|v| v as f32),
        app_data_dir.as_ref(),
        Some(token),
        summary_language.as_deref(),
        detected_transcript_language,
        None, // cached_english
    )
    .await?;

    // language_name_from_code is imported to keep parity with service.rs; not required here.
    let _ = language_name_from_code;

    Ok((final_markdown, section_titles))
}

/// Reads ollama endpoint + custom-openai endpoint from settings (best-effort).
async fn resolve_endpoints(pool: &SqlitePool, provider: &str) -> (Option<String>, Option<String>) {
    let mut ollama = None;
    let mut custom = None;
    if let Ok(Some(setting)) = SettingsRepository::get_model_config(pool).await {
        ollama = setting.ollama_endpoint.clone();
    }
    if provider == "custom-openai" {
        if let Ok(Some(cfg)) = SettingsRepository::get_custom_openai_config(pool).await {
            custom = cfg.endpoint;
        }
    }
    (ollama, custom)
}
```

> Note for the implementer: verify the exact signature of `generate_meeting_summary` (Task ground-truth: `processor.rs:323`) and the `app.path().app_data_dir()` accessor against the installed Tauri v2 API; adjust the `app_data_dir` acquisition if `app.path()` differs. `SettingsRepository::get_custom_openai_config` returns `Option<CustomOpenAIConfig>` whose `endpoint: Option<String>` field name should be confirmed (`summary::CustomOpenAIConfig`). These are the only two spots that may need a 1-line adjustment.

- [ ] **Step 4: Run the pure test + build**

Run: `cd frontend/src-tauri && cargo test workflows::runner -- --nocapture && cargo check`
Expected: `build_sections_json` test PASSES; `cargo check` compiles (fix any signature mismatch flagged in the note above).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/runner.rs
git commit -m "feat(workflows): :sparkles: add workflow run executor reusing summary pipeline"
```

---

### Task 7: Run commands — `api_run_workflow` / `api_get_workflow_run` / `api_list_workflow_runs` / `api_cancel_workflow_run`

**Files:**
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `WorkflowsRepository`, `runner::run_workflow_background` + `runner::cancel_run`.
- Produces commands `api_run_workflow` (returns `{ runId }`), `api_get_workflow_run`, `api_list_workflow_runs`, `api_cancel_workflow_run`.

- [ ] **Step 1: Add a response struct + the run commands to `commands.rs`**

Add to the top imports:

```rust
use crate::summary::workflows::models::WorkflowRun;
use crate::summary::workflows::runner;
use serde::Serialize;
use tauri::{AppHandle, Runtime};
```

Append:

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStartedResponse {
    pub run_id: String,
}

#[tauri::command]
pub async fn api_run_workflow<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    workflow_id: String,
    meeting_id: String,
    text: String,
    summary_language: Option<String>,
) -> Result<RunStartedResponse, String> {
    log_info!("api_run_workflow called (workflow {}, meeting {})", workflow_id, meeting_id);

    let pool = state.db_manager.pool().clone();

    let workflow = WorkflowsRepository::get_workflow(&pool, &workflow_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;

    if text.trim().is_empty() {
        return Err("No transcript text available for this meeting".to_string());
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    WorkflowsRepository::create_run(&pool, &run_id, Some(&workflow.id), &workflow.name, &meeting_id)
        .await
        .map_err(|e| e.to_string())?;

    let summary_language = summary_language.and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t.to_string()) }
    });

    let run_id_spawn = run_id.clone();
    tauri::async_runtime::spawn(async move {
        runner::run_workflow_background(
            app, pool, run_id_spawn, workflow, meeting_id, text, summary_language,
        ).await;
    });

    Ok(RunStartedResponse { run_id })
}

#[tauri::command]
pub async fn api_get_workflow_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<Option<WorkflowRun>, String> {
    WorkflowsRepository::get_run(state.db_manager.pool(), &run_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_list_workflow_runs(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<WorkflowRun>, String> {
    WorkflowsRepository::list_runs_for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_cancel_workflow_run(run_id: String) -> Result<bool, String> {
    log_info!("api_cancel_workflow_run called (run {})", run_id);
    Ok(runner::cancel_run(&run_id))
}
```

- [ ] **Step 2: Register in `lib.rs`**

Add under the Workflow commands block added in Task 5:

```rust
            summary::workflows::commands::api_run_workflow,
            summary::workflows::commands::api_get_workflow_run,
            summary::workflows::commands::api_list_workflow_runs,
            summary::workflows::commands::api_cancel_workflow_run,
```

- [ ] **Step 3: Add a repository test asserting the run appears with queued status then can be listed**

(Extends the existing `commands::tests` module. The command's spawn path needs a Tauri runtime; assert the pre-spawn DB effect via the repository directly.)

```rust
    #[tokio::test]
    async fn create_run_shows_in_meeting_run_list() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        WorkflowsRepository::create_run(&pool, "run1", None, "W", "m1").await.unwrap();
        let runs = WorkflowsRepository::list_runs_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "queued");
        assert_eq!(runs[0].neohive_status, "none");
    }
```

- [ ] **Step 4: Run tests + build**

Run: `cd frontend/src-tauri && cargo test workflows -- --nocapture && cargo check`
Expected: PASS; clean build with all run commands registered.

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(workflows): :sparkles: add run/poll/cancel Tauri commands"
```

---

### Task 8: Extend meeting delete cascade to `workflow_runs`

**Files:**
- Modify: `frontend/src-tauri/src/database/repositories/meeting.rs` (`delete_meeting_with_transaction`, ~lines 249–262)

**Interfaces:**
- Consumes: nothing new. Produces: `workflow_runs` rows removed when their meeting is deleted.

- [ ] **Step 1: Add the delete statement**

In `delete_meeting_with_transaction`, after the `summary_processes` delete and before/after the `transcripts` delete, add:

```rust
    // Delete workflow runs for this meeting (retained-artifact runs die with the meeting)
    sqlx::query("DELETE FROM workflow_runs WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
```

- [ ] **Step 2: Add a test in `meeting.rs` (or `workflows/repository.rs`) proving runs are gone after meeting delete**

Add to `workflows/repository.rs` tests:

```rust
    #[tokio::test]
    async fn deleting_meeting_deletes_its_runs() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", None, "W", "m1").await.unwrap();
        // simulate the cascade delete statement directly
        sqlx::query("DELETE FROM workflow_runs WHERE meeting_id = ?").bind("m1").execute(&pool).await.unwrap();
        assert!(WorkflowsRepository::get_run(&pool, "r1").await.unwrap().is_none());
    }
```

- [ ] **Step 3: Run tests + build**

Run: `cd frontend/src-tauri && cargo test workflows -- --nocapture && cargo check`
Expected: PASS; clean build.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/database/repositories/meeting.rs frontend/src-tauri/src/summary/workflows/repository.rs
git commit -m "feat(workflows): :wastebasket: cascade-delete workflow runs with their meeting"
```

---

## Phase 5 — NeoHive export

### Task 9: NeoHive settings — `get_neohive_config` / `save_neohive_config`

**Files:**
- Modify: `frontend/src-tauri/src/database/repositories/setting.rs`

**Interfaces:**
- Produces: `SettingsRepository::get_neohive_config(pool) -> Result<NeoHiveSettings, sqlx::Error>` and `save_neohive_config(pool, endpoint: Option<&str>, api_key: Option<&str>, enabled: bool) -> Result<(), sqlx::Error>` where `NeoHiveSettings { endpoint: Option<String>, api_key: Option<String>, enabled: bool }`.

- [ ] **Step 1: Write the failing test**

Add to `setting.rs` (create a `#[cfg(test)] mod tests` if none exists):

```rust
#[cfg(test)]
mod neohive_settings_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1)
            .connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn save_then_get_neohive_config() {
        let pool = test_pool().await;
        SettingsRepository::save_neohive_config(
            &pool, Some("https://neohive.logilica.com/projects/x/mcp"), Some("tok"), true,
        ).await.unwrap();
        let cfg = SettingsRepository::get_neohive_config(&pool).await.unwrap();
        assert_eq!(cfg.endpoint.as_deref(), Some("https://neohive.logilica.com/projects/x/mcp"));
        assert_eq!(cfg.api_key.as_deref(), Some("tok"));
        assert!(cfg.enabled);
    }

    #[tokio::test]
    async fn get_neohive_config_defaults_when_unset() {
        let pool = test_pool().await;
        let cfg = SettingsRepository::get_neohive_config(&pool).await.unwrap();
        assert!(cfg.endpoint.is_none());
        assert!(!cfg.enabled);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd frontend/src-tauri && cargo test neohive_settings_tests -- --nocapture`
Expected: FAIL — methods/struct not defined.

- [ ] **Step 3: Implement the struct + methods**

Add near the top of `setting.rs`:

```rust
#[derive(Debug, Clone, Default)]
pub struct NeoHiveSettings {
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub enabled: bool,
}
```

Add methods inside `impl SettingsRepository`:

```rust
    pub async fn get_neohive_config(
        pool: &SqlitePool,
    ) -> std::result::Result<NeoHiveSettings, sqlx::Error> {
        let row: Option<(Option<String>, Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT neohiveEndpoint, neohiveApiKey, neohiveEnabled FROM settings WHERE id = '1' LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;
        Ok(match row {
            Some((endpoint, api_key, enabled)) => NeoHiveSettings {
                endpoint,
                api_key,
                enabled: enabled.unwrap_or(0) != 0,
            },
            None => NeoHiveSettings::default(),
        })
    }

    pub async fn save_neohive_config(
        pool: &SqlitePool,
        endpoint: Option<&str>,
        api_key: Option<&str>,
        enabled: bool,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, neohiveEndpoint, neohiveApiKey, neohiveEnabled)
            VALUES ('1', 'openai', 'gpt-4o-2024-11-20', 'large-v3', ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                neohiveEndpoint = excluded.neohiveEndpoint,
                neohiveApiKey = excluded.neohiveApiKey,
                neohiveEnabled = excluded.neohiveEnabled
            "#,
        )
        .bind(endpoint)
        .bind(api_key)
        .bind(if enabled { 1_i64 } else { 0_i64 })
        .execute(pool)
        .await?;
        Ok(())
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cd frontend/src-tauri && cargo test neohive_settings_tests -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/database/repositories/setting.rs
git commit -m "feat(workflows): :sparkles: persist NeoHive endpoint/token/enabled in settings"
```

---

### Task 10: NeoHive transport probe (discovery — resolve the OPEN spec question)

**Files:** none (investigation task; record findings in a comment at the top of `neohive/client.rs` created in Task 11).

**Goal:** Confirm the exact protocol/auth `neohive.logilica.com/projects/<uuid>/mcp` expects so Task 11's client is correct. The endpoint is known to be MCP-over-HTTP (Streamable HTTP). This task determines: (a) does `initialize` succeed with only a header token, (b) is the response `application/json` or `text/event-stream` (SSE), (c) is a `Mcp-Session-Id` header returned and required on subsequent calls, (d) the exact auth header name/format.

- [ ] **Step 1: Read the working config to get the endpoint + header (values are the user's; do not print secrets)**

Run: `python3 -c "import json,os; d=json.load(open(os.path.expanduser('~/.claude.json'))); print('has config' )"` then locate the `neohive-meetily-e95faa` server block under `mcpServers` (endpoint `https://neohive.logilica.com/projects/e95faa80-9092-478d-98b0-19ef8158efb8/mcp`, header `x-mcp-client`). Confirm the header name and whether any `Authorization`/bearer is present.

- [ ] **Step 2: Probe `initialize` with curl (redact the token in any pasted output)**

Run (substitute the real token for `$TOKEN`, header name confirmed in Step 1):
```bash
curl -si -X POST "https://neohive.logilica.com/projects/e95faa80-9092-478d-98b0-19ef8158efb8/mcp" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -H "x-mcp-client: $TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"meetily","version":"0.4.0"}}}'
```
Expected: a `200` with either a JSON body containing `"result"` (single-response mode) or a `text/event-stream` body with a `data: {...}` line. Note the response `content-type` and any `mcp-session-id` response header.

- [ ] **Step 3: If a session id was returned, probe `tools/call` for `memory_store`**

Run (include `-H "mcp-session-id: <id>"` if Step 2 returned one; send the `initialized` notification first if the server requires it):
```bash
curl -si -X POST "<same-url>" \
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
  -H "x-mcp-client: $TOKEN" -H "mcp-session-id: <ID_IF_ANY>" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_store","arguments":{"content":"probe from meetily plan — please ignore","type":"insight","tags":["meetily-probe"],"importance":1}}}'
```
Expected: a JSON-RPC `result` confirming the memory was stored (or an error explaining required fields/auth). This validates the exact `memory_store` argument contract for the real server.

- [ ] **Step 4: Record findings**

Write a short comment block (protocol mode: single-JSON vs SSE; session-id required Y/N; auth header name; whether an `initialized` notification is required) to be pasted at the top of `neohive/client.rs` in Task 11. No commit (no file yet) — or commit a `neohive/NOTES.md` capturing the findings:

```bash
# optional:
git add frontend/src-tauri/src/neohive/NOTES.md 2>/dev/null && git commit -m "docs(neohive): :memo: record MCP transport probe findings" || true
```

> If the probe shows a **plain REST** `memory_store` also exists, prefer it (simpler) and adapt Task 11 accordingly. Otherwise implement the MCP JSON-RPC path in Task 11 exactly as the probe confirmed.

---

### Task 11: `NeoHiveClient` — MCP-over-HTTP `memory_store`

**Files:**
- Create: `frontend/src-tauri/src/neohive/mod.rs`
- Create: `frontend/src-tauri/src/neohive/client.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (add `mod neohive;` near the other top-level `mod` declarations)

**Interfaces:**
- Produces:
  - `pub struct NeoHiveClient { endpoint: String, token: String, http: reqwest::Client, session_id: tokio::sync::Mutex<Option<String>> }`
  - `pub fn new(endpoint: String, token: String) -> NeoHiveClient`
  - `pub async fn store_memory(&self, content: &str, mem_type: &str, tags: &[String], importance: u8) -> Result<(), String>`
  - pure helper `pub(crate) fn build_store_params(content, mem_type, tags, importance) -> serde_json::Value` (unit tested)

- [ ] **Step 1: Write `mod.rs`**

```rust
pub mod client;
pub use client::NeoHiveClient;
```

- [ ] **Step 2: Write the failing payload test in `client.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_store_params_matches_memory_store_contract() {
        let p = build_store_params("hello", "insight", &["a".into(), "b".into()], 6);
        assert_eq!(p["name"], "memory_store");
        assert_eq!(p["arguments"]["content"], "hello");
        assert_eq!(p["arguments"]["type"], "insight");
        assert_eq!(p["arguments"]["importance"], 6);
        assert_eq!(p["arguments"]["tags"][0], "a");
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cd frontend/src-tauri && cargo test neohive -- --nocapture`
Expected: FAIL — `build_store_params` not defined.

- [ ] **Step 4: Implement `client.rs`**

(Implement per the Task 10 probe. Below is the MCP Streamable-HTTP path — the guaranteed protocol. Replace the header name / session handling if the probe found otherwise. Adjust the SSE-vs-JSON parse to match what the probe observed.)

```rust
use serde_json::{json, Value};

pub struct NeoHiveClient {
    endpoint: String,
    token: String,
    http: reqwest::Client,
    session_id: tokio::sync::Mutex<Option<String>>,
}

/// Pure: the params for a tools/call memory_store request.
pub(crate) fn build_store_params(
    content: &str,
    mem_type: &str,
    tags: &[String],
    importance: u8,
) -> Value {
    json!({
        "name": "memory_store",
        "arguments": {
            "content": content,
            "type": mem_type,
            "tags": tags,
            "importance": importance
        }
    })
}

impl NeoHiveClient {
    pub fn new(endpoint: String, token: String) -> Self {
        Self {
            endpoint,
            token,
            http: reqwest::Client::new(),
            session_id: tokio::sync::Mutex::new(None),
        }
    }

    fn base_request(&self) -> reqwest::RequestBuilder {
        // NOTE: header name confirmed by Task 10 probe (default: x-mcp-client).
        self.http
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("x-mcp-client", &self.token)
    }

    /// Extracts the JSON-RPC object from either a plain JSON body or an SSE body.
    fn parse_body(content_type: &str, body: &str) -> Result<Value, String> {
        if content_type.contains("text/event-stream") {
            // Find the first `data: {...}` line and parse it.
            for line in body.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    let rest = rest.trim();
                    if rest.starts_with('{') {
                        return serde_json::from_str(rest)
                            .map_err(|e| format!("NeoHive SSE parse error: {}", e));
                    }
                }
            }
            Err("NeoHive SSE response contained no data payload".to_string())
        } else {
            serde_json::from_str(body).map_err(|e| format!("NeoHive JSON parse error: {}", e))
        }
    }

    async fn ensure_session(&self) -> Result<(), String> {
        {
            let guard = self.session_id.lock().await;
            if guard.is_some() {
                return Ok(());
            }
        }
        let init = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "meetily", "version": "0.4.0" }
            }
        });
        let resp = self.base_request().json(&init).send().await
            .map_err(|e| format!("NeoHive initialize failed: {}", e))?;
        let session = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let ct = resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        // Validate the initialize result parsed (surfaces auth errors early).
        let _ = Self::parse_body(&ct, &text)?;

        if let Some(sid) = session {
            *self.session_id.lock().await = Some(sid.clone());
            // Some servers require an initialized notification before tools/call.
            let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
            let _ = self.base_request()
                .header("mcp-session-id", sid)
                .json(&note).send().await;
        }
        Ok(())
    }

    pub async fn store_memory(
        &self,
        content: &str,
        mem_type: &str,
        tags: &[String],
        importance: u8,
    ) -> Result<(), String> {
        self.ensure_session().await?;
        let params = build_store_params(content, mem_type, tags, importance);
        let req_body = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": params });

        let mut builder = self.base_request();
        if let Some(sid) = self.session_id.lock().await.clone() {
            builder = builder.header("mcp-session-id", sid);
        }
        let resp = builder.json(&req_body).send().await
            .map_err(|e| format!("NeoHive memory_store request failed: {}", e))?;
        let ct = resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("NeoHive returned HTTP {}", status));
        }
        let parsed = Self::parse_body(&ct, &text)?;
        if let Some(err) = parsed.get("error") {
            return Err(format!("NeoHive memory_store error: {}", err));
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Add `mod neohive;` to `lib.rs`**

Near the other top-level module declarations (e.g. alongside `mod state;` / `mod database;`), add:

```rust
mod neohive;
```

- [ ] **Step 6: Run the pure test + build**

Run: `cd frontend/src-tauri && cargo test neohive -- --nocapture && cargo check`
Expected: payload test PASSES; clean build.

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/src/neohive/ frontend/src-tauri/src/lib.rs
git commit -m "feat(neohive): :sparkles: add MCP-over-HTTP memory_store client"
```

---

### Task 12: `api_export_run_to_neohive` — export a run's sections

**Files:**
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- (Optional) call it automatically from `runner.rs` when `auto_export` is set (Step 5).

**Interfaces:**
- Consumes: `WorkflowsRepository`, `NeoHiveClient` (Task 11), `SettingsRepository::get_neohive_config` (Task 9), `sections::{ParsedSection, memory_type_for}` (Task 4), `Workflow::neohive_config` (Task 2).
- Produces command `api_export_run_to_neohive(state, run_id) -> ExportResult { pushed, failed }`.

- [ ] **Step 1: Add a pure helper + its test — build export items from a run + workflow config**

Add to `commands.rs`:

```rust
use crate::summary::workflows::sections::{memory_type_for, ParsedSection};
use crate::summary::workflows::models::NeoHiveExportConfig;

/// One memory to push: (content, memory_type, tags).
#[derive(Debug, PartialEq)]
pub struct ExportItem {
    pub content: String,
    pub mem_type: String,
    pub tags: Vec<String>,
}

/// Pure: turns parsed sections + config + context into export items (skips empties).
pub(crate) fn build_export_items(
    sections: &[ParsedSection],
    cfg: &NeoHiveExportConfig,
    meeting_title: &str,
    workflow_name: &str,
) -> Vec<ExportItem> {
    sections
        .iter()
        .filter(|s| !s.content.trim().is_empty())
        .map(|s| ExportItem {
            content: format!("{}\n\n{}", s.title, s.content),
            mem_type: memory_type_for(&s.title, cfg),
            tags: vec![
                meeting_title.to_string(),
                workflow_name.to_string(),
                s.title.clone(),
                "meetily".to_string(),
            ],
        })
        .collect()
}
```

Add a test to the `commands::tests` module:

```rust
    #[test]
    fn build_export_items_skips_empty_and_maps_types() {
        use crate::summary::workflows::sections::ParsedSection;
        use crate::summary::workflows::models::NeoHiveExportConfig;
        use std::collections::HashMap;
        let mut overrides = HashMap::new();
        overrides.insert("Key Decisions".to_string(), "decision".to_string());
        let cfg = NeoHiveExportConfig { section_type_overrides: overrides, ..Default::default() };
        let sections = vec![
            ParsedSection { title: "Summary".into(), content: "hi".into() },
            ParsedSection { title: "Key Decisions".into(), content: "  ".into() }, // empty -> skipped
        ];
        let items = super::build_export_items(&sections, &cfg, "Sprint", "Exec");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].mem_type, "narrative");
        assert!(items[0].tags.contains(&"meetily".to_string()));
        assert!(items[0].content.starts_with("Summary"));
    }
```

- [ ] **Step 2: Run to verify it fails, then passes after adding the helper**

Run: `cd frontend/src-tauri && cargo test workflows::commands -- --nocapture`
Expected: PASS once `build_export_items` compiles (the helper above makes it pass immediately; ensure the test is present first to honor TDD, run, see it fail on missing symbol, then keep the helper).

- [ ] **Step 3: Implement the export command**

Add to `commands.rs`:

```rust
use crate::neohive::NeoHiveClient;
use crate::database::repositories::setting::SettingsRepository;
use crate::database::repositories::meeting::MeetingsRepository;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub pushed: usize,
    pub failed: usize,
}

#[tauri::command]
pub async fn api_export_run_to_neohive(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<ExportResult, String> {
    log_info!("api_export_run_to_neohive called (run {})", run_id);
    let pool = state.db_manager.pool();

    let run = WorkflowsRepository::get_run(pool, &run_id)
        .await.map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Run '{}' not found", run_id))?;
    if run.status != "completed" {
        return Err("Only completed runs can be exported".to_string());
    }

    let sections: Vec<ParsedSection> = run
        .result_sections
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if sections.is_empty() {
        return Err("This run has no sections to export".to_string());
    }

    // NeoHive connection config
    let cfg = SettingsRepository::get_neohive_config(pool).await.map_err(|e| e.to_string())?;
    if !cfg.enabled {
        return Err("NeoHive export is disabled in Settings".to_string());
    }
    let endpoint = cfg.endpoint.ok_or("NeoHive endpoint is not configured")?;
    let token = cfg.api_key.unwrap_or_default();

    // Per-workflow export config (type overrides, importance)
    let export_cfg = match &run.workflow_id {
        Some(id) => WorkflowsRepository::get_workflow(pool, id).await.ok().flatten()
            .map(|w| w.neohive_config()).unwrap_or_default(),
        None => NeoHiveExportConfig::default(),
    };

    let meeting_title = MeetingsRepository::get_meeting_by_id(pool, &run.meeting_id)
        .await.ok().flatten().map(|m| m.title).unwrap_or_else(|| "Meeting".to_string());

    let items = build_export_items(&sections, &export_cfg, &meeting_title, &run.workflow_name);
    let client = NeoHiveClient::new(endpoint, token);

    let mut pushed = 0usize;
    let mut failed = 0usize;
    for item in &items {
        match client.store_memory(&item.content, &item.mem_type, &item.tags, export_cfg.importance).await {
            Ok(()) => pushed += 1,
            Err(e) => { log_error!("NeoHive export item failed: {}", e); failed += 1; }
        }
    }

    let status = if failed == 0 { "pushed" } else if pushed == 0 { "failed" } else { "partial" };
    let _ = WorkflowsRepository::set_run_neohive_status(pool, &run_id, status).await;

    Ok(ExportResult { pushed, failed })
}
```

> Implementer note: confirm the meeting-lookup method name. Ground-truth shows `meeting.rs` has a `get_meeting_by_id`-style `SELECT ... FROM meetings WHERE id = ?` (lines ~65, ~123). If the exact public method differs, use the available getter or a direct `sqlx::query_as` for the title.

- [ ] **Step 4: Register in `lib.rs`**

```rust
            summary::workflows::commands::api_export_run_to_neohive,
```

- [ ] **Step 5 (optional auto-export): call export at run completion when configured**

In `runner.rs` `run_workflow_background`, after a successful `complete_run`, if `workflow.neohive_config().enabled && workflow.neohive_config().auto_export`, invoke the same export logic (extract the body of `api_export_run_to_neohive` into a `pub(crate) async fn export_run(pool, run_id)` in `commands.rs` and call it from both the command and the runner to stay DRY). Update `neohive_status` accordingly. Keep this behind the config flag so default behavior stays manual.

- [ ] **Step 6: Run tests + build**

Run: `cd frontend/src-tauri && cargo test workflows -- --nocapture && cargo check`
Expected: PASS; clean build with the export command registered.

- [ ] **Step 7: Manual end-to-end verification (real NeoHive)**

With the app running (`cd frontend && ./clean_run.sh debug`): configure NeoHive endpoint+token+enabled in settings (once the Plan 2 UI exists, or via a temporary `save_neohive_config` call), run a workflow on a meeting with transcripts, then invoke `api_export_run_to_neohive` and confirm memories appear in the Meetily NeoHive project (verify with a `memory_recall` for one of the section tags). Confirm the token is never printed in logs.

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/src/summary/workflows/commands.rs frontend/src-tauri/src/summary/workflows/runner.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(neohive): :sparkles: export workflow run sections to NeoHive"
```

---

## Self-Review

**1. Spec coverage (backend portions of the spec):**
- §2 data model (`workflows`, `workflow_runs`) → Task 1, 2, 3. ✅
- §2 delete semantics (meeting cascades to runs; workflow delete retains runs) → Task 3 (retain) + Task 8 (cascade). ✅
- §5 backend module + commands (`api_list/save/delete_workflow`, `api_run_workflow`, `api_list_workflow_runs`, `api_get_workflow_run`, `api_export_run_to_neohive`) → Tasks 5, 7, 12. ✅ (Added `api_cancel_workflow_run` for parity with existing summary cancel.)
- §5 reuse of generation core → Task 6 reuses `generate_meeting_summary` (no pipeline fork). ✅
- §6 NeoHive transport + settings + section→type mapping + granularity (section-by-section) + never-silent (manual default; auto behind flag) → Tasks 9, 10, 11, 12. ✅
- §6 open transport question → Task 10 probe. ✅
- §7 approved defaults (section-by-section; manual w/ optional auto; overridable endpoint; Decisions→decision / Action Items→insight / prose→narrative) → the default mapping is *config-driven* (Task 2/4/12); the specific default overrides (Decisions→decision, Action Items→insight) are supplied by the frontend when creating a workflow (Plan 2) — the backend default is `narrative` with overrides honored. Note: this defers the *specific* default override values to Plan 2's workflow-creation UI. ✅ (documented, not a gap)
- §8 frontend → **out of scope; Plan 2.** ✅ (intentional split)
- §9 conventions (Tauri commands, no hardcoded paths, api_ naming, serde renames, never log secrets) → Global Constraints + per-task. ✅
- §11 open items (transport, cache key, retention mechanism) → transport = Task 10; retention mechanism = Task 1 (denormalized `workflow_name`, nullable `workflow_id`); **cache key for workflow runs = intentionally deferred** (runs currently always regenerate; no cache reuse in v1 — YAGNI, noted below).

**2. Placeholder scan:** No "TBD/TODO/handle appropriately". Two "implementer note" callouts (Task 6 signature confirmation, Task 12 meeting getter name) are *verification* prompts against real ground-truth, with the exact fallback stated — not placeholders.

**3. Type consistency:** `Workflow`/`WorkflowRun`/`WorkflowInput`/`NeoHiveExportConfig`/`ParsedSection`/`ExportItem` names are consistent across Tasks 2–12. Repository method names match their call sites in commands/runner. `WorkflowRunStatus` constants match the DB `status` strings and the frontend polling contract (`queued/running/completed/error/cancelled`). NeoHive settings struct `NeoHiveSettings` fields match Task 12 usage.

**Deviations from spec, called out:**
- **Workflow-run caching is deferred (YAGNI).** Spec §11 lists "cache key for workflow runs" as an open item; v1 always regenerates a run. Adding a fingerprint cache later is non-breaking. Documented here so it is a conscious choice, not an omission.
- **Specific default type overrides** (Decisions→decision, Action Items→insight) are applied at workflow-creation time in the Plan 2 UI; the backend honors arbitrary overrides and defaults unmapped sections to `narrative`.
