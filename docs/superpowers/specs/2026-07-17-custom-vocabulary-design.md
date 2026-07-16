# Custom Vocabulary (domain-term dictionary) — Design

- **Date:** 2026-07-17
- **Status:** Proposed — awaiting spec review. Direction approved in chat: engine = **Whisper (whisper.cpp)**, scope = **Approach A** (corrections + Whisper `initial_prompt` biasing + description→LLM glossary), with the Parakeet acoustic path explicitly deferred to a v2.
- **Author:** Nader Awad (with Claude)
- **Scope:** Rust/Tauri core under `frontend/src-tauri/src` (transcription + summary paths, settings) + a Next.js settings panel. No changes to the archived Python/FastAPI backend.
- **Builds on / relates to:** This is **feature B** of a three-part effort agreed in this session — **B (custom vocabulary) → Standup workflow → A (in-app template editor)**. B is a *prerequisite* for the standup: the standup summary must split work by workstream (Snyk / Logilica Platform / NeoHive), which is impossible if the transcript mangles those very terms (e.g. "Snyk" → "sneak").

## 1. Problem & motivation

The transcription engines regularly mis-hear domain-specific proper nouns — "Snyk" becomes "sneak", "NeoHive" and "Logilica" get mangled into a range of spellings. Two downstream harms:

1. The raw transcript the user reads is wrong.
2. More importantly, the summarization LLM can't reason about terms it never receives — so a standup summary can't reliably attribute work to "Snyk vs Platform vs NeoHive" when those tokens are corrupted upstream.

The user wants a **custom vocabulary / dictionary**, modelled on the one in **TypeWhisper**, where they can add many domain terms (with a short description of what each is) and have transcription respect them.

### Research input — how TypeWhisper actually does it (from its open-source code)

Reading TypeWhisper's source (github.com/TypeWhisper/typewhisper-mac) corrected two assumptions and gave us the transferable design:

- **Two entry types, two mechanisms** (`DictionaryEntryType`): **Terms** are fed *into* the ASR as biasing *when the engine supports it* (Whisper via the `prompt`/`initial_prompt` conditioning field, ~224-token budget); **Corrections** are deterministic `original → replacement` find-and-replace applied *after* transcription, engine-agnostic.
- **The per-term "description" field is cosmetic in TypeWhisper** — it is a static UI label / pack blurb, never fed to any model. TypeWhisper's LLM step receives no vocabulary (verified by grep). *We* have an LLM summarization step TypeWhisper lacks, so we can make the description **functional** as a glossary — this is a deliberate improvement, not a copy.
- **The Parakeet path is acoustic, not textual.** TypeWhisper runs a *separate ~110M CTC keyword-spotter model* over the same audio (via FluidAudio), time-aligns detected custom words to the transcript, and rescores/substitutes spans. That is the effective-but-heavy piece, and the reason Parakeet biasing exists at all despite Parakeet taking only raw samples. We are **not** building this in v1.
- **Scaling to hundreds of terms** is done by budget-clipping a flat enabled list per backend (Parakeet cap 256 terms; Whisper ~224 tokens), not embeddings/RAG. Corrections have no budget (they're just N replaces).

### What already exists (baseline — do NOT rebuild)

Verified against the current code:

- **Whisper invocation:** `whisper_engine/whisper_engine.rs::transcribe_audio_with_confidence()` (fn ~:516) builds `FullParams::new(...)` (~:526) and calls `state.full(params, &audio)` (~:585). This is the method used by the live path and both batch paths. `transcribe_audio()` (~:633/:643/:740) is the internal variant. **Neither sets any prompt today** — `set_initial_prompt` / `set_tokens` are unused across the codebase, but `whisper-rs = 0.13.2` (Cargo.toml) exposes them.
- **Live text path:** `audio/transcription/worker.rs::transcribe_chunk_with_provider()` (~:485) returns the transcript for all three engine variants; it is assigned to `TranscriptUpdate.text` (~:277-278) and emitted. A global setting is already threaded into this task the same way we'll need (`get_language_preference_internal()`, worker.rs ~:526).
- **Batch text path:** both import (`audio/import.rs`) and retranscription (`audio/retranscription.rs`) converge on `audio/common.rs::create_transcript_segments()` (~:51), where segment text is finalized/trimmed (~:61). Callers: `import.rs` ~:838, `retranscription.rs` ~:626.
- **Settings storage:** single-row `settings` table (id='1') via `database/repositories/setting.rs`. **JSON-blob precedent (our model):** `customOpenAIConfig TEXT` — migration `20251105120000_add_pro_license_custom_openai.sql`, read via `SettingsRepository::get_custom_openai_config()` (~:291) which `serde_json::from_str`s the column. Commands are registered in `lib.rs`.
- **Summary path:** `summary/processor.rs::generate_meeting_summary(...)` takes a `custom_prompt` string and a `Template`; both the built-in summary (`summary/service.rs`) and the Workflows runner (`summary/workflows/runner.rs`) call through it. This is where the glossary attaches.

## 2. Goals / non-goals

**Goals (v1)**
1. A persisted **Custom Vocabulary** with two entry types — **Terms** (biasing) and **Corrections** (`misheard → correct`) — each with an optional **description**, an enabled flag, and (corrections) a case-sensitivity flag.
2. **Layer 1 — Corrections:** deterministic, engine-agnostic post-ASR replacement applied to both the live path and the batch paths.
3. **Layer 2 — Whisper term biasing:** feed enabled terms into `set_initial_prompt`, clipped to Whisper's conditioning budget. Whisper only.
4. **Layer 3 — Description glossary:** inject `term: description` pairs into the summarization prompt so the LLM disambiguates by meaning (powers the standup's Snyk/Platform/NeoHive split). Applies to any engine (it's at summary time).
5. A **Settings UI** to manage entries, with a clear per-backend capability hint ("term biasing applies to Whisper; corrections & glossary apply everywhere").

**Non-goals (v2 / YAGNI)**
- **Parakeet acoustic CTC rescoring** (the separate keyword-spotter model + time-aligned substitution). This is the heavy R&D piece and a new model dependency; user explicitly de-prioritized broad model coverage. Parakeet users still get Layers 1 & 3 in v1.
- **Auto-learn** (silently saving a correction when the user edits a single word). Nice, but a follow-up.
- **Importable term packs / categories**, community packs, HTTP bulk-management API.
- **Embeddings / RAG / phonetic index** for term selection — budget-clipping is sufficient at hundreds of terms.
- **Per-term boost weights / similarity thresholds** — meaningless for Whisper's prompt (it has no per-term weight); belongs to the deferred Parakeet CTC path.
- **Retroactively correcting already-saved transcripts.** Corrections apply going forward and on **retranscribe** (which re-runs `create_transcript_segments`); existing saved transcripts are untouched until retranscribed.

## 3. Mental model

- A **vocabulary entry** is either a **Term** ("bias the recognizer toward this word") or a **Correction** ("whenever you see X, write Y").
- Three independent layers stack, each useful alone:
  - **Terms → Whisper `initial_prompt`** (before ASR; Whisper only).
  - **Corrections → deterministic replace** (after ASR; every engine).
  - **Descriptions → LLM glossary** (at summary time; every engine).
- A term and a correction can coexist for the same word (bias Whisper toward "Snyk" *and* replace stray "sneak"→"Snyk"), and either can carry a description used only by the glossary.

## 4. Data model

A single JSON config stored on the `settings` table, mirroring `customOpenAIConfig` exactly.

```jsonc
// column: vocabularyConfig TEXT  (serde camelCase; null/absent = feature off/empty)
{
  "enabled": true,                 // global master switch
  "entries": [
    {
      "id": "uuid",
      "entryType": "term",         // "term" | "correction"
      "text": "Snyk",              // the term; or the misheard text for a correction
      "replacement": null,          // corrections only: the correct text
      "description": "Developer-security / SAST company and product.",  // optional; feeds the glossary
      "caseSensitive": false,       // corrections matching only
      "enabled": true,
      "createdAt": "RFC3339",
      "updatedAt": "RFC3339"
    },
    {
      "id": "uuid",
      "entryType": "correction",
      "text": "sneak",
      "replacement": "Snyk",
      "description": null,
      "caseSensitive": false,
      "enabled": true,
      "createdAt": "…", "updatedAt": "…"
    }
  ]
}
```

**Storage decision (flag at review):** the spec proposes a **JSON blob** rather than a dedicated `vocabulary_entries` table. In chat I leaned toward a table, but on reflection the blob is the smaller v1 change with no runtime downside — the list is always loaded *wholesale* (to build the correction set, the prompt string, and the glossary), so there is no query benefit, and edits are infrequent and off the hot path. A dedicated table becomes worthwhile only when **auto-learn / `usageCount`** arrives (v2), at which point we migrate. If you'd rather pay that cost now, say so at review.

## 5. Layers — implementation

### Layer 1 — Corrections (post-ASR, engine-agnostic)

New pure, unit-tested module `audio/transcription/vocabulary.rs`:

```rust
pub struct Correction { pub from: String, pub to: String, pub case_sensitive: bool }

/// Deterministic, whole-word, order-stable replacement. No-op on empty input.
pub fn apply_corrections(text: &str, corrections: &[Correction]) -> String { /* … */ }

/// Parse VocabularyConfig JSON → the enabled corrections (entryType=="correction").
pub fn corrections_from_config(json: Option<&str>) -> Vec<Correction> { /* … */ }
```

- **Matching:** whole-word (word-boundary aware) to avoid corrupting substrings; case-insensitive by default, per-entry `caseSensitive` opt-in; preserve surrounding punctuation. Multi-word `text` supported.
- **Application sites (the two chokepoints that cover 100% of text):**
  - Batch: inside `create_transcript_segments` (`audio/common.rs` ~:61). Add a `&[Correction]` parameter; the two callers (`import.rs` ~:838, `retranscription.rs` ~:626) load the config once and pass it in.
  - Live: in `worker.rs`, apply to `transcript` just before building `TranscriptUpdate` (~:277). Load the config once at task start (where the diarization session is initialised).

### Layer 2 — Whisper term biasing (`initial_prompt`)

- Build a prompt string from enabled `term` entries: comma-joined, ordered (e.g. by recency/priority), **clipped to Whisper's budget** (~224 tokens ≈ ~600 chars; conservative char clip in v1).
- In `whisper_engine.rs`, after `FullParams::new(...)` at ~:526 (and ~:643), call `params.set_initial_prompt(&vocab_prompt)` when the string is non-empty.
- Thread the value in via a global getter mirroring `get_language_preference_internal()` (e.g. `get_vocabulary_prompt_internal()`), set from the persisted config at task start. This keeps the engine method signature stable and matches the existing language-preference pattern.
- **Whisper only.** For Parakeet this is a no-op (documented in the UI capability hint).
- Caveat encoded in copy: `initial_prompt` is a *soft* bias, not a guarantee — it nudges spelling; Corrections (Layer 1) remain the deterministic backstop.

### Layer 3 — Description glossary (summary LLM)

- Build a glossary block from enabled entries that have a `description`:
  ```
  Glossary of domain terms (use these exact spellings; do not confuse them):
  - Snyk: Developer-security / SAST company and product.
  - NeoHive: our cognitive-memory system.
  - Logilica: our platform.
  ```
- Attach it to the summarization prompt in `summary/processor.rs::generate_meeting_summary` (prepended to / merged with the existing `custom_prompt` assembly), so it applies to **both** the built-in summary and Workflow runs. Gated by the global `enabled` switch and non-empty glossary.
- Load the config in the two entry points that call `generate_meeting_summary` (`summary/service.rs`, `summary/workflows/runner.rs`) — or, preferably, load it once inside `generate_meeting_summary` to avoid duplication.
- **Reviewable choice:** always-on (when described terms exist) vs. an opt-in. Proposed: **always-on** under the global switch — it's low-risk and directly serves the standup. Budget: at hundreds of described terms the glossary could grow; v1 clips it to a sane char budget and logs when clipped (embeddings/selection remain a v2 non-goal).

## 6. Settings, commands, migration

- **Migration** `frontend/src-tauri/migrations/<ts>_add_vocabulary.sql`: `ALTER TABLE settings ADD COLUMN vocabularyConfig TEXT;` (timestamp sorts after existing migrations).
- **Repository** (`setting.rs`): `get_vocabulary_config()` / `save_vocabulary_config()`, mirroring `get/save_custom_openai_config` (~:291).
- **Tauri commands:** `api_get_vocabulary_config` / `api_save_vocabulary_config`; register in `lib.rs`.
- **Load-once discipline:** each consumer (batch callers, live worker task start, summary path) reads the config once per operation, not per segment/chunk.

## 7. Frontend (Next.js settings panel)

- A **"Custom Vocabulary"** panel under **Settings → Transcription** (near Speaker Identification), following the existing settings-invoke pattern (mirror `ModelSettingsModal` / `SpeakerIdentificationSettings` config read/write).
- A unified table with a **Type** column (Term / Correction), plus `text`, `replacement` (corrections), `description`, `caseSensitive`, and an enable toggle per row. Add / edit / delete rows; a global enable switch.
- **Capability hint** (mirrors TypeWhisper issue #294's lesson): "Term biasing applies to Whisper transcription. Corrections and the summary glossary apply to every engine (including Parakeet)."
- **Bulk add** (paste newline-separated terms) is a small nice-to-have; acceptable to defer to a fast-follow.
- Types mirror the Rust struct in `frontend/src/types/` (serde camelCase), same as the workflow types.

## 8. Testing

- **Rust unit tests** (pure, sub-second):
  - `apply_corrections`: whole-word only (no substring corruption), case sensitivity on/off, order stability, punctuation preserved, no-op when list empty/disabled, multi-word terms.
  - term-prompt builder: budget clipping, ordering, empty case.
  - glossary builder: skips entries without descriptions, char-budget clip, empty case.
  - `VocabularyConfig` serde round-trip (camelCase, null fields).
- **Manual e2e:** with the config populated (Snyk / NeoHive / Logilica + a "sneak"→"Snyk" correction), record a short clip and confirm (a) the live transcript shows corrected text, (b) retranscribe corrects a batch meeting, and (c) a summary uses the correct terms and the glossary.

## 9. Open questions for review

1. **Storage:** JSON blob (proposed) vs. dedicated `vocabulary_entries` table now.
2. **Glossary:** always-on under the global switch (proposed) vs. an explicit opt-in toggle.
3. **Correction matching:** confirm whole-word + case-insensitive-by-default is the right default (vs. substring).
4. **Panel placement:** Settings → Transcription (proposed) vs. its own top-level settings section.

## 10. Sequencing

v1 ships Layers 1–3 for Whisper. Parakeet acoustic biasing (the CTC keyword-spotter path) is a documented v2. Once B is in, the **Standup workflow** is unblocked (it can rely on correct Snyk/Platform/NeoHive tokens + the glossary), and the **in-app template editor (A)** proceeds independently in the background.
