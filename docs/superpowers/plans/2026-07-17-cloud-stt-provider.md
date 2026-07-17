# Generic Cloud STT Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement task-by-task. Checkbox (`- [ ]`) steps.

**Goal:** Add an optional generic OpenAI-compatible cloud speech-to-text provider (OpenRouter / Groq / OpenAI / Custom) to Meetily's live + batch transcription, reusing the existing `TranscriptionProvider` trait. Local stays default; cloud is explicit opt-in.

**Architecture:** One `CloudProvider` implementing the existing trait (f32→WAV→multipart POST to `{base_url}/audio/transcriptions`) plugged into the already-provider-agnostic live worker via `engine.rs`; the batch paths are refactored off their `use_parakeet: bool` onto the same trait. Settings gain openrouter/custom keys + a custom base URL.

**Tech Stack:** Rust/Tauri (reqwest multipart, async-trait, sqlx), Next.js/TS.

## Global Constraints

- **No new dependencies** (`reqwest` multipart/json + `async-trait` already present).
- **Local remains the default and recommended path; cloud is opt-in** — never send audio to the cloud unless the user selected a cloud provider AND entered a key. Show a privacy note on cloud selection.
- Base URL presets: `openrouter`→`https://openrouter.ai/api/v1`, `groq`→`https://api.groq.com/openai/v1`, `openai`→`https://api.openai.com/v1`, `custom`→user-entered. Endpoint path = `{base_url}/audio/transcriptions`. Auth = `Authorization: Bearer <key>`.
- serde: `TranscriptConfig.base_url` ↔ `#[serde(rename="baseUrl")]`; sqlx column renames match existing camelCase (`openrouterApiKey`, `customApiKey`, `transcriptBaseUrl`).
- Migration filename must sort after `20260717000000_add_vocabulary.sql` → use `20260717010000_add_cloud_stt.sql`.
- Commit style: gitmoji conventional; **no `Co-Authored-By` / AI attribution**. Ships to `main` + release, no PR.
- **Sequencing:** Tasks 1→3 deliver the live path as a self-contained working unit; Task 4 (batch) builds on them; 5–6 finish UI + copy. If Task 4 proves too large, it can ship in a follow-up without blocking the live path.

---

## File Structure

**Create:** `frontend/src-tauri/src/audio/transcription/cloud_provider.rs`; `frontend/src-tauri/migrations/20260717010000_add_cloud_stt.sql`.
**Modify:** `audio/transcription/mod.rs`, `engine.rs`; `database/repositories/setting.rs`, `database/models.rs`, `api/api.rs`; `audio/import.rs`, `audio/retranscription.rs`, `audio/common.rs`; frontend `components/TranscriptSettings.tsx`, `components/Sidebar/index.tsx`, `services/configService.ts`; `README.md`, `components/ModelDownloadProgress.tsx`.

---

## Task 1: `CloudProvider` + WAV encoder (Rust, TDD)

**Files:** Create `frontend/src-tauri/src/audio/transcription/cloud_provider.rs`; modify `audio/transcription/mod.rs`.

**Interfaces produced:** `pcm16_wav_bytes(&[f32], u32) -> Vec<u8>`, `parse_transcription_text(&str) -> Result<String,String>`, `preset_base_url(&str) -> Option<&'static str>`, `struct CloudProvider` + `CloudProvider::new(base_url, api_key, model)` implementing `TranscriptionProvider`.

- [ ] **Step 1: Write `cloud_provider.rs`**

```rust
use super::provider::{TranscriptionError, TranscriptionProvider, TranscriptResult};
use async_trait::async_trait;
use reqwest::Client;

/// Base URL for a named cloud STT preset. `custom` returns None (caller supplies it).
pub fn preset_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "groq" => Some("https://api.groq.com/openai/v1"),
        "openai" => Some("https://api.openai.com/v1"),
        _ => None,
    }
}

/// Encode 16 kHz mono f32 samples to a 16-bit PCM WAV byte buffer.
pub fn pcm16_wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits: u16 = 16;
    let byte_rate: u32 = sample_rate * channels as u32 * (bits as u32 / 8);
    let block_align: u16 = channels * (bits / 8);
    let data_len: u32 = (samples.len() * 2) as u32;
    let file_size: u32 = 36 + data_len;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav
}

/// Extract transcript text from an OpenAI-compatible /audio/transcriptions JSON body.
pub fn parse_transcription_text(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid transcription response JSON: {e}"))?;
    v.get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("Transcription response missing 'text': {body}"))
}

pub struct CloudProvider {
    base_url: String,
    api_key: String,
    model: String,
    client: Client,
}

impl CloudProvider {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self { base_url: base_url.trim_end_matches('/').to_string(), api_key, model, client: Client::new() }
    }
}

#[async_trait]
impl TranscriptionProvider for CloudProvider {
    async fn transcribe(&self, audio: Vec<f32>, language: Option<String>)
        -> std::result::Result<TranscriptResult, TranscriptionError> {
        if self.api_key.trim().is_empty() {
            return Err(TranscriptionError::EngineFailed("Missing API key for cloud transcription".into()));
        }
        let wav = pcm16_wav_bytes(&audio, 16000);
        let part = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| TranscriptionError::EngineFailed(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", self.model.clone());
        if let Some(lang) = language.filter(|l| !l.is_empty() && l != "auto" && l != "auto-translate") {
            form = form.text("language", lang);
        }
        let url = format!("{}/audio/transcriptions", self.base_url);
        let resp = self.client.post(&url).bearer_auth(&self.api_key).multipart(form).send().await
            .map_err(|e| TranscriptionError::EngineFailed(format!("Cloud STT request failed: {e}")))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| TranscriptionError::EngineFailed(e.to_string()))?;
        if !status.is_success() {
            return Err(TranscriptionError::EngineFailed(format!("Cloud STT {status}: {body}")));
        }
        let text = parse_transcription_text(&body).map_err(TranscriptionError::EngineFailed)?;
        Ok(TranscriptResult { text, confidence: None, is_partial: false })
    }
    async fn is_model_loaded(&self) -> bool { true }
    async fn get_current_model(&self) -> Option<String> { Some(self.model.clone()) }
    fn provider_name(&self) -> &'static str { "Cloud" }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_header_and_length() {
        let s = vec![0.0f32, 1.0, -1.0, 0.5];
        let w = pcm16_wav_bytes(&s, 16000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(&w[36..40], b"data");
        assert_eq!(w.len(), 44 + s.len() * 2); // header + 2 bytes/sample
        // clamping: 1.0 -> 32767, -1.0 -> -32767
        let i0 = i16::from_le_bytes([w[44], w[45]]);
        assert_eq!(i0, 0);
        let i1 = i16::from_le_bytes([w[46], w[47]]);
        assert_eq!(i1, 32767);
    }
    #[test]
    fn parse_text_ok_and_errors() {
        assert_eq!(parse_transcription_text(r#"{"text":"  hi "}"#).unwrap(), "hi");
        assert!(parse_transcription_text(r#"{"nope":1}"#).is_err());
        assert!(parse_transcription_text("not json").is_err());
    }
    #[test]
    fn presets_resolve() {
        assert_eq!(preset_base_url("groq"), Some("https://api.groq.com/openai/v1"));
        assert_eq!(preset_base_url("openrouter"), Some("https://openrouter.ai/api/v1"));
        assert_eq!(preset_base_url("openai"), Some("https://api.openai.com/v1"));
        assert_eq!(preset_base_url("custom"), None);
    }
}
```

- [ ] **Step 2: Register in `mod.rs`** — add `pub mod cloud_provider;` and `pub use cloud_provider::CloudProvider;`.

- [ ] **Step 3: Test** — `cd frontend/src-tauri && cargo test --lib audio::transcription::cloud_provider 2>&1 | tail -20` → PASS.

- [ ] **Step 4: Commit** — `git commit -m "feat(transcription): :sparkles: generic OpenAI-compatible cloud STT provider + WAV encoder"`

---

## Task 2: Settings — migration, repo columns, config base_url, commands

**Files:** Create migration; modify `database/models.rs`, `database/repositories/setting.rs`, `api/api.rs`.

**Interfaces:** `transcript_settings` gains `openrouterApiKey`, `customApiKey`, `transcriptBaseUrl`; `save/get_transcript_api_key` handle `openrouter`/`custom`; `TranscriptConfig` gains `base_url: Option<String>`; `api_save_transcript_config` accepts an optional `base_url`; `api_get_transcript_config` returns a resolved `base_url`.

- [ ] **Step 1: Migration** `frontend/src-tauri/migrations/20260717010000_add_cloud_stt.sql`:
```sql
-- Cloud STT: OpenRouter/custom API keys + a custom base URL, on the single-row transcript_settings table.
ALTER TABLE transcript_settings ADD COLUMN openrouterApiKey TEXT;
ALTER TABLE transcript_settings ADD COLUMN customApiKey TEXT;
ALTER TABLE transcript_settings ADD COLUMN transcriptBaseUrl TEXT;
```

- [ ] **Step 2: `TranscriptSetting` struct** (`database/models.rs`) — add three fields mirroring the existing `#[sqlx(rename="...")] pub ..._api_key: Option<String>` pattern: `open_router_api_key` (`openrouterApiKey`), `custom_api_key` (`customApiKey`), `transcript_base_url` (`transcriptBaseUrl`).

- [ ] **Step 3: `setting.rs` key match arms** — in BOTH `save_transcript_api_key` and `get_transcript_api_key`, add to the provider→column match: `"openrouter" => "openrouterApiKey"`, `"custom" => "customApiKey"`. Add `save_transcript_base_url(pool, &str)` (upsert `transcriptBaseUrl`) and read it in `get_transcript_config` (it already `SELECT *`s, so `TranscriptSetting.transcript_base_url` is available).

- [ ] **Step 4: `TranscriptConfig` + commands** (`api/api.rs`):
  - Add `#[serde(rename="baseUrl")] pub base_url: Option<String>` to `TranscriptConfig`.
  - `api_get_transcript_config`: after loading provider/model/key, resolve base_url = `crate::audio::transcription::cloud_provider::preset_base_url(&provider).map(String)` else (for `custom`) the stored `transcriptBaseUrl`; include it. Default (no config) path keeps `base_url: None`.
  - `api_save_transcript_config`: add param `base_url: Option<String>`; when provider == `"custom"` and base_url present, `save_transcript_base_url`. (Presets don't need storing.)
  - Keep the existing key-save behavior (now covers openrouter/custom via the new match arms).

- [ ] **Step 5: Build** — `cd frontend/src-tauri && cargo build 2>&1 | tail -25` → clean. (Migration runs on next launch via the existing migrator.)

- [ ] **Step 6: Commit** — `git commit -m "feat(transcription): :sparkles: persist cloud STT keys + custom base URL in transcript settings"`

---

## Task 3: Live path wiring (`engine.rs`)

**Files:** modify `audio/transcription/engine.rs`.

**Interfaces:** `get_or_init_transcription_engine` returns `TranscriptionEngine::Provider(CloudProvider)` for cloud providers; `validate_transcription_model_ready` accepts cloud providers.

- [ ] **Step 1: Ensure the loaded config carries key + base_url.** Read `engine.rs`'s config-load in both fns. Where it builds `TranscriptConfig`, the cloud arm needs `api_key` + resolved `base_url`. If the primary load path doesn't already include the api key, fetch it (mirror `api_get_transcript_config`: `SettingsRepository::get_transcript_api_key(pool, &provider)` + preset/stored base URL). Keep the `api_key: None` fallback literals (lines ~76/84/172/180) as-is — those are local-default fallbacks and need no key.

- [ ] **Step 2: Cloud arm in `get_or_init_transcription_engine`** — insert BEFORE the `"localWhisper" | _` catch-all (~line 215):
```rust
        "openrouter" | "groq" | "openai" | "custom" => {
            let api_key = config.api_key.clone().unwrap_or_default();
            if api_key.trim().is_empty() {
                return Err(format!("Cloud transcription provider '{}' requires an API key (set it in Settings → Transcription).", config.provider));
            }
            if config.model.trim().is_empty() {
                return Err("Cloud transcription requires a model (e.g. openai/whisper-large-v3).".to_string());
            }
            let base_url = config.base_url.clone()
                .filter(|u| !u.trim().is_empty())
                .or_else(|| crate::audio::transcription::cloud_provider::preset_base_url(&config.provider).map(String::from))
                .ok_or_else(|| format!("No base URL configured for provider '{}'.", config.provider))?;
            info!("☁️ Initializing cloud transcription provider '{}' (model {})", config.provider, config.model);
            Ok(TranscriptionEngine::Provider(Arc::new(
                crate::audio::transcription::cloud_provider::CloudProvider::new(base_url, api_key, config.model.clone())
            )))
        }
```

- [ ] **Step 3: Cloud arm in `validate_transcription_model_ready`** — replace the `other => Err(...)` so cloud providers validate instead of being rejected:
```rust
        "openrouter" | "groq" | "openai" | "custom" => {
            // No local model to load; require an API key + model.
            if config.api_key.as_deref().unwrap_or("").trim().is_empty() {
                Err(format!("Cloud provider '{}' needs an API key — add it in Settings → Transcription.", config.provider))
            } else if config.model.trim().is_empty() {
                Err("Cloud transcription requires a model.".to_string())
            } else { Ok(()) }
        }
        other => { /* keep existing unsupported-provider error */ }
```
(Ensure `config` here also carries `api_key`/`base_url` as in Step 1.)

- [ ] **Step 4: Build + confirm live dispatch unchanged** — `cargo build 2>&1 | tail -25` clean. (No `worker.rs` change — the `Provider` arm at `worker.rs:624` already handles it.)

- [ ] **Step 5: Commit** — `git commit -m "feat(transcription): :sparkles: wire cloud STT provider into live transcription engine"`

---

## Task 4: Batch path — unify import + retranscription onto the provider trait

**Files:** modify `audio/import.rs`, `audio/retranscription.rs`, `audio/common.rs` (+ a shared helper, e.g. in `engine.rs`).

**Goal:** replace the `use_parakeet: bool` + two `Option<Arc<Engine>>` with a single `Arc<dyn TranscriptionProvider>` chosen by provider, and call `.transcribe(samples, language)` uniformly; support cloud in both paths.

- [ ] **Step 1: Shared selector.** Add to `engine.rs` (or a batch helper) an async fn that returns the batch provider for the configured/selected provider string:
```rust
pub async fn select_batch_provider<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>, provider: &str, model: Option<&str>,
) -> Result<std::sync::Arc<dyn TranscriptionProvider>, String> {
    match provider {
        "openrouter" | "groq" | "openai" | "custom" => {
            // load api_key + base_url from settings (as in Task 3 Step 1); build CloudProvider
        }
        "parakeet" => Ok(Arc::new(ParakeetProvider::new(get_or_init_parakeet(app, model).await?))),
        _ => Ok(Arc::new(WhisperProvider::new(get_or_init_whisper(app, model).await?))),
    }
}
```
(Reuse the batch files' existing `get_or_init_whisper`/`get_or_init_parakeet` or `engine.rs`'s — the implementer picks the least-invasive wiring; the key outcome is a single `Arc<dyn TranscriptionProvider>`.)

- [ ] **Step 2: Refactor `import.rs`.** Replace the `use_parakeet` decision (`:270`, `:334`), the two `Option<Arc<Engine>>` inits (`:517-526`), and both `if use_parakeet {...} else {...}` transcribe blocks (`:665-686`, `:783-797`) with: obtain `let provider = select_batch_provider(&app, &provider_str, model.as_deref()).await?;` once, then in each loop `match provider.transcribe(samples.clone(), language.clone()).await { Ok(r) => (r.text, r.confidence.unwrap_or(0.9)), Err(e) => { warn!(...); continue; } }`. Update `unload_engine_after_batch` call.

- [ ] **Step 3: Refactor `retranscription.rs`** the same way (`:186`, `:308-317`, `:455-476`, `:571-585`). Remove/relax the non-Whisper hard error at `:838-839` so cloud providers are allowed (cloud STT supports `language` via the request). Keep local Parakeet's existing language limitation note if applicable.

- [ ] **Step 4: `common.rs`** — change `unload_engine_after_batch(use_parakeet: bool)` to take the provider string (or an enum) and **no-op for cloud** (nothing to unload); keep whisper/parakeet unload. Update both callers.

- [ ] **Step 5: Build** — `cargo build 2>&1 | tail -30` clean. (No new unit tests — behavior is exercised by Task 1's provider tests + manual e2e; the gate is compilation + a clean diff.)

- [ ] **Step 6: Commit** — `git commit -m "feat(transcription): :sparkles: support cloud STT on import + retranscribe (unify batch onto provider trait)"`

---

## Task 5: Frontend — provider dropdown, base URL/model/key, privacy note

**Files:** modify `frontend/src/components/TranscriptSettings.tsx`, `frontend/src/components/Sidebar/index.tsx`, `frontend/src/services/configService.ts`.

- [ ] **Step 1: `TranscriptSettings.tsx`** — extend the provider union with `'openrouter' | 'custom'`; uncomment/replace the dropdown block (`:123-130`) to add `☁️ OpenRouter`, `☁️ Groq`, `☁️ OpenAI`, `☁️ Custom (OpenAI-compatible)` items; extend `requiresApiKey` to include `openrouter`/`groq`/`openai`/`custom`; extend `modelOptions` with suggestions (`openrouter: ['openai/whisper-large-v3','deepgram/nova-3','mistralai/voxtral-mini-transcribe-2602']`, `groq: ['whisper-large-v3-turbo','whisper-large-v3']`, `openai: ['whisper-1','gpt-4o-transcribe']`, `custom: []` free-text). Add a **base-URL `<Input>`** shown only when provider === `'custom'`, and a **privacy note** (e.g. an amber callout) shown whenever a cloud provider is selected: "Audio is sent to {provider} for transcription. Recordings otherwise stay on your device; local Whisper/Parakeet keep everything on-device." Load base URL alongside the key (extend the fetch).

- [ ] **Step 2: Thread base_url through save** — `Sidebar/index.tsx handleSaveTranscriptConfig` (`:212-238`): include `baseUrl` in the payload and pass it to `invoke('api_save_transcript_config', { provider, model, apiKey, baseUrl })`. Update `configService.ts` `TranscriptModelProps` / `getTranscriptConfig` to carry `baseUrl?: string | null`. Update the other `api_save_transcript_config` invoke sites (`ParakeetModelManager.tsx`, `WhisperModelManager.tsx`) to pass `baseUrl: null` (local) so the added param is always provided.

- [ ] **Step 3: Typecheck** — `cd frontend && npx tsc --noEmit 2>&1 | tail -15` → no NEW errors.

- [ ] **Step 4: Commit** — `git commit -m "feat(transcription): :sparkles: cloud STT provider settings UI (OpenRouter/Groq/OpenAI/Custom) + privacy note"`

---

## Task 6: Privacy copy — qualify the local-only claims

**Files:** modify `README.md`, `frontend/src/components/ModelDownloadProgress.tsx`.

- [ ] **Step 1: README** — in §"Local Transcription" and §"Privacy-First Design", change absolutes to "by default": e.g. "Transcribe meetings **on your device by default** using Whisper or Parakeet — **or opt in to a cloud provider** (OpenRouter/Groq/OpenAI) for higher accuracy. Local keeps everything on-device; cloud sends audio to the chosen provider."

- [ ] **Step 2: In-app string** — `ModelDownloadProgress.tsx:125`: scope the "Models run locally - no internet required for transcription" line to the local engines (e.g. "Local models run on-device — no internet required" and only show it for local providers), so it isn't shown/contradicted when a cloud provider is active.

- [ ] **Step 3: Typecheck** (if the tsx change needs it) + **Commit** — `git commit -m "docs(transcription): :memo: clarify local-default vs opt-in cloud transcription"`

---

## Self-Review

**Spec coverage:** provider+WAV (T1), settings/migration/config (T2), live wiring (T3), batch unify (T4), UI+privacy note (T5), copy (T6). Non-goals (native streaming, Deepgram-native SDK, per-word timestamps, retries) excluded.

**Placeholder scan:** T1/T2 ship full code + tests; T3 ships the arms; T4/T5 give exact sites + shapes to refactor (implementer reads the grounded line refs). Line numbers are `~` and must be confirmed against the file.

**Type consistency:** `base_url`↔`baseUrl`; provider strings `openrouter|groq|openai|custom` identical across engine arms, settings match, selector, and the frontend union; `CloudProvider::new(base_url, api_key, model)` and `preset_base_url` used identically in T1/T3/T4.
