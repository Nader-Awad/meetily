# Retroactive Speaker Diarization (Retranscribe + Import) — Design

- **Date:** 2026-07-09
- **Status:** Approved (design); pending implementation plan.
- **Author:** Nader Awad (with Claude)
- **Scope:** Rust/Tauri core only. Extend speaker diarization to the two batch pipelines — **retranscription** (re-run an existing meeting's stored audio) and **import** (bring in an external audio file) — so past/imported meetings get speaker labels, not just live recordings. Personal local fork.
- **Motivation:** Diarization currently runs ONLY in the live transcription worker (`audio/transcription/worker.rs`). Meetings recorded before the feature existed, or retranscribed/imported, get transcripts with `speaker` NULL. The stored audio is available and both batch paths already decode → VAD-segment → transcribe it, so labeling those segments is a natural extension. (Verified 2026-07-09: `DiarizationSession` is referenced only in `worker.rs`; `retranscription.rs`/`import.rs` have no diarization wiring and their INSERTs omit `speaker`.)

## 1. Current state (baseline)

- **Live path** (`worker.rs`): per-segment `DiarizationSession::label_segment` → `speaker` on `TranscriptUpdate`. It has two private helpers: `init_diarization_session(app) -> Option<DiarizationSession>` (returns `None` unless the feature is enabled AND the embedding model is present; seeds saved voice profiles via `SpeakerProfilesRepository`) and `persist_speaker_centroids(session, folder)` (writes `speakers.json` in the meeting folder). Live must resample chunks to 16 kHz because they arrive at device rate.
- **Retranscription** (`audio/retranscription.rs`): `decode_audio_file` → `get_speech_chunks_with_progress` (VAD) → `speech_segments` whose `.samples` are already **16 kHz mono** → a sequential transcribe loop (`~342`) collecting `all_transcripts: Vec<(String, f64, f64)>` = (text, start_ms, end_ms) → `create_transcript_segments(&all_transcripts)` → a direct `INSERT INTO transcripts (…, audio_start_time, audio_end_time, duration)` at line **444** (no `speaker`) → `write_transcripts_json(&folder_path, &segments)`. Has `app: AppHandle<R>` + `meeting_folder_path`.
- **Import** (`audio/import.rs`): structurally identical — `decode_audio_file(_with_progress)` → same VAD `speech_segments` (16 kHz) → transcribe loop (`~528`) → same `create_transcript_segments(&all_transcripts)` (line 633) → direct INSERT at line **722** (no `speaker`) → `write_transcripts_json(&meeting_folder, …)`. Has `app: AppHandle<R>` + a created `meeting_folder`.
- **Shared seam:** `audio/common.rs::create_transcript_segments(transcripts: &[(String, f64, f64)]) -> Vec<TranscriptSegment>` is `pub(crate)`, used by BOTH batch paths (+ unit tests). `api::TranscriptSegment` already has `speaker: Option<String>` (added earlier); `create_transcript_segments` currently sets it to `None`.

## 2. Goals / non-goals

**Goals**
1. When speaker identification is enabled and its model is present, **retranscribing** an existing meeting labels each segment with a speaker (`Speaker N` or a matched saved-profile name), and applies saved voice profiles (incl. "Me").
2. Same for **importing** an audio file.
3. Persist per-meeting centroids to `speakers.json` for retranscribed/imported meetings, so the existing rename + "remember this voice" flow works on them.
4. DRY: one implementation of the session-init + centroid-persistence helpers, shared by live + retranscribe + import.
5. Best-effort/isolated: if the feature is disabled or the model is absent, retranscribe/import behave exactly as today (no labels, no errors).

**Non-goals**
- No new UI — it rides the existing Retranscribe/Import buttons.
- No overlap/concurrent-speaker resolution (still out of scope, as for live).
- No change to the live worker's behavior beyond calling the shared helpers instead of its private copies.
- No re-labeling of meetings that are neither retranscribed nor imported (there is no separate "diarize this old meeting" button; retranscribe IS that action).

## 3. Architecture / components

**Part A — Extract shared helpers (DRY refactor).**
Move the two private helpers out of `worker.rs` into the diarization module at `diarization/commands.rs` (which already imports `crate::state::AppState`, `SpeakerProfilesRepository`, `super::models`, and `tauri::Runtime`), as:
- `pub async fn init_session<R: Runtime>(app: &AppHandle<R>) -> Option<DiarizationSession>` — the current `init_diarization_session` logic verbatim (enabled-check via `commands::is_enabled`, model-present check, resolve model path, load saved profiles, `DiarizationSession::with_profiles`; `None` on any failure).
- `pub async fn persist_speaker_centroids(session: &DiarizationSession, folder: Option<std::path::PathBuf>)` — the current trimmed worker version verbatim (guards on `snapshot.is_empty()`, writes `speakers.json`).
Update `worker.rs` to call `crate::diarization::commands::{init_session, persist_speaker_centroids}` and delete its private copies. Behavior of the live path is unchanged.

**Part B — Thread the speaker label through the shared batch seam.**
- Widen the collected transcripts to carry the label: `all_transcripts: Vec<(String, f64, f64, Option<String>)>` = (text, start_ms, end_ms, speaker), in BOTH `retranscription.rs` and `import.rs`.
- Change `common.rs::create_transcript_segments` signature to `create_transcript_segments(transcripts: &[(String, f64, f64, Option<String>)]) -> Vec<TranscriptSegment>` and set `speaker: <the 4th element>` on each produced `TranscriptSegment` (currently hardcoded `None`).
- In each batch transcribe loop: before the loop, `let mut session = crate::diarization::commands::init_session(&app).await;`. Inside the loop, after a segment transcribes to non-empty text, compute `let speaker = session.as_mut().and_then(|s| s.label_segment(&segment.samples));` and push `(text, start_ms, end_ms, speaker)`. (`segment.samples` are already 16 kHz mono — no resampling.)
- Persist: add `speaker` to the column list + a placeholder + `.bind(&segment.speaker)` in BOTH INSERTs (`retranscription.rs:444`, `import.rs:722`), matching column order. After the save, call `crate::diarization::commands::persist_speaker_centroids(session_ref, Some(folder))` in both paths (folder = `folder_path` for retranscription, `meeting_folder` for import) so `speakers.json` is written for the rename/remember flow.

**Part C — Best-effort / gating.**
`init_session` returning `None` (disabled / model missing / error) means `session` is `None`; `session.as_mut().and_then(...)` yields `None`, so `speaker` stays `None` and the batch path is byte-for-byte as before. Labeling must never abort or slow the transcription meaningfully; `label_segment` already logs+degrades on embedding error. Batch is a single sequential loop, so no `Arc<Mutex>` (simpler than live).

## 4. Data flow (retranscribe or import)
decode audio → VAD `speech_segments` (16 kHz mono) → for each: transcribe `samples` (existing) + `session.label_segment(samples)` (new) → `all_transcripts` (now 4-tuples) → `create_transcript_segments` (carries speaker) → INSERT (persists speaker) + `write_transcripts_json` → `persist_speaker_centroids(session, folder)` → `speakers.json`. Downstream: the saved `transcripts.speaker` shows as chips (existing frontend), and feeds summaries/workflows (existing).

## 5. Error handling & degradation
Strictly best-effort, identical philosophy to the live splice: any diarization failure (disabled, model absent, embedding error, profile-load error) results in no label + a logged warning; transcription/import proceed and save normally. No `?`/`unwrap`/`expect`/`panic` on the diarization path. Cancellation (retranscription's `RETRANSCRIPTION_CANCELLED`) behavior is unchanged.

## 6. Testing strategy
- **Rust unit tests:** update `create_transcript_segments`'s existing tests (in `retranscription.rs` `#[cfg(test)]` and any in `common.rs`/`import.rs`) to the new 4-tuple input and assert the `speaker` propagates to the output `TranscriptSegment` (incl. a `None` case). This is the pure, testable core.
- **Compile/build gate** for the moved helpers + the two spliced loops (the model-dependent labeling isn't unit-testable without the ONNX model — same as the live splice).
- **Manual E2E:** with speaker ID enabled + model downloaded — (1) Retranscribe an existing meeting → segments come back with `Speaker N` chips; a returning voice with a saved profile is auto-labeled; (2) Import an audio file → same; (3) rename + "remember this voice" on a retranscribed meeting works (reads the written `speakers.json`).

## 7. Files touched
- Modify `frontend/src-tauri/src/diarization/commands.rs` — add `init_session` + `persist_speaker_centroids` (moved from worker).
- Modify `frontend/src-tauri/src/audio/transcription/worker.rs` — call the moved helpers; remove the private copies (live behavior unchanged).
- Modify `frontend/src-tauri/src/audio/common.rs` — `create_transcript_segments` signature + `speaker` propagation.
- Modify `frontend/src-tauri/src/audio/retranscription.rs` — session init + per-segment label + `all_transcripts` 4-tuple + INSERT `speaker` + `persist_speaker_centroids`; update its `create_transcript_segments` tests.
- Modify `frontend/src-tauri/src/audio/import.rs` — the same splice for the import loop + INSERT + `persist_speaker_centroids`.

## 8. Conventions
- Best-effort isolation: diarization never breaks retranscription/import.
- No new dependency; no migration (the `transcripts.speaker` column already exists).
- Hot-path note: the batch transcribe loops are not the per-audio-chunk hot path (they run per VAD speech segment, seconds apart, in a background batch job), so standard `info!/warn!`/`debug!` logging is fine — no `perf_debug!` requirement here.
- Gitmoji commits; no AI attribution. Personal fork; local `main` only.

## 9. Open items to resolve during planning
- Confirm the exact `SpeechSegment` field for samples (`segment.samples`) and start/end (`start_timestamp_ms`/`end_timestamp_ms`) as used in both loops.
- Confirm all call sites + tests of `create_transcript_segments` when its signature changes (retranscription tests, import tests, common.rs).
- Confirm whether `persist_speaker_centroids` needs the session by reference after the transcribe loop (session must remain in scope through save).
- Confirm `import.rs`'s meeting-folder variable name (`meeting_folder`) + that it's still in scope at persist time.
