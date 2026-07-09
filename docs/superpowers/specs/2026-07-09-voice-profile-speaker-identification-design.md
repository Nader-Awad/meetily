# Voice-Profile Speaker Identification — Design

- **Date:** 2026-07-09
- **Status:** Approved (design); pending implementation plan
- **Author:** Nader Awad (with Claude)
- **Scope:** Rust/Tauri core (`frontend/src-tauri/src`) + Next.js transcript UI. Local, on-device, privacy-preserving. No changes to the archived Python/FastAPI backend.
- **Provenance:** Adapts the diarization slice of upstream PR [Zackriya-Solutions/meetily#538](https://github.com/Zackriya-Solutions/meetily/pull/538) ("feat: local speaker identification with voice profiles", author rodrigopg). This is a **personal local fork** (see NeoHive memory `meetily-personal-fork`); the adaptation stays local and preserves in-code credit to the original author. We take only the diarization-specific code, trimmed.

## 1. Problem & motivation

Meetily transcribes meetings but does not attribute speech to speakers. The user wants **voice-profile–based speaker identification**: label who is talking at what time, recognize individual voices, and — using the same mechanism — recognize themselves ("Me") vs. other participants. Identification must be by **voice fingerprint**, not by audio source (mic vs. system), so a person is recognized regardless of channel and across meetings.

### What already exists (baseline)
- **Transcription pipeline:** `audio/transcription/worker.rs` emits `TranscriptUpdate { text, source, audio_start_time, audio_end_time, duration, … }`; `audio/pipeline.rs` currently feeds the worker the **mixed** mic+system mono stream (chunks tagged `DeviceType::Microphone // Mixed audio`). Whisper (`whisper_engine/`) + a Parakeet engine.
- **Vestigial `speaker` column:** migration `20251110000001_add_speaker_field.sql` added `transcripts.speaker TEXT` (intended "'mic'/'system'"), but it is **unused** — not in the `Transcript` model struct, not saved/read, not displayed. We repurpose it for the diarization label.
- **ONNX runtime already present:** the Parakeet engine already pulls in `ort`, `ndarray`, `realfft`, `thiserror` — so the diarization engine needs **zero new Rust dependencies**.
- **On-demand model download pattern:** `parakeet_engine` already downloads models (~tmp+rename atomic, progress events) into the app data dir — the diarization model reuses this pattern.
- **`audio/stt.rs` is dead legacy** (screenpipe/pyannote leftovers, imports `screenpipe_core`) — NOT the active path and NOT a foundation for this work.

## 2. Goals / non-goals

**Goals (v1)**
1. Automatically cluster distinct voices in a meeting into `Speaker 1 / Speaker 2 / …` labels, per speech segment, with timestamps (already present on segments).
2. Let the user **rename** a speaker and **"remember this voice"** → persist the voice embedding as a **profile** reused to auto-label that voice in future meetings.
3. Self-detection via the same mechanism: rename one's own cluster to "Me" + remember, once.
4. Display speaker labels as colored chips in the transcript (live + saved).
5. Feed speaker labels into the summary text sent to the LLM, so the built-in summary **and workflow runs** attribute who said what.
6. Off by default; enabling triggers a one-time model download.
7. Graceful degradation: diarization failure never affects transcription.

**Non-goals (v1 — explicit)**
- Concurrent-speaker / overlap resolution (the PR's `overlap_detector.rs`/timeline machinery — mostly unwired + a hot-path cost). Fast-follow.
- Auto-enrolling "Me" from the microphone channel. Fast-follow (needs a mic-only enrollment path atop the mixed pipeline).
- The unrelated features bundled into PR #538 (Apple Speech provider, axum local HTTP API, configurable summary prompts, floating recording indicator) — **excluded entirely**.
- Perfect accuracy: diarization runs on the mixed remote channel; single-speaker segments cluster well, overlaps are marked ambiguous/unlabeled. This is acceptable for v1.

## 3. Source strategy — adapt, don't reinvent

The evaluation of PR #538 confirmed its diarization core is a working, self-contained, zero-new-dependency slice, and that our fork has **not diverged** at the integration points (our `TranscriptUpdate` and `transcripts` INSERT match the PR's pre-image). So:

- **Obtain the code:** `git fetch upstream pull/538/head` (upstream remote = `Zackriya-Solutions/meetily`), then lift the diarization-scoped files from that ref (copy/hand-port), rather than merging the whole 8k-line branch.
- **Lift verbatim (new files, zero conflict):** `diarization/{fbank,embedding,clustering,session,models,commands,mod}.rs`, `database/repositories/speaker_profile.rs`, and the two diarization migrations (`add_diarization_settings`, `add_speaker_profiles`). Keep their unit tests.
- **Re-splice by hand (low–medium conflict):** `audio/transcription/worker.rs` (per-segment labeling hook), `database/repositories/transcript.rs` (persist `speaker`), `database/models.rs` (`Transcript` gains `speaker` + the fields the label path needs), `lib.rs` (`pub mod diarization;` + register the diarization commands). Ignore all non-diarization hunks in these files.
- **Do NOT take:** `overlap_detector.rs`, `timeline.rs`, the `add_overlap_diarization` migration, `apple_speech_engine/*`, `local_api.rs` (axum), the floating indicator, summary-prompt-settings. Drop the worker's per-chunk `detect_overlap_regions_from_timeline` call (the perf problem) entirely.

## 4. Architecture / components

New module `frontend/src-tauri/src/diarization/`:
- `fbank.rs` — Kaldi-compatible 80-dim log-mel filterbank (25 ms Povey window, 10 ms shift, CMN). Pure Rust via `realfft`. (~3 unit tests.)
- `embedding.rs` — `EmbeddingExtractor`: WeSpeaker **CAM++** ONNX model via `ort`, L2-normalized 192-dim embedding.
- `clustering.rs` — `SpeakerClusterer`: online cosine-similarity clustering (running-mean centroids; thresholds ≈0.55 intra-meeting, ≈0.60 profile-match; speaker cap ~10), assigns `Speaker N`. `seed_profile()` for pre-known voices. (~3 tests.)
- `session.rs` — `DiarizationSession`: per-meeting orchestration; `with_profiles()` seeds saved profiles at start; `label_segment(...)` returns a label for a segment's audio. (~5 tests.) **Trim** any dependence on the excluded timeline/overlap machinery — v1 labels per segment directly.
- `models.rs` — model metadata + on-demand download (~28 MB, GitHub release URL) into `<app_data>/models/diarization/`, mirroring `parakeet_engine`'s atomic download + progress events.
- `commands.rs` — Tauri commands: `diarization_rename_speaker(meeting_id, speaker_id, new_name, save_profile: bool)`, plus enable/status/model-download commands. Registered in `lib.rs`.
- `mod.rs` — module wiring + re-exports.

Each file has one responsibility and a clear interface; the engine (fbank/embedding/clustering) is decoupled from the worker integration so it can be unit-tested without audio I/O.

## 5. Data model

- **New table** `speaker_profiles(id TEXT PK, name TEXT, embedding BLOB, created_at TEXT, updated_at TEXT)` via a new timestamped migration `…_add_speaker_profiles.sql`. Repository: `database/repositories/speaker_profile.rs` (CRUD; embedding (de)serialization — the PR has a test for this).
- **New table** `diarization_settings(enabled INTEGER …)` (single-row) via `add_diarization_settings`, default disabled.
- **Repurpose** `transcripts.speaker TEXT` to hold the label (`"Speaker 2"`, `"Me"`, `"Alice"`). Add `speaker: Option<String>` to the `Transcript` model struct and to the INSERT/SELECT in `database/repositories/transcript.rs` (currently absent). The column's *meaning* is repurposed (label instead of the never-used "mic/system"); the already-applied `20251110000001` migration is left as-is (no schema change — same column, same type).
- Per-meeting cluster centroids persist to `speakers.json` in the meeting folder (as the PR does), so a meeting's clustering state survives reload; profiles (the durable, cross-meeting store) live in `speaker_profiles`.

## 6. Transcription worker integration + performance

In `audio/transcription/worker.rs`, after a segment is transcribed:
1. If diarization is enabled and the session initialized, pass the segment's 16 kHz mono audio to `DiarizationSession::label_segment(...)` to get a label.
2. Attach the label to the emitted `TranscriptUpdate` (new `speaker` field) and persist it to `transcripts.speaker`.
3. Session init (`init_diarization_session`) loads the model + re-seeds saved profiles via `with_profiles()`; returns `None` on any failure (disabled / model missing / DB error).

**Performance requirements (fix, don't port):**
- Run the ONNX embedding inside `tokio::task::spawn_blocking` (or the worker's blocking context) — do not block the async worker loop on `session.run()`.
- Do **not** recompute over the full growing timeline per chunk (the PR's inefficiency); label the current segment only.
- Respect the hot-path logging convention (`perf_debug!`/`perf_trace!`), never plain `log::debug!` per chunk.

## 7. Model management

WeSpeaker CAM++ ONNX (Apache-2.0), ~28 MB, downloaded on demand from a pinned release URL into `<app_data>/models/diarization/` with tmp-file + atomic rename and progress events, mirroring `parakeet_engine`. Enabling the setting (or first use) triggers download; a verification step (size/hash) guards integrity. **Open item:** confirm the exact model URL + checksum from the PR and pin them.

## 8. Frontend

- **Speaker chips:** colored label chips on transcript segments (live + saved views). Reconcile the PR's `SpeakerChip.tsx` + edits to the transcript views into our current transcript components (`MeetingDetails/TranscriptPanel.tsx`, the virtualized transcript view, transcript context).
- **Rename dialog:** `SpeakerRenameDialog.tsx` — rename a speaker + a "remember this voice" checkbox (calls `diarization_rename_speaker` with `save_profile`).
- **Settings:** `SpeakerIdentificationSettings.tsx` — enable/disable toggle (off by default); enabling shows model-download progress.
- Frontend has **no test runner** (pre-existing, `pnpm lint` broken repo-wide) → verify via `tsc --noEmit` + manual.

## 9. Summary / workflows integration

When building the transcript text sent to the LLM (existing summary path **and** the workflows `run_workflow` path), prefix each segment with its speaker label (e.g. `"[Alice] …"`), gated on diarization being enabled and labels present. This gives both the built-in summary and workflow runs speaker attribution. Keep it a small, localized change at the transcript-assembly point.

## 10. Self-detection UX (v1)

No special "Me" mechanism in v1: the user renames their own cluster to "Me" and checks "remember this voice" once. Thereafter their voice is auto-labeled "Me" across meetings via the profile store. (Auto-seeding "Me" from the mic channel is a documented fast-follow.)

## 11. Error handling & degradation

Diarization is strictly best-effort and isolated: any failure (setting disabled, model absent/download-failed, embedding error, DB error) results in **no label** and a logged warning — transcription and the rest of the app are unaffected. Session init returns `None`; per-segment labeling returns the previous/None label rather than erroring.

## 12. Testing strategy

- **Rust unit tests (cargo test):** port the PR's pure-logic tests for the lifted modules — fbank framing/CMN, cosine clustering assignment, profile embedding (de)serialization, session label selection. Require Xcode present (cidre) as with the rest of the crate; note the `binaries/llama-helper` placeholder requirement for the build.
- **Manual E2E (the model-dependent path):** enable the setting → model downloads → run a meeting with 2–3 distinct voices → confirm speakers cluster into stable labels → rename + remember one as "Me" → in a second meeting confirm "Me" is auto-recognized by voice → confirm the summary/workflow output shows speaker attribution.
- No frontend unit tests (no runner); UI verified by `tsc --noEmit` + manual.

## 13. Out of scope (v1) — recap
Overlap/concurrent-speaker resolution; mic-channel "Me" auto-enrollment; all non-diarization features bundled in PR #538.

## 14. Open items to resolve during planning
- Exact `worker.rs` splice: where in the segment-emit path to call labeling, and how to obtain the segment's 16 kHz mono samples there.
- Exact set of `Transcript`/`TranscriptSegment` fields to add (the PR added several; take only what the label path needs — likely just `speaker`, plus whatever the save/read requires).
- The pinned WeSpeaker CAM++ model URL + checksum.
- Frontend reconciliation: mapping the PR's transcript-view edits onto our current components.
- Whether `session.rs`/`clustering.rs` can be lifted cleanly once `timeline.rs`/`overlap_detector.rs` are excluded (trim any references).

## 15. Conventions
- All behavior via Tauri commands in the Rust core; register in `lib.rs` `generate_handler!`. `api_*`/existing command-naming style; snake_case Rust with serde camelCase at the TS boundary.
- SQLx migrations: new timestamped `.sql` files (after `20260703000001`), embedded via `sqlx::migrate!`.
- Hot-path logging via `perf_debug!`/`perf_trace!`.
- No hardcoded paths (Tauri path APIs / app data dir).
- Preserve original-author credit in the lifted files.
