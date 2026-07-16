# Custom Vocabulary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user define domain terms (e.g. "Snyk", "NeoHive", "Logilica") so transcription and summaries stop mangling them, via a two-entry-type dictionary (Terms + Corrections) with optional per-term descriptions.

**Architecture:** One pure, unit-tested `vocabulary` Rust module holds the data model + all pure logic (deterministic corrections, Whisper prompt builder, glossary builder). Three thin wiring layers consume it: (1) **Corrections** — deterministic post-ASR find-and-replace at the batch chokepoint and the live-recording path (engine-agnostic); (2) **Whisper term biasing** — `set_initial_prompt` (Whisper only); (3) **Description glossary** — prepended to the summarization prompt. Config is a JSON blob on the single-row `settings` table (mirrors `customOpenAIConfig`); consumers with a DB pool read it directly, and the live hot path reads a process global refreshed at recording-start and on save.

**Tech Stack:** Rust / Tauri (sqlx runtime queries, whisper-rs 0.13.2), Next.js/TypeScript + shadcn UI, SQLite.

## Global Constraints

- **No new external dependencies.** Implement whole-word replacement by hand; do NOT add `regex` or any crate. (User preference: smallest viable incremental change.)
- **v1 scope = Whisper + Approach A** (corrections + Whisper `initial_prompt` + description→glossary). Parakeet acoustic CTC rescoring is an explicit non-goal (v2).
- **`WhisperParams::set_initial_prompt` panics on null bytes** (CString::new) — strip `\0` from any vocabulary string before passing it.
- **serde camelCase** on all config structs so they round-trip with the frontend (`#[serde(rename = "...")]`), matching `CustomOpenAIConfig`.
- **sqlx runtime queries only** (`sqlx::query(...)`, like `setting.rs`) — no `query!` macros, so no `.sqlx` offline prepare step is needed.
- **Settings table is single-row keyed `id = '1'`**; writes use `INSERT ... ON CONFLICT(id) DO UPDATE SET`.
- **Commit style:** gitmoji conventional commits (e.g. `feat(vocabulary): :sparkles: ...`). **No `Co-Authored-By` lines and no AI attribution** (per repo CLAUDE.md).
- **Error handling:** follow the local file's convention — `setting.rs`/repo returns `sqlx::Error`; commands return `Result<_, String>`; frontend uses try-catch + `toast`.

---

## File Structure

**Created:**
- `frontend/src-tauri/src/vocabulary/mod.rs` — data model (`VocabularyConfig`, `VocabularyEntry`, `Correction`) + pure logic (`apply_corrections`, `build_whisper_prompt`, `build_glossary`) + derivation methods. The whole feature's brain; fully unit-tested.
- `frontend/src-tauri/migrations/20260717000000_add_vocabulary.sql` — adds the `vocabularyConfig TEXT` column.
- `frontend/src/types/vocabulary.ts` — TS mirror of the config structs (camelCase).
- `frontend/src/components/settings/VocabularySettings.tsx` — the management UI panel.

**Modified:**
- `frontend/src-tauri/src/lib.rs` — `pub mod vocabulary;`, the `VOCABULARY_CONFIG` global + getter/setter, command registration.
- `frontend/src-tauri/src/database/repositories/setting.rs` — `get_vocabulary_config` / `save_vocabulary_config`.
- `frontend/src-tauri/src/api/api.rs` — `api_get_vocabulary_config` / `api_save_vocabulary_config`.
- `frontend/src-tauri/src/audio/common.rs` — apply corrections inside `create_transcript_segments`.
- `frontend/src-tauri/src/audio/import.rs`, `frontend/src-tauri/src/audio/retranscription.rs` — load corrections from pool, pass to `create_transcript_segments`.
- `frontend/src-tauri/src/audio/transcription/worker.rs` — apply corrections to live text; refresh global at recording start.
- `frontend/src-tauri/src/whisper_engine/whisper_engine.rs` — `set_initial_prompt` in both transcribe fns.
- `frontend/src-tauri/src/summary/service.rs`, `frontend/src-tauri/src/summary/workflows/runner.rs` — prepend glossary to `custom_prompt`.
- `frontend/src/app/settings/page.tsx` — register the new panel/tab.

---

## Task 1: `vocabulary` module — data model + pure logic (TDD)

**Files:**
- Create: `frontend/src-tauri/src/vocabulary/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (add `pub mod vocabulary;` near the other `pub mod` lines, ~line 38-58)

**Interfaces:**
- Produces:
  - `pub struct VocabularyEntry { id, entry_type, text, replacement, description, case_sensitive, enabled, created_at, updated_at }` (serde camelCase)
  - `pub struct VocabularyConfig { enabled: bool, entries: Vec<VocabularyEntry> }` with `Default`
  - `pub struct Correction { pub from: String, pub to: String, pub case_sensitive: bool }`
  - `impl VocabularyConfig { pub fn corrections(&self) -> Vec<Correction>; pub fn term_texts(&self) -> Vec<String>; pub fn glossary_entries(&self) -> Vec<(String, String)> }`
  - `pub fn apply_corrections(text: &str, corrections: &[Correction]) -> String`
  - `pub fn build_whisper_prompt(terms: &[String], max_chars: usize) -> String`
  - `pub fn build_glossary(entries: &[(String, String)], max_chars: usize) -> String`

- [ ] **Step 1: Write the module with pure logic**

Create `frontend/src-tauri/src/vocabulary/mod.rs`:

```rust
//! Custom vocabulary: a two-entry-type dictionary (Terms + Corrections) plus the
//! pure logic that consumes it. Terms bias Whisper (`initial_prompt`); Corrections
//! are deterministic post-ASR find-and-replace; descriptions feed a summary glossary.
//! This module is pure and has no I/O — persistence lives in the settings repository.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// One dictionary entry. `entry_type` is "term" (bias the recognizer) or
/// "correction" (replace `text` with `replacement`). `description` is optional
/// and, when present, feeds the summarization glossary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VocabularyEntry {
    pub id: String,
    #[serde(rename = "entryType")]
    pub entry_type: String,
    pub text: String,
    #[serde(default)]
    pub replacement: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "caseSensitive", default)]
    pub case_sensitive: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<String>,
}

/// The whole persisted vocabulary. `enabled` is a global master switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VocabularyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub entries: Vec<VocabularyEntry>,
}

impl Default for VocabularyConfig {
    fn default() -> Self {
        Self { enabled: true, entries: Vec::new() }
    }
}

/// A resolved correction ready to apply.
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    pub from: String,
    pub to: String,
    pub case_sensitive: bool,
}

impl VocabularyConfig {
    /// Enabled correction entries that have a replacement. Empty if the master
    /// switch is off.
    pub fn corrections(&self) -> Vec<Correction> {
        if !self.enabled {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.enabled && e.entry_type == "correction" && !e.text.is_empty())
            .filter_map(|e| {
                e.replacement.as_ref().map(|r| Correction {
                    from: e.text.clone(),
                    to: r.clone(),
                    case_sensitive: e.case_sensitive,
                })
            })
            .collect()
    }

    /// Enabled term texts (for Whisper biasing). Empty if the master switch is off.
    pub fn term_texts(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.enabled && e.entry_type == "term" && !e.text.trim().is_empty())
            .map(|e| e.text.clone())
            .collect()
    }

    /// (term, description) for every enabled entry that has a non-empty
    /// description (for the summary glossary). Empty if the master switch is off.
    pub fn glossary_entries(&self) -> Vec<(String, String)> {
        if !self.enabled {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.enabled)
            .filter_map(|e| {
                e.description
                    .as_ref()
                    .map(|d| d.trim())
                    .filter(|d| !d.is_empty())
                    .map(|d| (e.text.clone(), d.to_string()))
            })
            .collect()
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whole-word, boundary-aware replacement of `from` with `to`. Case-insensitive
/// unless `case_sensitive`. ASCII case folding only (fine for proper nouns);
/// non-ASCII case differences won't match unless `case_sensitive` is false and
/// the letters are ASCII. Left-to-right, non-overlapping.
fn replace_whole_word(haystack: &str, from: &str, to: &str, case_sensitive: bool) -> String {
    if from.is_empty() {
        return haystack.to_string();
    }
    let hay: Vec<char> = haystack.chars().collect();
    let pat: Vec<char> = from.chars().collect();
    let plen = pat.len();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while i < hay.len() {
        let end = i + plen;
        let is_match = end <= hay.len()
            && hay[i..end].iter().zip(pat.iter()).all(|(a, b)| {
                if case_sensitive {
                    a == b
                } else {
                    a.eq_ignore_ascii_case(b)
                }
            });
        let left_ok = i == 0 || !is_word_char(hay[i - 1]);
        let right_ok = end >= hay.len() || !is_word_char(hay[end]);
        if is_match && left_ok && right_ok {
            out.push_str(to);
            i = end;
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// Apply every correction in order to `text`.
pub fn apply_corrections(text: &str, corrections: &[Correction]) -> String {
    let mut result = text.to_string();
    for c in corrections {
        result = replace_whole_word(&result, &c.from, &c.to, c.case_sensitive);
    }
    result
}

/// Build a Whisper `initial_prompt` from term texts: comma-joined, `\0`-stripped,
/// clipped to `max_chars` on a term boundary. Empty when there are no terms.
pub fn build_whisper_prompt(terms: &[String], max_chars: usize) -> String {
    let mut out = String::new();
    for t in terms {
        let cleaned = t.replace('\0', "");
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        let candidate = if out.is_empty() {
            cleaned.to_string()
        } else {
            format!("{}, {}", out, cleaned)
        };
        if candidate.chars().count() > max_chars {
            break;
        }
        out = candidate;
    }
    out
}

/// Build a glossary block for the summarization prompt. Empty when no described
/// terms. Clipped to `max_chars` on a line boundary.
pub fn build_glossary(entries: &[(String, String)], max_chars: usize) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let header = "Glossary of domain terms (use these exact spellings; do not confuse them):\n";
    let mut body = String::from(header);
    for (term, desc) in entries {
        let line = format!("- {}: {}\n", term.trim(), desc.trim());
        if body.chars().count() + line.chars().count() > max_chars {
            break;
        }
        body.push_str(&line);
    }
    // If nothing but the header fit, treat as empty.
    if body == header {
        return String::new();
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corr(from: &str, to: &str, cs: bool) -> Correction {
        Correction { from: from.into(), to: to.into(), case_sensitive: cs }
    }

    #[test]
    fn corrects_case_insensitive_whole_word() {
        let c = vec![corr("sneak", "Snyk", false)];
        assert_eq!(apply_corrections("I ran a sneak scan", &c), "I ran a Snyk scan");
        assert_eq!(apply_corrections("Sneak found bugs", &c), "Snyk found bugs");
    }

    #[test]
    fn does_not_touch_substrings() {
        let c = vec![corr("sneak", "Snyk", false)];
        assert_eq!(apply_corrections("that was sneaky", &c), "that was sneaky");
    }

    #[test]
    fn preserves_adjacent_punctuation() {
        let c = vec![corr("sneak", "Snyk", false)];
        assert_eq!(apply_corrections("sneak, then done", &c), "Snyk, then done");
    }

    #[test]
    fn case_sensitive_only_matches_exact() {
        let c = vec![corr("OSS", "OSS project", true)];
        assert_eq!(apply_corrections("the oss thing", &c), "the oss thing");
        assert_eq!(apply_corrections("the OSS thing", &c), "the OSS project thing");
    }

    #[test]
    fn multi_word_from() {
        let c = vec![corr("near hive", "NeoHive", false)];
        assert_eq!(apply_corrections("the near hive system", &c), "the NeoHive system");
    }

    #[test]
    fn empty_from_is_noop() {
        let c = vec![corr("", "x", false)];
        assert_eq!(apply_corrections("hello", &c), "hello");
    }

    #[test]
    fn whisper_prompt_joins_and_clips() {
        let terms = vec!["Snyk".to_string(), "NeoHive".to_string(), "Logilica".to_string()];
        assert_eq!(build_whisper_prompt(&terms, 100), "Snyk, NeoHive, Logilica");
        // clip: only "Snyk" fits before the next term would exceed 6 chars
        assert_eq!(build_whisper_prompt(&terms, 6), "Snyk");
    }

    #[test]
    fn whisper_prompt_strips_null_bytes_and_blanks() {
        let terms = vec!["Sn\0yk".to_string(), "   ".to_string(), "NeoHive".to_string()];
        assert_eq!(build_whisper_prompt(&terms, 100), "Snyk, NeoHive");
    }

    #[test]
    fn glossary_builds_and_skips_when_empty() {
        assert_eq!(build_glossary(&[], 1000), "");
        let e = vec![("Snyk".to_string(), "SAST company".to_string())];
        let g = build_glossary(&e, 1000);
        assert!(g.contains("Glossary of domain terms"));
        assert!(g.contains("- Snyk: SAST company"));
    }

    #[test]
    fn config_derivations_respect_master_switch_and_types() {
        let json = r#"{
          "enabled": true,
          "entries": [
            {"id":"1","entryType":"term","text":"Snyk","description":"SAST company","enabled":true},
            {"id":"2","entryType":"correction","text":"sneak","replacement":"Snyk","enabled":true},
            {"id":"3","entryType":"correction","text":"off","replacement":"OFF","enabled":false}
          ]
        }"#;
        let cfg: VocabularyConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.term_texts(), vec!["Snyk".to_string()]);
        assert_eq!(cfg.corrections().len(), 1);
        assert_eq!(cfg.corrections()[0].to, "Snyk");
        assert_eq!(cfg.glossary_entries(), vec![("Snyk".to_string(), "SAST company".to_string())]);

        let mut off = cfg.clone();
        off.enabled = false;
        assert!(off.term_texts().is_empty());
        assert!(off.corrections().is_empty());
        assert!(off.glossary_entries().is_empty());
    }

    #[test]
    fn config_serde_round_trips_camel_case() {
        let cfg = VocabularyConfig {
            enabled: true,
            entries: vec![VocabularyEntry {
                id: "1".into(),
                entry_type: "correction".into(),
                text: "sneak".into(),
                replacement: Some("Snyk".into()),
                description: None,
                case_sensitive: false,
                enabled: true,
                created_at: None,
                updated_at: None,
            }],
        };
        let s = serde_json::to_string(&cfg).unwrap();
        assert!(s.contains("\"entryType\""));
        assert!(s.contains("\"caseSensitive\""));
        let back: VocabularyConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].entry_type, "correction");
    }
}
```

- [ ] **Step 2: Register the module**

In `frontend/src-tauri/src/lib.rs`, alongside the other `pub mod` declarations (~lines 38-58), add:

```rust
pub mod vocabulary;
```

- [ ] **Step 3: Run the tests — verify they pass**

Run: `cd frontend/src-tauri && cargo test --lib vocabulary:: 2>&1 | tail -30`
Expected: all `vocabulary::tests::*` PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/vocabulary/mod.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(vocabulary): :sparkles: add vocabulary module (model + corrections/prompt/glossary logic)"
```

---

## Task 2: Persistence — migration, repository, global, commands

**Files:**
- Create: `frontend/src-tauri/migrations/20260717000000_add_vocabulary.sql`
- Modify: `frontend/src-tauri/src/database/repositories/setting.rs`
- Modify: `frontend/src-tauri/src/api/api.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `crate::vocabulary::VocabularyConfig` (Task 1).
- Produces:
  - `SettingsRepository::get_vocabulary_config(pool) -> Result<Option<VocabularyConfig>, sqlx::Error>`
  - `SettingsRepository::save_vocabulary_config(pool, &VocabularyConfig) -> Result<(), sqlx::Error>`
  - `crate::get_vocabulary_config_internal() -> VocabularyConfig` and `crate::set_vocabulary_config_internal(VocabularyConfig)`
  - Commands `api_get_vocabulary_config` (reads DB) and `api_save_vocabulary_config` (writes DB + refreshes global)

- [ ] **Step 1: Write the migration**

Create `frontend/src-tauri/migrations/20260717000000_add_vocabulary.sql`:

```sql
-- Custom vocabulary: a two-entry-type dictionary (terms + corrections) with
-- optional per-term descriptions. JSON blob on the single-row settings table
-- (id = '1'; camelCase columns), mirroring customOpenAIConfig.
ALTER TABLE settings ADD COLUMN vocabularyConfig TEXT;
```

- [ ] **Step 2: Add repository get/save**

In `frontend/src-tauri/src/database/repositories/setting.rs`, add `use crate::vocabulary::VocabularyConfig;` near the top imports (next to `use crate::summary::CustomOpenAIConfig;`), then add inside `impl SettingsRepository` (mirroring `get/save_custom_openai_config` at 291-361):

```rust
    pub async fn get_vocabulary_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<VocabularyConfig>, sqlx::Error> {
        use sqlx::Row;
        let row = sqlx::query(
            r#"
            SELECT vocabularyConfig
            FROM settings
            WHERE id = '1'
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await?;
        match row {
            Some(record) => {
                let config_json: Option<String> = record.get("vocabularyConfig");
                if let Some(json) = config_json {
                    let config: VocabularyConfig = serde_json::from_str(&json).map_err(|e| {
                        sqlx::Error::Protocol(
                            format!("Invalid JSON in vocabularyConfig: {}", e).into(),
                        )
                    })?;
                    Ok(Some(config))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    pub async fn save_vocabulary_config(
        pool: &SqlitePool,
        config: &VocabularyConfig,
    ) -> std::result::Result<(), sqlx::Error> {
        let config_json = serde_json::to_string(config).map_err(|e| {
            sqlx::Error::Protocol(format!("Failed to serialize vocabularyConfig: {}", e).into())
        })?;
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, vocabularyConfig)
            VALUES ('1', 'openrouter', '', 'large-v3', $1)
            ON CONFLICT(id) DO UPDATE SET
                vocabularyConfig = excluded.vocabularyConfig
            "#,
        )
        .bind(config_json)
        .execute(pool)
        .await?;
        Ok(())
    }
```

Note: `ON CONFLICT` updates only `vocabularyConfig`, so an existing row keeps its real `provider`/`model`/`whisperModel`; the `VALUES` defaults only apply on the (rare) first-ever insert. This mirrors `save_custom_openai_config`.

- [ ] **Step 3: Add the process global + getter/setter**

In `frontend/src-tauri/src/lib.rs`, next to the `LANGUAGE_PREFERENCE` static (~line 70-71), add (mirroring its `LazyLock<StdMutex<...>>` shape) and import the type:

```rust
use crate::vocabulary::VocabularyConfig;

static VOCABULARY_CONFIG: std::sync::LazyLock<StdMutex<VocabularyConfig>> =
    std::sync::LazyLock::new(|| StdMutex::new(VocabularyConfig::default()));

pub fn get_vocabulary_config_internal() -> VocabularyConfig {
    VOCABULARY_CONFIG
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn set_vocabulary_config_internal(config: VocabularyConfig) {
    if let Ok(mut g) = VOCABULARY_CONFIG.lock() {
        *g = config;
    }
}
```

(If `StdMutex` is a local alias, reuse it exactly as `LANGUAGE_PREFERENCE` does; if `VocabularyConfig` is already imported elsewhere in `lib.rs`, don't duplicate the `use`.)

- [ ] **Step 4: Add the Tauri commands**

In `frontend/src-tauri/src/api/api.rs`, mirroring `api_get_custom_openai_config` (1257-1280) and `api_save_custom_openai_config` (1183-1253). Add `use crate::vocabulary::VocabularyConfig;` if not present:

```rust
#[tauri::command]
pub async fn api_get_vocabulary_config<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<Option<VocabularyConfig>, String> {
    let pool = state.db_manager.pool();
    crate::database::repositories::setting::SettingsRepository::get_vocabulary_config(pool)
        .await
        .map_err(|e| format!("Failed to get vocabulary config: {}", e))
}

#[tauri::command]
pub async fn api_save_vocabulary_config<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    config: VocabularyConfig,
) -> Result<(), String> {
    let pool = state.db_manager.pool();
    crate::database::repositories::setting::SettingsRepository::save_vocabulary_config(pool, &config)
        .await
        .map_err(|e| format!("Failed to save vocabulary config: {}", e))?;
    // Refresh the hot-path global so live recording + Whisper pick it up immediately.
    crate::set_vocabulary_config_internal(config);
    Ok(())
}
```

(Match the exact `AppState` import path and the `state.db_manager.pool()` accessor used by the neighboring custom-openai commands. If those commands don't take an `AppHandle` param, drop `_app` to match their signature.)

- [ ] **Step 5: Register the commands**

In `frontend/src-tauri/src/lib.rs`, inside `tauri::generate_handler![ ... ]`, next to the custom-openai config commands (~lines 659-661), add:

```rust
            api::api_get_vocabulary_config,
            api::api_save_vocabulary_config,
```

- [ ] **Step 6: Verify it compiles**

Run: `cd frontend/src-tauri && cargo build 2>&1 | tail -30`
Expected: builds clean (no errors). The new migration runs automatically on next app launch via the existing migrator.

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/migrations/20260717000000_add_vocabulary.sql frontend/src-tauri/src/database/repositories/setting.rs frontend/src-tauri/src/api/api.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(vocabulary): :sparkles: persist vocabulary config + global + Tauri commands"
```

---

## Task 3: Layer 1 — corrections wiring (batch + live)

**Files:**
- Modify: `frontend/src-tauri/src/audio/common.rs` (`create_transcript_segments`, ~51-70)
- Modify: `frontend/src-tauri/src/audio/import.rs` (~836)
- Modify: `frontend/src-tauri/src/audio/retranscription.rs` (~624)
- Modify: `frontend/src-tauri/src/audio/transcription/worker.rs` (~277-278 build; recording-start spot)

**Interfaces:**
- Consumes: `crate::vocabulary::{Correction, apply_corrections}`, `crate::get_vocabulary_config_internal`, `crate::set_vocabulary_config_internal`, `SettingsRepository::get_vocabulary_config`.

- [ ] **Step 1: Thread corrections into `create_transcript_segments`**

In `frontend/src-tauri/src/audio/common.rs`, change the signature and the text line (currently line 61 `text: text.trim().to_string(),`):

```rust
pub(crate) fn create_transcript_segments(
    transcripts: &[(String, f64, f64, Option<String>)],
    corrections: &[crate::vocabulary::Correction],
) -> Vec<TranscriptSegment> {
    transcripts
        .iter()
        .map(|(text, start_ms, end_ms, speaker)| {
            let start_seconds = start_ms / 1000.0;
            let end_seconds = end_ms / 1000.0;
            let duration = end_seconds - start_seconds;
            TranscriptSegment {
                id: format!("transcript-{}", Uuid::new_v4()),
                text: crate::vocabulary::apply_corrections(text.trim(), corrections),
                timestamp: chrono::Utc::now().to_rfc3339(),
                audio_start_time: Some(start_seconds),
                audio_end_time: Some(end_seconds),
                duration: Some(duration),
                speaker: speaker.clone(),
            }
        })
        .collect()
}
```

- [ ] **Step 2: Update the two batch callers to load + pass corrections**

In `frontend/src-tauri/src/audio/import.rs` (~836) and `frontend/src-tauri/src/audio/retranscription.rs` (~624), both have `AppState`/pool in scope right after the call (they fetch `app.try_state::<AppState>()` to save). Move/obtain the pool just before the call and load corrections from the DB (authoritative). Replace `let segments = create_transcript_segments(&all_transcripts);` with:

```rust
            let corrections = {
                let app_state = app.state::<AppState>();
                let pool = app_state.db_manager.pool();
                crate::database::repositories::setting::SettingsRepository::get_vocabulary_config(pool)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .corrections()
            };
            let segments = create_transcript_segments(&all_transcripts, &corrections);
```

(Use whichever state accessor the surrounding code already uses — `app.state::<AppState>()` or `app.try_state::<AppState>()`. Match the existing `db_manager.pool()` accessor. If a pool binding already exists a few lines below, hoist it above the call instead of re-fetching.)

- [ ] **Step 3: Apply corrections on the live path + refresh global at recording start**

In `frontend/src-tauri/src/audio/transcription/worker.rs`, just before the `let update = TranscriptUpdate { text: transcript, ... }` build (~line 277), correct the text using the hot-path global:

```rust
            let transcript = {
                let cfg = crate::get_vocabulary_config_internal();
                crate::vocabulary::apply_corrections(&transcript, &cfg.corrections())
            };
            let update = TranscriptUpdate {
                text: transcript,
                // ... unchanged ...
```

Then ensure the global is fresh for the session: locate where transcription/recording starts and the worker task is spawned (e.g. `start_transcription_task` in this file, or the recording-start command in `recording_commands.rs`) — a place with `AppState`/pool access — and load the config into the global once before the worker begins:

```rust
    // Refresh the vocabulary global from the DB so the live path (+ Whisper) is current.
    if let Some(app_state) = app.try_state::<AppState>() {
        let pool = app_state.db_manager.pool();
        if let Ok(cfg) = crate::database::repositories::setting::SettingsRepository::get_vocabulary_config(pool).await {
            crate::set_vocabulary_config_internal(cfg.unwrap_or_default());
        }
    }
```

(Adapt to the actual handle/state available at that call site. The `api_save_vocabulary_config` command already refreshes the global on edits, so this only covers the "app restarted, never re-saved" case.)

- [ ] **Step 4: Verify it compiles**

Run: `cd frontend/src-tauri && cargo build 2>&1 | tail -30`
Expected: builds clean.

- [ ] **Step 5: Manual verification (record its result in the commit/PR)**

There is no pure unit for wiring; verify behaviorally: launch the app, add a correction `sneak → Snyk` in Settings, save, then (a) do a short live recording saying "sneak", and (b) Retranscribe an existing meeting — confirm both show "Snyk". Pure correction behavior is already covered by Task 1 tests.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/audio/common.rs frontend/src-tauri/src/audio/import.rs frontend/src-tauri/src/audio/retranscription.rs frontend/src-tauri/src/audio/transcription/worker.rs
git commit -m "feat(vocabulary): :sparkles: apply correction dictionary to live + batch transcripts"
```

---

## Task 4: Layer 2 — Whisper `initial_prompt` term biasing

**Files:**
- Modify: `frontend/src-tauri/src/whisper_engine/whisper_engine.rs` (both `transcribe_audio_with_confidence` ~526-585 and `transcribe_audio` ~643-740)

**Interfaces:**
- Consumes: `crate::get_vocabulary_config_internal`, `crate::vocabulary::build_whisper_prompt`.

- [ ] **Step 1: Set the initial prompt in both transcribe fns**

In each function, after the block of `params.set_*(...)` calls and **before** `state.full(params, &audio_data)?;`, add:

```rust
        // Bias recognition toward the user's custom vocabulary terms (Whisper only).
        let vocab_prompt = crate::vocabulary::build_whisper_prompt(
            &crate::get_vocabulary_config_internal().term_texts(),
            600,
        );
        if !vocab_prompt.is_empty() {
            params.set_initial_prompt(&vocab_prompt);
        }
```

For `transcribe_audio_with_confidence` place it after ~line 568 (`params.set_single_segment(false);`). For `transcribe_audio` place it after ~line 685. `build_whisper_prompt` already strips `\0`, so the `set_initial_prompt` null-byte panic cannot trigger.

- [ ] **Step 2: Verify it compiles**

Run: `cd frontend/src-tauri && cargo build 2>&1 | tail -30`
Expected: builds clean.

- [ ] **Step 3: Manual verification**

Add a term "Snyk" in Settings, save, and transcribe a clip where the word is spoken; confirm the term is favored (soft bias — Corrections remain the deterministic backstop). Budget/clip logic is covered by Task 1 tests.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/whisper_engine/whisper_engine.rs
git commit -m "feat(vocabulary): :sparkles: bias Whisper transcription with custom terms via initial_prompt"
```

---

## Task 5: Layer 3 — description glossary in the summary prompt

**Files:**
- Modify: `frontend/src-tauri/src/summary/service.rs` (~508-529, before the `generate_meeting_summary` call)
- Modify: `frontend/src-tauri/src/summary/workflows/runner.rs` (~137-147)

**Interfaces:**
- Consumes: `crate::get_vocabulary_config_internal`, `crate::vocabulary::build_glossary`.

- [ ] **Step 1: Prepend the glossary in the built-in summary path**

In `frontend/src-tauri/src/summary/service.rs`, immediately before the `generate_meeting_summary(...)` call at ~line 508, shadow `custom_prompt` (the `String` param) with a glossary-prefixed version:

```rust
    let custom_prompt = {
        let glossary = crate::vocabulary::build_glossary(
            &crate::get_vocabulary_config_internal().glossary_entries(),
            2000,
        );
        if glossary.is_empty() {
            custom_prompt
        } else if custom_prompt.trim().is_empty() {
            glossary
        } else {
            format!("{}\n\n{}", glossary, custom_prompt)
        }
    };
```

Then pass `&custom_prompt` to the call exactly as before.

- [ ] **Step 2: Prepend the glossary in the workflows path**

In `frontend/src-tauri/src/summary/workflows/runner.rs`, right after `let custom_prompt = workflow.custom_prompt.clone().unwrap_or_default();` (~line 137), add:

```rust
    let custom_prompt = {
        let glossary = crate::vocabulary::build_glossary(
            &crate::get_vocabulary_config_internal().glossary_entries(),
            2000,
        );
        if glossary.is_empty() {
            custom_prompt
        } else if custom_prompt.trim().is_empty() {
            glossary
        } else {
            format!("{}\n\n{}", glossary, custom_prompt)
        }
    };
```

(The existing `&custom_prompt` argument at ~line 147 now carries the glossary. Both call sites read the process global — populated at recording start and refreshed on save — so no extra pool read is needed. If you prefer authoritative reads here, both sites have a pool in scope and may use `SettingsRepository::get_vocabulary_config(pool)` instead; the global is chosen for uniformity and is always current.)

- [ ] **Step 3: Verify it compiles**

Run: `cd frontend/src-tauri && cargo build 2>&1 | tail -30`
Expected: builds clean.

- [ ] **Step 4: Manual verification**

Add a term "Snyk" with description "developer-security / SAST company" and a term "NeoHive" with a description, save, then generate a summary/run a workflow on a meeting mentioning them — confirm the summary uses the correct terms/meanings. Glossary formatting/clip is covered by Task 1 tests.

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/summary/service.rs frontend/src-tauri/src/summary/workflows/runner.rs
git commit -m "feat(vocabulary): :sparkles: inject term-description glossary into summary prompt"
```

---

## Task 6: Frontend — vocabulary settings panel

**Files:**
- Create: `frontend/src/types/vocabulary.ts`
- Create: `frontend/src/components/settings/VocabularySettings.tsx`
- Modify: `frontend/src/app/settings/page.tsx`

**Interfaces:**
- Consumes: Tauri commands `api_get_vocabulary_config` / `api_save_vocabulary_config` (Task 2).

- [ ] **Step 1: Add the TS types**

Create `frontend/src/types/vocabulary.ts`:

```ts
// Mirrors crate::vocabulary::{VocabularyEntry, VocabularyConfig} (serde camelCase).
export type VocabularyEntryType = 'term' | 'correction';

export interface VocabularyEntry {
  id: string;
  entryType: VocabularyEntryType;
  text: string;
  replacement?: string | null;
  description?: string | null;
  caseSensitive: boolean;
  enabled: boolean;
  createdAt?: string | null;
  updatedAt?: string | null;
}

export interface VocabularyConfig {
  enabled: boolean;
  entries: VocabularyEntry[];
}

export const EMPTY_VOCABULARY_CONFIG: VocabularyConfig = { enabled: true, entries: [] };
```

- [ ] **Step 2: Write the panel**

Create `frontend/src/components/settings/VocabularySettings.tsx`, mirroring the get-on-mount / local-state / save-handler pattern from `WorkflowsSettings.tsx` (invoke on mount ~line 44, save handler ~line 65) and using the shared UI primitives (`Button`, `Input`, `Switch`, `Label`, `Select`, `Textarea`) the way `WorkflowEditor.tsx` imports them:

```tsx
'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Switch } from '@/components/ui/switch';
import { Label } from '@/components/ui/label';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Trash2, Plus } from 'lucide-react';
import { VocabularyConfig, VocabularyEntry, EMPTY_VOCABULARY_CONFIG } from '@/types/vocabulary';

function newEntry(): VocabularyEntry {
  return {
    id: (crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`),
    entryType: 'term',
    text: '',
    replacement: null,
    description: null,
    caseSensitive: false,
    enabled: true,
  };
}

export function VocabularySettings() {
  const [config, setConfig] = useState<VocabularyConfig>(EMPTY_VOCABULARY_CONFIG);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    invoke<VocabularyConfig | null>('api_get_vocabulary_config')
      .then((cfg) => { if (cfg) setConfig(cfg); })
      .catch((e) => console.error('Failed to load vocabulary config:', e));
  }, []);

  const update = (id: string, patch: Partial<VocabularyEntry>) =>
    setConfig((c) => ({ ...c, entries: c.entries.map((e) => (e.id === id ? { ...e, ...patch } : e)) }));
  const remove = (id: string) =>
    setConfig((c) => ({ ...c, entries: c.entries.filter((e) => e.id !== id) }));
  const add = () => setConfig((c) => ({ ...c, entries: [...c.entries, newEntry()] }));

  const save = async () => {
    setSaving(true);
    try {
      // Drop blank rows before persisting.
      const entries = config.entries.filter((e) => e.text.trim().length > 0);
      await invoke('api_save_vocabulary_config', { config: { ...config, entries } });
      setConfig((c) => ({ ...c, entries }));
      toast.success('Vocabulary saved');
    } catch (e) {
      console.error('Failed to save vocabulary:', e);
      toast.error('Failed to save vocabulary');
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-medium">Custom Vocabulary</h3>
          <p className="text-xs text-muted-foreground">
            Terms bias Whisper transcription; Corrections replace mis-heard text (every engine);
            Descriptions are given to the summary model as a glossary.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Label className="text-xs">Enabled</Label>
          <Switch checked={config.enabled} onCheckedChange={(v) => setConfig((c) => ({ ...c, enabled: v }))} />
        </div>
      </div>

      <div className="space-y-2">
        {config.entries.map((e) => (
          <div key={e.id} className="flex flex-wrap items-center gap-2 border rounded-md p-2">
            <Select value={e.entryType} onValueChange={(v) => update(e.id, { entryType: v as VocabularyEntry['entryType'] })}>
              <SelectTrigger className="w-[130px]"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="term">Term</SelectItem>
                <SelectItem value="correction">Correction</SelectItem>
              </SelectContent>
            </Select>
            <Input
              className="w-[160px]"
              placeholder={e.entryType === 'correction' ? 'Mis-heard text' : 'Term'}
              value={e.text}
              onChange={(ev) => update(e.id, { text: ev.target.value })}
            />
            {e.entryType === 'correction' && (
              <Input
                className="w-[160px]"
                placeholder="Correct text"
                value={e.replacement ?? ''}
                onChange={(ev) => update(e.id, { replacement: ev.target.value || null })}
              />
            )}
            <Input
              className="flex-1 min-w-[160px]"
              placeholder="Description (optional — used by the summary glossary)"
              value={e.description ?? ''}
              onChange={(ev) => update(e.id, { description: ev.target.value || null })}
            />
            {e.entryType === 'correction' && (
              <div className="flex items-center gap-1">
                <Label className="text-xs">Aa</Label>
                <Switch checked={e.caseSensitive} onCheckedChange={(v) => update(e.id, { caseSensitive: v })} />
              </div>
            )}
            <Switch checked={e.enabled} onCheckedChange={(v) => update(e.id, { enabled: v })} />
            <Button variant="ghost" size="icon" onClick={() => remove(e.id)}><Trash2 className="h-4 w-4" /></Button>
          </div>
        ))}
      </div>

      <div className="flex justify-between">
        <Button variant="outline" size="sm" onClick={add}><Plus className="h-4 w-4 mr-1" /> Add entry</Button>
        <Button size="sm" onClick={save} disabled={saving}>{saving ? 'Saving…' : 'Save vocabulary'}</Button>
      </div>
    </section>
  );
}
```

- [ ] **Step 3: Register the panel**

In `frontend/src/app/settings/page.tsx`: import the panel (near the other settings imports ~lines 13-14):

```tsx
import { VocabularySettings } from '@/components/settings/VocabularySettings';
```

Then render it inside the existing **Transcription** tab's `<TabsContent value="Transcriptionmodels">` block (alongside `<SpeakerIdentificationSettings />`, ~lines 121-127), e.g. below the speaker settings:

```tsx
              <VocabularySettings />
```

(If you'd rather it be its own tab, add a `{ value: 'vocabulary', label: 'Vocabulary', icon: ... }` entry to the `TABS` array ~lines 19-26 and a matching `<TabsContent value="vocabulary"><VocabularySettings /></TabsContent>`. Slotting it under Transcription is the smaller change and keeps it near where it takes effect.)

- [ ] **Step 4: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -20 && pnpm lint 2>&1 | tail -20`
Expected: no NEW errors. (Align any UI-primitive import path with `WorkflowEditor.tsx` if a path differs.)

- [ ] **Step 5: Commit**

```bash
git add frontend/src/types/vocabulary.ts frontend/src/components/settings/VocabularySettings.tsx frontend/src/app/settings/page.tsx
git commit -m "feat(vocabulary): :sparkles: add custom vocabulary settings panel"
```

---

## Self-Review

**Spec coverage:** goals map to tasks — two entry types + model (T1), persistence/commands (T2), Layer 1 corrections both paths (T3), Layer 2 Whisper biasing (T4), Layer 3 glossary (T5), settings UI (T6). Non-goals (Parakeet CTC, auto-learn, term packs, embeddings, table storage) are excluded. ✎ The spec's "capability hint" copy is folded into the T6 panel description text.

**Placeholder scan:** no TBD/TODO; pure logic ships full code + tests; wiring tasks give exact edits and `build`/`tsc`/`lint`/manual gates (wiring has no meaningful pure unit — the pure core is fully tested in T1). Line numbers are prefixed with intent ("after ~line N") because the implementer must confirm against the file.

**Type consistency:** `VocabularyConfig`/`VocabularyEntry`/`Correction` names, the `entryType`/`caseSensitive` camelCase renames, `get/set_vocabulary_config_internal`, `get/save_vocabulary_config`, and `api_get/save_vocabulary_config` are used identically across tasks and match the TS mirror in T6.

**Sequencing:** T1 → T2 are foundational and sequential. T3, T4, T5 depend on T1+T2 and touch disjoint files (audio / whisper_engine / summary) — safe to parallelize but each must `cargo build` clean. T6 depends on T2's commands and is independent of T3-T5 (different language/dir).
