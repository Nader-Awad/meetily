# Speaker Naming Redesign — Fix Greedy Re-attribution + Confidence-Gated Assignment

- **Date:** 2026-07-10
- **Status:** Approved (design); pending implementation plan.
- **Author:** Nader Awad (with Claude)
- **Scope:** The cluster→identity **naming/assignment layer** only. speakrs clustering, transcription, and the `speakers.json` schema are UNCHANGED. Two-release rollout: **v0.5.5 hotfix** (stop the greedy sweep + rename-dialog picklist) then **v0.5.6** (confidence-gated cross-meeting auto-assignment). Personal local fork.

## 1. Problem (diagnosed against v0.5.4 code)

`diarization_rename_speaker` calls `reattribute_matching_clusters` (`diarization/commands.rs`), which merges into the newly-named person **every other cluster whose centroid cosine ≥ `PROFILE_MATCH_THRESHOLD` (0.60)**. This is a **single-anchor, threshold-only** merge:
- No competition — it asks "is cluster X ≥ 0.60 to Alice?", never "is X closer to Alice or to someone else?". With one name entered there is no competitor, so every cluster above the bar collapses into that name ("it doesn't have others").
- 0.60 is far too permissive — distinct real speakers in one recording sit at ~0.5–0.7 cosine, so the bar catches other people.
- Corrections overcorrect — the merge is destructive (clusters collapse under one label, leaving duplicate same-name entries in `speakers.json`); renaming a mis-merged cluster re-runs the greedy sweep from the new centroid and oscillates.

**Confirmed baseline:** with speaker identification on, speakrs's per-meeting clustering (Speaker 1/2/3) is already good (user-confirmed); the defect is entirely in this naming/re-attribution step second-guessing a correct diarizer.

**Core principle for the redesign:** assign a cluster to a person only when that person is the *clear, confident winner*; otherwise leave it "Speaker N" (an honest "unknown — who is this?") and let the human decide. Corrections are always local — never a sweep.

## 2. Goals / non-goals

**Goals**
1. Renaming a speaker relabels ONLY that cluster — never auto-sweeps other clusters (kills the reported bug).
2. The rename dialog lets the user **pick an already-defined person** (saved voice profile, or a speaker already named in this meeting) in addition to typing a new name — making "these two clusters are the same person" an explicit, human-controlled action (the safe replacement for auto-merge).
3. Cross-meeting recognition (`map_local_speakers_to_profiles`) auto-labels a cluster with a known voice ONLY when that voice is the nearest match, above a high confidence threshold, AND beats the runner-up by a margin; otherwise the cluster stays "Speaker N" (unknown).
4. Naming a cluster enrolls/updates that voice profile ("training," accrued naturally) so future meetings recognize it; corrections update the specific cluster + its profile only.

**Non-goals**
- No change to speakrs clustering / segmentation / transcription (separation is already good).
- No separate "training mode" UI — naming a cluster IS enrollment; "unknown" IS "Speaker N".
- No automatic within-meeting cluster merging — merging is only via the explicit picklist (user picks an existing name).
- No new voice-profile embedding model or `speakers.json` schema change; CAM++/clusterer/profiles unchanged.
- Live recording path unchanged.

## 3. Part 1 — v0.5.5 hotfix

**3a. Backend — rename is local.** In `diarization/commands.rs::diarization_rename_speaker`, remove the call to `reattribute_matching_clusters` (and delete the now-dead greedy helpers `reattribute_matching_clusters`, `clusters_to_reattribute`, `load_all_centroids_from_folder`, plus their tests — v0.5.6 does NOT reuse the greedy version). Renaming keeps its existing behavior: `UPDATE transcripts SET speaker = new_name WHERE meeting_id = ? AND speaker = old_label`, relabel that one entry in `speakers.json`, and the optional `save_profile`. No other cluster is touched.

**3b. Frontend — rename-dialog picklist.** `SpeakerRenameDialog.tsx` currently shows only a free-text `name` box. Add a selectable list of **people already defined**, shown above the text box:
- Source = union of (a) saved voice profiles (fetched via the existing `diarization_list_profiles` command on dialog open) and (b) the names already assigned to *other* speakers in this meeting (passed in as a new prop `existingNames: string[]` from the parent that already knows the meeting's speaker labels). Deduplicate; exclude the current `speakerLabel` and any still-unnamed "Speaker N".
- Selecting an entry sets `name` to it (and the user confirms with Rename) — this relabels the current cluster to that existing name via the SAME `diarization_rename_speaker` call (a local, explicit merge with the same-named cluster). Typing a new name works exactly as today.
- If there are no defined people yet, the dialog looks/behaves as today (just the text box).

Shipping 3a + 3b together fully fixes both reported issues: no sweep, and deliberate consolidation via the picklist.

## 4. Part 2 — v0.5.6 confidence-gated auto-assignment

**4a. Competitive, margin-gated profile matching.** Replace `diarization/batch.rs::map_local_speakers_to_profiles`'s single-threshold (≥0.60 → name, else "Speaker N") logic with: for each cluster, compute the best and second-best saved-profile cosine; assign the best profile's name only if `best >= HIGH_MATCH_THRESHOLD` AND `best - second_best >= MATCH_MARGIN`; otherwise "Speaker N". New tuning constants (hard-coded, in `clustering.rs` or `batch.rs`): `HIGH_MATCH_THRESHOLD` (start ~0.72) and `MATCH_MARGIN` (start ~0.08) — tuned during implementation; do NOT reuse the permissive 0.60. This makes cross-meeting auto-labeling conservative: a clear known voice is labeled; anything uncertain or ambiguous stays "Speaker N" for the user.

**4b. Enrollment on naming (accrual).** Naming a cluster continues to save/update the voice profile (existing `save_profile` path). When the user picks an EXISTING profile from the picklist for a cluster, update that profile's stored centroid toward this meeting's cluster centroid (running-mean accrual, re-normalized) so recognition improves over time. (If accrual proves fiddly, v0.5.6 may ship match-only and defer accrual — flagged as an open item.)

**4c. Corrections stay local.** Because the hotfix already made rename local, a correction relabels only the clicked cluster (+ updates its profile). No sweep, so no overcorrection — this property is satisfied by Part 1 and preserved here.

**"Unknown — who is this?"** is represented by the existing "Speaker N" label plus the (now picklist-enabled) rename dialog; no dedicated UI is added. The behavioral guarantee is that the system leaves clusters as "Speaker N" whenever it is not confident, rather than guessing.

## 5. Data flow

Diarize (speakrs, unchanged) → clusters → **4a** auto-labels only clear-winner clusters, rest = "Speaker N" → transcript shows names + "Speaker N"s → user names a "Speaker N" via the dialog (**3b** picklist or free text) → **3a** relabels only that cluster + optional profile enroll/accrual (**4b**) → future meetings recognize enrolled voices via **4a**. No step sweeps other clusters.

## 6. Error handling & degradation

- Picklist best-effort: if `diarization_list_profiles` fails or returns none, the dialog falls back to free-text-only (no error). Missing `existingNames` prop → treated as empty.
- Rename itself is unchanged and must always succeed/fail exactly as before.
- 4a: if no profiles exist, every cluster is "Speaker N" (no auto-label) — the correct cold-start behavior. Any centroid/parse issue → treat as no-match ("Speaker N"), never a wrong guess. No `?`/`unwrap`/`panic` on the matching path.

## 7. Testing strategy

- **Rust unit (4a):** `map_local_speakers_to_profiles` with synthetic centroids — a clear winner (best high, margin big) → profile name; ambiguous (two profiles close) → "Speaker N"; best below `HIGH_MATCH_THRESHOLD` → "Speaker N"; no profiles → all "Speaker N". Include a case proving the OLD 0.60-only behavior would have mislabeled but the new gate does not.
- **Rust unit (4b, if shipped):** centroid accrual (running mean + renormalize) given an existing profile centroid + a new cluster centroid.
- **Hotfix backend (3a):** a test/assertion that `diarization_rename_speaker` no longer references the greedy re-attribution (the greedy fns are deleted); the existing rename behavior (single-cluster relabel) still works.
- **Frontend (3b):** `npx tsc --noEmit` clean (no new errors); manual — dialog lists saved profiles + this meeting's names, selecting one renames the cluster to it, typing a new name still works, empty state = text-box-only.
- **Manual E2E:** v0.5.5 — rename one speaker, confirm NO other speech changes; pick an existing name for a second cluster, confirm only that cluster merges. v0.5.6 — a saved voice is auto-labeled in a new meeting only when clearly that person; ambiguous voices stay "Speaker N"; correcting one does not disturb others.

## 8. Files touched

**v0.5.5**
- `frontend/src-tauri/src/diarization/commands.rs` — remove the `reattribute_matching_clusters` call + delete the greedy helpers/tests.
- `frontend/src/components/SpeakerRenameDialog.tsx` — add the picklist (fetch `diarization_list_profiles`; new `existingNames` prop; select-to-fill).
- The parent component that renders speaker chips / opens `SpeakerRenameDialog` — pass `existingNames` (the meeting's already-assigned speaker names).

**v0.5.6**
- `frontend/src-tauri/src/diarization/batch.rs` — competitive + margin-gated `map_local_speakers_to_profiles`; new threshold/margin consts.
- `frontend/src-tauri/src/diarization/commands.rs` (or `database/repositories/speaker_profile.rs`) — profile centroid accrual on naming/picking an existing profile (if shipped).

## 9. Open items to resolve during planning

- Confirm `diarization_list_profiles`'s exact return shape (fields for name/id) for the picklist.
- Confirm the parent component that opens `SpeakerRenameDialog` and how it enumerates the meeting's current speaker names (to supply `existingNames`).
- Tune `HIGH_MATCH_THRESHOLD` / `MATCH_MARGIN` starting values; decide where the consts live.
- Decide whether v0.5.6 includes centroid accrual (4b) or ships match-only first.
- Confirm whether deleting the greedy helpers leaves any other caller (grep) — they were added in v0.5.4 Task 7 and should have no other users.

## 10. Conventions

- Naming/assignment never guesses: clear-winner-or-"Speaker N".
- Corrections are always local; no cluster sweep.
- Keep speakrs/CAM++/clusterer/profiles/`speakers.json` schema unchanged; no migration.
- Gitmoji commits; no AI attribution. Personal fork; local `main` only. Ship v0.5.5 then v0.5.6 via `scripts/release.sh`; push `main` to the fork per the established flow.
