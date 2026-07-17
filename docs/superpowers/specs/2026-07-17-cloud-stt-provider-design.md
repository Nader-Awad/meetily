# Generic Cloud STT Provider — Design

- **Date:** 2026-07-17
- **Status:** Proposed — autonomous execution per the set-and-forget workflow (brainstorm Q&A was the gate). Ships to `main` + a release, no PR.
- **Author:** Nader Awad (with Claude)
- **Scope:** Rust/Tauri transcription pipeline (live + batch), settings/DB, a Next.js settings surface, and privacy copy. No changes to diarization or the summary side.
- **Decisions (from brainstorm):** cover **live + batch**; expose **OpenRouter / Groq / OpenAI / Custom** via one generic OpenAI-compatible client; **local stays default, cloud is explicit opt-in** with a clear "audio is sent to <provider>" note.

## 1. Problem & motivation

Local transcription (whisper.cpp `medium q5`) is weaker than cloud Whisper-large. The user wants an optional remote STT provider that's dirt cheap and better — reachable via **OpenRouter** (one key → Deepgram Nova-3 / Voxtral / whisper-large-v3), **Groq** (cheapest per hour), **OpenAI**, or a **custom** OpenAI-compatible endpoint. All four speak the OpenAI `/audio/transcriptions` shape, so a single client covers them.

## 2. What already exists (reuse, verified)

- **Provider abstraction is ready.** `audio/transcription/provider.rs`: `trait TranscriptionProvider { transcribe(Vec<f32>, Option<String>) -> Result<TranscriptResult, TranscriptionError>; is_model_loaded; get_current_model; provider_name }`, `TranscriptResult { text, confidence: Option<f32>, is_partial }`. `TranscriptionEngine::Provider(Arc<dyn TranscriptionProvider>)` (`engine.rs:15-19`). `WhisperProvider` (`whisper_provider.rs`, 53 lines) is the mirror.
- **Live dispatch is provider-agnostic already** — `worker.rs:624-671` handles `Provider(...)` end-to-end (calls `provider.transcribe`, emits `transcription-error` on failure). **No live-worker changes needed.**
- **Config/commands exist**: `TranscriptConfig { provider, model, api_key }` (`api.rs:99-105`); `api_get/save_transcript_config`, `api_get_transcript_api_key`; `setting.rs` get/save + per-provider key columns (`whisper/deepgram/elevenLabs/groq/openai ApiKey`) on `transcript_settings`.
- `reqwest` (features `multipart`, `json`, `stream`) and `async-trait` are already deps.

## 3. Gaps (what we add)

1. **No cloud provider impl** — new `audio/transcription/cloud_provider.rs`.
2. **No WAV encoder** — the only one is dead commented code (`lib_old_complex.rs:1676`). Need a live `f32 → 16-bit PCM WAV bytes` encoder.
3. **`engine.rs` never threads `config.api_key`** (hardcodes `None` at lines 76/84/172/180) and its provider match falls any unknown provider through to Whisper (`"localWhisper" | _`, `engine.rs:215`); `validate_transcription_model_ready` (`engine.rs:90-145`) hard-rejects non-local providers. Both need cloud arms.
4. **No `base_url` anywhere** and **no openrouter/custom key columns** — `transcript_settings` has no base-URL column; keys only for whisper/deepgram/elevenLabs/groq/openai. Migration needed.
5. **Batch paths are bool-branched** (`use_parakeet`) with **duplicate bespoke engine handling** in `import.rs` + `retranscription.rs` (+ `unload_engine_after_batch(bool)` in `common.rs`); `retranscription.rs:838` errors on non-Whisper. Refactor to a unified provider.
6. **Frontend** dropdown has cloud items commented out (`TranscriptSettings.tsx:123-130`); save path is `Sidebar/index.tsx handleSaveTranscriptConfig` → `api_save_transcript_config {provider, model, apiKey}` (no base_url).
7. **Privacy copy** (README §Local Transcription; `ModelDownloadProgress.tsx:125` "Models run locally – no internet required") must be qualified once cloud STT exists.

## 4. Design

**Generic client (one impl for all four).** `CloudProvider { base_url, api_key, model, client }` implements `TranscriptionProvider`:
- `transcribe(audio, language)`: `pcm16_wav_bytes(&audio, 16000)` → `reqwest` multipart POST to `{base_url}/audio/transcriptions` with `file=audio.wav` (mime `audio/wav`), `model`, and `language` if set + `Authorization: Bearer {api_key}` → parse JSON `{ "text": ... }` → `TranscriptResult { text, confidence: None, is_partial: false }`. Errors → `TranscriptionError::EngineFailed`.
- `is_model_loaded` → true; `get_current_model` → Some(model); `provider_name` → "Cloud".
- **Base-URL presets** (resolved in one helper): `openrouter` → `https://openrouter.ai/api/v1`, `groq` → `https://api.groq.com/openai/v1`, `openai` → `https://api.openai.com/v1`, `custom` → the stored custom base URL.

**Settings.** Migration adds to `transcript_settings`: `openrouterApiKey TEXT`, `customApiKey TEXT`, `transcriptBaseUrl TEXT` (custom endpoint). Extend `save/get_transcript_api_key` match arms (`openrouter`→openrouterApiKey, `custom`→customApiKey) + `TranscriptSetting`. `TranscriptConfig` gains `base_url: Option<String>` (resolved: preset for openrouter/groq/openai; stored `transcriptBaseUrl` for custom). `api_save_transcript_config` gains an optional `base_url` param (persisted for custom).

**Live path.** In `engine.rs`: add a cloud arm in `get_or_init_transcription_engine` (before the `_` catch-all) that builds `TranscriptionEngine::Provider(Arc::new(CloudProvider::new(base_url, api_key, model)))`, and a cloud arm in `validate_transcription_model_ready` that requires a non-empty api_key + model (no local model to load). Thread `config.api_key`/base_url through (fix the `None` sites). Worker unchanged.

**Batch path.** Refactor `import.rs` + `retranscription.rs` from `use_parakeet: bool` + two `Option<Arc<Engine>>` to a single `Arc<dyn TranscriptionProvider>` chosen by provider (Whisper/Parakeet/Cloud), and call `.transcribe(samples, language)` uniformly in the diarization-unit and VAD-segment loops. `unload_engine_after_batch` takes the provider (no-op for cloud). Allow cloud in `retranscription.rs` (cloud whisper supports `language`, so the non-Whisper hard-error is lifted for cloud).

**Frontend.** Uncomment + add dropdown items `openrouter`/`groq`/`openai`/`custom`; show an API-key input (exists) + a base-URL input for `custom` + a model input with suggested presets; render a **privacy warning** ("Audio will be sent to <provider> for transcription; recordings otherwise stay on your device") whenever a cloud provider is selected. Thread `base_url` through `handleSaveTranscriptConfig` + `api_save_transcript_config`.

**Privacy copy.** Qualify README §"Local Transcription"/"Privacy-First" to say local is the default/recommended path (cloud STT is opt-in), and scope the in-app "Models run locally – no internet required" line to the local engines.

## 5. Non-goals (YAGNI)

- Native token-by-token streaming (OpenRouter/Groq/OpenAI STT are request/response; chunked live is effectively real-time). Deepgram-native WebSocket is out (use Deepgram via OpenRouter instead).
- Deepgram/ElevenLabs **native** SDK providers (keep those dropdown entries dormant; Deepgram is reachable via OpenRouter's OpenAI-compatible endpoint).
- Per-word timestamps from the cloud API (live uses chunk-boundary timing already; batch keeps VAD-segment timing).
- Retries/backoff beyond a basic timeout + surfaced error (fast-follow if needed).

## 6. Testing

- Rust unit tests: `pcm16_wav_bytes` (RIFF header fields, byte length = 44 + 2·samples, clamping); cloud response parsing helper (extract `text` from the JSON body; error on non-2xx). Base-URL preset resolver. The HTTP round-trip itself is integration (not unit-tested; verified manually).
- `cargo build` for the wiring; frontend `tsc --noEmit`.
- Manual e2e (documented, not agent-run): pick OpenRouter + `openai/whisper-large-v3` + key, record → live transcript; Retranscribe an old meeting with the cloud provider; confirm local remains default and the privacy note shows.
