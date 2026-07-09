# NeoHive Connection Auth Methods — Design

- **Date:** 2026-07-09
- **Status:** Approved (design); pending implementation plan.
- **Author:** Nader Awad (with Claude)
- **Scope:** Generalize NeoHive connection authentication in the Rust/Tauri core + the settings UI. Personal local fork.
- **Motivation:** NeoHive auth is currently hardcoded to a single method — a Cloudflare Access service token. The fork owner uses Cloudflare, but teammates connecting to their own NeoHive instances may use other access models (notably Tailscale, where being on the tailnet means **no app-layer credentials** are needed). Support multiple auth methods so any NeoHive instance can be connected.

## 1. Current state (baseline)

- `neohive/client.rs`: `NeoHiveClient::new(endpoint, access_client_id, access_client_secret)` unconditionally sends `CF-Access-Client-Id` + `CF-Access-Client-Secret` headers (plus the invariant `Content-Type`, `Accept`, `x-mcp-client: meetily`, and per-call `mcp-session-id`).
- Settings columns (`settings` table): `neohiveEndpoint`, `neohiveApiKey` (vestigial), `neohiveEnabled INTEGER`, `neohiveAccessClientId`, `neohiveAccessClientSecret`.
- `SettingsRepository::{get,save}_neohive_config` read/write those columns; `get` SELECTs endpoint/clientId/clientSecret/enabled.
- Commands `api_get_neohive_config` / `api_save_neohive_config` (in `summary/workflows/commands.rs`) expose endpoint + clientId + clientSecret + enabled. **`api_get_neohive_config` returns the secret to the local webview** (no masking) — the established convention; the UI masks with a show/hide toggle, mirroring provider API keys.
- The client is constructed in `api_run_workflow`/export path from the config.
- Frontend: `NeoHiveSettings` type `{endpoint, accessClientId, accessClientSecret, enabled}`; `WorkflowsSettings.tsx` renders endpoint + clientId + clientSecret + an enable Switch, loads via `api_get_neohive_config`, saves via `api_save_neohive_config`.
- **reqwest 0.11** is the HTTP client — it provides `.bearer_auth(token)` and `.basic_auth(user, Some(pass))` on `RequestBuilder`, so Bearer/Basic need **no new dependency** (no `base64` crate).

## 2. Goals / non-goals

**Goals**
1. Support these NeoHive connection auth methods: **Cloudflare Access** (existing), **Bearer token / API key**, **Basic auth** (username/password), **Custom header** (arbitrary name+value), **None / network-level** (no app credentials — Tailscale/LAN/VPN).
2. Introduce one auth-method abstraction and thread it through the client, settings model, config commands, and settings UI.
3. **Backward compatible:** the owner's existing Cloudflare config keeps working with no reconfiguration.
4. Keep secret handling consistent with the current convention (return-to-local-webview + UI show/hide).

**Non-goals**
- OAuth/OIDC interactive flows, mTLS, or Tailscale-specific identity-header *generation* (Tailscale is handled simply as "no app credentials — you're on the tailnet"). If a Tailscale-fronted instance needs a token, that's covered by Bearer/Custom-header.
- Multiple simultaneous custom headers (one custom header is the v1 escape hatch).
- Per-workflow auth (auth is a single connection-level config, as today).

## 3. Auth abstraction (`neohive/client.rs`, or a small `neohive/auth.rs`)

A plain enum plus an explicit constructor from `(type string, fields JSON)` and a method that applies itself to a reqwest `RequestBuilder`:

```rust
#[derive(Debug, Clone)]
pub enum NeoHiveAuth {
    CloudflareAccess { client_id: String, client_secret: String },
    Bearer { token: String },
    Basic { username: String, password: String },
    CustomHeader { header_name: String, header_value: String },
    None,
}
```
- **Storage model = two parts** (matching the settings columns AND the UI's dropdown-plus-fields split, so the enum's representation never has to agree with a serde tag):
  - the **type** is a snake_case string — `cloudflare_access` | `bearer` | `basic` | `custom_header` | `none` — stored in the `neohiveAuthType` column and sent as `authType` at the TS boundary;
  - the **fields** are a small JSON object (camelCase keys: `clientId`, `clientSecret`, `token`, `username`, `password`, `headerName`, `headerValue`) stored in `neohiveAuthConfig` and sent as `authConfig`.
- `NeoHiveAuth::from_parts(auth_type: &str, config: &serde_json::Value) -> Result<Self, String>` maps the type string + JSON fields into the enum (missing/blank required fields → a clear error; unknown/`none`/absent type → `None`). This is the single source of truth for the mapping; there is **no** `#[serde(tag = …)]` on the enum, so there is no camelCase-tag-vs-snake_case-column contradiction. (Storing the type as its own column mirrors how `provider` is a column while `customOpenAIConfig` is JSON, and keeps SQL/debugging legible.)
- `apply(self, b: reqwest::RequestBuilder) -> reqwest::RequestBuilder`:
  - `CloudflareAccess` → `.header("CF-Access-Client-Id", id).header("CF-Access-Client-Secret", secret)`
  - `Bearer` → `.bearer_auth(token)`
  - `Basic` → `.basic_auth(username, Some(password))`
  - `CustomHeader` → `.header(header_name, header_value)`
  - `None` → unchanged
- The invariant headers stay applied for all methods (unchanged).
- `NeoHiveClient::new(endpoint: String, auth: NeoHiveAuth) -> Self` replaces the 3-arg constructor; each request builder is passed through `auth.apply(...)`. Update the single construction site in `summary/workflows/commands.rs`.

## 4. Data model / settings

New migration `20260709000002_add_neohive_auth_method.sql` (after the diarization migrations):
```sql
ALTER TABLE settings ADD COLUMN neohiveAuthType TEXT;     -- cloudflare_access | bearer | basic | custom_header | none
ALTER TABLE settings ADD COLUMN neohiveAuthConfig TEXT;   -- JSON: method-specific fields (secrets included; local DB)

-- Backfill existing Cloudflare Access config into the generic model so it keeps working.
UPDATE settings
SET neohiveAuthType = 'cloudflare_access',
    neohiveAuthConfig = json_object('clientId', neohiveAccessClientId, 'clientSecret', neohiveAccessClientSecret)
WHERE neohiveAccessClientId IS NOT NULL OR neohiveAccessClientSecret IS NOT NULL;
```
- `neohiveEndpoint` + `neohiveEnabled` stay. The old `neohiveAccessClientId/Secret` columns become **vestigial** post-backfill (code reads the new columns); left in place (non-destructive, mirrors how `neohiveApiKey` was left vestigial).
- Rows with no prior CF config get `neohiveAuthType = NULL` → treated as unconfigured (equivalent to `none` with no endpoint).
- `SettingsRepository::get_neohive_config` returns `{ endpoint, enabled, auth_type: Option<String>, auth_config: Option<String> /* JSON */ }`. `save_neohive_config(endpoint, enabled, auth_type, auth_config)` upserts those. The `NeoHiveConfig` internal struct + its tests update accordingly (keep a `save_then_get` test for a non-CF method, e.g. bearer).

## 5. Config commands + secret handling (`summary/workflows/commands.rs`)

- `NeoHiveConfigResponse` → `{ endpoint: Option<String>, enabled: bool, authType: Option<String>, authConfig: Option<serde_json::Value> }` (camelCase serde). **Secrets are returned to the local webview** (unchanged convention); the UI masks them.
- `api_get_neohive_config` returns the above (parsing the stored `authConfig` JSON string into a `serde_json::Value`).
- `api_save_neohive_config(endpoint: Option<String>, enabled: bool, auth_type: Option<String>, auth_config: Option<serde_json::Value>)` — serializes `auth_config` to a JSON string and stores it. (Old param signature `access_client_id/secret` is replaced.)
- Client construction (export path): read `auth_type` + `auth_config`, build the `NeoHiveAuth` enum (map type string + parse JSON fields), `NeoHiveClient::new(endpoint, auth)`. If `auth_type` is `none`/unset, still connect (network-level) as long as an endpoint is present.

## 6. Frontend (`types/workflow.ts` + `components/workflows/WorkflowsSettings.tsx`)

- `NeoHiveSettings` type becomes: `{ endpoint, enabled, authType: NeoHiveAuthType, authConfig: NeoHiveAuthConfig }` where `NeoHiveAuthType = 'cloudflare_access' | 'bearer' | 'basic' | 'custom_header' | 'none'` and `authConfig` is a partial record of the possible fields (`clientId?, clientSecret?, token?, username?, password?, headerName?, headerValue?`).
- `WorkflowsSettings.tsx` NeoHive section: keep endpoint + enable Switch; add an **auth-method `<select>`/dropdown**; render only the selected method's fields:
  - `cloudflare_access` → Client Id + Client Secret (secret: show/hide)
  - `bearer` → Token (show/hide)
  - `basic` → Username + Password (password: show/hide)
  - `custom_header` → Header Name + Header Value (value: show/hide)
  - `none` → a short "No credentials — reachable via your network (e.g. Tailscale/LAN)." note, no fields
- On load, map the response (default to `cloudflare_access` when `authType` is set that way; if unset and legacy CF fields exist they've been backfilled, so `authType` will be `cloudflare_access`). Save sends `authType` + `authConfig` (only the active method's fields).
- Secret inputs reuse the app's existing show/hide pattern. No test runner → verify with `tsc --noEmit` + manual.

## 7. Testing

- **Rust unit tests** (`cargo test`): `NeoHiveAuth::apply` per variant — build a dummy `reqwest::Request` via a `RequestBuilder`, then assert `request.headers()`:
  - `CloudflareAccess` → both CF headers present with the right values.
  - `Bearer` → `Authorization: Bearer <token>`.
  - `Basic` → `Authorization: Basic <base64(user:pass)>` (assert the expected encoded value).
  - `CustomHeader` → the arbitrary header present.
  - `None` → no `Authorization`/`CF-Access-*` header.
  - `NeoHiveAuth::from_parts` mapping: each snake_case type string + its fields JSON produces the right variant; a blank/missing required field errors; an unknown/absent type yields `None`.
- **Repository test:** `save_then_get` for a non-CF method (e.g. bearer) round-trips `authType` + `authConfig`; a backfill/compat check that a pre-existing CF config reads back as `cloudflare_access`.
- **Frontend:** `tsc --noEmit` + manual (switch methods, save, confirm export still works on the owner's CF instance).

## 8. Backward compatibility & migration safety

- The migration backfills the owner's live CF config into the new model → **no reconfiguration, export keeps working**.
- Migration dated `20260709000002` (after `20260709000001`) so it applies in order on the existing DB.
- No new Rust dependency (reqwest built-ins for Bearer/Basic). No new crate.

## 9. Conventions
- New behavior via existing Tauri commands (generalized signatures); register nothing new (commands already registered).
- Secrets: returned to the local webview (existing convention), never logged; the settings row is local.
- SQLx timestamped migration; serde camelCase at the TS boundary; snake_case `authType` discriminator values in SQL.
- Gitmoji commits; no AI attribution. Personal fork; local `main` only.

## 10. Open items to resolve during planning
- Confirm the exact internal `NeoHiveConfig` struct shape in `SettingsRepository` and update its two existing tests without breaking the export path.
- Decide whether `save_neohive_config` also clears the vestigial `neohiveAccessClientId/Secret` columns (leave as-is is fine; they're ignored).
- Confirm reqwest 0.11 `basic_auth`/`bearer_auth` header output values for the assertions.
