# Speaker Recognition Visibility (v0.6.0) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Give cross-meeting speaker recognition (a) a "Saved voices" management view and (b) confirmable near-match suggestions ("Speaker 2 · Alice?") — without loosening the conservative confident-match gate.

**Architecture:** Backend computes a near-match suggestion per unmatched cluster at diarization time and stores it in the per-meeting `speakers.json` (`suggested:{name,score}`); a read command exposes it; the transcript chip shows a subtle hint and clicking pre-fills the rename dialog as an explicit pick. A separate frontend settings view lists/renames/deletes saved profiles.

**Tech Stack:** Rust (Tauri core), serde_json, sqlx; Next.js/React + TypeScript.

## Global Constraints

- `HIGH_MATCH_THRESHOLD = 0.72` (confident auto-label) is UNCHANGED. Suggestions NEVER auto-apply — the user always confirms. Nothing is silently relabeled.
- Best-effort everywhere: no `?`/`unwrap`/`expect`/panic on the suggestion / persistence / command / fetch paths. Missing/failed data → no suggestion (chip shows plain "Speaker N"), never an error.
- No DB migration. Suggestions live in `speakers.json` as an additive `suggested` field; existing readers (`load_centroid_from_folder`, `relabel_and_merge_centroids`) already read only `label`/`centroid`/`segments`, so they ignore it (forward-compatible).
- No change to the live recording path's labeling.
- Run main-app cargo from `frontend/src-tauri`; frontend gate = `cd frontend && npx tsc --noEmit` with NO new errors (a pre-existing `bun:test` tsc error is unrelated; no frontend unit-test runner). Pre-existing `audio::device_detection` 2 test failures are NOT yours.
- Gitmoji conventional commits; NO `Co-Authored-By` / NO AI-agent mention. Local `main` only; do not push during implementation.

---

### Task 1: `suggest_near_matches` (pure)

**Files:** Modify `frontend/src-tauri/src/diarization/batch.rs`

**Interfaces:**
- Consumes: `super::clustering::cosine_similarity`, existing `HIGH_MATCH_THRESHOLD` (0.72), `MATCH_MARGIN` (0.08).
- Produces: `pub const SUGGEST_FLOOR: f32 = 0.62;` and `pub fn suggest_near_matches(local_centroids: &[(String, Vec<f32>)], profiles: &[(String, Vec<f32>)], name_map: &std::collections::HashMap<String, String>) -> std::collections::HashMap<String, (String, f32)>` (keyed by the cluster's FINAL label → (profile_name, score)). Consumed by Task 3.

- [ ] **Step 1: Write the failing tests** in `batch.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn suggests_near_match_for_unmatched_cluster() {
        // best ~0.66 (in [0.62, 0.72)), clear top → suggestion.
        let cluster = unit(vec![1.0, 1.1, 0.0]); // cos to [1,0,0] ~ 0.67
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Speaker 1".to_string()); // unmatched
        let out = suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map);
        let s = out.get("Speaker 1").expect("should suggest");
        assert_eq!(s.0, "Alice");
        assert!(s.1 >= SUGGEST_FLOOR && s.1 < HIGH_MATCH_THRESHOLD, "score {}", s.1);
    }

    #[test]
    fn no_suggestion_for_confidently_matched_cluster() {
        // name_map already resolved to the profile name → not a "Speaker N" → skip.
        let cluster = unit(vec![1.0, 0.02, 0.0]);
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Alice".to_string());
        let out = suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map);
        assert!(out.is_empty());
    }

    #[test]
    fn no_suggestion_below_floor() {
        let cluster = unit(vec![1.0, 2.0, 0.0]); // cos to [1,0,0] ~ 0.447 < 0.62
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Speaker 1".to_string());
        assert!(suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map).is_empty());
    }

    #[test]
    fn no_suggestion_when_ambiguous() {
        let cluster = unit(vec![1.0, 1.0, 0.0]);
        let profiles = vec![
            ("Alice".to_string(), unit(vec![1.0, 0.9, 0.0])),
            ("Bob".to_string(), unit(vec![0.9, 1.0, 0.0])),
        ];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Speaker 1".to_string());
        assert!(suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map).is_empty());
    }
```

- [ ] **Step 2: Run to fail.** `cd frontend/src-tauri && cargo test --lib diarization::batch 2>&1 | tail -20` → `suggest_near_matches`/`SUGGEST_FLOOR` not found.

- [ ] **Step 3: Implement** in `batch.rs`:

```rust
/// Near-match suggestion band floor. A cluster that did NOT confidently match
/// (below HIGH_MATCH_THRESHOLD) but whose best profile cosine is at least this,
/// and clearly ahead of the runner-up, is surfaced as a confirmable suggestion
/// rather than a silent "Speaker N". Never auto-applied.
pub const SUGGEST_FLOOR: f32 = 0.62;

/// For each cluster whose final label is still an unnamed "Speaker N" (i.e. it
/// did not confidently match a profile in `name_map`), return the best
/// near-match suggestion (SUGGEST_FLOOR ≤ cosine < HIGH_MATCH_THRESHOLD, clear
/// top candidate by ≥ MATCH_MARGIN), keyed by the cluster's FINAL label. Pure.
pub fn suggest_near_matches(
    local_centroids: &[(String, Vec<f32>)],
    profiles: &[(String, Vec<f32>)],
    name_map: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, (String, f32)> {
    let mut out = std::collections::HashMap::new();
    for (local, centroid) in local_centroids {
        let final_label = match name_map.get(local) {
            Some(l) => l,
            None => continue,
        };
        // Skip clusters that confidently matched a profile (their final label IS
        // a profile name); only unmatched "Speaker N" clusters get a suggestion.
        if profiles.iter().any(|(n, _)| n == final_label) {
            continue;
        }
        let mut sims: Vec<(&String, f32)> = profiles
            .iter()
            .map(|(name, emb)| (name, cosine_similarity(centroid, emb)))
            .collect();
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((name, best)) = sims.first() {
            let runner_up = sims.get(1).map(|(_, s)| *s).unwrap_or(0.0);
            if *best >= SUGGEST_FLOOR
                && *best < HIGH_MATCH_THRESHOLD
                && (*best - runner_up) >= MATCH_MARGIN
            {
                out.insert(final_label.clone(), ((*name).clone(), *best));
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run tests + build.** `cargo test --lib diarization::batch 2>&1 | tail -15` (4 new pass) + `cargo build 2>&1 | tail -6`.

- [ ] **Step 5: Commit.**
```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/batch.rs
git commit -m "feat(speakers): :sparkles: compute near-match speaker suggestions"
```

---

### Task 2: Persist suggestions + `diarization_get_suggestions` command

**Files:** Modify `frontend/src-tauri/src/diarization/commands.rs`, `frontend/src-tauri/src/lib.rs`

**Interfaces:**
- Changes `persist_labeled_centroids` signature to accept a suggestions map (keyed by label).
- Produces `#[command] pub async fn diarization_get_suggestions(state, meeting_id: String) -> Result<HashMap<String, SuggestionDto>, String>` where `SuggestionDto { name: String, score: f32 }`. Consumed by Task 6.

- [ ] **Step 1: Extend `persist_labeled_centroids`.** Its current signature is `pub async fn persist_labeled_centroids(folder: Option<std::path::PathBuf>, centroids: &[(String, Vec<f32>, usize)])`. Change to add a suggestions param and write the optional `suggested` field:
```rust
pub async fn persist_labeled_centroids(
    folder: Option<std::path::PathBuf>,
    centroids: &[(String, Vec<f32>, usize)],
    suggestions: &std::collections::HashMap<String, (String, f32)>,
) {
    // ... existing empty/None guards unchanged ...
    // where each entry JSON is built, include `suggested` when present:
    //   let mut entry = serde_json::json!({ "label": label, "centroid": centroid, "segments": count });
    //   if let Some((name, score)) = suggestions.get(label) {
    //       entry["suggested"] = serde_json::json!({ "name": name, "score": score });
    //   }
    //   entry
}
```
(Adapt the existing `.map(|(label, centroid, count)| json!({...}))` to the `let mut entry = ...; if let Some(...) { entry["suggested"] = ...; } entry` form. Keep `version`/`label`/`centroid`/`segments` exactly as-is.)

- [ ] **Step 2: Update `persist_speaker_centroids`** (the live-path delegator at ~line 363) to pass an empty map: `persist_labeled_centroids(folder, &session.centroid_snapshot(), &std::collections::HashMap::new()).await;`. (Live path never has suggestions.)

- [ ] **Step 3: Add the read command.** In `commands.rs`:
```rust
#[derive(serde::Serialize)]
pub struct SuggestionDto {
    pub name: String,
    pub score: f32,
}

/// Best-effort: read per-cluster near-match suggestions from a meeting's
/// speakers.json. Returns { label -> {name, score} } for entries that carry a
/// `suggested` field. Empty map on any missing/unparseable data.
#[command]
pub async fn diarization_get_suggestions(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<std::collections::HashMap<String, SuggestionDto>, String> {
    let pool = state.db_manager.pool();
    let folder_path: Option<String> = sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(&meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to look up meeting folder: {}", e))?
        .flatten();
    let mut out = std::collections::HashMap::new();
    let folder = match folder_path { Some(f) => f, None => return Ok(out) };
    let path = std::path::Path::new(&folder).join("speakers.json");
    let content = match std::fs::read_to_string(&path) { Ok(c) => c, Err(_) => return Ok(out) };
    let json: serde_json::Value = match serde_json::from_str(&content) { Ok(j) => j, Err(_) => return Ok(out) };
    if let Some(arr) = json.get("speakers").and_then(|s| s.as_array()) {
        for s in arr {
            if let (Some(label), Some(sug)) = (s.get("label").and_then(|l| l.as_str()), s.get("suggested")) {
                if let (Some(name), Some(score)) = (
                    sug.get("name").and_then(|n| n.as_str()),
                    sug.get("score").and_then(|v| v.as_f64()),
                ) {
                    out.insert(label.to_string(), SuggestionDto { name: name.to_string(), score: score as f32 });
                }
            }
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: Register the command** in `frontend/src-tauri/src/lib.rs` `generate_handler!` (after `diarization::commands::diarization_delete_profile,` ~line 690): `diarization::commands::diarization_get_suggestions,`.

- [ ] **Step 5: Build + test.** `cargo build 2>&1 | tail -12` + `cargo test --lib diarization 2>&1 | tail -8` (existing tests still pass; the persist signature change compiles at its 3 call sites — retranscription/import updated in Task 3, and `persist_speaker_centroids` here in Step 2).
  NOTE: after this task, `retranscription.rs`/`import.rs` still call the 2-arg `persist_labeled_centroids` → they WON'T compile until Task 3. To keep this task's build green, ALSO update those two call sites in this task to pass `&std::collections::HashMap::new()` as a temporary third arg (Task 3 replaces the empty map with real suggestions). Do that so `cargo build` is green here.

- [ ] **Step 6: Commit.**
```bash
git add frontend/src-tauri/src/diarization/commands.rs frontend/src-tauri/src/lib.rs frontend/src-tauri/src/audio/retranscription.rs frontend/src-tauri/src/audio/import.rs
git commit -m "feat(speakers): :sparkles: persist + expose near-match suggestions in speakers.json"
```

---

### Task 3: Wire suggestions into the batch paths

**Files:** Modify `frontend/src-tauri/src/audio/retranscription.rs`, `frontend/src-tauri/src/audio/import.rs`

**Interfaces:** Consumes `batch::suggest_near_matches` (Task 1) + the 3-arg `persist_labeled_centroids` (Task 2).

- [ ] **Step 1: In `retranscription.rs`**, where `final_centroids` is built (~line 528, right after `map_local_speakers_to_profiles` produces `name_map` and `merge_centroids_by_final_label` runs), also compute suggestions:
```rust
        let suggestions = crate::diarization::batch::suggest_near_matches(&local_centroids, &profiles, &name_map);
```
Hold it in a variable in the same scope as `final_centroids` (e.g. `let mut final_suggestions: std::collections::HashMap<String,(String,f32)> = std::collections::HashMap::new();` declared alongside `final_centroids`, assigned here). Then at the persist site (~line 675) pass it: `persist_labeled_centroids(Some(folder_path.clone()), &centroids, &final_suggestions).await;` (replace the temporary empty map from Task 2 Step 5).

- [ ] **Step 2: Mirror the same change in `import.rs`** (`suggest_near_matches` after its `name_map`/`merge_centroids_by_final_label` ~line 738; pass the suggestions at the persist site ~line 855). `local_centroids`, `profiles`, and `name_map` exist in the same scope there (same structure as retranscription).

- [ ] **Step 3: Build + test.** `cargo build 2>&1 | tail -12` + `cargo test --lib diarization 2>&1 | tail -6` + `cargo test --lib audio::retranscription 2>&1 | tail -6`. Clean; existing tests pass.

- [ ] **Step 4: Commit.**
```bash
git add frontend/src-tauri/src/audio/retranscription.rs frontend/src-tauri/src/audio/import.rs
git commit -m "feat(speakers): :sparkles: record near-match suggestions when diarizing (retranscribe + import)"
```

---

### Task 4: "Saved voices" settings view

**Files:** Create `frontend/src/components/SavedVoicesSettings.tsx`; modify the Transcription settings mount point (the component that renders `SpeakerIdentificationSettings` — find via `grep -rn "SpeakerIdentificationSettings" frontend/src/app frontend/src/components`).

**Interfaces:** Consumes existing commands `diarization_list_profiles` (→ `Array<{id,name}>`), `diarization_rename_profile({id,name})`, `diarization_delete_profile({id})`.

- [ ] **Step 1: Create `SavedVoicesSettings.tsx`.** A self-contained component: on mount `invoke('diarization_list_profiles')` → state list; render each row with the name, an inline **Rename** (prompt/inline input → `invoke('diarization_rename_profile', { id, name })`) and a **Delete** (confirm → `invoke('diarization_delete_profile', { id })`), then refetch. Empty state text: "No saved voices yet — name a speaker with 'Remember this voice' on to build recognition." Match the styling idioms of `SpeakerIdentificationSettings.tsx` (same `Label`/`Button`/toast usage). All calls in try/catch with a `toast.error` on failure (best-effort). Include a heading "Saved voices".

- [ ] **Step 2: Mount it** directly under `SpeakerIdentificationSettings` in the Transcription settings screen (read the mount file first; insert `<SavedVoicesSettings />` after `<SpeakerIdentificationSettings />`, importing it).

- [ ] **Step 3: Type-check.** `cd frontend && npx tsc --noEmit 2>&1 | tail -10` → no new errors.

- [ ] **Step 4: Commit.**
```bash
git add frontend/src/components/SavedVoicesSettings.tsx frontend/src/  # + the mount file
git commit -m "feat(speakers): :sparkles: add a Saved voices settings view (list, rename, delete)"
```

---

### Task 5: `suggestedName` prop on `SpeakerRenameDialog`

**Files:** Modify `frontend/src/components/SpeakerRenameDialog.tsx`

**Interfaces:** Produces an optional `suggestedName?: string` prop. Consumed by Task 6.

- [ ] **Step 1: Add the prop + initialize from it.** Add `suggestedName?: string` to `SpeakerRenameDialogProps`. Initialize state so that when `suggestedName` is provided, the dialog opens ready to confirm it as an EXPLICIT pick (so the v0.5.7 typed-duplicate confirm does NOT fire): `const [name, setName] = useState(suggestedName ?? '');` and `const [pickedFromList, setPickedFromList] = useState(!!suggestedName);`. When `suggestedName` is set, render a one-line muted note under the title: "Suggested from a saved voice." (e.g. `{suggestedName && <p className="text-xs text-gray-500">Suggested from a saved voice.</p>}`). No other logic changes — confirming runs the normal rename (which accrues + syncs).

- [ ] **Step 2: Type-check.** `cd frontend && npx tsc --noEmit 2>&1 | tail -10` → no new errors.

- [ ] **Step 3: Commit.**
```bash
git add frontend/src/components/SpeakerRenameDialog.tsx
git commit -m "feat(speakers): :sparkles: pre-fill the rename dialog from a voice suggestion"
```

---

### Task 6: Surface suggestions on the transcript speaker chip

**Files:** Modify `frontend/src/components/MeetingDetails/TranscriptPanel.tsx`, `frontend/src/components/VirtualizedTranscriptView.tsx`, `frontend/src/components/TranscriptView.tsx` (the chip); read them first to match current prop flow.

**Interfaces:** Consumes `diarization_get_suggestions` (Task 2) + `SpeakerRenameDialog`'s `suggestedName` prop (Task 5).

- [ ] **Step 1: Fetch suggestions in `TranscriptPanel.tsx`.** Where the panel has `meetingId`, add an effect: `invoke<Record<string, { name: string; score: number }>>('diarization_get_suggestions', { meetingId })` → state `suggestions` (best-effort: catch → `{}`). Refetch in the existing `onRenamed` callback (after a rename, suggestions may change). Pass `suggestions` down to `VirtualizedTranscriptView`.

- [ ] **Step 2: Thread `suggestions` to the chip.** `VirtualizedTranscriptView` passes the per-segment suggestion (`suggestions[segment.speaker]` when `segment.speaker` is an unnamed `Speaker N`) into the chip render (`TranscriptView` or wherever the chip lives). Only show a hint when the label is an unnamed placeholder (`/^Speaker \d+$/`) AND a suggestion exists for it.

- [ ] **Step 3: Render the hint + wire the click.** In the speaker-chip render, when a suggestion exists, append a subtle muted hint: e.g. `Speaker 2` chip followed by `· {suggestion.name}?` in a lighter style. The existing chip click already opens the rename dialog (via `onSpeakerClick(label)` → `setRenameSpeaker(label)`); extend `TranscriptPanel` so that when it opens `SpeakerRenameDialog` for a label that has a suggestion, it passes `suggestedName={suggestions[renameSpeaker]?.name}`. (So clicking a suggested chip opens the dialog pre-filled.)

- [ ] **Step 4: Type-check + build the frontend.** `cd frontend && npx tsc --noEmit 2>&1 | tail -12` → no new errors.

- [ ] **Step 5: Commit.**
```bash
git add frontend/src/components/MeetingDetails/TranscriptPanel.tsx frontend/src/components/VirtualizedTranscriptView.tsx frontend/src/components/TranscriptView.tsx
git commit -m "feat(speakers): :sparkles: show near-match suggestions on speaker chips with one-click confirm"
```

---

## Self-review

- **Spec coverage:** Part A → Task 4. Part B: compute → Task 1; persist+expose → Task 2; wire → Task 3; dialog pre-fill → Task 5; chip surfacing → Task 6. Thresholds (SUGGEST_FLOOR 0.62, reuse HIGH_MATCH_THRESHOLD/MATCH_MARGIN) → Task 1. Caveat (only newly-diarized) is inherent (suggestions written at diarization time). ✓
- **Build-green ordering:** Task 2 temporarily passes empty maps at the retranscription/import call sites so the crate compiles before Task 3 wires real suggestions. ✓
- **Placeholder scan:** backend tasks carry full code; frontend tasks give exact files + adaptive steps (component structure must be read) — acceptable for UI wiring. No TBD/TODO. ✓
- **Type consistency:** `suggest_near_matches(...) -> HashMap<String,(String,f32)>`; `persist_labeled_centroids(folder, centroids, &HashMap<String,(String,f32)>)`; `SuggestionDto{name,score}`; `suggestedName?: string`; suggestions map `Record<string,{name,score}>` — consistent across tasks. ✓
