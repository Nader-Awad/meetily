# Retroactive Speaker Diarization (Retranscribe + Import) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Retranscribe and Import batch pipelines apply speaker diarization (labels + saved voice profiles) so existing/imported meetings get `speaker` labels, not just live recordings.

**Architecture:** Extract the live worker's two diarization helpers into the diarization module so all three callers share them; widen the shared `create_transcript_segments` seam to carry a speaker label; splice per-segment `label_segment` into the retranscription and import transcribe loops (their VAD segments are already 16 kHz mono — no resampling), persist `speaker` in each INSERT, and write `speakers.json` so rename/"remember" works retroactively. Best-effort: disabled/no-model → no labels, batch behaves exactly as today.

**Tech Stack:** Rust / Tauri v2, SQLx/SQLite, existing `diarization` module + `DiarizationSession`.

## Global Constraints
- Personal local fork; branch `feature/retroactive-diarization`; merge to local `main` only; no push.
- **Best-effort / isolated:** diarization must never break or abort retranscription/import. No `?`/`unwrap`/`expect`/`panic` on the diarization path; a `None` session (feature disabled or model absent) means no labels and byte-for-byte prior behavior.
- **No new dependency; no migration** (the `transcripts.speaker` column already exists).
- VAD `speech_segments` in both batch loops are already **16 kHz mono** (`segment.samples`) — do NOT resample.
- Live worker behavior is unchanged (Task 1 only swaps its private helpers for the shared ones).
- Build env: run cargo from `frontend/src-tauri`; Xcode + `binaries/llama-helper-aarch64-apple-darwin` placeholder present; the 2 pre-existing `audio::device_detection` unit-test failures are unrelated (scope test runs to the relevant module).
- Gitmoji conventional commits; no AI attribution / no `Co-Authored-By`.

## File Structure
- Modify `frontend/src-tauri/src/diarization/commands.rs` — add `pub async fn init_session` + `pub async fn persist_speaker_centroids` (moved from worker). (Task 1)
- Modify `frontend/src-tauri/src/audio/transcription/worker.rs` — call the shared helpers; delete the private copies. (Task 1)
- Modify `frontend/src-tauri/src/audio/common.rs` — `create_transcript_segments` carries speaker. (Task 2)
- Modify `frontend/src-tauri/src/audio/retranscription.rs` — 4-tuple `all_transcripts` + tests (Task 2), then the diarization splice + INSERT + persist (Task 3).
- Modify `frontend/src-tauri/src/audio/import.rs` — 4-tuple `all_transcripts` (Task 2), then the diarization splice + INSERT + persist (Task 4).

---

## Task 1: Extract the shared diarization helpers (DRY, behavior-preserving)

**Files:**
- Modify: `frontend/src-tauri/src/diarization/commands.rs`
- Modify: `frontend/src-tauri/src/audio/transcription/worker.rs`

**Interfaces:**
- Produces: `pub async fn crate::diarization::commands::init_session<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<crate::diarization::DiarizationSession>` and `pub async fn crate::diarization::commands::persist_speaker_centroids(session: &crate::diarization::DiarizationSession, folder: Option<std::path::PathBuf>)`. Consumed by Task 3 + Task 4 (and now by the live worker).

- [ ] **Step 1: Add `Manager` to the tauri import in `commands.rs`** — the moved `init_session` calls `app.try_state`, which needs the `Manager` trait. Change `use tauri::{command, AppHandle, Runtime};` to:
```rust
use tauri::{command, AppHandle, Manager, Runtime};
```

- [ ] **Step 2: Add the two shared helpers to `commands.rs`** (append near the other functions). These are the worker's helpers, adapted to `commands.rs`'s local paths (`is_enabled` is a sibling fn; `super::models`; `super::DiarizationSession`; `SpeakerProfilesRepository` + `AppState` already imported):
```rust
/// Create a diarization session for a recording/batch job when the feature is
/// enabled AND the embedding model is present. Seeds saved voice profiles so
/// returning speakers are labeled by name. Any failure returns None so speaker
/// labels are simply absent — transcription is never affected.
pub async fn init_session<R: Runtime>(
    app: &AppHandle<R>,
) -> Option<super::DiarizationSession> {
    let enabled = match app.try_state::<AppState>() {
        Some(state) => is_enabled(state.db_manager.pool()).await,
        None => false,
    };
    if !enabled {
        log::info!("🎙️ Speaker identification disabled");
        return None;
    }
    if !super::models::is_embedding_model_present(app) {
        log::warn!("🎙️ Speaker identification enabled but embedding model not downloaded - labels disabled");
        return None;
    }
    let model_path = match super::models::embedding_model_path(app) {
        Ok(path) => path,
        Err(e) => {
            log::warn!("🎙️ Could not resolve diarization model path: {}", e);
            return None;
        }
    };

    let profiles: Vec<(String, Vec<f32>)> = match app.try_state::<AppState>() {
        Some(state) => match SpeakerProfilesRepository::list(state.db_manager.pool()).await {
            Ok(profiles) => profiles.into_iter().map(|p| (p.name, p.embedding)).collect(),
            Err(e) => {
                log::warn!("🎙️ Failed to load voice profiles, continuing without: {}", e);
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let profile_count = profiles.len();

    match super::DiarizationSession::with_profiles(&model_path, profiles) {
        Ok(session) => {
            log::info!(
                "🎙️ ✅ Speaker identification active ({} saved profile{})",
                profile_count,
                if profile_count == 1 { "" } else { "s" }
            );
            Some(session)
        }
        Err(e) => {
            log::warn!("🎙️ Failed to initialize speaker identification: {}", e);
            None
        }
    }
}

/// Persist a session's speaker centroids to speakers.json in the meeting folder
/// so a later rename can save the voice as a persistent profile.
pub async fn persist_speaker_centroids(
    session: &super::DiarizationSession,
    folder: Option<std::path::PathBuf>,
) {
    let snapshot = session.centroid_snapshot();
    if snapshot.is_empty() {
        return;
    }
    let folder = match folder {
        Some(folder) => folder,
        None => {
            log::warn!("🎙️ No meeting folder available - speaker centroids not persisted");
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
        Ok(Ok(())) => log::info!("🎙️ Saved {} speaker centroid(s) to {}", snapshot.len(), path.display()),
        Ok(Err(e)) => log::warn!("🎙️ Failed to write speakers.json: {}", e),
        Err(e) => log::warn!("🎙️ Failed to serialize speaker centroids: {}", e),
    }
}
```

- [ ] **Step 3: Delete the two private helpers from `worker.rs`** — remove `async fn init_diarization_session<R: Runtime>(...)` (currently ~line 51) and `async fn persist_speaker_centroids(...)` (currently ~line 112) in their entirety.

- [ ] **Step 4: Update the worker's two call sites** — in `worker.rs`:
  - The init call (currently `init_diarization_session(&app).await` inside `Arc::new(tokio::sync::Mutex::new( ... ))`) → `crate::diarization::commands::init_session(&app).await`.
  - The end-of-task persist call (currently `persist_speaker_centroids(session, folder).await`) → `crate::diarization::commands::persist_speaker_centroids(session, folder).await`.
  (No other worker logic changes — the `Arc<Mutex>`, resample, `label_segment`, and emit all stay.)

- [ ] **Step 5: Build + run diarization tests**
```bash
cd frontend/src-tauri
cargo build 2>&1 | tail -15
cargo test --lib diarization 2>&1 | tail -10
```
Expected: builds cleanly; the 7 diarization unit tests pass. (This is a behavior-preserving move — the live path still initializes + persists identically, just via the shared functions.)

- [ ] **Step 6: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/commands.rs frontend/src-tauri/src/audio/transcription/worker.rs
git commit -m "refactor(speakers): :recycle: share diarization session init + centroid persistence across pipelines"
```

---

## Task 2: `create_transcript_segments` carries a speaker label

**Files:**
- Modify: `frontend/src-tauri/src/audio/common.rs` (`create_transcript_segments` ~line 51)
- Modify: `frontend/src-tauri/src/audio/retranscription.rs` (`all_transcripts` decl line 339; the loop push ~line 400; tests ~838-910)
- Modify: `frontend/src-tauri/src/audio/import.rs` (`all_transcripts` decl line 548; the loop push ~line 605)

**Interfaces:**
- Produces: `create_transcript_segments(transcripts: &[(String, f64, f64, Option<String>)]) -> Vec<TranscriptSegment>` (adds a 4th tuple element = speaker; sets `TranscriptSegment.speaker`). Consumed by Tasks 3 + 4.
- This task is a pure mechanical widening: both call sites push `None` for the new element and the INSERTs are unchanged, so runtime behavior is identical (speaker stays NULL). Tasks 3/4 replace the `None` with a real label.

- [ ] **Step 1: Update the `create_transcript_segments` unit tests (fail first)** — in `retranscription.rs`'s `#[cfg(test)] mod`, change the existing tests' tuples to 4-tuples and add a speaker-propagation assertion. Replace the bodies of `test_create_transcript_segments_empty/single/multiple` (and any others there) so their input tuples have a 4th element, e.g.:
```rust
    #[test]
    fn test_create_transcript_segments_empty() {
        let transcripts: Vec<(String, f64, f64, Option<String>)> = vec![];
        let segments = create_transcript_segments(&transcripts);
        assert!(segments.is_empty());
    }

    #[test]
    fn test_create_transcript_segments_single() {
        let transcripts = vec![("Hello world".to_string(), 0.0, 1500.0, None)];
        let segments = create_transcript_segments(&transcripts);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Hello world");
        assert_eq!(segments[0].audio_start_time, Some(0.0));
        assert_eq!(segments[0].audio_end_time, Some(1.5));
        assert_eq!(segments[0].duration, Some(1.5));
        assert_eq!(segments[0].speaker, None);
    }

    #[test]
    fn test_create_transcript_segments_propagates_speaker() {
        let transcripts = vec![
            ("Hi".to_string(), 0.0, 1000.0, Some("Speaker 1".to_string())),
            ("Bye".to_string(), 1000.0, 2000.0, None),
        ];
        let segments = create_transcript_segments(&transcripts);
        assert_eq!(segments[0].speaker.as_deref(), Some("Speaker 1"));
        assert_eq!(segments[1].speaker, None);
    }
```
Also update `test_create_transcript_segments_multiple` and `_trims_whitespace` / `_generates_unique_ids` (if present) to add `, None` as the 4th element of each tuple. (Keep their other assertions unchanged.)

- [ ] **Step 2: Run tests to confirm they fail**
```bash
cd frontend/src-tauri && cargo test --lib audio::retranscription 2>&1 | tail -20
```
Expected: compile error (`create_transcript_segments` takes 3-tuples; the tests + call sites now mismatch).

- [ ] **Step 3: Widen `create_transcript_segments`** in `common.rs`:
```rust
/// Create transcript segments from transcription results.
/// Each tuple is (text, start_ms, end_ms, speaker) from VAD timestamps + optional diarization label.
pub(crate) fn create_transcript_segments(transcripts: &[(String, f64, f64, Option<String>)]) -> Vec<TranscriptSegment> {
    transcripts
        .iter()
        .map(|(text, start_ms, end_ms, speaker)| {
            let start_seconds = start_ms / 1000.0;
            let end_seconds = end_ms / 1000.0;
            let duration = end_seconds - start_seconds;

            TranscriptSegment {
                id: format!("transcript-{}", Uuid::new_v4()),
                text: text.trim().to_string(),
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

- [ ] **Step 4: Update the two call sites to compile (push `None` for now)**
  - `retranscription.rs:339`: change `let mut all_transcripts: Vec<(String, f64, f64)> = Vec::new();` → `let mut all_transcripts: Vec<(String, f64, f64, Option<String>)> = Vec::new();`
  - `retranscription.rs` loop push (`all_transcripts.push((text, segment.start_timestamp_ms, segment.end_timestamp_ms));`) → `all_transcripts.push((text, segment.start_timestamp_ms, segment.end_timestamp_ms, None));`
  - `import.rs:548`: same decl change to `Vec<(String, f64, f64, Option<String>)>`.
  - `import.rs` loop push (`all_transcripts.push((text, segment.start_timestamp_ms, segment.end_timestamp_ms));`) → add `, None)`.

- [ ] **Step 5: Run tests + build**
```bash
cd frontend/src-tauri
cargo test --lib audio::retranscription 2>&1 | tail -15
cargo build 2>&1 | tail -10
```
Expected: the `create_transcript_segments` tests pass (incl. the new speaker-propagation test); crate builds. Behavior unchanged (speaker still NULL everywhere).

- [ ] **Step 6: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/audio/common.rs frontend/src-tauri/src/audio/retranscription.rs frontend/src-tauri/src/audio/import.rs
git commit -m "feat(speakers): :sparkles: thread optional speaker label through create_transcript_segments"
```

---

## Task 3: Splice diarization into the Retranscription loop

**Files:**
- Modify: `frontend/src-tauri/src/audio/retranscription.rs`

**Interfaces:**
- Consumes: `crate::diarization::commands::{init_session, persist_speaker_centroids}` (Task 1); the 4-tuple `all_transcripts` + `create_transcript_segments` (Task 2).

- [ ] **Step 1: Initialize a diarization session before the transcribe loop** — in `run_retranscription`, immediately after `let mut all_transcripts: Vec<(String, f64, f64, Option<String>)> = Vec::new();` (line 339), add:
```rust
    // Best-effort speaker diarization: None unless the feature is enabled + model present.
    let mut diarization = crate::diarization::commands::init_session(&app).await;
```

- [ ] **Step 2: Label each segment in the loop** — in the transcribe loop, change the non-empty-text push from `all_transcripts.push((text, segment.start_timestamp_ms, segment.end_timestamp_ms, None));` (the `None` from Task 2) to compute + push the label:
```rust
            let speaker = diarization
                .as_mut()
                .and_then(|s| s.label_segment(&segment.samples));
            all_transcripts.push((text, segment.start_timestamp_ms, segment.end_timestamp_ms, speaker));
```
(`segment.samples` are 16 kHz mono — no resampling. `label_segment` is best-effort and returns `None`/previous label on any embedding error.)

- [ ] **Step 3: Persist the speaker in the INSERT** — in the save transaction (the `for segment in &segments` loop, INSERT at line 444), add the `speaker` column + placeholder + bind:
```rust
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&segment.id)
        .bind(&meeting_id)
        .bind(&segment.text)
        .bind(&segment.timestamp)
        .bind(segment.audio_start_time)
        .bind(segment.audio_end_time)
        .bind(segment.duration)
        .bind(&segment.speaker)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow!("Failed to insert transcript: {}", e))?;
```

- [ ] **Step 4: Persist centroids after the commit** — after `tx.commit().await...?` succeeds (and near the existing `write_transcripts_json(&folder_path, &segments)` call), add:
```rust
    if let Some(session) = diarization.as_ref() {
        crate::diarization::commands::persist_speaker_centroids(session, Some(folder_path.clone())).await;
    }
```
(`folder_path: PathBuf` is in scope from the top of `run_retranscription`. Use `.clone()` if it's borrowed later; if `folder_path` is already consumed, capture it earlier into a local `let folder_for_speakers = folder_path.clone();` right after it's created and use that here.)

- [ ] **Step 5: Build + tests**
```bash
cd frontend/src-tauri
cargo build 2>&1 | tail -15
cargo test --lib audio::retranscription 2>&1 | tail -10
```
Expected: builds; the `create_transcript_segments` tests still pass. (The model-dependent labeling path is verified by manual E2E, not a unit test — same as the live splice.)

- [ ] **Step 6: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/audio/retranscription.rs
git commit -m "feat(speakers): :sparkles: label speakers when retranscribing an existing meeting"
```

---

## Task 4: Splice diarization into the Import loop

**Files:**
- Modify: `frontend/src-tauri/src/audio/import.rs`

**Interfaces:**
- Consumes: `crate::diarization::commands::{init_session, persist_speaker_centroids}` (Task 1); the 4-tuple `all_transcripts` + `create_transcript_segments` (Task 2).

- [ ] **Step 1: Initialize a diarization session before the transcribe loop** — in `run_import`, immediately after `let mut all_transcripts: Vec<(String, f64, f64, Option<String>)> = Vec::new();` (line 548), add:
```rust
    // Best-effort speaker diarization: None unless the feature is enabled + model present.
    let mut diarization = crate::diarization::commands::init_session(&app).await;
```

- [ ] **Step 2: Label each segment in the loop** — change the non-empty-text push (currently `all_transcripts.push((text, segment.start_timestamp_ms, segment.end_timestamp_ms, None));` from Task 2) to:
```rust
            let speaker = diarization
                .as_mut()
                .and_then(|s| s.label_segment(&segment.samples));
            all_transcripts.push((text, segment.start_timestamp_ms, segment.end_timestamp_ms, speaker));
```

- [ ] **Step 3: Persist the speaker in the INSERT** — the import save runs inside the transcript-insert loop (INSERT at line 722). Add `speaker` to that INSERT's columns + placeholder + bind:
```rust
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&segment.id)
        .bind(&meeting_id)
        .bind(&segment.text)
        .bind(&segment.timestamp)
        .bind(segment.audio_start_time)
        .bind(segment.audio_end_time)
        .bind(segment.duration)
        .bind(&segment.speaker)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow!("Failed to insert transcript: {}", e))?;
```
(If this INSERT lives inside a helper like `create_meeting_with_transcripts` that only receives `segments`, edit it there — the `segments` already carry `speaker` from Task 2/`create_transcript_segments`, so no signature change is needed, only the INSERT columns + bind.)

- [ ] **Step 4: Persist centroids after the meeting is saved** — after the import's save transaction commits (near the existing `write_transcripts_json(&meeting_folder, &segments)` call), add:
```rust
    if let Some(session) = diarization.as_ref() {
        crate::diarization::commands::persist_speaker_centroids(session, Some(meeting_folder.clone())).await;
    }
```
(`meeting_folder: PathBuf` is in scope from `run_import`. If it's been moved/consumed by the save call, capture `let folder_for_speakers = meeting_folder.clone();` right after `create_meeting_folder` and use that.)

- [ ] **Step 5: Build**
```bash
cd frontend/src-tauri
cargo build 2>&1 | tail -15
cargo test --lib audio::import 2>&1 | tail -10
```
Expected: builds; import's existing unit tests pass.

- [ ] **Step 6: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/audio/import.rs
git commit -m "feat(speakers): :sparkles: label speakers when importing an audio file"
```

---

## Manual verification (after all tasks)
With speaker ID enabled + the model downloaded (a full build via `frontend/build-gpu.sh`):
1. **Retranscribe** an existing meeting with 2–3 voices → segments come back with `Speaker N` chips; a voice with a saved profile is auto-labeled by name.
2. **Import** an audio file → same.
3. On a retranscribed meeting, **rename + "remember this voice"** works (it reads the `speakers.json` this feature now writes).
4. With the feature **disabled** (or model absent) → retranscribe/import work exactly as before, no chips, no errors.

## Self-Review
**Spec coverage:** §3 Part A (extract shared helpers + worker uses them) → Task 1. §3 Part B (label threading: create_transcript_segments widen + both loops label + both INSERTs + speakers.json) → Tasks 2/3/4. §3 Part C (best-effort/gating, no Arc<Mutex>) → Tasks 3/4 (`init_session` None-path + sequential `session.as_mut()`). §6 testing (create_transcript_segments tests + build gate + manual E2E) → Task 2 tests + build steps + Manual verification. §7 files → all covered. ✓
**Placeholder scan:** none — every step has exact code + commands. The `folder_path`/`meeting_folder` `.clone()` note is a concrete, conditional instruction (capture-early if consumed), not a placeholder.
**Type consistency:** `init_session(&AppHandle) -> Option<DiarizationSession>`, `persist_speaker_centroids(&DiarizationSession, Option<PathBuf>)`, `create_transcript_segments(&[(String,f64,f64,Option<String>)]) -> Vec<TranscriptSegment>`, and the `all_transcripts` 4-tuple are used identically across Tasks 1–4. `label_segment(&[f32]) -> Option<String>` matches the existing `DiarizationSession` API. Build-green ordering: Task 2 widens the seam with both call sites pushing `None` (compiles, no behavior change); Tasks 3/4 then replace `None` with the real label + add `speaker` to their INSERT — each task independently compiles and is reviewable.
