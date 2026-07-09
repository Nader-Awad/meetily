# Voice-Profile Speaker Identification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add local, voice-fingerprint speaker identification to Meetily — cluster distinct voices per meeting into `Speaker N` labels, let the user rename + "remember" a voice as a persistent profile (including "Me"), display speaker chips in transcripts, and feed speaker labels into the summary and workflow LLM prompts.

**Architecture:** Adapt the diarization core of upstream PR #538 (fbank → WeSpeaker CAM++ ONNX embedding via `ort` → online cosine clustering → persistent voice profiles), **trimmed** of its overlap/timeline machinery and all non-diarization bundled features. A new `diarization/` Rust module is spliced into the live transcription worker (best-effort, never blocks transcription); the vestigial `transcripts.speaker` column is repurposed to hold the label; the frontend renders chips + rename/settings UI. On macOS the embedding session registers `ort`'s CoreML execution provider (ANE/GPU) with CPU fallback.

**Tech Stack:** Rust / Tauri v2 core (crate `meetily`, lib `app_lib`), SQLx/SQLite (embedded `sqlx::migrate!`), `ort` 2.0.0-rc.10 (ONNX Runtime), `ndarray`, `realfft`, `futures-util` (all already present). Next.js / React / TypeScript frontend, Tailwind, `sonner` toasts, shadcn-style `ui/*` primitives.

## Global Constraints

- **Personal local fork.** Merge to LOCAL `main` only. Do NOT push to any remote or open upstream PRs (NeoHive memory `meetily-personal-fork`). Default to NOT pushing.
- **Credential safety.** Never print/commit full API keys/tokens/secrets; this feature touches none, but keep it so.
- **Source of lifted code:** PR #538 is available locally as git ref **`pr-538`** (fetched via `git fetch upstream pull/538/head:pr-538`). Lift files with `git show pr-538:<path> > <dest>`. If the ref is missing, re-run the fetch first.
- **Take ONLY the diarization slice.** EXCLUDE `diarization/overlap_detector.rs`, `diarization/timeline.rs`, the `add_overlap_diarization` migration, `apple_speech_*`, `local_api.rs` (axum), summary-prompt-settings, the floating recording indicator, and every other non-diarization hunk PR #538 bundles into shared files (`api/api.rs`, `recording_saver.rs`, `recording_commands.rs`, `summary/*`, `database/models.rs`, `transcript.rs`, and all unrelated `.tsx`). Only the `speaker`-specific hunks are in scope.
- **No new Rust dependencies.** `ort`, `ndarray`, `realfft`, `futures-util`, `thiserror` are already in `Cargo.toml`. The only build-config change is enabling `ort`'s `coreml` feature on macOS (Task 1).
- **Migration ordering (critical):** the latest existing migration is `20260703000001_neohive_access_token.sql`. New migrations MUST sort AFTER it — use the `20260709…` prefix (NOT the PR's `20260610…`, which would sort before applied migrations and break sqlx). The `transcripts.speaker TEXT` column already exists (migration `20251110000001`); do NOT add another for it — repurpose it.
- **Naming:** audio devices are "microphone"/"system", never "input"/"output".
- **Hot-path logging:** in per-chunk code use `perf_debug!`/`perf_trace!`, never plain `log::debug!`. Per-*segment* logging (seconds apart) may use `info!`/`warn!` as the lifted code does.
- **Tauri commands:** all new behavior via `#[command]` fns registered in `lib.rs` `generate_handler!`. Rust snake_case ↔ JS camelCase auto-converts; the frontend calls e.g. `invoke('diarization_rename_speaker', { meetingId, oldLabel, newName, saveProfile })`.
- **Build prerequisites (already satisfied on this machine):** Xcode is installed (required by `cidre`/ScreenCaptureKit for the crate to compile at all); a `binaries/llama-helper-aarch64-apple-darwin` placeholder exists for `cargo test`/dev (the real sidecar is built by `frontend/build-gpu.sh`). Run all Rust commands from `frontend/src-tauri/`.
- **Frontend has no test runner** (`pnpm lint` is broken repo-wide). Verify frontend tasks with `npx tsc --noEmit` (run from `frontend/`) + manual inspection.
- **Preserve original-author credit** (rodrigopg / PR #538) in the header comments of lifted files (they already carry module doc comments; keep them).
- **Commit style:** gitmoji conventional commits, e.g. `feat(speakers): :sparkles: …`. No AI attribution / no `Co-Authored-By`.
- **Branch:** work on `feature/speaker-identification` (already checked out).

---

## File Structure

**New files (Rust core, `frontend/src-tauri/`):**
- `migrations/20260709000000_add_diarization_settings.sql` — single-row feature toggle table.
- `migrations/20260709000001_add_speaker_profiles.sql` — persistent voice profiles table.
- `src/diarization/mod.rs` — module wiring + `pub use session::DiarizationSession`.
- `src/diarization/fbank.rs` — Kaldi 80-dim log-mel filterbank (verbatim lift).
- `src/diarization/embedding.rs` — WeSpeaker CAM++ ONNX embedding (lift + macOS CoreML EP).
- `src/diarization/clustering.rs` — online cosine clustering (verbatim lift).
- `src/diarization/session.rs` — per-meeting orchestration (lift, **trimmed** of timeline/overlap).
- `src/diarization/models.rs` — model path + on-demand download (verbatim lift).
- `src/diarization/commands.rs` — Tauri commands: status/enable/download/rename/profiles (verbatim lift).
- `src/database/repositories/speaker_profile.rs` — voice-profile CRUD + blob (de)serialization (verbatim lift).

**Modified files (Rust core):**
- `Cargo.toml` — macOS-only `ort` `coreml` feature.
- `src/lib.rs` — `pub mod diarization;` + register 7 diarization commands.
- `src/database/repositories/mod.rs` — `pub mod speaker_profile;`.
- `src/database/models.rs` — `Transcript.speaker: Option<String>`.
- `src/api/api.rs` — `MeetingTranscript.speaker`, `TranscriptSegment.speaker`, `From<Transcript>` mapping.
- `src/database/repositories/transcript.rs` — `speaker` column in the `save_transcript` INSERT.
- `src/audio/recording_saver.rs` — `TranscriptSegment.speaker` field.
- `src/audio/recording_commands.rs` — carry `speaker` from event into the saved segment (2 listeners).
- `src/audio/transcription/worker.rs` — the diarization splice (trimmed).

**New files (frontend, `frontend/src/`):**
- `components/SpeakerIdentificationSettings.tsx` — enable toggle + model download + remembered voices (verbatim lift).
- `components/SpeakerRenameDialog.tsx` — rename + "remember this voice" (verbatim lift).

**Modified files (frontend):**
- `types/index.ts` — `speaker?` on `Transcript`, `TranscriptUpdate`, `TranscriptSegmentData`.
- `contexts/TranscriptContext.tsx` — capture `speaker` from the live event.
- `components/SettingTabs.tsx` (or the settings page) — mount `SpeakerIdentificationSettings`.
- `components/TranscriptView.tsx` + `components/VirtualizedTranscriptView.tsx` — render speaker chips.
- `components/MeetingDetails/TranscriptPanel.tsx` — pass `speaker` through + open the rename dialog on chip click (saved view).
- `components/MeetingDetails/SummaryPanel.tsx` — prefix each segment with `[speaker]` when building the LLM transcript text (§9).

---

## Task 1: Enable `ort` CoreML execution provider on macOS

**Files:**
- Modify: `frontend/src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: existing `ort = { version = "2.0.0-rc.10" }` base dependency.
- Produces: the `coreml` cargo feature (hence `ort::execution_providers::CoreMLExecutionProvider`) is available on macOS builds only; consumed by Task 4.

- [ ] **Step 1: Add the macOS-only target dependency**

In `frontend/src-tauri/Cargo.toml`, leave the existing base line as-is:
```toml
ort = { version = "2.0.0-rc.10" }  # ONNX Runtime for Parakeet models
```
and add a target-specific section (place it near the other `[target.…]` blocks, or at the end of the dependency area). Cargo unifies this with the base dep, adding `coreml` only when compiling for macOS so non-Apple builds never pull the Apple EP:
```toml
# Speaker-embedding inference offloads to the Apple Neural Engine / GPU via
# ort's CoreML execution provider on macOS, with CPU fallback (see
# src/diarization/embedding.rs). macOS-only so non-Apple builds stay CPU.
[target.'cfg(target_os = "macos")'.dependencies]
ort = { version = "2.0.0-rc.10", features = ["coreml"] }
```

- [ ] **Step 2: Verify the feature resolves on macOS**

Run (from `frontend/src-tauri/`):
```bash
cargo tree -e features -i ort 2>/dev/null | grep -i coreml
```
Expected: a line mentioning the `coreml` feature (confirms it is enabled for this macOS build). If empty, re-check the target cfg string.

- [ ] **Step 3: Confirm the crate still builds**

Run:
```bash
cargo build 2>&1 | tail -20
```
Expected: builds successfully (no new errors introduced by the dependency change). This may take several minutes on a cold cache.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/Cargo.toml frontend/src-tauri/Cargo.lock
git commit -m "build(speakers): :heavy_plus_sign: enable ort CoreML execution provider on macOS"
```

---

## Task 2: Voice-profile table, migration, and repository

**Files:**
- Create: `frontend/src-tauri/migrations/20260709000000_add_diarization_settings.sql`
- Create: `frontend/src-tauri/migrations/20260709000001_add_speaker_profiles.sql`
- Create: `frontend/src-tauri/src/database/repositories/speaker_profile.rs`
- Modify: `frontend/src-tauri/src/database/repositories/mod.rs`

**Interfaces:**
- Produces:
  - Table `diarization_settings(id TEXT PK DEFAULT '1', enabled INTEGER NOT NULL DEFAULT 0)`.
  - Table `speaker_profiles(id TEXT PK, name TEXT NOT NULL, embedding BLOB NOT NULL, created_at TIMESTAMP, updated_at TIMESTAMP)`.
  - `SpeakerProfilesRepository` with `async list(pool) -> Result<Vec<SpeakerProfile>, SqlxError>`, `async create(pool, name: &str, embedding: &[f32]) -> Result<String, SqlxError>`, `async rename(pool, id, name) -> Result<(), SqlxError>`, `async delete(pool, id) -> Result<(), SqlxError>`.
  - `SpeakerProfile { id: String, name: String, embedding: Vec<f32> }` and free fns `embedding_to_blob(&[f32]) -> Vec<u8>` / `blob_to_embedding(&[u8]) -> Vec<f32>`.
  - Consumed by Task 7 (commands) and Task 9 (worker profile seeding).

- [ ] **Step 1: Write the `diarization_settings` migration**

Create `frontend/src-tauri/migrations/20260709000000_add_diarization_settings.sql`:
```sql
-- Speaker identification (diarization) feature settings.
-- Single-row table; 'enabled' gates the live diarization pipeline.
CREATE TABLE IF NOT EXISTS diarization_settings (
    id TEXT PRIMARY KEY DEFAULT '1',
    enabled INTEGER NOT NULL DEFAULT 0
);
```

- [ ] **Step 2: Write the `speaker_profiles` migration**

Create `frontend/src-tauri/migrations/20260709000001_add_speaker_profiles.sql`:
```sql
-- Persistent voice profiles for speaker identification.
-- One centroid embedding per named profile (f32 little-endian BLOB).
-- Embeddings are derived locally and never leave the device.
CREATE TABLE IF NOT EXISTS speaker_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    embedding BLOB NOT NULL,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);
```

- [ ] **Step 3: Lift the repository verbatim**

```bash
git show pr-538:frontend/src-tauri/src/database/repositories/speaker_profile.rs \
  > frontend/src-tauri/src/database/repositories/speaker_profile.rs
```
This file is self-contained (depends only on `chrono`, `serde`, `sqlx`, `uuid` — all present) and ships a `blob_roundtrip` unit test. Verify its contents match the interface above:
```bash
grep -nE "pub async fn (list|create|rename|delete)|embedding_to_blob|blob_to_embedding|fn blob_roundtrip" \
  frontend/src-tauri/src/database/repositories/speaker_profile.rs
```
Expected: all six symbols present.

- [ ] **Step 4: Register the module**

In `frontend/src-tauri/src/database/repositories/mod.rs`, add the line (keep alphabetical-ish ordering with the existing `meeting`/`setting`/`summary`/`transcript`/`transcript_chunk`):
```rust
pub mod speaker_profile;
```

- [ ] **Step 5: Run the repository unit test**

Run (from `frontend/src-tauri/`):
```bash
cargo test --lib database::repositories::speaker_profile::tests::blob_roundtrip -- --nocapture 2>&1 | tail -20
```
Expected: `test ... blob_roundtrip ... ok`. (Migration application itself is verified at app boot / manual E2E — SQLx embeds these `.sql` files via `sqlx::migrate!` at compile time, so a clean build proves they are discovered.)

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/migrations/20260709000000_add_diarization_settings.sql \
        frontend/src-tauri/migrations/20260709000001_add_speaker_profiles.sql \
        frontend/src-tauri/src/database/repositories/speaker_profile.rs \
        frontend/src-tauri/src/database/repositories/mod.rs
git commit -m "feat(speakers): :card_file_box: add speaker_profiles + diarization_settings tables and voice-profile repository"
```

---

## Task 3: Diarization module skeleton + fbank frontend

**Files:**
- Create: `frontend/src-tauri/src/diarization/fbank.rs`
- Create: `frontend/src-tauri/src/diarization/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

**Interfaces:**
- Produces:
  - `diarization::fbank::FbankComputer::new() -> FbankComputer` and `.compute(&self, samples: &[f32]) -> Vec<[f32; NUM_MEL_BINS]>` (CMN log-mel features; empty when audio < one frame).
  - `pub const NUM_MEL_BINS: usize = 80;`, `pub const SAMPLE_RATE: usize = 16_000;`.
  - Consumed by Task 4 (embedding).
- Note: this task establishes the `diarization` module in the crate so subsequent tasks' unit tests compile. `mod.rs` grows one `pub mod` per task; it must never reference a file that does not yet exist.

- [ ] **Step 1: Lift `fbank.rs` verbatim**

```bash
git show pr-538:frontend/src-tauri/src/diarization/fbank.rs \
  > frontend/src-tauri/src/diarization/fbank.rs
```
It depends only on `realfft` (present) and `std`. It ships 3 unit tests (`frame_count_matches_snip_edges`, `short_audio_returns_empty`, `cmn_zero_means`).

- [ ] **Step 2: Create a minimal `mod.rs` (fbank only)**

Create `frontend/src-tauri/src/diarization/mod.rs`. **Do NOT** copy the PR's `mod.rs` (it declares the excluded `overlap_detector`/`timeline`). Write exactly:
```rust
// diarization/mod.rs
//
// Speaker identification (diarization) for the live transcription pipeline.
// Rust-native and fully local: WeSpeaker CAM++ ONNX embeddings (via ort, the
// same runtime Parakeet uses) + online cosine clustering. Adapted from the
// diarization slice of upstream PR #538 (author: rodrigopg), trimmed of the
// overlap/timeline machinery. See docs in each module.

pub mod fbank;
```

- [ ] **Step 3: Declare the module in `lib.rs`**

In `frontend/src-tauri/src/lib.rs`, in the top-level `pub mod` block (currently `analytics`…`whisper_engine` around lines 38–57), add (keep it alphabetical — after `database`, before `neohive`):
```rust
pub mod diarization;
```

- [ ] **Step 4: Run the fbank unit tests**

```bash
cargo test --lib diarization::fbank 2>&1 | tail -20
```
Expected: 3 tests pass (`frame_count_matches_snip_edges`, `short_audio_returns_empty`, `cmn_zero_means`).

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/diarization/fbank.rs \
        frontend/src-tauri/src/diarization/mod.rs \
        frontend/src-tauri/src/lib.rs
git commit -m "feat(speakers): :sparkles: add diarization module skeleton + Kaldi fbank frontend"
```

---

## Task 4: WeSpeaker embedding extractor (with macOS CoreML EP)

**Files:**
- Create: `frontend/src-tauri/src/diarization/embedding.rs`
- Modify: `frontend/src-tauri/src/diarization/mod.rs`

**Interfaces:**
- Consumes: `fbank::{FbankComputer, NUM_MEL_BINS}` (Task 3); `ort` CoreML feature on macOS (Task 1).
- Produces:
  - `diarization::embedding::EmbeddingExtractor::new(model_path: &Path) -> Result<Self, EmbeddingError>` and `.compute(&mut self, samples_16k: &[f32]) -> Result<Vec<f32>, EmbeddingError>` (L2-normalized 192-dim embedding).
  - `pub enum EmbeddingError` (`Ort`, `AudioTooShort`, `NoOutput`).
  - Consumed by Task 6 (session).

- [ ] **Step 1: Lift `embedding.rs`**

```bash
git show pr-538:frontend/src-tauri/src/diarization/embedding.rs \
  > frontend/src-tauri/src/diarization/embedding.rs
```

- [ ] **Step 2: Register the CoreML execution provider on macOS**

The lifted file builds a CPU-only session. Edit it so macOS offloads to the ANE/GPU with automatic CPU fallback, and non-macOS stays CPU (matching Task 1's macOS-only feature).

Replace the import line:
```rust
use ort::execution_providers::CPUExecutionProvider;
```
with:
```rust
use ort::execution_providers::CPUExecutionProvider;
#[cfg(target_os = "macos")]
use ort::execution_providers::CoreMLExecutionProvider;
```

Replace the session builder in `EmbeddingExtractor::new` — change this block:
```rust
        let session = Session::builder()?
            .with_execution_providers(vec![CPUExecutionProvider::default().build()])?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(2)?
            .commit_from_file(model_path)?;
```
to:
```rust
        // ort tries execution providers in order and silently falls back to
        // the next one if a provider is unavailable. On macOS we prefer the
        // CoreML EP (Apple Neural Engine / GPU) and fall back to CPU; every
        // other platform is CPU-only. The per-segment embedding load is light,
        // so CPU-only is a fully acceptable fallback.
        let mut providers = Vec::new();
        #[cfg(target_os = "macos")]
        providers.push(CoreMLExecutionProvider::default().build());
        providers.push(CPUExecutionProvider::default().build());

        let session = Session::builder()?
            .with_execution_providers(providers)?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(2)?
            .commit_from_file(model_path)?;
```

- [ ] **Step 3: Wire it into `mod.rs`**

In `frontend/src-tauri/src/diarization/mod.rs`, add after `pub mod fbank;`:
```rust
pub mod embedding;
```

- [ ] **Step 4: Verify it compiles (no standalone unit test — runtime needs the ONNX model)**

```bash
cargo build 2>&1 | tail -20
```
Expected: builds successfully. (Confirms the CoreML EP symbol resolves under the macOS feature and the `ort` API usage is correct.)

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/diarization/embedding.rs \
        frontend/src-tauri/src/diarization/mod.rs
git commit -m "feat(speakers): :sparkles: add WeSpeaker CAM++ embedding extractor with macOS CoreML EP"
```

---

## Task 5: Online speaker clustering

**Files:**
- Create: `frontend/src-tauri/src/diarization/clustering.rs`
- Modify: `frontend/src-tauri/src/diarization/mod.rs`

**Interfaces:**
- Produces:
  - `diarization::clustering::SpeakerClusterer::new() -> Self`, `.assign(&mut self, embedding: &[f32]) -> String` (returns `"Speaker N"` or a matched profile name), `.seed_profile(&mut self, name: &str, centroid: Vec<f32>)`, `.last_label(&self) -> Option<String>`, `.centroids(&self) -> impl Iterator<Item = (&str, &[f32], usize)>`, `.relabel(&mut self, old, new)`, `.with_max_anonymous_speakers(usize)`, `.anon_speaker_count(&self) -> usize`.
  - `pub const CLUSTER_SIMILARITY_THRESHOLD: f32 = 0.55; PROFILE_MATCH_THRESHOLD: f32 = 0.60; DEFAULT_MAX_ANONYMOUS_SPEAKERS: usize = 10;`.
  - `pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32`.
  - Consumed by Task 6 (session).

- [ ] **Step 1: Lift `clustering.rs` verbatim**

```bash
git show pr-538:frontend/src-tauri/src/diarization/clustering.rs \
  > frontend/src-tauri/src/diarization/clustering.rs
```
Self-contained (no `super::` deps). Ships 3 tests (`same_voice_same_cluster`, `different_voice_new_cluster`, `caps_anonymous_speakers_and_reuses_existing_label_for_outliers`).

- [ ] **Step 2: Wire it into `mod.rs`**

In `frontend/src-tauri/src/diarization/mod.rs`, add:
```rust
pub mod clustering;
```

- [ ] **Step 3: Run the clustering unit tests**

```bash
cargo test --lib diarization::clustering 2>&1 | tail -20
```
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/diarization/clustering.rs \
        frontend/src-tauri/src/diarization/mod.rs
git commit -m "feat(speakers): :sparkles: add online cosine speaker clustering"
```

---

## Task 6: Diarization session (trimmed) + model management

**Files:**
- Create: `frontend/src-tauri/src/diarization/session.rs`
- Create: `frontend/src-tauri/src/diarization/models.rs`
- Modify: `frontend/src-tauri/src/diarization/mod.rs`

**Interfaces:**
- Consumes: `embedding::{EmbeddingExtractor, EmbeddingError}` (Task 4), `clustering::SpeakerClusterer` (Task 5).
- Produces:
  - `diarization::session::DiarizationSession` with `new(&Path) -> Result<Self, EmbeddingError>`, `with_profiles(&Path, Vec<(String, Vec<f32>)>) -> Result<Self, EmbeddingError>`, `label_segment(&mut self, samples_16k: &[f32]) -> Option<String>`, `centroid_snapshot(&self) -> Vec<(String, Vec<f32>, usize)>`, `clusterer(&self)`, `clusterer_mut(&mut self)`.
  - `diarization::models` free fns: `models_dir(&AppHandle) -> Result<PathBuf,String>`, `embedding_model_path(&AppHandle) -> Result<PathBuf,String>`, `is_embedding_model_present(&AppHandle) -> bool`, `async download_embedding_model(&AppHandle) -> Result<(),String>`, and `pub const EMBEDDING_MODEL_FILENAME`, `EMBEDDING_MODEL_URL`.
  - Consumed by Task 7 (commands) and Task 9 (worker).

- [ ] **Step 1: Lift `models.rs` verbatim**

```bash
git show pr-538:frontend/src-tauri/src/diarization/models.rs \
  > frontend/src-tauri/src/diarization/models.rs
```
Self-contained (`futures-util`, `reqwest`, `tauri`, `tokio`, `serde_json` — all present). Pins the WeSpeaker CAM++ model URL (`https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_CAM%2B%2B.onnx`, ~28 MB), downloads to `<app_data>/models/diarization/` with `.tmp`+atomic-rename and `diarization-model-download-progress` events, and verifies via HTTP status + a >1 MB size check. (No separate checksum ships with the PR; the size guard + atomic rename are the integrity check for v1.)

- [ ] **Step 2: Create the TRIMMED `session.rs`**

The PR's `session.rs` depends on the excluded `timeline` module (`RollingDiarizationBuffer`, `SpeakerTimeline`, `label_segment_at`, `timeline_snapshot`). Do NOT lift it verbatim. Create `frontend/src-tauri/src/diarization/session.rs` with exactly this trimmed content (timeline-free; keeps the per-segment `label_segment` path v1 uses):
```rust
// diarization/session.rs
//
// Per-recording diarization state: embedding extractor + online clusterer.
// Created when a recording starts (if the feature is enabled and the model
// is present) and dropped when it ends. Adapted from upstream PR #538
// (author: rodrigopg), trimmed to the per-segment labeling path (the
// rolling-window timeline / overlap machinery is out of scope for v1).

use super::clustering::SpeakerClusterer;
use super::embedding::{EmbeddingError, EmbeddingExtractor};
use std::path::Path;

/// Minimum samples needed for the fbank frontend to produce the 10 frames
/// required by EmbeddingExtractor::compute (25ms frame + 9 * 10ms shifts).
const MIN_SAMPLES_FOR_EMBEDDING: usize = 1_840;

fn has_enough_samples_for_embedding(samples_len: usize) -> bool {
    samples_len >= MIN_SAMPLES_FOR_EMBEDDING
}

pub struct DiarizationSession {
    extractor: EmbeddingExtractor,
    clusterer: SpeakerClusterer,
}

impl DiarizationSession {
    pub fn new(embedding_model_path: &Path) -> Result<Self, EmbeddingError> {
        Self::with_profiles(embedding_model_path, Vec::new())
    }

    /// Create a session pre-seeded with saved voice profiles (name, centroid)
    /// so returning speakers are labeled by name instead of "Speaker N".
    pub fn with_profiles(
        embedding_model_path: &Path,
        profiles: Vec<(String, Vec<f32>)>,
    ) -> Result<Self, EmbeddingError> {
        let mut clusterer = SpeakerClusterer::new();
        for (name, centroid) in profiles {
            clusterer.seed_profile(&name, centroid);
        }
        Ok(Self {
            extractor: EmbeddingExtractor::new(embedding_model_path)?,
            clusterer,
        })
    }

    /// (label, centroid, segment count) snapshot for persisting this
    /// recording's speakers (written to speakers.json at recording end).
    pub fn centroid_snapshot(&self) -> Vec<(String, Vec<f32>, usize)> {
        self.clusterer
            .centroids()
            .map(|(label, centroid, count)| (label.to_string(), centroid.to_vec(), count))
            .collect()
    }

    /// Assign a speaker label to a 16kHz mono speech segment.
    /// Returns None only when no label can be produced (e.g. first segment
    /// is too short). Diarization failures must never break transcription —
    /// errors are logged and degrade to the previous label or None.
    pub fn label_segment(&mut self, samples_16k: &[f32]) -> Option<String> {
        if !has_enough_samples_for_embedding(samples_16k.len()) {
            return self.clusterer.last_label();
        }
        match self.extractor.compute(samples_16k) {
            Ok(embedding) => Some(self.clusterer.assign(&embedding)),
            Err(e) => {
                log::warn!(
                    "Diarization embedding failed, carrying previous label: {}",
                    e
                );
                self.clusterer.last_label()
            }
        }
    }

    pub fn clusterer(&self) -> &SpeakerClusterer {
        &self.clusterer
    }

    pub fn clusterer_mut(&mut self) -> &mut SpeakerClusterer {
        &mut self.clusterer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_gate_matches_minimum_fbank_frames() {
        assert!(!has_enough_samples_for_embedding(
            MIN_SAMPLES_FOR_EMBEDDING - 1
        ));
        assert!(has_enough_samples_for_embedding(MIN_SAMPLES_FOR_EMBEDDING));
    }
}
```

- [ ] **Step 3: Wire both into `mod.rs`**

In `frontend/src-tauri/src/diarization/mod.rs`, add `models` and `session`, plus the re-export. After this task `mod.rs` should read:
```rust
pub mod clustering;
pub mod embedding;
pub mod fbank;
pub mod models;
pub mod session;

pub use session::DiarizationSession;
```

- [ ] **Step 4: Run the session test + build**

```bash
cargo test --lib diarization::session 2>&1 | tail -20
cargo build 2>&1 | tail -10
```
Expected: `embedding_gate_matches_minimum_fbank_frames` passes; crate builds.

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/diarization/session.rs \
        frontend/src-tauri/src/diarization/models.rs \
        frontend/src-tauri/src/diarization/mod.rs
git commit -m "feat(speakers): :sparkles: add trimmed diarization session + on-demand model download"
```

---

## Task 7: Diarization Tauri commands + registration

**Files:**
- Create: `frontend/src-tauri/src/diarization/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `SpeakerProfilesRepository` (Task 2), `models` (Task 6), `crate::state::AppState` (has `db_manager.pool() -> &SqlitePool`).
- Produces (Tauri commands, all registered in `generate_handler!`):
  - `diarization_get_status(app, state) -> Result<serde_json::Value, String>` → `{ enabled, model_present, model_filename }`.
  - `diarization_set_enabled(state, enabled: bool) -> Result<(), String>`.
  - `diarization_download_model(app) -> Result<(), String>`.
  - `diarization_rename_speaker(state, meeting_id, old_label, new_name, save_profile: bool) -> Result<serde_json::Value, String>` → `{ updated_segments, profile_saved }`. **JS call:** `invoke('diarization_rename_speaker', { meetingId, oldLabel, newName, saveProfile })`.
  - `diarization_list_profiles(state) -> Result<Vec<serde_json::Value>, String>` → `[{ id, name }]`.
  - `diarization_rename_profile(state, id, name) -> Result<(), String>`.
  - `diarization_delete_profile(state, id) -> Result<(), String>`.
  - `pub async fn is_enabled(pool: &SqlitePool) -> bool` (used by Task 9).

- [ ] **Step 1: Lift `commands.rs` verbatim**

```bash
git show pr-538:frontend/src-tauri/src/diarization/commands.rs \
  > frontend/src-tauri/src/diarization/commands.rs
```
It references only `crate::database::repositories::speaker_profile::SpeakerProfilesRepository`, `crate::state::AppState`, `super::models`, `sqlx`, `tauri`, `serde_json` — all present. It has no timeline/overlap dependency. (It reads centroids from the meeting folder's `speakers.json` for the "remember" path — written by Task 9.)

- [ ] **Step 2: Wire `commands` into `mod.rs`**

In `frontend/src-tauri/src/diarization/mod.rs`, add `pub mod commands;` (keep the list alphabetical — before `embedding`):
```rust
pub mod clustering;
pub mod commands;
pub mod embedding;
pub mod fbank;
pub mod models;
pub mod session;

pub use session::DiarizationSession;
```

- [ ] **Step 3: Register the 7 commands in `generate_handler!`**

In `frontend/src-tauri/src/lib.rs`, inside the `tauri::generate_handler![ … ]` list (the workflow commands end around line 682), add these lines just after `summary::workflows::commands::api_cancel_workflow_run,`:
```rust
            diarization::commands::diarization_get_status,
            diarization::commands::diarization_set_enabled,
            diarization::commands::diarization_download_model,
            diarization::commands::diarization_rename_speaker,
            diarization::commands::diarization_list_profiles,
            diarization::commands::diarization_rename_profile,
            diarization::commands::diarization_delete_profile,
```
(Ensure the preceding line keeps its trailing comma and you do not break the closing `])`.)

- [ ] **Step 4: Build + run the whole diarization module's tests**

```bash
cargo build 2>&1 | tail -20
cargo test --lib diarization 2>&1 | tail -20
```
Expected: builds; all diarization unit tests (fbank ×3, clustering ×3, session ×1) pass. A build failure here usually means a command signature mismatch with `AppState`/`generate_handler` — re-check Step 3.

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/diarization/commands.rs \
        frontend/src-tauri/src/diarization/mod.rs \
        frontend/src-tauri/src/lib.rs
git commit -m "feat(speakers): :sparkles: add diarization Tauri commands + register handlers"
```

---

## Task 8: Thread the speaker label through the data model + persistence

**Files:**
- Modify: `frontend/src-tauri/src/database/models.rs`
- Modify: `frontend/src-tauri/src/api/api.rs`
- Modify: `frontend/src-tauri/src/database/repositories/transcript.rs`
- Modify: `frontend/src-tauri/src/audio/recording_saver.rs`
- Modify: `frontend/src-tauri/src/audio/recording_commands.rs`

**Interfaces:**
- Consumes: the existing `transcripts.speaker TEXT` column (migration `20251110000001`), `TranscriptUpdate.speaker` (added in Task 9 — this task compiles independently because it only reads/writes the label, but the live value is populated by Task 9).
- Produces:
  - `database::models::Transcript.speaker: Option<String>` (auto-populated by `SELECT * FROM transcripts` in `meeting.rs`).
  - `api::MeetingTranscript.speaker: Option<String>` + `api::TranscriptSegment.speaker: Option<String>` + the `From<Transcript> for MeetingTranscript` mapping carrying it.
  - `save_transcript` INSERT persists `speaker`.
  - `recording_saver::TranscriptSegment.speaker: Option<String>`; both `recording_commands` transcript-update listeners set it from the event.

- [ ] **Step 1: Add `speaker` to the `Transcript` DB model**

In `frontend/src-tauri/src/database/models.rs`, in the `Transcript` struct (starts line ~26; ends with `duration` at line ~37), add a field after `pub duration: Option<f64>,`:
```rust
    // Speaker identification label (e.g. "Speaker 1" or a profile name)
    pub speaker: Option<String>,
```
(`Transcript` derives `FromRow`; the `speaker` column already exists, so `SELECT *` populates it.)

- [ ] **Step 2: Add `speaker` to the API structs + conversion**

In `frontend/src-tauri/src/api/api.rs`:

(a) In `MeetingTranscript` (struct at line ~129, fields end with `duration` at ~139), add after `pub duration: Option<f64>,`:
```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
```

(b) In `TranscriptSegment` (struct at line ~180, fields end with `duration` at ~189), add after `pub duration: Option<f64>,`:
```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
```

(c) Find the `From<Transcript> for MeetingTranscript` (or the equivalent mapping that builds a `MeetingTranscript` from a `database::models::Transcript`) and add `speaker: transcript.speaker,` (or `speaker: t.speaker,` matching the local binding) to the constructed struct. Locate it with:
```bash
grep -nE "MeetingTranscript \{|impl From<.*Transcript> for MeetingTranscript|fn from" frontend/src-tauri/src/api/api.rs
```
Add the field alongside the existing `audio_start_time`/`audio_end_time`/`duration` assignments.

- [ ] **Step 3: Add the conversion regression test**

In `frontend/src-tauri/src/api/api.rs`, add (inside the existing `#[cfg(test)] mod tests { … }` if present, else create one at file end) a test that pins the mapping. Adapt the field names to the local `Transcript` constructor (it has `id, meeting_id, transcript, timestamp, summary, action_items, key_points, audio_start_time, audio_end_time, duration, speaker`):
```rust
    #[test]
    fn meeting_transcript_conversion_preserves_speaker_label() {
        let transcript = crate::database::models::Transcript {
            id: "t1".to_string(),
            meeting_id: "m1".to_string(),
            transcript: "hello".to_string(),
            timestamp: "00:00:01".to_string(),
            summary: None,
            action_items: None,
            key_points: None,
            audio_start_time: Some(1.0),
            audio_end_time: Some(2.0),
            duration: Some(1.0),
            speaker: Some("Speaker 2".to_string()),
        };
        let meeting_transcript: MeetingTranscript = transcript.into();
        assert_eq!(meeting_transcript.speaker.as_deref(), Some("Speaker 2"));
    }
```
(If the local conversion is a free function rather than `From`, call that instead of `.into()`.)

- [ ] **Step 4: Persist `speaker` in the `save_transcript` INSERT**

In `frontend/src-tauri/src/database/repositories/transcript.rs`, update the segment INSERT in `save_transcript` (lines ~49–60). Change the column list + placeholders + binds:
```rust
            let result = sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&transcript_id)
            .bind(&meeting_id)
            .bind(&segment.text)
            .bind(&segment.timestamp)
            .bind(segment.audio_start_time)
            .bind(segment.audio_end_time)
            .bind(segment.duration)
            .bind(&segment.speaker)
            .execute(&mut *transaction)
            .await;
```
(Leave the `import.rs` and `retranscription.rs` INSERTs unchanged — they will simply leave `speaker` NULL, which is valid.)

- [ ] **Step 5: Add `speaker` to the live-recording segment struct**

In `frontend/src-tauri/src/audio/recording_saver.rs`, in `TranscriptSegment` (struct at line ~16, ends with `pub sequence_id: u64,`), add:
```rust
    // Speaker identification label; None when the feature is disabled
    pub speaker: Option<String>,
```
Then fix the other constructor(s) of this struct so it still compiles. There is one at `recording_saver.rs:123` (`let segment = TranscriptSegment { … }`) — add `speaker: None,` there.

- [ ] **Step 6: Carry `speaker` from the event in both listeners**

In `frontend/src-tauri/src/audio/recording_commands.rs`, both transcript-update listeners build a `recording_saver::TranscriptSegment` (at lines ~269 and ~440, from the deserialized `update`). In each, add to the struct literal:
```rust
                    speaker: update.speaker.clone(),
```
(The `update` binding is the `TranscriptUpdate` payload; its `speaker` field is added in Task 9. If Task 9 is done after this, temporarily this will not compile — so implement Task 9 before building, or add the `speaker` field to `TranscriptUpdate` first. Reviewers: verify against Task 9's `TranscriptUpdate`.)

- [ ] **Step 7: Build + run the conversion test**

> Note: Step 6 references `update.speaker`, which is added in Task 9. If executing strictly in order, defer the *build* verification of Step 6/7 until Task 9 is applied, OR apply Task 9 Step 1 (the `TranscriptUpdate.speaker` field) first. The rest of this task builds independently.

```bash
cargo test --lib api::api::tests::meeting_transcript_conversion_preserves_speaker_label 2>&1 | tail -20
```
Expected: the conversion test passes (once the crate compiles).

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/src/database/models.rs \
        frontend/src-tauri/src/api/api.rs \
        frontend/src-tauri/src/database/repositories/transcript.rs \
        frontend/src-tauri/src/audio/recording_saver.rs \
        frontend/src-tauri/src/audio/recording_commands.rs
git commit -m "feat(speakers): :sparkles: thread speaker label through transcript model + persistence"
```

---

## Task 9: Splice diarization into the transcription worker (trimmed, best-effort)

**Files:**
- Modify: `frontend/src-tauri/src/audio/transcription/worker.rs`

**Interfaces:**
- Consumes: `diarization::{DiarizationSession, commands::is_enabled, models}` (Tasks 6–7), `SpeakerProfilesRepository` (Task 2), `crate::audio::audio_processing::resample_audio(&[f32], u32, u32) -> Vec<f32>`, `crate::audio::recording_commands::get_meeting_folder_path() -> Result<Option<String>, String>`, `crate::state::AppState`.
- Produces: `TranscriptUpdate.speaker: Option<String>` populated per segment; per-meeting centroids written to `<meeting_folder>/speakers.json` at recording end. All strictly best-effort — any failure leaves `speaker = None` and never affects transcription.

- [ ] **Step 1: Add `speaker` to `TranscriptUpdate`**

In `frontend/src-tauri/src/audio/transcription/worker.rs`, in the `TranscriptUpdate` struct (fields end with `pub duration: f64,`), add ONLY the speaker field (NOT the PR's overlap fields):
```rust
    // Speaker identification label ("Speaker 1" or a saved profile name);
    // None when the feature is disabled or no label could be computed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
```

- [ ] **Step 2: Add the imports the splice needs**

At the top of `worker.rs`, ensure the `tauri` import includes `Manager` (needed for `app.try_state`) and add nothing overlap-related. Change:
```rust
use tauri::{AppHandle, Emitter, Runtime};
```
to:
```rust
use tauri::{AppHandle, Emitter, Manager, Runtime};
```
(Do NOT add the PR's `use crate::diarization::overlap_detector::…` line.)

- [ ] **Step 3: Add the session-init helper**

Immediately before `pub fn start_transcription_task`, add this helper (verbatim from PR #538 — it is overlap-free):
```rust
/// Create the per-recording diarization session when the feature is enabled
/// and the embedding model has been downloaded. Any failure returns None so
/// speaker labels are simply absent — transcription is never affected.
async fn init_diarization_session<R: Runtime>(
    app: &AppHandle<R>,
) -> Option<crate::diarization::DiarizationSession> {
    let enabled = match app.try_state::<crate::state::AppState>() {
        Some(state) => crate::diarization::commands::is_enabled(state.db_manager.pool()).await,
        None => false,
    };
    if !enabled {
        info!("🎙️ Speaker identification disabled for this recording");
        return None;
    }
    if !crate::diarization::models::is_embedding_model_present(app) {
        warn!("🎙️ Speaker identification enabled but embedding model not downloaded - labels disabled");
        return None;
    }
    let model_path = match crate::diarization::models::embedding_model_path(app) {
        Ok(path) => path,
        Err(e) => {
            warn!("🎙️ Could not resolve diarization model path: {}", e);
            return None;
        }
    };

    // Seed saved voice profiles so returning speakers are labeled by name
    let profiles: Vec<(String, Vec<f32>)> = match app.try_state::<crate::state::AppState>() {
        Some(state) => {
            match crate::database::repositories::speaker_profile::SpeakerProfilesRepository::list(
                state.db_manager.pool(),
            )
            .await
            {
                Ok(profiles) => profiles.into_iter().map(|p| (p.name, p.embedding)).collect(),
                Err(e) => {
                    warn!("🎙️ Failed to load voice profiles, continuing without: {}", e);
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };
    let profile_count = profiles.len();

    match crate::diarization::DiarizationSession::with_profiles(&model_path, profiles) {
        Ok(session) => {
            info!(
                "🎙️ ✅ Speaker identification active for this recording ({} saved profile{})",
                profile_count,
                if profile_count == 1 { "" } else { "s" }
            );
            Some(session)
        }
        Err(e) => {
            warn!("🎙️ Failed to initialize speaker identification: {}", e);
            None
        }
    }
}
```

- [ ] **Step 4: Add the centroid-persistence helper (TRIMMED — no timeline)**

After `init_diarization_session`, add this helper. It is the PR's `persist_speaker_centroids` **with the timeline snapshot removed** (v1 has no timeline):
```rust
/// Persist this recording's speaker centroids to speakers.json in the meeting
/// folder so a later rename can save the voice as a profile. The folder must
/// be captured while the recording manager is still alive.
async fn persist_speaker_centroids(
    session: &crate::diarization::DiarizationSession,
    folder: Option<std::path::PathBuf>,
) {
    let snapshot = session.centroid_snapshot();
    if snapshot.is_empty() {
        return;
    }
    let folder = match folder {
        Some(folder) => folder,
        None => {
            warn!("🎙️ No meeting folder available - speaker centroids not persisted");
            return;
        }
    };
    let json = serde_json::json!({
        "version": "1.0",
        "speakers": snapshot.iter().map(|(label, centroid, count)| {
            serde_json::json!({ "label": label, "centroid": centroid, "segments": count })
        }).collect::<Vec<_>>(),
    });
    let path = folder.join("speakers.json");
    match serde_json::to_string(&json).map(|s| std::fs::write(&path, s)) {
        Ok(Ok(())) => info!(
            "🎙️ Saved {} speaker centroid(s) to {}",
            snapshot.len(),
            path.display()
        ),
        Ok(Err(e)) => warn!("🎙️ Failed to write speakers.json: {}", e),
        Err(e) => warn!("🎙️ Failed to serialize speaker centroids: {}", e),
    }
}
```

- [ ] **Step 5: Initialize the session + folder holders inside the task**

Inside `start_transcription_task`'s spawned async block, right after the `transcription_engine` is obtained (before `const NUM_WORKERS`), add:
```rust
        // Initialize speaker identification if enabled and its model is present.
        // Failure only disables speaker labels; transcription proceeds normally.
        let diarization_session = Arc::new(tokio::sync::Mutex::new(
            init_diarization_session(&app).await,
        ));
        // Meeting folder for speakers.json, captured lazily while the recording
        // manager still exists (it is torn down during stop).
        let diarization_folder: Arc<tokio::sync::Mutex<Option<std::path::PathBuf>>> =
            Arc::new(tokio::sync::Mutex::new(None));
```

- [ ] **Step 6: Clone the holders into each worker**

In the `for worker_id in 0..NUM_WORKERS` loop, where the other `*_clone` bindings are created (after `let chunks_queued_clone = chunks_queued.clone();`), add:
```rust
            let diarization_clone = diarization_session.clone();
            let diarization_folder_clone = diarization_folder.clone();
```

- [ ] **Step 7: Resample the chunk to 16 kHz mono for the embedding**

Inside the worker loop, after `let chunk_duration = chunk.data.len() as f64 / chunk.sample_rate as f64;` and BEFORE the chunk is moved into `transcribe_chunk_with_provider(&engine_clone, chunk, …)`, capture the samples (the chunk is consumed by STT, so clone/resample first):
```rust
                            let chunk_id_for_logging = chunk.chunk_id;

                            // Keep segment samples for speaker embedding (STT consumes the chunk).
                            // Diarization (fbank + WeSpeaker) requires 16kHz mono; chunks arrive at
                            // the device rate (e.g. 48kHz), so resample to match — feeding raw 48kHz
                            // to the fbank frontend would corrupt the frequency mapping.
                            let diarization_samples: Option<Vec<f32>> = {
                                let guard = diarization_clone.lock().await;
                                if guard.is_some() {
                                    if chunk.sample_rate != 16000 {
                                        Some(crate::audio::audio_processing::resample_audio(
                                            &chunk.data,
                                            chunk.sample_rate,
                                            16000,
                                        ))
                                    } else {
                                        Some(chunk.data.clone())
                                    }
                                } else {
                                    None
                                }
                            };
```

- [ ] **Step 8: Compute the label and attach it (TRIMMED — no overlap)**

In the `Ok((transcript, confidence_opt, is_partial))` arm, after `audio_start_time`/`audio_end_time` are computed and BEFORE the `let update = TranscriptUpdate { … }` literal, add the trimmed labeling block (replaces the PR's overlap-laden version):
```rust
                                        // Assign a speaker label from this segment's voice embedding.
                                        // Best-effort: any failure yields None and never affects the transcript.
                                        let speaker: Option<String> = if let Some(samples) =
                                            &diarization_samples
                                        {
                                            // Capture the meeting folder once, while recording is live.
                                            {
                                                let mut folder_guard =
                                                    diarization_folder_clone.lock().await;
                                                if folder_guard.is_none() {
                                                    if let Ok(Some(folder)) =
                                                        crate::audio::recording_commands::get_meeting_folder_path().await
                                                    {
                                                        *folder_guard =
                                                            Some(std::path::PathBuf::from(folder));
                                                    }
                                                }
                                            }
                                            let mut guard = diarization_clone.lock().await;
                                            guard.as_mut().and_then(|session| session.label_segment(samples))
                                        } else {
                                            None
                                        };
                                        if should_log_this_chunk && speaker.is_some() {
                                            info!(
                                                "🎙️ Worker {} labeled chunk {} as speaker={:?}",
                                                worker_id, chunk_id_for_logging, speaker
                                            );
                                        }
```
Then in the `TranscriptUpdate { … }` literal, add after `duration: chunk_duration,`:
```rust
                                            speaker,
```

- [ ] **Step 9: Persist centroids at task end**

After the worker loop joins and before the final verification logic (mirror the PR's placement — near the end of the spawned block, after all workers are done), add:
```rust
        // Persist speaker centroids so a later rename can save the voice as a
        // profile (must run before the recording manager is torn down).
        if let Some(session) = diarization_session.lock().await.as_ref() {
            let folder = diarization_folder.lock().await.clone();
            persist_speaker_centroids(session, folder).await;
        }
```

- [ ] **Step 10: Build (there is no pure unit test for the worker splice)**

```bash
cargo build 2>&1 | tail -30
```
Expected: builds cleanly. Common failures: a moved-`chunk` borrow (ensure Step 7's clone happens before `chunk` is passed to STT), or a missing `Manager` import (Step 2). Also run the full module tests to confirm nothing regressed:
```bash
cargo test --lib diarization 2>&1 | tail -10
```

- [ ] **Step 11: Commit**

```bash
git add frontend/src-tauri/src/audio/transcription/worker.rs
git commit -m "feat(speakers): :sparkles: splice best-effort per-segment diarization into the transcription worker"
```

---

## Task 10: Frontend types + capture speaker from the live event

**Files:**
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/contexts/TranscriptContext.tsx`

**Interfaces:**
- Consumes: the `speaker` field emitted on `transcript-update` (Task 9) and returned in `MeetingTranscript` (Task 8).
- Produces: `Transcript.speaker?: string`, `TranscriptUpdate.speaker?: string`, `TranscriptSegmentData.speaker?: string`; live transcripts carry `speaker`; because `StorageService.saveMeeting` passes whole `Transcript` objects to `api_save_transcript`, the label persists automatically (no storageService change needed).

- [ ] **Step 1: Add `speaker` to the TS types**

In `frontend/src/types/index.ts`:

(a) `Transcript` (interface at line ~7) — add after `duration?: number;`:
```ts
  speaker?: string; // Speaker identification label ("Speaker 1" / profile name)
```
(b) `TranscriptUpdate` (interface at line ~21) — add after `duration: number;`:
```ts
  speaker?: string; // Speaker identification label, when diarization is enabled
```
(c) `TranscriptSegmentData` (interface at line ~104) — add after `confidence?: number;`:
```ts
  speaker?: string; // Speaker identification label for chip rendering
```

- [ ] **Step 2: Capture `speaker` on the live transcript**

In `frontend/src/contexts/TranscriptContext.tsx`, in the main transcript-update listener where `newTranscript: Transcript` is built from `update` (around line ~306), add to the object literal (after `duration: update.duration,`):
```ts
            speaker: update.speaker,
```
(The other `newTranscript` at line ~416 is a manual add with no audio update — leave it without `speaker`.)

- [ ] **Step 3: Typecheck**

Run (from `frontend/`):
```bash
npx tsc --noEmit 2>&1 | tail -20
```
Expected: no new type errors introduced by these files.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/types/index.ts frontend/src/contexts/TranscriptContext.tsx
git commit -m "feat(speakers): :sparkles: carry speaker label through frontend transcript types + live context"
```

---

## Task 11: Speaker-identification settings panel

**Files:**
- Create: `frontend/src/components/SpeakerIdentificationSettings.tsx`
- Modify: `frontend/src/components/SettingTabs.tsx` (or the settings surface that hosts recording/transcript settings)

**Interfaces:**
- Consumes: commands `diarization_get_status`, `diarization_set_enabled`, `diarization_download_model`, `diarization_list_profiles`, `diarization_rename_profile`, `diarization_delete_profile` (Task 7); event `diarization-model-download-progress` (Task 6); `ui/{switch,progress,button,label}`, `sonner`, `lucide-react` (all present).
- Produces: a mounted settings section with an off-by-default enable toggle, on-demand model download with progress, and a "remembered voices" list (rename/forget).

- [ ] **Step 1: Lift the component verbatim**

```bash
git show pr-538:frontend/src/components/SpeakerIdentificationSettings.tsx \
  > frontend/src/components/SpeakerIdentificationSettings.tsx
```
It imports only `./ui/button`, `./ui/label`, `./ui/switch`, `./ui/progress`, `lucide-react`, `sonner`, and `@tauri-apps/api` — all present in this tree.

- [ ] **Step 2: Mount it in the settings UI**

Decide the host by inspecting the settings surface:
```bash
grep -nE "RecordingSettings|TranscriptSettings|TabsContent|export function|export default" frontend/src/components/SettingTabs.tsx
```
Import and render `<SpeakerIdentificationSettings />` within the same tab/section that hosts `RecordingSettings` (or the "Recording"/"Transcription" settings tab). Add the import:
```tsx
import { SpeakerIdentificationSettings } from './SpeakerIdentificationSettings';
```
and place `<SpeakerIdentificationSettings />` beneath the existing recording/transcription settings block in that tab's JSX. (If `SettingTabs.tsx` is not the right host, mount it in the component that renders `RecordingSettings` — the goal is that it appears in the app's settings, in the recording/transcription area.)

- [ ] **Step 3: Typecheck**

```bash
npx tsc --noEmit 2>&1 | tail -20
```
Expected: no new type errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/SpeakerIdentificationSettings.tsx frontend/src/components/SettingTabs.tsx
git commit -m "feat(speakers): :sparkles: add speaker identification settings panel"
```

---

## Task 12: Speaker chips + rename dialog in the transcript views

**Files:**
- Create: `frontend/src/components/SpeakerRenameDialog.tsx`
- Modify: `frontend/src/components/TranscriptView.tsx`
- Modify: `frontend/src/components/VirtualizedTranscriptView.tsx`
- Modify: `frontend/src/components/MeetingDetails/TranscriptPanel.tsx`

**Interfaces:**
- Consumes: `Transcript.speaker` / `TranscriptSegmentData.speaker` (Task 10); command `diarization_rename_speaker` (Task 7); `ui/{dialog,button,input,label}`, `sonner` (present).
- Produces: a colored speaker chip before each labeled segment (live + saved); in the saved-meeting view, clicking a chip opens `SpeakerRenameDialog` to rename + optionally remember the voice.

- [ ] **Step 1: Lift the rename dialog verbatim**

```bash
git show pr-538:frontend/src/components/SpeakerRenameDialog.tsx \
  > frontend/src/components/SpeakerRenameDialog.tsx
```
Props: `{ meetingId: string; speakerLabel: string; onClose: () => void; onRenamed: () => void | Promise<void> }`. It invokes `diarization_rename_speaker` with `{ meetingId, oldLabel, newName, saveProfile }` and toasts the result. Imports only `./ui/{dialog,button,input,label}`, `sonner`, `@tauri-apps/api/core` — all present.

- [ ] **Step 2: Pass `speaker` through to the segment renderers**

In `frontend/src/components/MeetingDetails/TranscriptPanel.tsx`, the transcripts are mapped into segment data (around line ~58: `transcripts.map(t => ({ … timestamp, text … }))`). Add `speaker: t.speaker,` to that mapped object so it reaches `TranscriptView`/`VirtualizedTranscriptView`:
```tsx
    return transcripts.map(t => ({
      // …existing fields (id, timestamp, endTime, text, confidence)…
      speaker: t.speaker,
    }));
```

- [ ] **Step 3: Render the chip in `TranscriptView.tsx`**

In `frontend/src/components/TranscriptView.tsx`, where each segment's text is rendered, render a chip before the text when `segment.speaker` (or the local segment variable's `.speaker`) is present. Add a small helper for a stable color and the chip markup, e.g.:
```tsx
// Deterministic chip color from the label so the same speaker keeps one color.
function speakerChipColor(label: string): string {
  const palette = [
    'bg-blue-100 text-blue-700', 'bg-green-100 text-green-700',
    'bg-purple-100 text-purple-700', 'bg-amber-100 text-amber-700',
    'bg-pink-100 text-pink-700', 'bg-cyan-100 text-cyan-700',
  ];
  let hash = 0;
  for (let i = 0; i < label.length; i++) hash = (hash * 31 + label.charCodeAt(i)) | 0;
  return palette[Math.abs(hash) % palette.length];
}
```
and, immediately before the segment text node:
```tsx
{segment.speaker && (
  <span className={`inline-block mr-2 px-1.5 py-0.5 rounded text-xs font-medium ${speakerChipColor(segment.speaker)}`}>
    {segment.speaker}
  </span>
)}
```
(Adapt `segment` to the local variable name used in this component's map.)

- [ ] **Step 4: Render the chip in `VirtualizedTranscriptView.tsx`**

Apply the same chip (reuse the `speakerChipColor` helper — either duplicate the small function locally or lift it into a shared util; a local copy is acceptable given no shared util exists) before the segment text in `frontend/src/components/VirtualizedTranscriptView.tsx`. Ensure the row's `TranscriptSegmentData` carries `speaker` (Task 10 added the type field; Step 2 populates it).

- [ ] **Step 5: Wire the rename dialog in the saved-meeting view**

In `frontend/src/components/MeetingDetails/TranscriptPanel.tsx` (the saved-meeting transcript view — it has `meetingId` in scope), make the chip clickable to open `SpeakerRenameDialog`. Add state:
```tsx
const [renameSpeaker, setRenameSpeaker] = useState<string | null>(null);
```
Pass an `onSpeakerClick?: (label: string) => void` down to the segment renderer (or, if the views are rendered inline here, attach `onClick={() => setRenameSpeaker(segment.speaker!)}` to the chip). Render the dialog when a label is selected:
```tsx
{renameSpeaker && (
  <SpeakerRenameDialog
    meetingId={meetingId}
    speakerLabel={renameSpeaker}
    onClose={() => setRenameSpeaker(null)}
    onRenamed={async () => { setRenameSpeaker(null); await reloadTranscripts(); }}
  />
)}
```
Use the panel's existing transcript-refetch function for `onRenamed` (find it with `grep -nE "getMeeting|reload|refetch|loadTranscripts|fetchTranscripts" frontend/src/components/MeetingDetails/TranscriptPanel.tsx`); if none exists, call the existing meeting-detail loader so renamed labels re-render. Add the import:
```tsx
import { SpeakerRenameDialog } from '@/components/SpeakerRenameDialog';
```
(In the live recording view, chips are display-only — no rename wiring there in v1.)

- [ ] **Step 6: Typecheck**

```bash
npx tsc --noEmit 2>&1 | tail -30
```
Expected: no new type errors.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/components/SpeakerRenameDialog.tsx \
        frontend/src/components/TranscriptView.tsx \
        frontend/src/components/VirtualizedTranscriptView.tsx \
        frontend/src/components/MeetingDetails/TranscriptPanel.tsx
git commit -m "feat(speakers): :sparkles: render speaker chips + wire rename/remember dialog"
```

---

## Task 13: Feed speaker labels into summary + workflow prompts (§9)

**Files:**
- Modify: `frontend/src/components/MeetingDetails/SummaryPanel.tsx`

**Interfaces:**
- Consumes: `Transcript.speaker` (Task 10). The transcript text passed to both the summary generator and `runWorkflow` is assembled here.
- Produces: when a segment has a `speaker`, the LLM transcript text prefixes it with `[speaker] ` so both the built-in summary and workflow runs attribute who said what. No backend change — the assembled `text` string flows through `api_process_transcript` and `run_workflow` unchanged.

- [ ] **Step 1: Prefix the speaker label at both assembly sites**

In `frontend/src/components/MeetingDetails/SummaryPanel.tsx`, there are two identical assembly expressions (lines ~370 and ~444):
```tsx
transcriptText={transcripts.map((t) => t.text).join('\n')}
```
Replace BOTH with a speaker-aware join:
```tsx
transcriptText={transcripts.map((t) => (t.speaker ? `[${t.speaker}] ${t.text}` : t.text)).join('\n')}
```
(If a `useMemo`-ed value is cleaner given both sites are identical, extract a `const speakerAwareTranscript = useMemo(() => transcripts.map((t) => t.speaker ? \`[${t.speaker}] ${t.text}\` : t.text).join('\n'), [transcripts]);` and use it in both places — but the inline form above is sufficient and matches the existing style.)

- [ ] **Step 2: Typecheck**

```bash
npx tsc --noEmit 2>&1 | tail -20
```
Expected: no new type errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/MeetingDetails/SummaryPanel.tsx
git commit -m "feat(speakers): :sparkles: prefix transcript with speaker labels for summary + workflow prompts"
```

---

## Manual End-to-End Verification (after all tasks)

These paths are model-dependent and cannot be unit-tested. Run against a full build (`frontend/build-gpu.sh` for the real llama-helper sidecar, or `pnpm run tauri:dev:metal` for dev):

1. **Enable + download:** Settings → Speaker identification → toggle on → "Download speaker model (~28 MB)" → progress completes, status shows model present.
2. **Cluster:** record a meeting with 2–3 distinct voices → transcript segments show stable `Speaker 1/2/3` chips.
3. **Rename + remember:** click a chip → rename to "Me" (or "Alice") + check "Remember this voice" → all that speaker's segments relabel; a toast confirms the voice was remembered; the profile appears under Settings → Remembered voices.
4. **Cross-meeting recognition:** record a second meeting where that voice speaks → it is auto-labeled "Me"/"Alice" (not `Speaker N`).
5. **Summary/workflow attribution:** generate the built-in summary and run a workflow → the output reflects speaker attribution (the LLM saw `[Me] …` / `[Alice] …` prefixes).
6. **Graceful degradation:** with the feature off (or the model absent), recording + transcription + summaries work exactly as before, with no speaker chips and no errors.

---

## Self-Review

**Spec coverage** (design §1–§15):
- §4 module (fbank/embedding/clustering/session/models/commands/mod) → Tasks 3–7. ✓
- §4 macOS CoreML EP + §6 acceleration + §14 `ort` coreml feature → Tasks 1, 4. ✓
- §5 data model (speaker_profiles + diarization_settings tables, repository, repurposed `transcripts.speaker`, `Transcript.speaker`) → Tasks 2, 8. ✓
- §6 worker integration + performance (spawn/label per-segment only, no full-timeline recompute, resample to 16k, best-effort) → Task 9. ✓ (Embedding runs synchronously under the worker's per-chunk `await` on a small model once per speech segment — matching the light-load design; no full-timeline recompute since the timeline path is excluded.)
- §7 model management (pinned URL, atomic download, progress, size guard) → Task 6 (models.rs verbatim). ✓
- §8 frontend (chips, rename dialog, settings, tsc verification) → Tasks 10–12. ✓
- §9 summary/workflow speaker prefixing → Task 13. ✓
- §10 self-detection v1 (rename cluster → "Me" + remember) → Tasks 7, 12 (no special mechanism; via rename+profile). ✓
- §11 error handling / degradation → Task 9 (init returns None; label best-effort). ✓
- §12 testing (ported pure-logic tests + manual E2E) → Tasks 2–8 unit tests + Manual E2E section. ✓
- §13 out-of-scope (overlap/timeline/mic-"Me"/bundled features) → excluded per Global Constraints + trimmed session (Task 6) + trimmed worker (Task 9). ✓
- §14 open items — all resolved during planning: worker splice point + 16k samples via `resample_audio` (Task 9); Transcript fields = just `speaker` (Task 8); model URL pinned, size-guard integrity (Task 6); frontend reconciliation onto current components (Tasks 11–13); session lifts cleanly once timeline excluded (Task 6 trimmed content); `ort` coreml feature enabled macOS-only + EP registration with CPU fallback (Tasks 1, 4). ✓

**Placeholder scan:** No TBD/TODO; every code step shows exact content or an exact `git show` extraction + exact edits. Verbatim lifts are intentional (byte-exact DSP/ONNX code + its own tests) — the extraction command is the precise, non-placeholder instruction.

**Type consistency:** `DiarizationSession::{with_profiles,label_segment,centroid_snapshot}`, `SpeakerProfilesRepository::{list,create}`, `models::{is_embedding_model_present,embedding_model_path}`, `commands::is_enabled`, and `TranscriptUpdate.speaker`/`Transcript.speaker`/`TranscriptSegment.speaker` names are used identically across Tasks 6–13. The rename command uses `old_label` (Rust) ↔ `oldLabel` (JS) as in the lifted `commands.rs` + `SpeakerRenameDialog.tsx`. Cross-task dependency noted: Task 8 Step 6 references `TranscriptUpdate.speaker` from Task 9 Step 1 — flagged inline with guidance to apply that field first.
