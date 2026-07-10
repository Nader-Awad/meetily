# Neural Speaker-Turn Segmentation for Batch Diarization — Design

- **Date:** 2026-07-10
- **Status:** Approved (design, revised for the speakrs-sidecar approach); pending implementation plan.
- **Author:** Nader Awad (with Claude)
- **Scope:** Rust/Tauri core + a new sidecar crate, **batch paths only** (Retranscribe + Import). The live recording path is unchanged. Personal local fork.
- **Motivation:** Current diarization computes **one speaker embedding per VAD speech segment** and assigns it one label (`diarization/session.rs:60`). VAD is tuned to bridge pauses up to **2 s** (`audio/vad.rs:49`) with no max segment length, so a normal back-and-forth puts two speakers inside one segment; its embedding is an average of both voices and both get one label — the reported *"two speakers roped into one."* No embedding-model or threshold change fixes a blended segment; we need real **speaker-turn detection**.

## 1. Approach decisions (how we got here)

1. **Cheap levers rejected as the fix.** Raising the clustering threshold and swapping the embedding model help distinctness between clean segments but cannot split a segment that already blends two voices. (Kept only as a possible future fallback.)
2. **Neural segmentation chosen.** The pyannote 3.0 segmentation model detects speaker turns. Hand-rolling the powerset decode is high-risk (and `pyannote-rs` only decodes speech/non-speech, so it is not a usable reference).
3. **`speakrs` chosen as the engine.** The `speakrs` crate (v0.5.0, Apache-2.0) implements the full pyannote pipeline (segmentation → powerset decode → overlap-add → binarization → embedding → PLDA → VBx clustering) and returns speaker-labeled turns. API: `OwnedDiarizationPipeline::from_pretrained(ExecutionMode::CoreMl)` → `run(&mut self, &[f32]) -> DiarizationResult` (input **mono 16 kHz f32**) → `result.discrete_diarization.to_segments() -> Vec<Segment{ start: f32 sec, end: f32 sec, speaker: String }>`. Models download on first use (`online` feature, HF `avencera/speakrs-models`) or load from `SPEAKRS_MODELS_DIR`.
4. **Sidecar isolation chosen (not in-process).** `speakrs` requires `ort ^2.0.0-rc.12`, `ndarray ^0.17`, and a BLAS backend; the main app pins `ort 2.0.0-rc.10` + `ndarray 0.16` and its Parakeet + CAM++ transcription depends on them. To avoid destabilizing working transcription, `speakrs` lives in a **separate `diarize-helper` sidecar crate** (mirroring the existing `llama-helper` sidecar), with its own `ort`/BLAS, invoked as a subprocess. The main app's `ort`/`ndarray`/transcription are untouched.
5. **CAM++ kept for identity.** `speakrs` yields per-file speaker labels (e.g. `SPEAKER_00`). Cross-meeting identity (the "Me"/named voice-profile feature) stays in the main app: CAM++ embeds each turn, and turn-speakers are mapped to saved profiles. Existing profiles + `speakers.json` remain valid (no embedding-space change to profiles).

## 2. Goals / non-goals

**Goals**
1. On Retranscribe/Import, when speaker identification is enabled and the sidecar + its models are available, detect speaker **turns** with speakrs and label each transcription unit by turn, so a two-person exchange no longer collapses into one speaker.
2. Keep CAM++, the `SpeakerClusterer`, saved voice profiles, and `speakers.json` — existing profiles stay valid.
3. Add human-anchored **cluster-level re-attribution** on rename: naming a speaker re-checks the meeting's other cluster centroids and merges those within threshold into the named speaker.
4. Best-effort/isolated: if the sidecar is missing/fails, the feature is disabled, or models are absent, fall back to today's v0.5.3 behavior with no errors and no broken transcription. The main app never depends on the sidecar building.

**Non-goals**
- No change to the **live** recording path.
- No cloud/API diarization.
- No per-word-timestamp text splitting (engines return plain text; we make transcription units single-speaker instead).
- No new settings UI; hard-coded defaults.
- No overlapping-speech attribution in the transcript (dominant speaker per unit; overlaps a later enhancement).
- No per-segment embedding persistence — re-attribution is **cluster-level** using `speakers.json` centroids.
- No `ort`/`ndarray`/BLAS change in the main app (that is the whole point of the sidecar).

## 3. Architecture — speakrs sidecar + main-app alignment (batch only)

**Component A — `diarize-helper` sidecar (new workspace crate).** A standalone binary depending on `speakrs = { version = "0.5", features = ["coreml", <blas>] }`, mirroring `llama-helper`. One-shot CLI: reads a mono-16 kHz WAV path + a models directory, runs the speakrs pipeline, and prints JSON `[{ "start_ms": u64, "end_ms": u64, "speaker": String }, …]` to stdout, non-zero exit + stderr message on failure. Its `ort`/BLAS are fully private to this crate. Built + copied to `src-tauri/binaries/diarize-helper-<target-triple>` by `build-gpu.sh` and declared as a Tauri `externalBin`, exactly like `llama-helper`.

**Component B — Sidecar client (new, `diarization/segmenter.rs`).** In the main app: given the decoded 16 kHz mono samples, write a temp WAV, resolve + spawn the bundled `diarize-helper` sidecar (mirroring how `llama-helper` is resolved/spawned), read stdout, parse JSON into `Vec<DiarTurn { start_ms: u64, end_ms: u64, speaker: String }>`. Returns `Option<Vec<DiarTurn>>` — `None` on any failure (sidecar missing, non-zero exit, parse error), so the caller degrades gracefully. Cleans up the temp WAV.

**Component C — Turn→unit + CAM++ identity (new orchestrator, `diarization/batch.rs`).** Given the turns and the decoded audio: merge adjacent same-speaker turns, drop/merge sub-minimum-duration turns, and for each resulting unit slice the audio and (a) hand it to transcription and (b) embed it with **CAM++** to accumulate a per-turn-speaker centroid. Map each speakrs speaker to a saved profile name via CAM++ centroid cosine-match (reusing `clustering::cosine_similarity` + `PROFILE_MATCH_THRESHOLD`), else `Speaker N`. Persist the per-speaker CAM++ centroids to `speakers.json` (existing schema) so rename/"remember voice" works.

**Component D — Batch splices (`audio/retranscription.rs`, `audio/import.rs`).** When the sidecar path is available, build the transcription units from turns (Component C) instead of raw VAD segments; transcribe each unit with the existing engine → `(text, start_ms, end_ms, speaker)`; INSERT (`speaker` already the 8th column) + `write_transcripts_json` + persist centroids. When unavailable, unchanged v0.5.3 VAD path. Long single-speaker units still get the existing `>25 s` `split_segment_at_silence` for transcription quality (all sub-pieces keep that unit's speaker).

**Component E — Cluster-level re-attribution (`diarization/commands.rs::diarization_rename_speaker`, ~line 87).** When the user names `Speaker N → Alice`, load the meeting's cluster centroids from `speakers.json`, cosine-compare every other cluster centroid to Alice's, and relabel any at/above the merge threshold to Alice (rewriting the affected `transcripts.speaker` rows + `speakers.json`). Uses `clustering::cosine_similarity`; no per-segment embeddings.

## 4. Data flow (Retranscribe or Import, enabled + sidecar available)

decode audio (16 kHz mono, already produced at `retranscription.rs:202` / `import.rs:385`) → **sidecar** (Components A/B) → turns → merge/min-duration (Component C) → per-unit: transcribe (existing engine) + CAM++ embed → `all_transcripts: Vec<(text, start_ms, end_ms, speaker)>` → map speakers→profiles → `create_transcript_segments` → INSERT (`speaker`) → `write_transcripts_json` → `persist_speaker_centroids` → `speakers.json`. On rename: **re-attribution** (Component E) consolidates over-split clusters.

## 5. Error handling & degradation

Strictly best-effort. If the sidecar binary is absent, exits non-zero, times out, or its output fails to parse — or diarization is disabled — the batch path falls back to the **v0.5.3 behavior** (VAD segments → CAM++ per-segment label, or no labels): never an error, never broken transcription. No `?`/`unwrap`/`expect`/`panic` on the sidecar/diarization path in the main app. The sidecar itself returns a non-zero exit + stderr on any internal failure (including model-download failure). Diarization is a background batch step, so `info!/warn!/debug!` logging is fine.

## 6. Profile & data compatibility

CAM++, the `SpeakerClusterer`, the `speaker_profiles` table, and the `speakers.json` schema (`{version, speakers:[{label,centroid,segments}]}`) are unchanged → **existing saved profiles and prior `speakers.json` remain valid**. No DB migration. The main app's `ort`/`ndarray`/BLAS are unchanged.

## 7. Testing strategy

- **Sidecar unit tests (`diarize-helper`):** JSON output serialization shape; that a synthetic/short WAV runs the pipeline and emits well-formed JSON (a real model run is a smoke test, not CI, since it needs the downloaded models — same rationale as the CAM++ path).
- **Client unit tests (`diarization/segmenter.rs`):** parse a sample sidecar JSON string into `Vec<DiarTurn>`; malformed/empty JSON → `None`; non-zero exit → `None`.
- **Orchestrator unit tests (`diarization/batch.rs`):** turn merging (adjacent same-speaker), min-duration drop/merge, and speaker→profile mapping given synthetic centroids (match vs no-match, incl. the `Speaker N` fallback).
- **Re-attribution unit tests (Component E):** given cluster centroids + a named centroid, assert clusters within threshold relabel and others don't (incl. a no-op case).
- **Build/smoke gate:** the sidecar builds standalone and diarizes a sample WAV into sensible turns; the main app spawns it and parses the result.
- **Manual E2E:** feature enabled + models present — Retranscribe the reported problem meeting (2–3 conversational voices) → speakers separate; name one + "remember this voice" → re-attribution merges duplicates; feature-off / sidecar-absent → behaves exactly as v0.5.3.

## 8. Files touched (anticipated)

- Create `diarize-helper/` (workspace crate: `Cargo.toml`, `src/main.rs`) — speakrs one-shot CLI.
- Modify root `Cargo.toml` — add `diarize-helper` to `members`.
- Modify `frontend/build-gpu.sh` — build the sidecar (release) + copy to `frontend/src-tauri/binaries/diarize-helper-<triple>` (mirror the llama-helper steps).
- Modify `frontend/src-tauri/tauri.conf.json` — add the sidecar to `bundle.externalBin`.
- Modify `frontend/src-tauri/capabilities/*` — allow spawning the sidecar (mirror llama-helper's shell permission).
- Create `frontend/src-tauri/src/diarization/segmenter.rs` — temp-WAV + sidecar spawn + JSON parse → `Vec<DiarTurn>` (Component B).
- Create `frontend/src-tauri/src/diarization/batch.rs` — turn→unit + CAM++ speaker→profile mapping + centroid persistence (Component C).
- Modify `frontend/src-tauri/src/diarization/mod.rs` — export the new modules + `DiarTurn`.
- Modify `frontend/src-tauri/src/diarization/commands.rs` — cluster-level re-attribution in `diarization_rename_speaker` (Component E).
- Modify `frontend/src-tauri/src/audio/retranscription.rs` (~172–494) and `frontend/src-tauri/src/audio/import.rs` (~311–666, `create_meeting_with_transcripts` ~700) — turn-based transcription units when the sidecar path is available; unchanged fallback otherwise.
- Modify `frontend/src-tauri/src/audio/common.rs` if a helper for slicing audio by time range is shared.
- Frontend (minimal) — surface a "preparing speaker models" state on first diarized batch run (sidecar downloads speakrs models on first use); no new settings screen.

## 9. Open items to resolve during planning

- **Sidecar go/no-go spike:** confirm `speakrs` 0.5.0 builds standalone on this machine — resolve the BLAS backend (`openblas-static` needs a Fortran toolchain; `openblas-system` needs `brew install openblas` + env) and confirm CoreML works — before building anything on top. If it can't build, stop and reconsider (cheap-levers fallback).
- **speakrs model provisioning:** rely on the `online` feature (first-run HF download) with `SPEAKRS_MODELS_DIR` pointed at `<app_data>/models/diarization/speakrs`, or pre-download; decide progress/UX for the first run.
- **Sidecar IPC shape:** one-shot CLI (WAV path + models dir → JSON on stdout) vs the llama-helper-style persistent stdin/stdout loop. Default: one-shot (batch job).
- **Decode/merge parameters:** min transcription-unit duration, same-speaker merge gap, and the speaker→profile / re-attribution cosine thresholds (reuse/nudge `PROFILE_MATCH_THRESHOLD`).
- **Sidecar path resolution + spawn** in the main app — mirror exactly how `llama-helper` is resolved/spawned and how its shell permission/`externalBin` are declared.
- **Timeout** for the sidecar call (diarization of a long meeting can take a while even at 300–900× realtime).

## 10. Conventions

- Best-effort isolation: diarization never breaks Retranscribe/Import; the sidecar never touches the main app's `ort`/`ndarray`/BLAS.
- Keep CAM++, the clusterer, profiles, and `speakers.json` unchanged (compatibility).
- speakrs + its `ort`/BLAS are confined to the `diarize-helper` crate; Apache-2.0.
- Gitmoji commits; no AI attribution. Personal fork; local `main` only. Ship via `scripts/release.sh` (next: v0.5.4) after merge; push `main` to the fork per the established flow.
