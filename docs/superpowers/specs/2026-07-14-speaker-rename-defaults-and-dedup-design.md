# Speaker-Rename UX: Remember-by-Default + Typed-Duplicate Confirmation — Design

- **Date:** 2026-07-14
- **Status:** Approved (design); pending implementation plan.
- **Author:** Nader Awad (with Claude)
- **Scope:** Frontend only — `frontend/src/components/SpeakerRenameDialog.tsx`. No backend change. Personal local fork. Ships as v0.5.7.
- **Motivation:** (1) Users forget to tick "Remember this voice," so voice profiles don't accrue as expected — it should default ON and be a conscious opt-out. (2) Typing a name that already exists silently merges/accrues into that profile — dangerous now that Remember defaults on, since it could reinforce the WRONG person's voice without the user realizing. The user should be asked.

## 1. Current state

`SpeakerRenameDialog.tsx` (after v0.5.5/v0.5.6): free-text `name` + a picklist of `candidates` (saved profiles ∪ this-meeting names, excluding `Speaker N` placeholders + the current label) + a `saveProfile` checkbox (`useState(false)`). `handleRename` invokes `diarization_rename_speaker({ meetingId, oldLabel, newName, saveProfile })`. Backend (v0.5.6): with `saveProfile`, it finds a profile by exact name → accrues (`accrue_centroid` + `update_embedding`) if found, else creates — so a name collision reinforces the existing profile.

## 2. Goals / non-goals

**Goals**
1. `saveProfile` defaults to ON (user consciously unchecks to skip).
2. When the user TYPES a name that collides (case-insensitive) with an existing person, confirm before renaming: use the existing person (merge/reinforce), or go back and differentiate the name.
3. "Use existing" sends the CANONICAL existing name so the backend accrues into the right profile (also closes the case-sensitivity gap).

**Non-goals**
- No backend change (the "use existing" path reuses the existing accrue-on-name-match; "different person" leads to a unique name → normal create).
- No auto-generated alternate name for the "different person" branch — the user edits it themselves.
- The confirm does NOT fire when the user PICKS a candidate from the picklist (that's already an explicit "same person").
- No change to speakrs / matching / profiles schema / other dialogs.

## 3. Design (all in `SpeakerRenameDialog.tsx`)

**3a. Remember default ON.** `const [saveProfile, setSaveProfile] = useState(true);`. Label/behavior otherwise unchanged.

**3b. Track pick-vs-type.** Add `const [pickedFromList, setPickedFromList] = useState(false);`. A picklist chip's `onClick` sets the name AND `setPickedFromList(true)`. The `<Input onChange>` sets `setPickedFromList(false)` (any manual edit means "typed"). This distinguishes an explicit picklist selection from a typed name.

**3c. Typed-duplicate confirmation.** Add `const [pendingDuplicate, setPendingDuplicate] = useState<string | null>(null);`.
- On Rename click (`handleRename`): if `!pickedFromList`, compute a case-insensitive match of `name.trim()` against `candidates` (`candidates.find(c => c.toLowerCase() === name.trim().toLowerCase())`). If a match exists AND it isn't identical to the typed string, set `pendingDuplicate = <canonical candidate>` and return early (do NOT invoke yet). Otherwise proceed to the invoke as today.
- When `pendingDuplicate` is set, render a confirmation view (replacing the form body): "A speaker named **{pendingDuplicate}** already exists. Is this the same person?" with:
  - **Use existing {pendingDuplicate}** → set `name` to the canonical `pendingDuplicate`, clear `pendingDuplicate`, and invoke the rename with `newName = pendingDuplicate` (canonical). This reinforces/merges the right profile.
  - **No — different person** → clear `pendingDuplicate` and return to the form (leave the typed name so the user can edit it to differentiate); focus the input.
  - Cancel closes the dialog.
- If the typed name exactly equals a candidate already (case + text), we still confirm (it's genuinely a duplicate) — the point is to never silently merge a typed name.

**Interaction with 3a:** because Remember now defaults ON, the confirmation is the guard that prevents accidentally reinforcing the wrong profile from a typed collision.

## 4. Error handling

The confirm is pure client-side gating before the existing `invoke`. The invoke/`try-catch`/toast flow is unchanged. If `candidates` is empty (no profiles, no named speakers), no collision is possible and behavior is exactly as today.

## 5. Testing

- `npx tsc --noEmit` clean (no new errors; a pre-existing `bun:test` tsc error is unrelated). No frontend unit-test runner exists.
- Manual: (a) open rename → "Remember" is pre-checked; (b) type a brand-new name → renames directly, no confirm; (c) type an existing name (any casing) → confirm appears → "Use existing" reinforces that profile / "different person" returns to edit; (d) PICK a chip from the picklist → renames directly, no confirm; (e) uncheck Remember + rename → no profile saved.

## 6. Files touched

- `frontend/src/components/SpeakerRenameDialog.tsx` — the three additions (default ON, pick-vs-type flag, typed-duplicate confirm view).

## 7. Conventions

- Frontend-only; no backend/Rust change; no new dependency.
- Gitmoji commits; no AI attribution. Personal fork; local `main` only. Ship v0.5.7 via `scripts/release.sh`; push `main` to the fork per the established flow.
