# Neural Speaker-Turn Segmentation (speakrs sidecar) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix "two speakers roped into one label" on the Retranscribe/Import batch paths by detecting real speaker turns with the `speakrs` diarization crate — isolated in a `diarize-helper` sidecar — then transcribing per single-speaker turn, mapping turns to saved CAM++ voice profiles, and adding cluster-level re-attribution on rename.

**Architecture:** `speakrs` (pyannote-3.0 segmentation + embedding + clustering) runs in a standalone `diarize-helper` sidecar crate (its own `ort` rc.12 + BLAS, like `llama-helper`), invoked as a subprocess: main app writes decoded 16 kHz mono f32 samples to a temp file → sidecar returns `[{start_ms,end_ms,speaker}]` JSON. The main app keeps CAM++/clusterer/profiles unchanged and uses them only to map speakrs speakers to saved profile names and to persist `speakers.json`. Everything is best-effort: sidecar absent/failed → fall back to today's v0.5.3 VAD path.

**Tech Stack:** Rust, Tauri v2, `speakrs` 0.5 (sidecar only), `ort` (main app stays on 2.0.0-rc.10), sqlx/SQLite, existing Whisper/Parakeet engines, existing CAM++ ONNX diarization.

## Global Constraints

- **Batch paths only** (`audio/retranscription.rs`, `audio/import.rs`); the live worker (`audio/transcription/worker.rs`) is UNCHANGED.
- **The main app's `ort` (2.0.0-rc.10), `ndarray` (0.16), and BLAS are UNCHANGED.** `speakrs` and its `ort`/BLAS live ONLY in the `diarize-helper` crate. If a step would edit `frontend/src-tauri/Cargo.toml`'s `ort`/`ndarray`, it is wrong.
- **Best-effort isolation:** no `?`/`unwrap`/`expect`/`panic` on the sidecar/diarization path in the main app. Any failure (sidecar missing, non-zero exit, timeout, parse error, feature disabled) → fall back to the existing v0.5.3 VAD-per-segment behavior (or no labels). Transcription must never break.
- **Keep CAM++, `SpeakerClusterer`, `speaker_profiles` table, and the `speakers.json` schema `{version, speakers:[{label,centroid,segments}]}` UNCHANGED.** No DB migration. Existing profiles stay valid.
- **`speakrs` dependency:** `speakrs = { version = "0.5", features = ["coreml", <one BLAS feature>] }` — in `diarize-helper/Cargo.toml` ONLY. Apache-2.0.
- **Audio interchange:** raw little-endian `f32` samples, mono, **16 kHz** (no WAV container — the app already holds `Vec<f32>` 16 kHz mono; the sidecar reads raw f32 LE). speakrs `run()` takes `&[f32]`.
- **Commits:** gitmoji conventional; **NO `Co-Authored-By`, NO AI/agent mention** anywhere in commit messages. Local `main` only; do NOT push during implementation.
- **Run all main-app cargo from `frontend/src-tauri`; run sidecar cargo from the repo root** (`cargo build -p diarize-helper`). It is a Cargo workspace. Pre-existing failing tests `audio::device_detection::{test_builtin_mic_detection, test_calculate_buffer_timeout_bluetooth}` are NOT yours — ignore them.

---

### Task 1: `diarize-helper` sidecar crate + go/no-go build spike

**Files:**
- Create: `diarize-helper/Cargo.toml`
- Create: `diarize-helper/src/main.rs`
- Modify: `Cargo.toml` (repo root — add workspace member)

**Interfaces:**
- Produces: a binary `diarize-helper` that reads `--samples <path-to-raw-f32-le>` and `--models-dir <dir>`, runs speakrs, and prints to stdout JSON `[{"start_ms": <u64>, "end_ms": <u64>, "speaker": "<String>"}, …]`; exit 0 on success, non-zero + stderr message on any failure. Consumed by Task 3.

> **This is a GO/NO-GO gate.** If `speakrs` cannot be built on this machine after the BLAS ladder below, STOP and report BLOCKED to the controller with the exact linker/build error — do not proceed to later tasks. The fallback (documented in the spec) is the cheap threshold+model levers, which is a different plan.

- [ ] **Step 1: Add the crate to the workspace.** Edit repo-root `Cargo.toml` `members`:

```toml
members = [
    "frontend/src-tauri",
    "llama-helper",
    "diarize-helper"
]
```

- [ ] **Step 2: Write `diarize-helper/Cargo.toml`** (start with `openblas-static`; the BLAS feature may change in Step 6):

```toml
[package]
name = "diarize-helper"
version = "0.1.0"
edition = "2021"

[dependencies]
speakrs = { version = "0.5", features = ["coreml", "openblas-static"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"

[profile.release]
codegen-units = 1
lto = true
```

- [ ] **Step 3: Write `diarize-helper/src/main.rs`.** Reads raw f32 LE samples + models dir, runs speakrs, prints JSON turns:

```rust
use std::io::Read;
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use speakrs::{ExecutionMode, OwnedDiarizationPipeline};

#[derive(Serialize)]
struct Turn {
    start_ms: u64,
    end_ms: u64,
    speaker: String,
}

fn arg_value(flag: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(a) = args.next() {
        if a == flag {
            return args.next();
        }
    }
    None
}

fn read_f32_le(path: &str) -> Result<Vec<f32>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("open samples file {}", path))?
        .read_to_end(&mut bytes)
        .context("read samples file")?;
    if bytes.len() % 4 != 0 {
        return Err(anyhow!("samples file length {} not a multiple of 4", bytes.len()));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn run() -> Result<()> {
    let samples_path = arg_value("--samples").ok_or_else(|| anyhow!("missing --samples"))?;
    let models_dir = arg_value("--models-dir").ok_or_else(|| anyhow!("missing --models-dir"))?;
    // speakrs loads its models from SPEAKRS_MODELS_DIR (downloads there on first run via the `online` feature).
    std::env::set_var("SPEAKRS_MODELS_DIR", &models_dir);

    let samples = read_f32_le(&samples_path)?;
    let mut pipeline = OwnedDiarizationPipeline::from_pretrained(ExecutionMode::CoreMl)
        .map_err(|e| anyhow!("speakrs from_pretrained failed: {e}"))?;
    let result = pipeline
        .run(&samples)
        .map_err(|e| anyhow!("speakrs run failed: {e}"))?;

    let turns: Vec<Turn> = result
        .discrete_diarization
        .to_segments()
        .into_iter()
        .map(|s| Turn {
            start_ms: (s.start.max(0.0) * 1000.0) as u64,
            end_ms: (s.end.max(0.0) * 1000.0) as u64,
            speaker: s.speaker,
        })
        .collect();

    println!("{}", serde_json::to_string(&turns)?);
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("diarize-helper error: {e:#}");
        std::process::exit(1);
    }
}
```

> NOTE: `ExecutionMode`, `OwnedDiarizationPipeline`, `run(&[f32])`, and `result.discrete_diarization.to_segments()` yielding `Segment { start: f32, end: f32, speaker: String }` (seconds) are the verified speakrs 0.5 API. If a symbol differs at build time, consult `cargo doc -p speakrs --open` or docs.rs/speakrs/0.5.0 and adapt the call site (report the adaptation) — do NOT change the JSON output shape.

- [ ] **Step 4: First build attempt.** From the repo root:

```bash
cargo build -p diarize-helper --release 2>&1 | tail -30
```
Expected: either success, or a BLAS/Fortran link error (handled next).

- [ ] **Step 5: If it built, smoke-test and skip to Step 7.** Otherwise do Step 6.

- [ ] **Step 6: Resolve BLAS (decision ladder).** `openblas-static` needs a Fortran toolchain; if the build failed on OpenBLAS/`gfortran`/linker:
  1. Install a Fortran compiler and retry static: `brew install gcc` (provides `gfortran`), then re-run Step 4.
  2. If static still fails, switch to system OpenBLAS: `brew install openblas`, change the feature in `diarize-helper/Cargo.toml` to `features = ["coreml", "openblas-system"]`, export `export OPENBLAS_SYSTEM=1` (and if needed `export OPENBLAS_DIR="$(brew --prefix openblas)"`), and re-run Step 4.
  3. If neither works, STOP — report BLOCKED with the exact error. Do not proceed.

- [ ] **Step 7: Smoke test on real samples.** Create a raw-f32 sample from any wav you have, or generate 5 s of silence+tone, then run the binary and confirm valid JSON:

```bash
# Generate 5s of 16kHz mono f32 LE test samples (silence) if you have no wav handy:
python3 -c "import struct,sys; sys.stdout.buffer.write(b''.join(struct.pack('<f',0.0) for _ in range(16000*5)))" > /tmp/diar_samples.f32
mkdir -p /tmp/speakrs-models
./target/release/diarize-helper --samples /tmp/diar_samples.f32 --models-dir /tmp/speakrs-models 2>&1 | tail -20
```
Expected: on first run it downloads speakrs models into `/tmp/speakrs-models` (network), then prints a JSON array (likely `[]` for pure silence — that is a valid PASS: the pipeline ran and emitted well-formed JSON). Any panic or non-JSON stderr is a failure to investigate.

- [ ] **Step 8: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add Cargo.toml diarize-helper/
git commit -m "feat(speakers): :sparkles: add diarize-helper sidecar (speakrs pyannote diarization)"
```

---

### Task 2: Build + bundle the sidecar (mirror llama-helper)

**Files:**
- Modify: `frontend/build-gpu.sh`
- Modify: `frontend/src-tauri/tauri.conf.json`
- Modify: `frontend/src-tauri/capabilities/` (the capability file that grants `llama-helper` its shell/sidecar permission)

**Interfaces:**
- Consumes: the `diarize-helper` binary from Task 1.
- Produces: `frontend/src-tauri/binaries/diarize-helper-<target-triple>` present after `build-gpu.sh`, declared as a Tauri `externalBin` and permitted to spawn. Consumed by Task 3's path resolution.

- [ ] **Step 1: Find the llama-helper build+copy block in `build-gpu.sh`.**

```bash
grep -n "llama-helper" frontend/build-gpu.sh
```
Read the surrounding block (the `cargo build ... -p llama-helper` and the `cp ... src-tauri/binaries/llama-helper-<triple>` lines).

- [ ] **Step 2: Add an analogous diarize-helper build+copy** immediately after the llama-helper block, using the SAME target-triple detection variable the script already computes (e.g. `$TARGET_TRIPLE`/`aarch64-apple-darwin`). Mirror the exact form of the llama-helper lines, substituting `diarize-helper`:

```bash
echo "🗣️  Building diarize-helper sidecar (release)..."
( cd "$ROOT_DIR" && cargo build -p diarize-helper --release )   # use the same root-dir var the script already uses
cp "$ROOT_DIR/target/release/diarize-helper" "./src-tauri/binaries/diarize-helper-${TARGET_TRIPLE}"
echo "✅ Copied diarize-helper to ./src-tauri/binaries/diarize-helper-${TARGET_TRIPLE}"
```
(Match the script's actual variable names — read them in Step 1; do not invent new ones.)

- [ ] **Step 3: Declare the sidecar in `tauri.conf.json`.** Find `bundle.externalBin` (which already lists the llama-helper sidecar) and add the diarize-helper entry alongside it:

```json
"externalBin": [
  "binaries/llama-helper",
  "binaries/diarize-helper"
]
```
(Preserve the existing entries exactly; add `binaries/diarize-helper`.)

- [ ] **Step 4: Grant spawn permission.** Find the capability granting llama-helper its shell permission:

```bash
grep -rn "llama-helper" frontend/src-tauri/capabilities/
```
Add a matching `shell:allow-execute` / sidecar permission entry for `diarize-helper`, mirroring the llama-helper entry's exact shape in the same capability file.

- [ ] **Step 5: Verify the sidecar builds + copies via the script path.** From `frontend`:

```bash
cargo build -p diarize-helper --release 2>&1 | tail -5
ls -la src-tauri/binaries/ | grep diarize-helper || echo "run the copy step from build-gpu.sh"
```
Manually run the copy line from Step 2 to place the binary, then confirm it exists. (A full `build-gpu.sh` run happens at release time; here just confirm the binary builds and the copy lands.)

- [ ] **Step 6: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/build-gpu.sh frontend/src-tauri/tauri.conf.json frontend/src-tauri/capabilities/
git commit -m "build(speakers): :hammer: build + bundle diarize-helper sidecar"
```

---

### Task 3: Sidecar client — `diarization/segmenter.rs`

**Files:**
- Create: `frontend/src-tauri/src/diarization/segmenter.rs`
- Modify: `frontend/src-tauri/src/diarization/mod.rs`

**Interfaces:**
- Consumes: the bundled `diarize-helper` sidecar (Tasks 1–2); `AppHandle<R>`; decoded 16 kHz mono `&[f32]`.
- Produces: `pub struct DiarTurn { pub start_ms: u64, pub end_ms: u64, pub speaker: String }` and `pub async fn run_segmenter<R: Runtime>(app: &AppHandle<R>, samples_16k: &[f32]) -> Option<Vec<DiarTurn>>` (None on any failure). Also `pub(crate) fn parse_turns_json(stdout: &str) -> Option<Vec<DiarTurn>>`. Consumed by Task 4/5/6.

- [ ] **Step 1: Write the failing parse test.** In `segmenter.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_turns() {
        let json = r#"[{"start_ms":0,"end_ms":1500,"speaker":"SPEAKER_00"},
                       {"start_ms":1500,"end_ms":3000,"speaker":"SPEAKER_01"}]"#;
        let turns = parse_turns_json(json).expect("should parse");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].end_ms, 1500);
        assert_eq!(turns[1].speaker, "SPEAKER_01");
    }

    #[test]
    fn empty_array_is_some_empty() {
        assert_eq!(parse_turns_json("[]").map(|t| t.len()), Some(0));
    }

    #[test]
    fn malformed_is_none() {
        assert!(parse_turns_json("not json").is_none());
        assert!(parse_turns_json("").is_none());
    }
}
```

- [ ] **Step 2: Run it to see it fail (function undefined).**

```bash
cd frontend/src-tauri && cargo test --lib diarization::segmenter 2>&1 | tail -15
```
Expected: compile error — `parse_turns_json` / `DiarTurn` not found.

- [ ] **Step 3: Implement `DiarTurn` + `parse_turns_json` + `run_segmenter`.** Full file:

```rust
// diarization/segmenter.rs
//
// Calls the `diarize-helper` sidecar (speakrs pyannote diarization) as a
// subprocess: writes the decoded 16 kHz mono samples as raw f32 LE to a temp
// file, runs the bundled sidecar, and parses its JSON turns from stdout.
// Best-effort: any failure returns None so the batch path degrades to the
// existing VAD behavior — diarization never breaks transcription.

use serde::Deserialize;
use std::io::Write;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_shell::ShellExt;

#[derive(Debug, Clone, Deserialize)]
pub struct DiarTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
}

pub(crate) fn parse_turns_json(stdout: &str) -> Option<Vec<DiarTurn>> {
    serde_json::from_str::<Vec<DiarTurn>>(stdout.trim()).ok()
}

/// Directory speakrs models live in (downloaded on first sidecar run).
fn speakrs_models_dir<R: Runtime>(app: &AppHandle<R>) -> Option<std::path::PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()?
        .join("models")
        .join("diarization")
        .join("speakrs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Run the diarize-helper sidecar over 16 kHz mono samples. None on any failure.
pub async fn run_segmenter<R: Runtime>(
    app: &AppHandle<R>,
    samples_16k: &[f32],
) -> Option<Vec<DiarTurn>> {
    if samples_16k.is_empty() {
        return None;
    }
    let models_dir = speakrs_models_dir(app)?;

    // Write raw f32 LE samples to a temp file.
    let tmp = std::env::temp_dir().join(format!("meetily_diar_{}.f32", std::process::id()));
    {
        let mut f = match std::fs::File::create(&tmp) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("🗣️ diarization: could not create temp samples file: {}", e);
                return None;
            }
        };
        let mut buf = Vec::with_capacity(samples_16k.len() * 4);
        for s in samples_16k {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        if let Err(e) = f.write_all(&buf) {
            log::warn!("🗣️ diarization: could not write temp samples: {}", e);
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
    }

    // Spawn the bundled sidecar (mirrors how llama-helper is resolved via the shell plugin).
    let result = (|| async {
        let cmd = app
            .shell()
            .sidecar("diarize-helper")
            .ok()?
            .args([
                "--samples",
                tmp.to_str()?,
                "--models-dir",
                models_dir.to_str()?,
            ]);
        cmd.output().await.ok()
    })()
    .await;

    let _ = std::fs::remove_file(&tmp);

    let output = result?;
    if !output.status.success() {
        log::warn!(
            "🗣️ diarization sidecar exited non-zero: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let turns = parse_turns_json(&stdout);
    match &turns {
        Some(t) => log::info!("🗣️ diarization sidecar produced {} turn(s)", t.len()),
        None => log::warn!("🗣️ diarization sidecar output was not valid JSON"),
    }
    turns
}
```

> NOTE: `app.shell().sidecar("diarize-helper")` is the Tauri v2 shell-plugin sidecar API — confirm the exact call by reading how `llama-helper` is spawned in the main app (`grep -rn "sidecar\|\.shell()" frontend/src-tauri/src/summary/`). If llama-helper uses a different resolution (e.g. a resolved binary path + `std::process::Command`), MIRROR that exact mechanism instead; keep the `Option` return + None-on-failure contract.

- [ ] **Step 4: Export from `mod.rs`.** Add:

```rust
pub mod segmenter;
pub use segmenter::{run_segmenter, DiarTurn};
```

- [ ] **Step 5: Run tests + build.**

```bash
cd frontend/src-tauri
cargo test --lib diarization::segmenter 2>&1 | tail -10
cargo build 2>&1 | tail -10
```
Expected: the 3 parse tests pass; the crate builds. (The spawn path is build-checked; a live sidecar run is covered by manual E2E.)

- [ ] **Step 6: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/segmenter.rs frontend/src-tauri/src/diarization/mod.rs
git commit -m "feat(speakers): :sparkles: diarize-helper sidecar client + turn parsing"
```

---

### Task 4: Orchestrator — turn merging + CAM++ speaker→profile mapping (`diarization/batch.rs`)

**Files:**
- Create: `frontend/src-tauri/src/diarization/batch.rs`
- Modify: `frontend/src-tauri/src/diarization/mod.rs`

**Interfaces:**
- Consumes: `DiarTurn` (Task 3); `clustering::cosine_similarity`, `clustering::PROFILE_MATCH_THRESHOLD`.
- Produces:
  - `pub struct DiarUnit { pub start_ms: u64, pub end_ms: u64, pub speaker_local: String }`
  - `pub fn merge_turns(turns: &[DiarTurn], min_unit_ms: u64, merge_gap_ms: u64) -> Vec<DiarUnit>` — merges adjacent same-speaker turns across gaps ≤ `merge_gap_ms`, drops units shorter than `min_unit_ms`.
  - `pub fn map_local_speakers_to_profiles(local_centroids: &[(String, Vec<f32>)], profiles: &[(String, Vec<f32>)]) -> std::collections::HashMap<String, String>` — maps each local speaker label to a profile name when cosine ≥ `PROFILE_MATCH_THRESHOLD`, else to `Speaker N` (stable numbering by first appearance).
  - Constants `pub const MIN_UNIT_MS: u64 = 1000; pub const MERGE_GAP_MS: u64 = 500;`
  Consumed by Tasks 5 and 6.

- [ ] **Step 1: Write failing tests.** In `batch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::segmenter::DiarTurn;

    fn t(start: u64, end: u64, spk: &str) -> DiarTurn {
        DiarTurn { start_ms: start, end_ms: end, speaker: spk.to_string() }
    }

    #[test]
    fn merges_adjacent_same_speaker() {
        let turns = vec![t(0, 1200, "A"), t(1400, 2600, "A"), t(2600, 4000, "B")];
        let units = merge_turns(&turns, MIN_UNIT_MS, MERGE_GAP_MS);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].speaker_local, "A");
        assert_eq!(units[0].start_ms, 0);
        assert_eq!(units[0].end_ms, 2600);
        assert_eq!(units[1].speaker_local, "B");
    }

    #[test]
    fn drops_sub_minimum_units() {
        let turns = vec![t(0, 1500, "A"), t(1500, 1900, "B")]; // B is 400ms < 1000ms
        let units = merge_turns(&turns, MIN_UNIT_MS, MERGE_GAP_MS);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].speaker_local, "A");
    }

    #[test]
    fn does_not_merge_across_large_gap() {
        let turns = vec![t(0, 1200, "A"), t(3000, 4200, "A")]; // 1800ms gap > 500ms
        let units = merge_turns(&turns, MIN_UNIT_MS, MERGE_GAP_MS);
        assert_eq!(units.len(), 2);
    }

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    #[test]
    fn maps_matching_local_speaker_to_profile_else_speaker_n() {
        let alice = unit(vec![1.0, 0.05, 0.0]);
        let locals = vec![
            ("A".to_string(), unit(vec![0.98, 0.1, 0.05])), // close to alice
            ("B".to_string(), unit(vec![0.0, 1.0, 0.0])),   // not alice
        ];
        let profiles = vec![("Alice".to_string(), alice)];
        let map = map_local_speakers_to_profiles(&locals, &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Alice"));
        assert_eq!(map.get("B").map(String::as_str), Some("Speaker 1"));
    }
}
```

- [ ] **Step 2: Run to see it fail.**

```bash
cd frontend/src-tauri && cargo test --lib diarization::batch 2>&1 | tail -15
```
Expected: compile error — items not found.

- [ ] **Step 3: Implement `batch.rs`.**

```rust
// diarization/batch.rs
//
// Pure turn→unit merging and speakrs-speaker→saved-profile mapping for the
// batch (Retranscribe/Import) diarization path. No I/O, no model calls here —
// callers supply CAM++ centroids; this module is fully unit-testable.

use super::clustering::{cosine_similarity, PROFILE_MATCH_THRESHOLD};
use super::segmenter::DiarTurn;
use std::collections::HashMap;

pub const MIN_UNIT_MS: u64 = 1000;
pub const MERGE_GAP_MS: u64 = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct DiarUnit {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_local: String,
}

/// Merge adjacent same-speaker turns (gap ≤ merge_gap_ms) and drop units
/// shorter than min_unit_ms. Turns are assumed time-ordered.
pub fn merge_turns(turns: &[DiarTurn], min_unit_ms: u64, merge_gap_ms: u64) -> Vec<DiarUnit> {
    let mut units: Vec<DiarUnit> = Vec::new();
    for turn in turns {
        if let Some(last) = units.last_mut() {
            if last.speaker_local == turn.speaker
                && turn.start_ms >= last.end_ms
                && turn.start_ms - last.end_ms <= merge_gap_ms
            {
                last.end_ms = turn.end_ms.max(last.end_ms);
                continue;
            }
        }
        units.push(DiarUnit {
            start_ms: turn.start_ms,
            end_ms: turn.end_ms,
            speaker_local: turn.speaker.clone(),
        });
    }
    units
        .into_iter()
        .filter(|u| u.end_ms.saturating_sub(u.start_ms) >= min_unit_ms)
        .collect()
}

/// Map each local speakrs speaker to a saved profile name (cosine ≥ threshold)
/// or a stable "Speaker N" label (numbered by first appearance).
pub fn map_local_speakers_to_profiles(
    local_centroids: &[(String, Vec<f32>)],
    profiles: &[(String, Vec<f32>)],
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut anon: usize = 0;
    for (local, centroid) in local_centroids {
        let best = profiles
            .iter()
            .map(|(name, emb)| (name, cosine_similarity(centroid, emb)))
            .filter(|(_, sim)| *sim >= PROFILE_MATCH_THRESHOLD)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let label = match best {
            Some((name, _)) => name.clone(),
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

- [ ] **Step 4: Export from `mod.rs`.** Add:

```rust
pub mod batch;
```

- [ ] **Step 5: Run tests + build.**

```bash
cd frontend/src-tauri
cargo test --lib diarization::batch 2>&1 | tail -10
cargo build 2>&1 | tail -5
```
Expected: 4 tests pass; builds.

- [ ] **Step 6: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/batch.rs frontend/src-tauri/src/diarization/mod.rs
git commit -m "feat(speakers): :sparkles: turn merging + speaker-to-profile mapping for batch diarization"
```

---

### Task 5: Splice the sidecar diarization into Retranscription

**Files:**
- Modify: `frontend/src-tauri/src/audio/retranscription.rs`

**Interfaces:**
- Consumes: `diarization::run_segmenter` (Task 3), `diarization::batch::{merge_turns, map_local_speakers_to_profiles, DiarUnit, MIN_UNIT_MS, MERGE_GAP_MS}` (Task 4), `diarization::commands::{init_session, persist_speaker_centroids}` (existing), the existing transcription engines, `common::{create_transcript_segments, split_segment_at_silence, write_transcripts_json}`.
- Produces: retranscription that, when diarization is enabled + the sidecar yields turns, transcribes per single-speaker unit with correct speaker labels; unchanged VAD fallback otherwise.

- [ ] **Step 1: Read the current diarized-VAD loop.** Read `frontend/src-tauri/src/audio/retranscription.rs` from the decode (line ~202) through the INSERT (~450) and `persist_speaker_centroids` (~483). Note: the decoded audio is available right after `decode_audio_file`; `all_transcripts: Vec<(String, f64, f64, Option<String>)>` is the collection; `folder_path` is in scope; `diarization` session is created via `init_session`.

- [ ] **Step 2: Add a helper to slice samples by time (shared).** In `frontend/src-tauri/src/audio/common.rs`, add (near `split_segment_at_silence`):

```rust
/// Slice 16 kHz mono samples for a [start_ms, end_ms) window (clamped).
pub(crate) fn slice_samples_16k(samples: &[f32], start_ms: u64, end_ms: u64) -> Vec<f32> {
    let sr = 16_000u64;
    let start = ((start_ms * sr) / 1000) as usize;
    let end = (((end_ms * sr) / 1000) as usize).min(samples.len());
    if start >= end {
        return Vec::new();
    }
    samples[start..end].to_vec()
}
```

- [ ] **Step 3: Add the turn-based branch.** After the full decoded 16 kHz mono samples are available (the `Vec<f32>` produced from `decode_audio_file`; confirm its variable name — it is the same buffer VAD consumes) and after `let mut diarization = crate::diarization::commands::init_session(&app).await;`, insert a diarization-turn attempt that, on success, REPLACES the VAD segment loop for building `all_transcripts`. Structure (adapt names to the file):

```rust
    // Neural speaker turns via the diarize-helper sidecar (best-effort).
    // `decoded_16k` is the full 16 kHz mono sample buffer used for VAD below.
    let diar_units: Option<Vec<crate::diarization::batch::DiarUnit>> = if diarization.is_some() {
        crate::diarization::run_segmenter(&app, &decoded_16k)
            .await
            .map(|turns| {
                crate::diarization::batch::merge_turns(
                    &turns,
                    crate::diarization::batch::MIN_UNIT_MS,
                    crate::diarization::batch::MERGE_GAP_MS,
                )
            })
            .filter(|units| !units.is_empty())
    } else {
        None
    };
```

- [ ] **Step 4: When `diar_units` is `Some`, transcribe per unit + collect CAM++ centroids.** Replace the population of `all_transcripts` with a branch: if `Some(units)`, iterate units; for each, `let unit_samples = crate::audio::common::slice_samples_16k(&decoded_16k, u.start_ms, u.end_ms);` then transcribe `unit_samples` with the SAME engine call the current loop uses (`transcribe_audio_with_confidence` / parakeet), and for a non-empty transcript, embed the unit with CAM++ via the session to accumulate a per-local-speaker centroid. Accumulate `local_centroids: Vec<(String, Vec<f32>)>` by averaging embeddings per `speaker_local` (use the session's embedding path; if `label_segment` is the only exposed embed, call it but keep the returned label only for centroid bookkeeping — see note). Push `(text, u.start_ms as f64, u.end_ms as f64, Some(local_label))` where `local_label` is `u.speaker_local` for now (final names applied in Step 5). If `None`, keep the existing VAD loop verbatim.

> IMPLEMENTATION NOTE: the cleanest embedding accumulation is to add a small method on `DiarizationSession` that returns the raw CAM++ embedding for samples without clustering (e.g. `pub fn embed(&mut self, samples_16k: &[f32]) -> Option<Vec<f32>>`, wrapping the existing extractor). Add it in `diarization/session.rs` if it does not already exist, and average embeddings per `speaker_local`. This keeps the mapping in Task 4's pure function.

- [ ] **Step 5: Apply profile names.** After the unit loop, build `local_centroids`, load saved profiles (`SpeakerProfilesRepository::list`), compute `let name_map = crate::diarization::batch::map_local_speakers_to_profiles(&local_centroids, &profiles);`, and rewrite each collected tuple's speaker from the local label to `name_map[&local]`. Also build the centroid set to persist to `speakers.json` under the FINAL labels (write via a small local step or by seeding a fresh clusterer — reuse `persist_speaker_centroids`-compatible shape `{label, centroid, segments}`).

- [ ] **Step 6: Persist.** The existing INSERT (`speaker` = 8th column) and `write_transcripts_json` are unchanged (they read from `segments`/`all_transcripts`). Ensure `speakers.json` is written with the final per-speaker centroids (so rename/"remember voice" works). The existing `persist_speaker_centroids(session, folder)` call persists the session's clusters; for the turn path, write the final-label centroids instead (add a sibling `persist_labeled_centroids(folder, &[(String, Vec<f32>, usize)])` in `diarization/commands.rs` if needed, mirroring the existing JSON shape).

- [ ] **Step 7: Build + tests.**

```bash
cd frontend/src-tauri
cargo build 2>&1 | tail -20
cargo test --lib audio::retranscription 2>&1 | tail -10
cargo test --lib diarization 2>&1 | tail -10
```
Expected: builds; existing `create_transcript_segments` tests still pass; diarization tests pass. Fallback path (sidecar `None`) is byte-for-byte the v0.5.3 behavior.

- [ ] **Step 8: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/audio/retranscription.rs frontend/src-tauri/src/audio/common.rs frontend/src-tauri/src/diarization/
git commit -m "feat(speakers): :sparkles: neural speaker turns on retranscription (sidecar + per-turn transcription)"
```

---

### Task 6: Splice the sidecar diarization into Import

**Files:**
- Modify: `frontend/src-tauri/src/audio/import.rs`

**Interfaces:**
- Consumes: same as Task 5, plus `create_meeting_with_transcripts` (`import.rs:700`, INSERT at ~732).
- Produces: import that labels speakers via the sidecar turn path when available; unchanged VAD fallback otherwise.

- [ ] **Step 1: Read `run_import` (~311) + `create_meeting_with_transcripts` (~700).** Confirm the decoded 16 kHz mono buffer (from `decode_audio_file_with_progress`, ~385), the `all_transcripts` collection, `meeting_folder` in scope, and the INSERT/`persist_speaker_centroids` (~662) points.

- [ ] **Step 2: Apply the SAME turn-based branch as Task 5.** Mirror Task 5 Steps 3–6 exactly against `import.rs`'s variable names: build `diar_units` from `run_segmenter` on the decoded buffer, transcribe per unit, accumulate CAM++ centroids, map to profiles, write `speakers.json` to `meeting_folder`. The INSERT lives in `create_meeting_with_transcripts` and already binds `speaker` (8th column) — no signature change. Keep the existing VAD loop as the `None` fallback verbatim.

> Use the shared helpers from Tasks 3–5 (`run_segmenter`, `merge_turns`, `map_local_speakers_to_profiles`, `slice_samples_16k`, the `embed`/`persist_labeled_centroids` helpers). Do NOT duplicate their logic — call them.

- [ ] **Step 3: Build + tests.**

```bash
cd frontend/src-tauri
cargo build 2>&1 | tail -20
cargo test --lib audio::import 2>&1 | tail -10
```
Expected: builds; import tests pass; fallback path unchanged.

- [ ] **Step 4: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/audio/import.rs
git commit -m "feat(speakers): :sparkles: neural speaker turns on audio import (sidecar + per-turn transcription)"
```

---

### Task 7: Cluster-level re-attribution on rename

**Files:**
- Modify: `frontend/src-tauri/src/diarization/commands.rs` (`diarization_rename_speaker`, ~line 87; `load_centroid_from_folder`, ~line 61)

**Interfaces:**
- Consumes: `clustering::cosine_similarity`, `PROFILE_MATCH_THRESHOLD`, `load_centroid_from_folder`, `speakers.json`.
- Produces: after a rename, other clusters whose centroid matches the named speaker's centroid (cosine ≥ threshold) are relabeled to the new name in both the DB `transcripts` rows and `speakers.json`.

- [ ] **Step 1: Write a failing unit test for the selection logic.** Add a pure helper `clusters_to_reattribute(named_centroid, others: &[(String, Vec<f32>)], threshold: f32) -> Vec<String>` and test it in `commands.rs`:

```rust
#[cfg(test)]
mod reattribution_tests {
    use super::*;
    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }
    #[test]
    fn selects_only_matching_clusters() {
        let named = unit(vec![1.0, 0.05, 0.0]);
        let others = vec![
            ("Speaker 2".to_string(), unit(vec![0.97, 0.12, 0.0])), // match
            ("Speaker 3".to_string(), unit(vec![0.0, 1.0, 0.0])),   // no match
        ];
        let hits = clusters_to_reattribute(&named, &others, 0.6);
        assert_eq!(hits, vec!["Speaker 2".to_string()]);
    }
    #[test]
    fn empty_when_none_match() {
        let named = unit(vec![1.0, 0.0, 0.0]);
        let others = vec![("Speaker 2".to_string(), unit(vec![0.0, 1.0, 0.0]))];
        assert!(clusters_to_reattribute(&named, &others, 0.6).is_empty());
    }
}
```

- [ ] **Step 2: Run to see it fail.**

```bash
cd frontend/src-tauri && cargo test --lib diarization::commands 2>&1 | tail -15
```
Expected: `clusters_to_reattribute` not found.

- [ ] **Step 3: Implement the helper.** In `commands.rs`:

```rust
/// Cluster labels (from the meeting's speakers.json) whose centroid matches the
/// newly-named speaker's centroid at/above `threshold` — candidates to merge
/// into the new name.
pub(crate) fn clusters_to_reattribute(
    named_centroid: &[f32],
    others: &[(String, Vec<f32>)],
    threshold: f32,
) -> Vec<String> {
    others
        .iter()
        .filter(|(_, c)| super::clustering::cosine_similarity(named_centroid, c) >= threshold)
        .map(|(label, _)| label.clone())
        .collect()
}
```

- [ ] **Step 4: Wire it into `diarization_rename_speaker`.** After the existing rename (which sets `old_label → new_label` for the meeting and optionally saves a profile), load ALL cluster centroids from the meeting's `speakers.json` (extend/reuse `load_centroid_from_folder`, or add `load_all_centroids_from_folder(folder) -> Vec<(String, Vec<f32>)>`), take the named speaker's centroid, compute `clusters_to_reattribute(&named, &others_excluding_named, PROFILE_MATCH_THRESHOLD)`, and for each hit run the same DB update the rename uses (`UPDATE transcripts SET speaker = ?new WHERE meeting_id = ? AND speaker = ?hit`) plus relabel it in `speakers.json`. Log how many clusters were merged. All best-effort: on any load/parse failure, skip re-attribution (the primary rename still succeeds).

- [ ] **Step 5: Build + tests.**

```bash
cd frontend/src-tauri
cargo test --lib diarization::commands 2>&1 | tail -10
cargo build 2>&1 | tail -5
```
Expected: the 2 selection tests pass; builds.

- [ ] **Step 6: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src-tauri/src/diarization/commands.rs
git commit -m "feat(speakers): :sparkles: cluster-level re-attribution when naming a speaker"
```

---

### Task 8: Frontend — first-run model prep affordance (minimal)

**Files:**
- Modify: the diarization settings/UX component that already handles the CAM++ model download (`grep -rn "diarization-model-download-progress\|diarization_download_model\|Speaker" frontend/src/` to locate `SpeakerIdentificationSettings.tsx`).

**Interfaces:**
- Consumes: existing diarization enable/status commands.
- Produces: a lightweight note that the FIRST retranscribe/import with speaker ID on will download the speaker-segmentation models (the sidecar fetches speakrs models on first run), so the first run is slower. No new settings screen.

- [ ] **Step 1: Locate the component + copy string.**

```bash
grep -rn "Speaker identification\|diarization" frontend/src/ | grep -i "tsx" | head
```

- [ ] **Step 2: Add a one-line helper note** under the existing Speaker Identification toggle text, e.g.: "The first time you retranscribe or import with this on, speaker models (~a few hundred MB) download once." Match the surrounding copy/style. No logic change.

- [ ] **Step 3: Type-check.**

```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -10
```
Expected: no NEW errors (the repo has a known pre-existing `bun:test` tsc error unrelated to this change).

- [ ] **Step 4: Commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src/
git commit -m "docs(speakers): :memo: note first-run speaker-model download in settings"
```

---

## Notes for the executor

- **Task 1 is a hard gate.** If `speakrs` will not build after the BLAS ladder, STOP and report — the rest of the plan depends on it.
- **The main-app `Cargo.toml` (`frontend/src-tauri/Cargo.toml`) must not change its `ort`/`ndarray` versions.** speakrs lives only in `diarize-helper`.
- **Every diarization call in the main app is best-effort** — grep your diff for `unwrap()/expect()/?` on the segmenter/sidecar path and remove them; the fallback to the v0.5.3 VAD loop must be byte-for-byte preserved.
- After all tasks: final whole-branch review, then merge to local `main`, bump to **v0.5.4**, `scripts/release.sh`, and push `main` to the fork (per the established flow).
