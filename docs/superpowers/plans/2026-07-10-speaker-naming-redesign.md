# Speaker Naming Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the greedy speaker re-attribution (naming one speaker sweeps others) and make cross-meeting recognition confidence-gated, with a rename-dialog picklist of already-defined people.

**Architecture:** Two-release rollout on the cluster→identity naming layer only (speakrs clustering, transcription, `speakers.json` schema all unchanged). **v0.5.5:** renaming becomes local (delete the greedy sweep) + the rename dialog offers a picklist of existing people. **v0.5.6:** `map_local_speakers_to_profiles` auto-labels a cluster with a known voice only when it is the clear winner (nearest + high threshold + margin), else "Speaker N".

**Tech Stack:** Rust (Tauri v2 core), Next.js/React + TypeScript frontend, sqlx/SQLite, existing CAM++ diarization.

## Global Constraints

- **Naming never guesses:** a cluster gets a person's name only when that person is the clear winner; otherwise it stays "Speaker N" (the "unknown — who is this?" state).
- **Corrections are always local** — renaming/relabeling touches only the clicked cluster; NEVER sweep other clusters.
- speakrs clustering, transcription, CAM++, the `SpeakerClusterer`, the `speaker_profiles` table, and the `speakers.json` schema are UNCHANGED. No DB migration. The main app's `ort`/`ndarray` are unchanged.
- Keep `load_centroid_from_folder` (the SINGULAR helper, used by `save_profile`). Only the v0.5.4 Task-7 greedy helpers are deleted.
- Run main-app cargo from `frontend/src-tauri`; frontend checks from `frontend`. Pre-existing failing tests `audio::device_detection::{test_builtin_mic_detection, test_calculate_buffer_timeout_bluetooth}` are NOT yours. `pnpm lint`/`next lint` is broken repo-wide — gate the frontend on `npx tsc --noEmit` (a pre-existing `bun:test` tsc error is not yours).
- Commits: gitmoji conventional; **NO `Co-Authored-By`, NO AI/agent mention**. Local `main` only; do NOT push during implementation.

---

## Release v0.5.5 (hotfix)

### Task 1: Remove the greedy re-attribution (rename becomes local)

**Files:**
- Modify: `frontend/src-tauri/src/diarization/commands.rs`

**Interfaces:**
- Produces: `diarization_rename_speaker` that relabels ONLY the clicked cluster (existing single-cluster UPDATE + `speakers.json` relabel + optional `save_profile`), with the greedy sweep gone. No signature change.

- [ ] **Step 1: Delete the call to the greedy sweep.** In `diarization_rename_speaker`, remove the line (currently ~301):
```rust
        reattribute_matching_clusters(pool, &meeting_id, folder, new_name, &old_label).await;
```
(Remove the whole statement + any now-empty enclosing `if` it sits in. Leave the primary rename — the `UPDATE transcripts ... WHERE meeting_id = ? AND speaker = ?`, the `speakers.json` single-label relabel, and the `save_profile` block — exactly as-is.)

- [ ] **Step 2: Delete the now-dead greedy helpers + their tests.** Remove entirely from `commands.rs`:
  - `async fn reattribute_matching_clusters(...)` (~line 142)
  - `pub(crate) fn clusters_to_reattribute(...)` (~line 124)
  - `fn load_all_centroids_from_folder(...)` (~line 87)
  - the `#[cfg(test)] mod` re-attribution tests that call `clusters_to_reattribute` (the `selects_only_matching_clusters` / `empty_when_none_match` tests, ~lines 420-435)

  **Do NOT delete `load_centroid_from_folder` (the singular one, ~line 61)** — it is still used by the `save_profile` path.

- [ ] **Step 3: Confirm no dangling references.**
```bash
cd frontend/src-tauri && grep -rn "reattribute_matching_clusters\|clusters_to_reattribute\|load_all_centroids_from_folder" src/
```
Expected: NO matches (all gone).

- [ ] **Step 4: Build + test.**
```bash
cd frontend/src-tauri
cargo build 2>&1 | tail -10
cargo test --lib diarization 2>&1 | tail -10
```
Expected: builds cleanly (no `unused` warnings for the deleted fns); diarization tests pass (minus the 2 deleted re-attribution tests).

- [ ] **Step 5: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/commands.rs
git commit -m "fix(speakers): :bug: rename a speaker without sweeping other clusters"
```

---

### Task 2: Rename-dialog picklist of already-defined people

**Files:**
- Modify: `frontend/src/components/SpeakerRenameDialog.tsx`
- Modify: `frontend/src/components/MeetingDetails/TranscriptPanel.tsx`

**Interfaces:**
- Consumes: the existing `diarization_list_profiles` command → returns `Array<{ id: string; name: string; embedding: number[] }>` (only `name` is used here).
- Produces: `SpeakerRenameDialog` accepts a new optional prop `existingNames?: string[]`; renders a selectable list of defined people (saved-profile names ∪ `existingNames`), excluding the current `speakerLabel` and unnamed `Speaker N` placeholders; selecting one fills the name field.

- [ ] **Step 1: Add the `existingNames` prop + profile fetch + picklist to `SpeakerRenameDialog.tsx`.** Update the props interface and component:
```tsx
interface SpeakerRenameDialogProps {
  meetingId: string;
  speakerLabel: string;
  existingNames?: string[];
  onClose: () => void;
  onRenamed: () => void | Promise<void>;
}
```
Add, inside the component (after the existing `useState`s), a fetch of saved-profile names and a computed, de-duplicated candidate list:
```tsx
  const [profileNames, setProfileNames] = useState<string[]>([]);
  useEffect(() => {
    (async () => {
      try {
        const profiles = await invoke<Array<{ name: string }>>('diarization_list_profiles');
        setProfileNames(profiles.map((p) => p.name));
      } catch {
        setProfileNames([]); // best-effort: fall back to text-only
      }
    })();
  }, []);

  const isPlaceholder = (n: string) => /^Speaker \d+$/.test(n.trim());
  const candidates = Array.from(
    new Set([...(existingNames ?? []), ...profileNames].map((n) => n.trim()))
  )
    .filter((n) => n.length > 0 && !isPlaceholder(n) && n !== speakerLabel)
    .sort((a, b) => a.localeCompare(b));
```
(Add `useEffect` to the existing `import { useState } from 'react';` → `import { useState, useEffect } from 'react';`.)

- [ ] **Step 2: Render the picklist above the name input** (inside the `<div className="space-y-3 py-2">`, before the Name `<div>`):
```tsx
          {candidates.length > 0 && (
            <div>
              <Label className="text-sm">Assign to someone already added</Label>
              <div className="flex flex-wrap gap-1.5 mt-1">
                {candidates.map((c) => (
                  <button
                    key={c}
                    type="button"
                    onClick={() => setName(c)}
                    className={`px-2 py-1 rounded text-xs border ${
                      name.trim() === c
                        ? 'bg-blue-600 text-white border-blue-600'
                        : 'bg-white text-gray-700 border-gray-300 hover:bg-gray-50'
                    }`}
                  >
                    {c}
                  </button>
                ))}
              </div>
              <p className="text-xs text-gray-500 mt-1">…or type a new name below.</p>
            </div>
          )}
```
(The existing Name input + Rename button already read `name`, so selecting a chip fills the field and the user confirms with Rename — no other change to `handleRename`.)

- [ ] **Step 2b: Verify the dialog type-checks.**
```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -15
```
Expected: no NEW errors (the pre-existing `bun:test` error is unrelated).

- [ ] **Step 3: Pass `existingNames` from `TranscriptPanel.tsx`.** Derive the meeting's already-assigned real speaker names from the segments and pass them to the dialog. Add, near the `convertedSegments` memo:
```tsx
  const existingSpeakerNames = useMemo(
    () =>
      Array.from(
        new Set(
          convertedSegments
            .map((s) => (s.speaker ?? '').trim())
            .filter((n) => n.length > 0 && !/^Speaker \d+$/.test(n))
        )
      ),
    [convertedSegments]
  );
```
Then pass it on the dialog (existing block ~line 119):
```tsx
        <SpeakerRenameDialog
          meetingId={meetingId}
          speakerLabel={renameSpeaker}
          existingNames={existingSpeakerNames}
          onClose={() => setRenameSpeaker(null)}
          onRenamed={async () => {
            setRenameSpeaker(null);
            await onRefetchTranscripts?.();
          }}
        />
```
(Ensure `useMemo` is imported in `TranscriptPanel.tsx`; it already uses `useMemo` for `convertedSegments`, so no import change.)

- [ ] **Step 4: Type-check the whole frontend.**
```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -15
```
Expected: no new errors.

- [ ] **Step 5: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src/components/SpeakerRenameDialog.tsx frontend/src/components/MeetingDetails/TranscriptPanel.tsx
git commit -m "feat(speakers): :sparkles: pick an already-added person when naming a speaker"
```

> After Task 2: this is the v0.5.5 cut point. The bug is fixed and the picklist is in. (Release choreography — merge to main, bump v0.5.5, `release.sh`, push — is handled by the controller after review, not inside these tasks.)

---

## Release v0.5.6

### Task 3: Confidence-gated, competitive profile matching

**Files:**
- Modify: `frontend/src-tauri/src/diarization/batch.rs`

**Interfaces:**
- Consumes: `super::clustering::cosine_similarity`.
- Produces: `map_local_speakers_to_profiles` (same signature) now assigns a profile name only on a clear win; new consts `pub const HIGH_MATCH_THRESHOLD: f32 = 0.72;` and `pub const MATCH_MARGIN: f32 = 0.08;`.

- [ ] **Step 1: Write the failing tests.** In `batch.rs`'s `#[cfg(test)] mod tests`, add (keep the existing `maps_matching_local_speaker_to_profile_else_speaker_n` test — it still holds for a clear winner):
```rust
    #[test]
    fn ambiguous_match_is_unknown_speaker_n() {
        // two profiles both close to the cluster, within the margin → not confident
        let cluster = unit(vec![1.0, 1.0, 0.0]);
        let profiles = vec![
            ("Alice".to_string(), unit(vec![1.0, 0.9, 0.0])),
            ("Bob".to_string(), unit(vec![0.9, 1.0, 0.0])),
        ];
        let map = map_local_speakers_to_profiles(&[("A".to_string(), cluster)], &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Speaker 1"));
    }

    #[test]
    fn weak_best_below_high_threshold_is_unknown() {
        // best match is ~0.64 cosine — above the OLD 0.60 bar (would have mislabeled),
        // below HIGH_MATCH_THRESHOLD → must stay "Speaker 1", not "Alice".
        let cluster = unit(vec![1.0, 1.2, 0.0]);
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let sim = crate::diarization::clustering::cosine_similarity(
            &unit(vec![1.0, 1.2, 0.0]),
            &unit(vec![1.0, 0.0, 0.0]),
        );
        assert!(sim > 0.60 && sim < HIGH_MATCH_THRESHOLD, "test fixture sim={sim}");
        let map = map_local_speakers_to_profiles(&[("A".to_string(), cluster)], &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Speaker 1"));
    }

    #[test]
    fn single_strong_profile_matches() {
        let cluster = unit(vec![1.0, 0.02, 0.0]);
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let map = map_local_speakers_to_profiles(&[("A".to_string(), cluster)], &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Alice"));
    }
```

- [ ] **Step 2: Run to see them fail.**
```bash
cd frontend/src-tauri && cargo test --lib diarization::batch 2>&1 | tail -20
```
Expected: `ambiguous_match_is_unknown_speaker_n` and `weak_best_below_high_threshold_is_unknown` FAIL (current logic assigns on the ≥0.60 single-threshold).

- [ ] **Step 3: Add the consts + rewrite the matching to be competitive + margin-gated.** In `batch.rs`, add the consts near the top and replace the body of `map_local_speakers_to_profiles`:
```rust
/// A cluster only adopts a saved profile's name when it is a CLEAR winner:
/// nearest profile, cosine ≥ HIGH_MATCH_THRESHOLD, and ahead of the runner-up
/// by ≥ MATCH_MARGIN. Otherwise the cluster stays "Speaker N" (unknown) — we
/// never force a guess. Deliberately stricter than the intra-meeting
/// clustering threshold; a weak/ambiguous match must read as unknown.
pub const HIGH_MATCH_THRESHOLD: f32 = 0.72;
pub const MATCH_MARGIN: f32 = 0.08;

pub fn map_local_speakers_to_profiles(
    local_centroids: &[(String, Vec<f32>)],
    profiles: &[(String, Vec<f32>)],
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut anon: usize = 0;
    for (local, centroid) in local_centroids {
        let mut sims: Vec<(&String, f32)> = profiles
            .iter()
            .map(|(name, emb)| (name, cosine_similarity(centroid, emb)))
            .collect();
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let confident = match sims.first() {
            Some((name, best)) => {
                let runner_up = sims.get(1).map(|(_, s)| *s).unwrap_or(0.0);
                if *best >= HIGH_MATCH_THRESHOLD && (*best - runner_up) >= MATCH_MARGIN {
                    Some((*name).clone())
                } else {
                    None
                }
            }
            None => None,
        };

        let label = match confident {
            Some(name) => name,
            None => {
                anon += 1;
                format!("Speaker {}", anon)
            }
        };
        map.insert(local.clone(), label);
    }
    map
}
```
(Remove the old `PROFILE_MATCH_THRESHOLD`-based body. Keep the `use super::clustering::cosine_similarity;` import; drop the `PROFILE_MATCH_THRESHOLD` import from this file if it becomes unused — check with the compiler.)

- [ ] **Step 4: Run tests + build.**
```bash
cd frontend/src-tauri
cargo test --lib diarization::batch 2>&1 | tail -15
cargo build 2>&1 | tail -8
```
Expected: all batch tests pass (the two new ones now green; the clear-winner test still green); builds clean.

- [ ] **Step 5: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/batch.rs
git commit -m "feat(speakers): :sparkles: only auto-label a speaker on a clear, confident voice match"
```

---

### Task 4 (optional — may defer): Voice-profile centroid accrual on confirmation

**Files:**
- Modify: `frontend/src-tauri/src/database/repositories/speaker_profile.rs`
- Modify: `frontend/src-tauri/src/diarization/commands.rs`

**Interfaces:**
- Consumes: `SpeakerProfilesRepository` (`SpeakerProfile { id, name, embedding: Vec<f32> }`, `list`), `load_centroid_from_folder`.
- Produces: when a speaker is renamed to a name that MATCHES an existing profile, that profile's stored `embedding` is updated toward this meeting's cluster centroid (running mean, re-normalized), improving future recognition.

> **YAGNI / defer note:** Task 3 (match-only) is a complete, shippable v0.5.6. Accrual is an enhancement. The controller may ship v0.5.6 after Task 3 and treat Task 4 as a follow-up. Only implement Task 4 if explicitly continuing.

- [ ] **Step 1: Write the failing test for the pure accrual helper.** In `speaker_profile.rs` (or a small pure helper module), add:
```rust
#[cfg(test)]
mod accrual_tests {
    use super::*;
    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }
    #[test]
    fn accrue_moves_toward_new_and_stays_unit() {
        let existing = unit(vec![1.0, 0.0, 0.0]);
        let new = unit(vec![0.0, 1.0, 0.0]);
        let out = accrue_centroid(&existing, 4, &new); // 4 prior segments
        let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "result must be unit-norm, got {norm}");
        // moved toward `new` on axis 1 but still dominated by `existing` on axis 0
        assert!(out[0] > out[1] && out[1] > 0.0);
    }
}
```

- [ ] **Step 2: Run it to fail.**
```bash
cd frontend/src-tauri && cargo test --lib speaker_profile 2>&1 | tail -10
```
Expected: `accrue_centroid` not found.

- [ ] **Step 3: Implement `accrue_centroid` (pure).** In `speaker_profile.rs`:
```rust
/// Running-mean accrual of a saved profile centroid toward a newly-confirmed
/// cluster centroid, then re-normalized. `prior_count` is how many segments the
/// existing centroid already represents (weight of the old value).
pub fn accrue_centroid(existing: &[f32], prior_count: usize, new: &[f32]) -> Vec<f32> {
    if existing.len() != new.len() || existing.is_empty() {
        return existing.to_vec();
    }
    let w = prior_count.max(1) as f32;
    let mut out: Vec<f32> = existing
        .iter()
        .zip(new.iter())
        .map(|(e, n)| (e * w + n) / (w + 1.0))
        .collect();
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut out {
            *x /= norm;
        }
    }
    out
}
```
Add a repository method `update_embedding(pool, id, embedding: &[f32])` if one does not already exist (mirror the existing insert's f32→LE-BLOB encoding).

- [ ] **Step 4: Wire accrual into the rename path.** In `diarization_rename_speaker`, after the primary rename, if `new_name` equals an existing profile's `name` (from `SpeakerProfilesRepository::list`) AND this meeting's cluster centroid for `old_label` is available via `load_centroid_from_folder(folder, old_label)`, compute `accrue_centroid(&profile.embedding, <prior segment count if tracked, else a fixed weight like 8>, &cluster_centroid)` and persist via `update_embedding`. Best-effort: any failure logs + skips; the rename still succeeds. NO `?`/`unwrap`/`panic` on this path.

- [ ] **Step 5: Test + build.**
```bash
cd frontend/src-tauri
cargo test --lib speaker_profile 2>&1 | tail -10
cargo build 2>&1 | tail -8
```
Expected: accrual test passes; builds clean.

- [ ] **Step 6: Commit**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/database/repositories/speaker_profile.rs frontend/src-tauri/src/diarization/commands.rs
git commit -m "feat(speakers): :sparkles: strengthen a voice profile each time you confirm it"
```

---

## Notes for the executor

- **v0.5.5 = Tasks 1-2**, **v0.5.6 = Task 3 (+ optional Task 4).** After Task 2's review, merge to local `main`, bump to v0.5.5, `scripts/release.sh`, push `main` to the fork. Repeat for v0.5.6 after Task 3 (and Task 4 if built).
- Every diarization/naming path is best-effort — no `unwrap`/`expect`/panicking `?` added; a failure degrades to "Speaker N" or a no-op, never a wrong guess or a crash.
- Do not touch speakrs, the sidecar, CAM++, the clusterer, the live worker, or the main app's `ort`/`ndarray`.
