# NeoHive Connection Auth Methods Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalize NeoHive connection auth from the hardcoded Cloudflare Access service token to five selectable methods — Cloudflare Access, Bearer/API-key, Basic, Custom-header, and None/network-level (Tailscale/LAN) — across the client, settings storage, commands, and settings UI, backward-compatibly.

**Architecture:** Introduce one `NeoHiveAuth` enum in `neohive/client.rs` that maps a `(type string, fields JSON)` pair to the right reqwest request headers (using reqwest's built-in `.bearer_auth()`/`.basic_auth()`). Store the method as a snake_case `neohiveAuthType` column + a fields-only `neohiveAuthConfig` JSON column (migrating the existing CF config into it). Generalize the config commands + the settings UI to `authType` + `authConfig`.

**Tech Stack:** Rust / Tauri v2, SQLx/SQLite (embedded `sqlx::migrate!`), reqwest 0.11, Next.js/React/TypeScript, Tailwind.

## Global Constraints

- **Personal local fork:** work on branch `feature/neohive-auth-methods`; merge to local `main` only; do not push code to any remote.
- **Backward compatible:** the owner's existing Cloudflare Access config must keep working with no reconfiguration (the migration backfills it).
- **No new Rust dependency:** Bearer/Basic use reqwest's `.bearer_auth()`/`.basic_auth()`; no `base64` crate.
- **Auth methods (exactly five):** `cloudflare_access`, `bearer`, `basic`, `custom_header`, `none`. Discriminator strings are **snake_case** in SQL + at the TS boundary (`authType`); the fields JSON (`authConfig`) uses **camelCase** keys: `clientId`, `clientSecret`, `token`, `username`, `password`, `headerName`, `headerValue`.
- **Secret handling:** unchanged convention — `api_get_neohive_config` returns secrets to the local webview; the UI masks with a show/hide toggle. Never log secrets.
- **Invariant request headers** stay for every method: `Content-Type: application/json`, `Accept: application/json, text/event-stream`, `x-mcp-client: meetily`, and per-call `mcp-session-id`.
- **Migration ordering:** new migration dated after `20260709000001` (the diarization migrations) — use `20260709000002`.
- **Build env:** Xcode present + `binaries/llama-helper-aarch64-apple-darwin` placeholder present (full-crate `cargo test`/`cargo build` from `frontend/src-tauri`; the 2 pre-existing `audio::device_detection` test failures are unrelated). Frontend gate: `cd frontend && npx tsc --noEmit` (one pre-existing unrelated `bun:test` error is expected).
- **Commits:** gitmoji conventional commits; no AI attribution / no `Co-Authored-By`.

## File Structure
- Modify `frontend/src-tauri/src/neohive/client.rs` — add `NeoHiveAuth` enum + `from_parts` + `apply`; refactor `NeoHiveClient` to hold `auth`. (Task 1)
- Modify `frontend/src-tauri/src/neohive/mod.rs` — export `NeoHiveAuth`. (Task 1)
- Modify `frontend/src-tauri/src/summary/workflows/commands.rs` — export-path client construction (Task 1), then the config response + get/save commands + export path rewrite (Task 2).
- Create `frontend/src-tauri/migrations/20260709000002_add_neohive_auth_method.sql` — new columns + backfill. (Task 2)
- Modify `frontend/src-tauri/src/database/repositories/setting.rs` — `NeoHiveSettings` struct + `get/save_neohive_config` + tests. (Task 2)
- Modify `frontend/src/types/workflow.ts` — `NeoHiveSettings` type + auth types. (Task 3)
- Modify `frontend/src/components/workflows/WorkflowsSettings.tsx` — method dropdown + per-method fields + load/save mapping. (Task 3)

---

## Task 1: `NeoHiveAuth` abstraction in the client

**Files:**
- Modify: `frontend/src-tauri/src/neohive/client.rs`
- Modify: `frontend/src-tauri/src/neohive/mod.rs`
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs:227`

**Interfaces:**
- Produces: `pub enum NeoHiveAuth { CloudflareAccess{client_id,client_secret}, Bearer{token}, Basic{username,password}, CustomHeader{header_name,header_value}, None }`; `pub fn NeoHiveAuth::from_parts(auth_type: Option<&str>, config: &serde_json::Value) -> Result<Self, String>`; `NeoHiveClient::new(endpoint: String, auth: NeoHiveAuth) -> Self`. Consumed by Task 2 (export path uses `from_parts`).

- [ ] **Step 1: Add the failing tests** (append to the `#[cfg(test)] mod tests` in `client.rs`)

```rust
    fn header_val(req: &reqwest::Request, name: &str) -> Option<String> {
        req.headers().get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
    }

    #[test]
    fn apply_cloudflare_sets_cf_headers_and_no_auth() {
        let req = NeoHiveAuth::CloudflareAccess { client_id: "id1".into(), client_secret: "sec1".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert_eq!(header_val(&req, "cf-access-client-id").as_deref(), Some("id1"));
        assert_eq!(header_val(&req, "cf-access-client-secret").as_deref(), Some("sec1"));
        assert!(header_val(&req, "authorization").is_none());
    }

    #[test]
    fn apply_bearer_sets_authorization() {
        let req = NeoHiveAuth::Bearer { token: "tok".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert_eq!(header_val(&req, "authorization").as_deref(), Some("Bearer tok"));
    }

    #[test]
    fn apply_basic_sets_base64_authorization() {
        let req = NeoHiveAuth::Basic { username: "user".into(), password: "pass".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        // base64("user:pass") == "dXNlcjpwYXNz"
        assert_eq!(header_val(&req, "authorization").as_deref(), Some("Basic dXNlcjpwYXNz"));
    }

    #[test]
    fn apply_custom_header_sets_it() {
        let req = NeoHiveAuth::CustomHeader { header_name: "X-Api-Key".into(), header_value: "k".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert_eq!(header_val(&req, "x-api-key").as_deref(), Some("k"));
    }

    #[test]
    fn apply_none_sets_no_auth_headers() {
        let req = NeoHiveAuth::None.apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert!(header_val(&req, "authorization").is_none());
        assert!(header_val(&req, "cf-access-client-id").is_none());
    }

    #[test]
    fn from_parts_maps_types_and_validates() {
        use serde_json::json;
        assert!(matches!(NeoHiveAuth::from_parts(Some("cloudflare_access"), &json!({"clientId":"a","clientSecret":"b"})).unwrap(), NeoHiveAuth::CloudflareAccess{..}));
        assert!(matches!(NeoHiveAuth::from_parts(Some("bearer"), &json!({"token":"t"})).unwrap(), NeoHiveAuth::Bearer{..}));
        assert!(matches!(NeoHiveAuth::from_parts(Some("none"), &json!({})).unwrap(), NeoHiveAuth::None));
        assert!(matches!(NeoHiveAuth::from_parts(None, &json!({})).unwrap(), NeoHiveAuth::None));
        assert!(NeoHiveAuth::from_parts(Some("bearer"), &json!({"token":""})).is_err());     // blank required field
        assert!(NeoHiveAuth::from_parts(Some("mystery"), &json!({})).is_err());               // unknown type
    }
```

- [ ] **Step 2: Run the tests to confirm they fail**

Run: `cd frontend/src-tauri && cargo test --lib neohive::client 2>&1 | tail -20`
Expected: compile error (`NeoHiveAuth` not found / `apply` not found).

- [ ] **Step 3: Add the `NeoHiveAuth` enum + `from_parts` + `apply`** (in `client.rs`, above `pub struct NeoHiveClient` or just below the `use`)

```rust
/// Authentication for the NeoHive MCP endpoint. Built from a stored
/// (type string, fields JSON) pair via `from_parts` and applied to each request.
#[derive(Debug, Clone)]
pub enum NeoHiveAuth {
    CloudflareAccess { client_id: String, client_secret: String },
    Bearer { token: String },
    Basic { username: String, password: String },
    CustomHeader { header_name: String, header_value: String },
    None,
}

impl NeoHiveAuth {
    /// Map the snake_case type + camelCase fields JSON into an auth method.
    /// Blank/missing required fields for a known type, or an unknown type, error.
    pub fn from_parts(auth_type: Option<&str>, config: &serde_json::Value) -> Result<Self, String> {
        let req = |k: &str| -> Result<String, String> {
            config.get(k).and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .ok_or_else(|| format!("NeoHive auth: missing/empty field '{}'", k))
        };
        match auth_type {
            Some("cloudflare_access") => Ok(NeoHiveAuth::CloudflareAccess { client_id: req("clientId")?, client_secret: req("clientSecret")? }),
            Some("bearer") => Ok(NeoHiveAuth::Bearer { token: req("token")? }),
            Some("basic") => Ok(NeoHiveAuth::Basic { username: req("username")?, password: req("password")? }),
            Some("custom_header") => Ok(NeoHiveAuth::CustomHeader { header_name: req("headerName")?, header_value: req("headerValue")? }),
            Some("none") | None => Ok(NeoHiveAuth::None),
            Some(other) => Err(format!("NeoHive auth: unknown type '{}'", other)),
        }
    }

    /// Attach this method's auth to a request builder. Invariant headers are set by the caller.
    fn apply(&self, b: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            NeoHiveAuth::CloudflareAccess { client_id, client_secret } => b
                .header("CF-Access-Client-Id", client_id)
                .header("CF-Access-Client-Secret", client_secret),
            NeoHiveAuth::Bearer { token } => b.bearer_auth(token),
            NeoHiveAuth::Basic { username, password } => b.basic_auth(username, Some(password)),
            NeoHiveAuth::CustomHeader { header_name, header_value } => b.header(header_name.as_str(), header_value.as_str()),
            NeoHiveAuth::None => b,
        }
    }
}
```

- [ ] **Step 4: Refactor `NeoHiveClient` to hold `auth`**

Change the struct fields (replace `access_client_id` + `access_client_secret` with `auth`):
```rust
pub struct NeoHiveClient {
    endpoint: String,
    auth: NeoHiveAuth,
    http: reqwest::Client,
    session_id: tokio::sync::Mutex<Option<String>>,
}
```
Change `new` (was `new(endpoint, access_client_id, access_client_secret)`):
```rust
    pub fn new(endpoint: String, auth: NeoHiveAuth) -> Self {
        Self {
            endpoint,
            auth,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            session_id: tokio::sync::Mutex::new(None),
        }
    }
```
Change `base_request` (apply invariant headers, then the auth):
```rust
    fn base_request(&self) -> reqwest::RequestBuilder {
        let b = self.http
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("x-mcp-client", "meetily");
        self.auth.apply(b)
    }
```

- [ ] **Step 5: Export `NeoHiveAuth`** — in `frontend/src-tauri/src/neohive/mod.rs`, alongside `pub use client::NeoHiveClient;` add:
```rust
pub use client::NeoHiveAuth;
```

- [ ] **Step 6: Fix the single construction site so the crate compiles** — in `frontend/src-tauri/src/summary/workflows/commands.rs`, line 227 currently reads:
```rust
    let client = NeoHiveClient::new(endpoint, client_id, client_secret);
```
Change it to (keeps the `client_id`/`client_secret` reads at lines 205-206 for now — Task 2 replaces them):
```rust
    let client = NeoHiveClient::new(endpoint, crate::neohive::NeoHiveAuth::CloudflareAccess { client_id, client_secret });
```

- [ ] **Step 7: Run the tests + build**

Run:
```bash
cd frontend/src-tauri
cargo test --lib neohive::client 2>&1 | tail -20
cargo build 2>&1 | tail -10
```
Expected: the 6 new tests + 4 existing client tests pass; crate builds (behavior unchanged — still Cloudflare Access at runtime).

- [ ] **Step 8: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/neohive/client.rs frontend/src-tauri/src/neohive/mod.rs frontend/src-tauri/src/summary/workflows/commands.rs
git commit -m "feat(neohive): :sparkles: add NeoHiveAuth method abstraction (CF/Bearer/Basic/custom/none)"
```

---

## Task 2: Settings storage + config commands for the auth method

**Files:**
- Create: `frontend/src-tauri/migrations/20260709000002_add_neohive_auth_method.sql`
- Modify: `frontend/src-tauri/src/database/repositories/setting.rs` (`NeoHiveSettings` struct ~28-33, `get_neohive_config` ~365, `save_neohive_config` ~389, tests ~430-455)
- Modify: `frontend/src-tauri/src/summary/workflows/commands.rs` (`NeoHiveConfigResponse` ~255, `api_get_neohive_config` ~264, `api_save_neohive_config` ~280, export path lines 205-206 + 227)

**Interfaces:**
- Consumes: `NeoHiveAuth::from_parts` (Task 1).
- Produces: `NeoHiveSettings { endpoint: Option<String>, enabled: bool, auth_type: Option<String>, auth_config: Option<String> }`; `get_neohive_config(pool) -> Result<NeoHiveSettings, sqlx::Error>`; `save_neohive_config(pool, endpoint: Option<&str>, enabled: bool, auth_type: Option<&str>, auth_config: Option<&str>) -> Result<(), sqlx::Error>`; commands `api_get_neohive_config` (returns `{endpoint, enabled, authType, authConfig}`) + `api_save_neohive_config(endpoint, enabled, auth_type, auth_config)`.

- [ ] **Step 1: Write the migration** — create `frontend/src-tauri/migrations/20260709000002_add_neohive_auth_method.sql`:
```sql
-- Generalize NeoHive connection auth beyond the single Cloudflare Access method.
-- neohiveAuthType: cloudflare_access | bearer | basic | custom_header | none
-- neohiveAuthConfig: JSON object of method-specific fields (camelCase keys).
ALTER TABLE settings ADD COLUMN neohiveAuthType TEXT;
ALTER TABLE settings ADD COLUMN neohiveAuthConfig TEXT;

-- Backfill any existing Cloudflare Access config so it keeps working unchanged.
UPDATE settings
SET neohiveAuthType = 'cloudflare_access',
    neohiveAuthConfig = json_object('clientId', neohiveAccessClientId, 'clientSecret', neohiveAccessClientSecret)
WHERE neohiveAccessClientId IS NOT NULL OR neohiveAccessClientSecret IS NOT NULL;
```

- [ ] **Step 2: Update the repo tests to the new shape (failing first)** — in `setting.rs`, replace the two tests in `mod neohive_settings_tests` with:
```rust
    #[tokio::test]
    async fn save_then_get_neohive_config_bearer() {
        let pool = test_pool().await;
        SettingsRepository::save_neohive_config(
            &pool,
            Some("https://neo.example/mcp"),
            true,
            Some("bearer"),
            Some(r#"{"token":"tok-123"}"#),
        ).await.unwrap();
        let cfg = SettingsRepository::get_neohive_config(&pool).await.unwrap();
        assert_eq!(cfg.endpoint.as_deref(), Some("https://neo.example/mcp"));
        assert!(cfg.enabled);
        assert_eq!(cfg.auth_type.as_deref(), Some("bearer"));
        assert_eq!(cfg.auth_config.as_deref(), Some(r#"{"token":"tok-123"}"#));
    }

    #[tokio::test]
    async fn get_neohive_config_defaults_when_unset() {
        let pool = test_pool().await;
        let cfg = SettingsRepository::get_neohive_config(&pool).await.unwrap();
        assert!(cfg.endpoint.is_none());
        assert!(cfg.auth_type.is_none());
        assert!(cfg.auth_config.is_none());
        assert!(!cfg.enabled);
    }
```

- [ ] **Step 3: Run the tests to confirm they fail**

Run: `cd frontend/src-tauri && cargo test --lib database::repositories::setting 2>&1 | tail -20`
Expected: compile errors (`auth_type`/`auth_config` fields don't exist; `save_neohive_config` arity mismatch).

- [ ] **Step 4: Update the `NeoHiveSettings` struct** (`setting.rs` ~28-33) — replace with:
```rust
pub struct NeoHiveSettings {
    pub endpoint: Option<String>,
    pub enabled: bool,
    pub auth_type: Option<String>,
    pub auth_config: Option<String>, // JSON string of method fields
}
```
(Keep the existing derives on this struct — e.g. `#[derive(Debug, Clone, Default)]` if present — unchanged.)

- [ ] **Step 5: Update `get_neohive_config`** (~365) — new SELECT + mapping:
```rust
    pub async fn get_neohive_config(
        pool: &SqlitePool,
    ) -> std::result::Result<NeoHiveSettings, sqlx::Error> {
        let row: Option<(Option<String>, Option<i64>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT neohiveEndpoint, neohiveEnabled, neohiveAuthType, neohiveAuthConfig FROM settings WHERE id = '1' LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;
        Ok(match row {
            Some((endpoint, enabled, auth_type, auth_config)) => NeoHiveSettings {
                endpoint,
                enabled: enabled.unwrap_or(0) != 0,
                auth_type,
                auth_config,
            },
            None => NeoHiveSettings::default(),
        })
    }
```

- [ ] **Step 6: Update `save_neohive_config`** (~389) — new params + upsert columns:
```rust
    pub async fn save_neohive_config(
        pool: &SqlitePool,
        endpoint: Option<&str>,
        enabled: bool,
        auth_type: Option<&str>,
        auth_config: Option<&str>,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, neohiveEndpoint, neohiveEnabled, neohiveAuthType, neohiveAuthConfig)
            VALUES ('1', 'openai', 'gpt-4o-2024-11-20', 'large-v3', ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                neohiveEndpoint = excluded.neohiveEndpoint,
                neohiveEnabled = excluded.neohiveEnabled,
                neohiveAuthType = excluded.neohiveAuthType,
                neohiveAuthConfig = excluded.neohiveAuthConfig
            "#,
        )
        .bind(endpoint)
        .bind(if enabled { 1_i64 } else { 0_i64 })
        .bind(auth_type)
        .bind(auth_config)
        .execute(pool)
        .await?;
        Ok(())
    }
```

- [ ] **Step 7: Update the config commands** (`commands.rs`) — replace `NeoHiveConfigResponse` (~255) + `api_get_neohive_config` (~264) + `api_save_neohive_config` (~280) with:
```rust
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeoHiveConfigResponse {
    pub endpoint: Option<String>,
    pub enabled: bool,
    pub auth_type: Option<String>,
    pub auth_config: Option<serde_json::Value>,
}

#[tauri::command]
pub async fn api_get_neohive_config(
    state: tauri::State<'_, AppState>,
) -> Result<NeoHiveConfigResponse, String> {
    log_info!("api_get_neohive_config called");
    let cfg = SettingsRepository::get_neohive_config(state.db_manager.pool())
        .await
        .map_err(|e| { log_error!("api_get_neohive_config failed: {}", e); e.to_string() })?;
    let auth_config = cfg.auth_config.as_deref().and_then(|s| serde_json::from_str(s).ok());
    Ok(NeoHiveConfigResponse {
        endpoint: cfg.endpoint,
        enabled: cfg.enabled,
        auth_type: cfg.auth_type,
        auth_config,
    })
}

#[tauri::command]
pub async fn api_save_neohive_config(
    state: tauri::State<'_, AppState>,
    endpoint: Option<String>,
    enabled: bool,
    auth_type: Option<String>,
    auth_config: Option<serde_json::Value>,
) -> Result<(), String> {
    log_info!("api_save_neohive_config called (enabled={}, authType={:?})", enabled, auth_type);
    let auth_config_str = auth_config.map(|v| v.to_string());
    SettingsRepository::save_neohive_config(
        state.db_manager.pool(),
        endpoint.as_deref(),
        enabled,
        auth_type.as_deref(),
        auth_config_str.as_deref(),
    )
    .await
    .map_err(|e| { log_error!("api_save_neohive_config failed: {}", e); e.to_string() })
}
```

- [ ] **Step 8: Rewrite the export-path client construction** (`commands.rs` lines 204-206 + 227) — replace:
```rust
    let endpoint = neo.endpoint.ok_or("NeoHive endpoint is not configured")?;
    let client_id = neo.access_client_id.ok_or("NeoHive Access Client Id is not configured")?;
    let client_secret = neo.access_client_secret.ok_or("NeoHive Access Client Secret is not configured")?;
```
with:
```rust
    let endpoint = neo.endpoint.ok_or("NeoHive endpoint is not configured")?;
    let auth_config_val: serde_json::Value = neo.auth_config.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let auth = crate::neohive::NeoHiveAuth::from_parts(neo.auth_type.as_deref(), &auth_config_val)?;
```
and replace line 227 (`let client = NeoHiveClient::new(endpoint, crate::neohive::NeoHiveAuth::CloudflareAccess { client_id, client_secret });` from Task 1) with:
```rust
    let client = NeoHiveClient::new(endpoint, auth);
```

- [ ] **Step 9: Run the tests + build**

Run:
```bash
cd frontend/src-tauri
cargo test --lib database::repositories::setting 2>&1 | tail -20
cargo build 2>&1 | tail -15
```
Expected: `save_then_get_neohive_config_bearer` + `get_neohive_config_defaults_when_unset` pass; crate builds. (Note: the SQLx migration compiles into the binary; a clean build proves the new migration is well-formed.)

- [ ] **Step 10: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/migrations/20260709000002_add_neohive_auth_method.sql frontend/src-tauri/src/database/repositories/setting.rs frontend/src-tauri/src/summary/workflows/commands.rs
git commit -m "feat(neohive): :card_file_box: store auth method + config JSON; generalize config commands (backfills Cloudflare)"
```

---

## Task 3: Settings UI — method dropdown + per-method fields

**Files:**
- Modify: `frontend/src/types/workflow.ts` (`NeoHiveSettings` ~73-79)
- Modify: `frontend/src/components/workflows/WorkflowsSettings.tsx` (`neo` state ~22, load ~28-35, `saveNeo` ~39-50, NeoHive JSX section ~55-102)

**Interfaces:**
- Consumes: commands `api_get_neohive_config` (returns `{endpoint, enabled, authType, authConfig}`) + `api_save_neohive_config({endpoint, enabled, authType, authConfig})` (Task 2).
- Produces: the settings UI for all five methods.

- [ ] **Step 1: Update the TS types** — in `frontend/src/types/workflow.ts`, replace the `NeoHiveSettings` interface (~73-79) with:
```ts
export type NeoHiveAuthType = 'cloudflare_access' | 'bearer' | 'basic' | 'custom_header' | 'none';

/** Method-specific auth fields (camelCase; mirrors the Rust JSON config). */
export interface NeoHiveAuthConfig {
  clientId?: string;
  clientSecret?: string;
  token?: string;
  username?: string;
  password?: string;
  headerName?: string;
  headerValue?: string;
}

/** NeoHive connection settings (from api_get_neohive_config). */
export interface NeoHiveSettings {
  endpoint: string | null;
  enabled: boolean;
  authType: NeoHiveAuthType;
  authConfig: NeoHiveAuthConfig;
}
```

- [ ] **Step 2: Update the `neo` state + load + save in `WorkflowsSettings.tsx`**

Change the initial state (~22-24) to:
```tsx
  const [neo, setNeo] = useState<NeoHiveSettings>({
    endpoint: null, enabled: false, authType: 'cloudflare_access', authConfig: {},
  });
```
Change the load effect (~28-35) to:
```tsx
    invoke<NeoHiveSettings>('api_get_neohive_config')
      .then((cfg) => setNeo({
        endpoint: cfg.endpoint ?? DEFAULT_ENDPOINT,
        enabled: cfg.enabled,
        authType: cfg.authType ?? 'cloudflare_access',
        authConfig: cfg.authConfig ?? {},
      }))
      .catch((e) => console.error('Failed to load NeoHive config:', e));
```
Change `saveNeo` (~39-50) to:
```tsx
    try {
      await invoke('api_save_neohive_config', {
        endpoint: neo.endpoint || null,
        enabled: neo.enabled,
        authType: neo.authType,
        authConfig: neo.authConfig,
      });
      toast.success('NeoHive settings saved');
    } catch (e) {
      console.error('Failed to save NeoHive settings:', e);
      toast.error('Failed to save NeoHive settings');
    }
```
Add a small field-setter helper near the top of the component body (after the `neo` state):
```tsx
  const setField = (k: keyof import('@/types/workflow').NeoHiveAuthConfig, v: string) =>
    setNeo((n) => ({ ...n, authConfig: { ...n.authConfig, [k]: v } }));
```

- [ ] **Step 3: Replace the NeoHive JSX section** (~55-102, from `{/* NeoHive connection */}` through the closing `</section>` before `{/* Workflows list */}`) with:
```tsx
      {/* NeoHive connection */}
      <section className="space-y-3 border rounded-lg p-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="font-medium">NeoHive export</h3>
            <p className="text-xs text-muted-foreground">
              Connect to your NeoHive project. Your own infrastructure.
            </p>
          </div>
          <Switch checked={neo.enabled} onCheckedChange={(v) => setNeo((n) => ({ ...n, enabled: v }))} />
        </div>

        <div className="space-y-1">
          <Label>Endpoint</Label>
          <Input
            value={neo.endpoint ?? ''}
            onChange={(e) => setNeo((n) => ({ ...n, endpoint: e.target.value }))}
            placeholder={DEFAULT_ENDPOINT}
          />
        </div>

        <div className="space-y-1">
          <Label>Authentication method</Label>
          <select
            className="w-full border rounded-md h-9 px-2 text-sm bg-transparent"
            value={neo.authType}
            onChange={(e) => setNeo((n) => ({ ...n, authType: e.target.value as NeoHiveSettings['authType'] }))}
          >
            <option value="cloudflare_access">Cloudflare Access service token</option>
            <option value="bearer">Bearer token / API key</option>
            <option value="basic">Basic auth (username / password)</option>
            <option value="custom_header">Custom header</option>
            <option value="none">None (network-level, e.g. Tailscale / LAN)</option>
          </select>
        </div>

        {neo.authType === 'cloudflare_access' && (
          <>
            <div className="space-y-1">
              <Label>Access Client Id</Label>
              <Input value={neo.authConfig.clientId ?? ''} onChange={(e) => setField('clientId', e.target.value)} placeholder="xxxxxxxx.access" />
            </div>
            <div className="space-y-1">
              <Label>Access Client Secret</Label>
              <div className="flex gap-2">
                <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.clientSecret ?? ''} onChange={(e) => setField('clientSecret', e.target.value)} placeholder="Cloudflare Access client secret" />
                <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
              </div>
            </div>
          </>
        )}

        {neo.authType === 'bearer' && (
          <div className="space-y-1">
            <Label>Token</Label>
            <div className="flex gap-2">
              <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.token ?? ''} onChange={(e) => setField('token', e.target.value)} placeholder="Bearer token / API key" />
              <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
            </div>
          </div>
        )}

        {neo.authType === 'basic' && (
          <>
            <div className="space-y-1">
              <Label>Username</Label>
              <Input value={neo.authConfig.username ?? ''} onChange={(e) => setField('username', e.target.value)} placeholder="username" />
            </div>
            <div className="space-y-1">
              <Label>Password</Label>
              <div className="flex gap-2">
                <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.password ?? ''} onChange={(e) => setField('password', e.target.value)} placeholder="password" />
                <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
              </div>
            </div>
          </>
        )}

        {neo.authType === 'custom_header' && (
          <>
            <div className="space-y-1">
              <Label>Header name</Label>
              <Input value={neo.authConfig.headerName ?? ''} onChange={(e) => setField('headerName', e.target.value)} placeholder="X-Api-Key" />
            </div>
            <div className="space-y-1">
              <Label>Header value</Label>
              <div className="flex gap-2">
                <Input type={showSecret ? 'text' : 'password'} value={neo.authConfig.headerValue ?? ''} onChange={(e) => setField('headerValue', e.target.value)} placeholder="header value" />
                <Button variant="outline" size="icon" onClick={() => setShowSecret((s) => !s)}>{showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
              </div>
            </div>
          </>
        )}

        {neo.authType === 'none' && (
          <p className="text-xs text-muted-foreground">No credentials — Meetily reaches NeoHive over your network (e.g. Tailscale, LAN, or VPN). Only the endpoint is used.</p>
        )}

        <div className="flex justify-end"><Button onClick={saveNeo}>Save NeoHive settings</Button></div>
      </section>
```

- [ ] **Step 4: Typecheck**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -E "error TS" | grep -viE "bun:test"`
Expected: no output (no new type errors; the lone pre-existing `bun:test` error is filtered out).

- [ ] **Step 5: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src/types/workflow.ts frontend/src/components/workflows/WorkflowsSettings.tsx
git commit -m "feat(neohive): :sparkles: settings UI for selectable NeoHive auth methods"
```

---

## Manual verification (after all tasks)
- Your existing Cloudflare config: open Settings → Workflows → NeoHive export; the method should show **Cloudflare Access** with your endpoint + client id/secret intact (backfilled). Run a workflow export → still works.
- Switch to **Bearer** / **None**, save, reload — the choice + fields persist. (A `none` config with just an endpoint should connect against a network-reachable NeoHive with no auth headers.)

## Self-Review

**Spec coverage:**
- §3 auth enum + `from_parts` + `apply` (5 methods, reqwest built-ins) → Task 1. ✓
- §4 migration (columns + backfill) + `NeoHiveSettings` shape + get/save → Task 2. ✓
- §5 `NeoHiveConfigResponse` + get/save commands (secrets returned to webview) + export-path `from_parts` → Task 2. ✓
- §6 frontend types + method dropdown + per-method fields + load/save → Task 3. ✓
- §7 tests: `apply` per variant + `from_parts` (Task 1); repo round-trip + defaults (Task 2); tsc + manual (Task 3). ✓
- §8 backward-compat backfill + migration ordering (`20260709000002`) + no new dep → Tasks 1-2 + Global Constraints. ✓
- §10 open items: `NeoHiveSettings` struct shape confirmed (endpoint/access_client_id/access_client_secret/enabled → endpoint/enabled/auth_type/auth_config); old CF columns left as-is (not cleared) — documented; reqwest `basic_auth` output pinned to `Basic dXNlcjpwYXNz` for "user:pass". ✓

**Placeholder scan:** none — every step has exact code/commands. The migration + all edits show full content at their anchors.

**Type consistency:** `NeoHiveAuth::{from_parts, apply}`, `NeoHiveClient::new(endpoint, auth)`, `NeoHiveSettings{endpoint, enabled, auth_type, auth_config}`, `save_neohive_config(pool, endpoint, enabled, auth_type, auth_config)`, and the TS `{endpoint, enabled, authType, authConfig}` + snake_case type strings / camelCase field keys are used identically across Tasks 1-3. Cross-task note: Task 1 wires the export path to `CloudflareAccess{client_id, client_secret}` (compiles against the still-CF config); Task 2 rewrites that same block to `from_parts` after changing the config shape — the double-touch is intentional to keep each task's build green.
