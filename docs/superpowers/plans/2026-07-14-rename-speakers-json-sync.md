# Fix: keep speakers.json in sync on speaker rename (v0.5.8) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Fix the "Voice data was not available for this meeting" warning on re-attribution: renaming a speaker updates the transcript but never relabels the meeting's `speakers.json`, so a second rename can't find the voice centroid under its current name.

**Architecture:** In `diarization_rename_speaker`, after loading the centroid by `old_label` (for the profile save), relabel the `speakers.json` entry `old_label → new_name` in lockstep — merging entries that now share the new name. Add a pure, unit-tested relabel+merge helper + a best-effort file wrapper. Backend-only.

**Tech Stack:** Rust (Tauri core), serde_json, sqlx.

## Global Constraints

- Backend-only — `frontend/src-tauri/src/diarization/commands.rs` only. No frontend, no migration, no new dependency, no `ort`/`ndarray` change.
- Best-effort: the `speakers.json` relabel must NEVER fail the rename — missing/unparseable file or absent `old_label` → silent no-op. No `?`/`unwrap`/`expect`/panic on this path.
- Order: keep loading the centroid by `old_label` for `save_profile` BEFORE relabeling (so the profile save still works in the same call). The relabel runs regardless of `save_profile` (so the file stays in sync even when the user unchecks it).
- `speakers.json` schema unchanged: `{"version":"1.0","speakers":[{"label","centroid","segments"}]}`. Preserve `segments`.
- The primary `UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?` and the existing `save_profile` accrue/create logic are UNCHANGED.
- Gitmoji conventional commit; NO `Co-Authored-By` / NO AI or agent mention. Local `main` only; do not push during implementation.

---

### Task 1: Relabel speakers.json in lockstep with rename

**Files:**
- Modify: `frontend/src-tauri/src/diarization/commands.rs`

**Interfaces:**
- Produces: `pub(crate) fn relabel_and_merge_centroids(speakers: Vec<(String, Vec<f32>, usize)>, old_label: &str, new_name: &str) -> Vec<(String, Vec<f32>, usize)>` (pure); a private `fn relabel_speaker_in_folder(folder: &str, old_label: &str, new_name: &str)` (best-effort I/O); and a call to the latter inside `diarization_rename_speaker`.

- [ ] **Step 1: Write the failing tests.** In `commands.rs`'s `#[cfg(test)] mod` (add one if none), add:

```rust
    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    #[test]
    fn relabel_simple_no_collision() {
        let speakers = vec![
            ("Speaker 1".to_string(), unit(vec![1.0, 0.0, 0.0]), 3),
            ("Speaker 2".to_string(), unit(vec![0.0, 1.0, 0.0]), 5),
        ];
        let out = relabel_and_merge_centroids(speakers, "Speaker 2", "Bob");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "Speaker 1");
        assert_eq!(out[1].0, "Bob");
        assert_eq!(out[1].2, 5); // segments preserved
    }

    #[test]
    fn relabel_merges_on_name_collision() {
        // "Speaker 2" is renamed to "Bob", but a "Bob" already exists → merge into one.
        let speakers = vec![
            ("Bob".to_string(), unit(vec![1.0, 0.0, 0.0]), 4),
            ("Speaker 2".to_string(), unit(vec![1.0, 0.2, 0.0]), 2),
        ];
        let out = relabel_and_merge_centroids(speakers, "Speaker 2", "Bob");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "Bob");
        assert_eq!(out[0].2, 6); // 4 + 2 segments summed
        let norm = out[0].1.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "merged centroid must be unit-norm, got {norm}");
    }

    #[test]
    fn relabel_absent_old_label_is_unchanged() {
        let speakers = vec![("Speaker 1".to_string(), unit(vec![1.0, 0.0, 0.0]), 3)];
        let out = relabel_and_merge_centroids(speakers.clone(), "Nope", "Bob");
        assert_eq!(out, speakers);
    }
```

- [ ] **Step 2: Run to see them fail.**

Run: `cd frontend/src-tauri && cargo test --lib diarization::commands 2>&1 | tail -20`
Expected: `relabel_and_merge_centroids` not found (compile error).

- [ ] **Step 3: Implement the pure helper** in `commands.rs` (near `load_centroid_from_folder`):

```rust
/// Relabel `old_label` → `new_name` across (label, centroid, segments) speaker
/// entries, merging any that now share `new_name` into one: a segment-weighted
/// mean centroid, re-normalized, with segments summed. The merged entry keeps
/// the first occurrence's position. Pure (no I/O) so it is unit-tested.
pub(crate) fn relabel_and_merge_centroids(
    speakers: Vec<(String, Vec<f32>, usize)>,
    old_label: &str,
    new_name: &str,
) -> Vec<(String, Vec<f32>, usize)> {
    let mut out: Vec<(String, Vec<f32>, usize)> = Vec::new();
    let mut merged_idx: Option<usize> = None;
    for (label, centroid, segments) in speakers {
        let label = if label == old_label { new_name.to_string() } else { label };
        if label == new_name {
            match merged_idx {
                None => {
                    merged_idx = Some(out.len());
                    out.push((label, centroid, segments));
                }
                Some(i) => {
                    let existing = &mut out[i];
                    let w0 = existing.2.max(1) as f32;
                    let w1 = segments.max(1) as f32;
                    if existing.1.len() == centroid.len() {
                        for (a, b) in existing.1.iter_mut().zip(centroid.iter()) {
                            *a = (*a * w0 + *b * w1) / (w0 + w1);
                        }
                        let norm = existing.1.iter().map(|v| v * v).sum::<f32>().sqrt();
                        if norm > 0.0 {
                            for v in existing.1.iter_mut() {
                                *v /= norm;
                            }
                        }
                    }
                    existing.2 += segments;
                }
            }
        } else {
            out.push((label, centroid, segments));
        }
    }
    out
}
```

- [ ] **Step 4: Run tests to confirm they pass.**

Run: `cd frontend/src-tauri && cargo test --lib diarization::commands 2>&1 | tail -15`
Expected: the 3 new tests pass.

- [ ] **Step 5: Add the best-effort file wrapper** in `commands.rs` (near `load_centroid_from_folder`; reuse its `speakers.json` parsing shape):

```rust
/// Best-effort: keep speakers.json labels in lockstep with a rename by
/// relabeling `old_label` → `new_name` (merging duplicates via
/// `relabel_and_merge_centroids`). No-op if the file is missing/unparseable or
/// `old_label` is not present — never fails the rename.
fn relabel_speaker_in_folder(folder: &str, old_label: &str, new_name: &str) {
    let path = std::path::Path::new(folder).join("speakers.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(j) => j,
        Err(_) => return,
    };
    let arr = match json.get("speakers").and_then(|s| s.as_array()) {
        Some(a) => a,
        None => return,
    };
    let speakers: Vec<(String, Vec<f32>, usize)> = arr
        .iter()
        .filter_map(|s| {
            let label = s.get("label")?.as_str()?.to_string();
            let centroid: Vec<f32> = s
                .get("centroid")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect();
            let segments = s.get("segments").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if centroid.is_empty() {
                None
            } else {
                Some((label, centroid, segments))
            }
        })
        .collect();
    if !speakers.iter().any(|(l, _, _)| l == old_label) {
        return; // nothing to relabel
    }
    let relabeled = relabel_and_merge_centroids(speakers, old_label, new_name);
    let out = serde_json::json!({
        "version": "1.0",
        "speakers": relabeled.into_iter().map(|(label, centroid, segments)| {
            serde_json::json!({ "label": label, "centroid": centroid, "segments": segments })
        }).collect::<Vec<_>>(),
    });
    match serde_json::to_string(&out).map(|s| std::fs::write(&path, s)) {
        Ok(Ok(())) => log::info!(
            "🎙️ Relabeled speakers.json '{}' → '{}' in {}",
            old_label, new_name, folder
        ),
        Ok(Err(e)) => log::warn!("🎙️ Failed to write speakers.json after relabel: {}", e),
        Err(e) => log::warn!("🎙️ Failed to serialize speakers.json after relabel: {}", e),
    }
}
```

- [ ] **Step 6: Wire it into `diarization_rename_speaker`.** Read the function first. Currently the `SELECT folder_path FROM meetings WHERE id = ?` lookup lives INSIDE `if save_profile`. Change so the folder path is available regardless:
  - Move the `folder_path: Option<String>` lookup to ABOVE the `if save_profile` block (fetch it unconditionally). Keep the `save_profile` block using that same `folder_path` for `load_centroid_from_folder`.
  - AFTER the `if save_profile { … }` block (so the centroid load-by-`old_label` has already run), add:
```rust
    // Keep speakers.json labels in lockstep with the transcript so a later
    // re-attribution can still find this voice under its current name.
    if let Some(folder) = folder_path.as_deref() {
        relabel_speaker_in_folder(folder, &old_label, new_name);
    }
```
  (If moving the lookup changes any `?`/error handling, keep the existing `.map_err(...)?` on the SELECT exactly as it was — just hoist the whole statement up.)

- [ ] **Step 7: Build + test.**

```bash
cd frontend/src-tauri
cargo test --lib diarization 2>&1 | tail -12
cargo build 2>&1 | tail -8
```
Expected: the 3 new tests + existing diarization tests pass; build clean. (2 pre-existing `audio::device_detection` failures are NOT yours.)

- [ ] **Step 8: Commit.**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/commands.rs
git commit -m "fix(speakers): :bug: keep speakers.json labels in sync on rename so re-attribution finds the voice"
```

---

## Self-review

- **Root cause coverage:** rename updated transcripts but not `speakers.json`, so the second rename's `load_centroid_from_folder(old_label)` missed → warning. Fix relabels `speakers.json` in lockstep (Step 6), so the current label always has its centroid. ✓
- **Best-effort:** `relabel_speaker_in_folder` uses `match … return` on every fallible op, never `?`/`unwrap`; the rename result is unaffected. ✓
- **Merge correctness:** `relabel_and_merge_centroids` is pure + unit-tested for no-collision, collision-merge (segments summed, unit-norm), and absent-label. ✓
- **Type consistency:** helper takes/returns `Vec<(String, Vec<f32>, usize)>`; the wrapper parses/serializes the `{label,centroid,segments}` shape matching `persist_labeled_centroids`. ✓
