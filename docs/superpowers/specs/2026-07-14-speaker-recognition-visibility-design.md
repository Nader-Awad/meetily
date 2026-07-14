# Cross-Meeting Speaker Recognition: Visibility + Confirm-Suggestions — Design

- **Date:** 2026-07-14
- **Status:** Approved (design); pending implementation plan.
- **Author:** Nader Awad (with Claude)
- **Scope:** Rust/Tauri core + Next.js frontend. Ships as **v0.6.0**. Personal local fork.
- **Motivation:** Cross-meeting recognition works (retranscribe loads saved profiles and matches each cluster), but it is deliberately conservative — a cluster only auto-adopts a saved name at cosine ≥ `HIGH_MATCH_THRESHOLD` (0.72) with margin. The same voice across different recordings often lands ~0.62–0.71, so genuine matches stay "Speaker N" with no explanation, and there is no way to see which voices are even saved. Rather than loosen the gate (which would reintroduce confident mislabeling), give the user (a) visibility into saved voices and (b) a *confirmable* near-match suggestion so recognition happens with a click, never a silent guess.

## 1. Current state

- Saved voices live in the `speaker_profiles` table; commands `diarization_list_profiles` (→ `Vec<{id,name,embedding}>`), `diarization_rename_profile(id,name)`, `diarization_delete_profile(id)` exist. **No UI surfaces them** — `diarization_list_profiles` is only used by the rename-dialog picklist.
- Batch diarization (`audio/retranscription.rs`, `audio/import.rs`) → speakrs clusters → CAM++ per-cluster centroids → `diarization::batch::map_local_speakers_to_profiles(local_centroids, profiles)` returns a final label per local speaker: a saved name on a confident win, else `Speaker N`. Per-cluster centroids persist to `speakers.json` (`{version, speakers:[{label,centroid,segments}]}`) via `persist_labeled_centroids`.
- The rename dialog (`SpeakerRenameDialog.tsx`) supports free-text + a picklist of existing people, and confirms on a typed duplicate; renaming accrues into the matching profile (v0.5.6–v0.5.8).

## 2. Goals / non-goals

**Goals**
1. A **"Saved voices" view** (Settings → Transcription) listing saved profiles by name, each with **rename** and **delete**.
2. **Near-match suggestions:** at diarization time, a cluster that did NOT confidently match but is a clear near-match to one saved voice (in a suggestion band) records that suggestion; the transcript shows it as a subtle chip hint **"Speaker 2 · Alice?"**; clicking opens the rename dialog **pre-filled with the suggested name as an explicit pick**, so one confirm applies it (rename → accrue → sync). The label is never changed without confirmation.
3. Keep auto-labeling conservative — `HIGH_MATCH_THRESHOLD` (0.72) is UNCHANGED. Suggestions do not auto-apply.

**Non-goals**
- Do NOT loosen the confident-match threshold.
- No retroactive suggestions for already-diarized meetings (suggestions are computed at diarization time; older meetings simply show no hint).
- No DB migration — suggestions live in the per-meeting `speakers.json`.
- No change to the live recording path's labeling.

## 3. Part A — "Saved voices" view (frontend)

A new subsection in the Transcription settings (near `SpeakerIdentificationSettings`), e.g. a `SavedVoicesSettings.tsx`:
- On mount, `invoke('diarization_list_profiles')` → list rows (name; optionally a small note). Empty state: "No saved voices yet — name a speaker and keep 'Remember this voice' on."
- Each row: **Rename** (inline edit → `diarization_rename_profile(id, name)`) and **Delete** (confirm → `diarization_delete_profile(id)`), then refetch.
- Best-effort: command failures show a toast; no crash. No backend change (all three commands exist).

## 4. Part B — near-match suggestions

**4a. Compute suggestions (`diarization/batch.rs`).** Add a pure companion to the mapping:
`pub fn suggest_near_matches(local_centroids: &[(String, Vec<f32>)], profiles: &[(String, Vec<f32>)], name_map: &HashMap<String, String>) -> HashMap<String, (String, f32)>`
— for each local speaker whose `name_map` value is a `Speaker N` label (i.e. it did NOT confidently match), compute best + runner-up profile cosine; if `SUGGEST_FLOOR ≤ best < HIGH_MATCH_THRESHOLD` AND `(best - runner_up) ≥ MATCH_MARGIN` (clear top candidate), record `final_label → (profile_name, best)`. New const `pub const SUGGEST_FLOOR: f32 = 0.62;`. Pure + unit-tested.

**4b. Persist suggestions.** Extend the batch persistence so a `speakers.json` entry may carry an optional `suggested`: `{label, centroid, segments, suggested?: {name, score}}`. `persist_labeled_centroids` gains an optional per-label suggestion map (or a sibling that accepts it); entries without a suggestion omit the field. `load_centroid_from_folder` / `relabel_and_merge_centroids` ignore the extra field (forward-compatible).

**4c. Expose suggestions.** New read-only command `diarization_get_suggestions(meeting_id) -> HashMap<String, SuggestionDto>` where `SuggestionDto { name: String, score: f32 }`, keyed by speaker label — reads the meeting's `speakers.json` and returns entries that have `suggested`. Best-effort (missing file → empty map).

**4d. Surface in the transcript (frontend).** Where the meeting's transcript loads (`TranscriptPanel` / the meeting-details view), fetch `diarization_get_suggestions(meetingId)` once and pass a `suggestions: Record<label, {name, score}>` map down to the speaker chip (`TranscriptView.tsx` / `VirtualizedTranscriptView.tsx`). For a chip whose label is an unnamed `Speaker N` AND has a suggestion, render a subtle hint appended to the chip — `Speaker 2 · Alice?` (muted styling). Clicking the chip opens `SpeakerRenameDialog` for that label **with the suggested name pre-filled and marked as an explicit pick** (so v0.5.7's typed-duplicate confirm does NOT fire — the user is confirming a known voice). Confirm runs the normal rename → accrue → `speakers.json` relabel.

**4e. Interaction detail.** `SpeakerRenameDialog` gains an optional `suggestedName?: string` prop; when set, it initializes `name = suggestedName` and `pickedFromList = true` (treated as selecting an existing person) and shows a one-line note "Suggested from a saved voice." Everything else unchanged.

## 5. Data flow

Diarize (batch) → clusters → `map_local_speakers_to_profiles` (final labels, unchanged) + `suggest_near_matches` (suggestions for the `Speaker N` clusters) → persist `speakers.json` with optional `suggested` per entry → transcript view fetches `diarization_get_suggestions` → chip shows `Speaker N · Name?` → click → rename dialog pre-filled (explicit pick) → confirm → existing rename (accrue + relabel speakers.json).

## 6. Thresholds / parameters

- `HIGH_MATCH_THRESHOLD = 0.72` (unchanged; confident auto-label).
- `SUGGEST_FLOOR = 0.62` (new; below this, no suggestion — too weak to bother).
- `MATCH_MARGIN = 0.08` (reused; a suggestion still requires a clear top candidate).
- All hard-coded (no settings UI), tunable in code.

## 7. Error handling & degradation

- All new paths best-effort: no `?`/`unwrap`/`expect`/panic on the suggestion/persistence/command paths. Missing/failed `speakers.json` or command → no suggestions (chips just show `Speaker N`), never an error.
- Saved-voices view command failures → toast; the rest of settings unaffected.
- Older meetings (no `suggested` in `speakers.json`) → no hint, exactly as today.

## 8. Testing

- **Rust unit (`suggest_near_matches`):** near-match in band → suggestion; best ≥ 0.72 (confident) → no suggestion (already labeled); best < floor → none; ambiguous (two profiles within margin) → none; only suggests for `Speaker N` locals.
- **Rust unit (persistence):** an entry with a suggestion serializes `suggested:{name,score}`; round-trips; `relabel_and_merge_centroids`/`load_centroid_from_folder` still parse entries that carry the extra field.
- **Build/command gate** for `diarization_get_suggestions`.
- **Frontend:** `npx tsc --noEmit` clean; manual — Saved-voices view lists/rename/delete; a retranscribe with a near-match voice shows `Speaker N · Name?`, clicking pre-fills the dialog (no duplicate-confirm nag), confirming applies + reinforces; older meetings show no hint.

## 9. Files touched (anticipated)

- Create `frontend/src/components/SavedVoicesSettings.tsx` + mount it in the Transcription settings (near `SpeakerIdentificationSettings`).
- Modify `frontend/src-tauri/src/diarization/batch.rs` — `suggest_near_matches` + `SUGGEST_FLOOR` + tests.
- Modify `frontend/src-tauri/src/diarization/commands.rs` — `diarization_get_suggestions` command; extend the persistence to write optional `suggested`; register the command in the Tauri handler.
- Modify `frontend/src-tauri/src/audio/retranscription.rs` and `audio/import.rs` — call `suggest_near_matches` and pass suggestions into persistence.
- Modify `frontend/src-tauri/src/lib.rs` — register `diarization_get_suggestions` in `generate_handler!`.
- Modify `frontend/src/components/SpeakerRenameDialog.tsx` — `suggestedName` prop (pre-fill + explicit-pick + note).
- Modify the transcript view + chip (`TranscriptView.tsx`, `VirtualizedTranscriptView.tsx`, `MeetingDetails/TranscriptPanel.tsx`) — fetch suggestions + render the chip hint + open the dialog with `suggestedName`.

## 10. Open items for planning

- Confirm the exact persistence seam (extend `persist_labeled_centroids` signature vs a sibling that takes a `HashMap<label,(name,score)>`) and how retranscription/import pass the suggestion map.
- Confirm how the transcript view fetches per-meeting data (so the suggestions fetch fits the existing load) and how the chip components receive props.
- Confirm the Transcription settings mount point for the Saved-voices view.

## 11. Conventions

- Auto-labeling stays conservative; suggestions require an explicit confirm (never a silent guess).
- Best-effort everywhere; no migration; `speakers.json` `suggested` field is additive/forward-compatible.
- Gitmoji commits; no AI attribution. Personal fork; local `main` only. Ship v0.6.0 via `scripts/release.sh`; push `main` to the fork.
