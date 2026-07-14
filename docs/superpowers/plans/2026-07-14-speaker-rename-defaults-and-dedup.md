# Speaker-Rename UX (v0.5.7) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the speaker rename dialog, default "Remember this voice" ON, and confirm before renaming to a name that already exists (typed collision) so the wrong voice profile isn't silently reinforced.

**Architecture:** Frontend-only change to `SpeakerRenameDialog.tsx`. Add two bits of state (`pickedFromList`, `pendingDuplicate`); the Rename action guards TYPED names against a case-insensitive collision with existing people and shows an in-dialog confirmation; "use existing" sends the canonical name (reuses the backend accrue-on-name-match), "different person" returns to editing. No backend change.

**Tech Stack:** Next.js/React + TypeScript, Tauri `invoke`.

## Global Constraints

- Frontend-only — modify ONLY `frontend/src/components/SpeakerRenameDialog.tsx`. No backend/Rust change, no new dependency.
- The confirm fires only for TYPED names, NOT picklist selections (a chip click is an explicit "same person").
- Collision detection is case-insensitive; "use existing" sends the CANONICAL existing name.
- Gate = `cd frontend && npx tsc --noEmit` with NO new errors (a pre-existing `bun:test` tsc error is unrelated; there is no frontend unit-test runner). `pnpm lint`/`next lint` is broken repo-wide — do not rely on it.
- Gitmoji conventional commit; NO `Co-Authored-By` / NO AI-agent mention. Local `main` only; do not push during implementation.

---

### Task 1: Remember-by-default + typed-duplicate confirmation

**Files:**
- Modify: `frontend/src/components/SpeakerRenameDialog.tsx` (full replacement below)

**Interfaces:**
- Consumes: existing `diarization_list_profiles` command + `existingNames` prop (both already present). No new props, no signature change.
- Produces: no exported-interface change; purely internal dialog behavior.

- [ ] **Step 1: Replace the file with the version below.** This changes `saveProfile` default to `true`, adds `pickedFromList` + `pendingDuplicate` state, wires the pick-vs-type flag on the chips + input, and adds the confirmation view + collision guard. Everything else (candidates computation, toasts, layout) is unchanged.

```tsx
'use client';

// Rename a detected speaker ("Speaker 1" → "Alice") across all segments of a
// meeting. Remembers the voice as a persistent profile by DEFAULT (opt out per
// rename) so future recordings label this speaker automatically. When a TYPED
// name collides with an existing person, confirms before merging so the wrong
// voice profile isn't silently reinforced.

import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Label } from './ui/label';
import { toast } from 'sonner';

interface SpeakerRenameDialogProps {
  meetingId: string;
  speakerLabel: string;
  existingNames?: string[];
  onClose: () => void;
  onRenamed: () => void | Promise<void>;
}

interface RenameResult {
  updated_segments: number;
  profile_saved: boolean;
}

export function SpeakerRenameDialog({
  meetingId,
  speakerLabel,
  existingNames,
  onClose,
  onRenamed,
}: SpeakerRenameDialogProps) {
  const [name, setName] = useState('');
  const [saveProfile, setSaveProfile] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  // Whether the current name came from clicking a picklist chip (explicit
  // "same person") vs being typed. Only typed collisions get the confirm.
  const [pickedFromList, setPickedFromList] = useState(false);
  // When set, the typed name collides with an existing person and we show a
  // confirmation instead of renaming. Holds the CANONICAL existing name.
  const [pendingDuplicate, setPendingDuplicate] = useState<string | null>(null);

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

  const doRename = async (finalName: string) => {
    setPendingDuplicate(null);
    setIsSaving(true);
    try {
      const result = await invoke<RenameResult>('diarization_rename_speaker', {
        meetingId,
        oldLabel: speakerLabel,
        newName: finalName,
        saveProfile,
      });
      toast.success(
        `Renamed ${speakerLabel} to ${finalName} (${result.updated_segments} segments)` +
          (result.profile_saved ? ' — voice remembered' : '')
      );
      if (saveProfile && !result.profile_saved) {
        toast.warning(
          'Voice data was not available for this meeting, so the profile was not saved.'
        );
      }
      await onRenamed();
    } catch (err) {
      console.error('Failed to rename speaker:', err);
      toast.error(`Failed to rename speaker: ${err}`);
    } finally {
      setIsSaving(false);
    }
  };

  const handleRename = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    // Guard TYPED names against colliding with an existing person; picking a
    // chip is already an explicit "same person" choice, so skip the guard then.
    if (!pickedFromList) {
      const match = candidates.find((c) => c.toLowerCase() === trimmed.toLowerCase());
      if (match) {
        setPendingDuplicate(match);
        return;
      }
    }
    void doRename(trimmed);
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Rename {speakerLabel}</DialogTitle>
          <DialogDescription>
            The new name is applied to every segment from this speaker in this meeting.
          </DialogDescription>
        </DialogHeader>

        {pendingDuplicate ? (
          <div className="space-y-3 py-2">
            <p className="text-sm text-gray-700">
              A speaker named <span className="font-semibold">{pendingDuplicate}</span> already
              exists. Is this the same person?
            </p>
            <div className="flex flex-col gap-2">
              <Button onClick={() => doRename(pendingDuplicate)} disabled={isSaving}>
                {isSaving ? 'Renaming…' : `Yes — use existing ${pendingDuplicate}`}
              </Button>
              <Button
                variant="outline"
                onClick={() => {
                  setPendingDuplicate(null);
                  setPickedFromList(false);
                  requestAnimationFrame(() =>
                    document.getElementById('speaker-name')?.focus()
                  );
                }}
                disabled={isSaving}
              >
                No — it&apos;s a different person
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-3 py-2">
            {candidates.length > 0 && (
              <div>
                <Label className="text-sm">Assign to someone already added</Label>
                <div className="flex flex-wrap gap-1.5 mt-1">
                  {candidates.map((c) => (
                    <button
                      key={c}
                      type="button"
                      onClick={() => {
                        setName(c);
                        setPickedFromList(true);
                      }}
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
            <div>
              <Label htmlFor="speaker-name" className="text-sm">
                Name
              </Label>
              <Input
                id="speaker-name"
                autoFocus
                value={name}
                placeholder="e.g. Alice"
                onChange={(e) => {
                  setName(e.target.value);
                  setPickedFromList(false);
                }}
                onKeyDown={(e) => e.key === 'Enter' && handleRename()}
              />
            </div>
            <label className="flex items-center gap-2 text-sm text-gray-600 cursor-pointer">
              <input
                type="checkbox"
                checked={saveProfile}
                onChange={(e) => setSaveProfile(e.target.checked)}
                className="rounded border-gray-300"
              />
              Remember this voice for future meetings (stored only on this device)
            </label>
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={isSaving}>
            Cancel
          </Button>
          {!pendingDuplicate && (
            <Button onClick={handleRename} disabled={!name.trim() || isSaving}>
              {isSaving ? 'Renaming…' : 'Rename'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

- [ ] **Step 2: Type-check.**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`
Expected: no NEW errors. (Only the known pre-existing `bun:test` module-resolution error in `tests/lib/blocknote-markdown.test.ts` may appear — that is not from this change.)

- [ ] **Step 3: Commit.**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add frontend/src/components/SpeakerRenameDialog.tsx
git commit -m "feat(speakers): :sparkles: remember voice by default + confirm on duplicate name"
```

---

## Self-review

- **Spec coverage:** §3a (default ON) → `useState(true)`. §3b (pick-vs-type) → `pickedFromList` set on chip click / cleared on input change. §3c (typed-duplicate confirm) → `handleRename` collision guard + `pendingDuplicate` view + `doRename(pendingDuplicate)` sends canonical. §4 (empty candidates → no collision possible) → `candidates.find` over an empty list returns undefined → normal rename. ✓
- **Placeholder scan:** none — the full file is provided. ✓
- **Type consistency:** `doRename(finalName: string)`, `pendingDuplicate: string | null`, `pickedFromList: boolean` used consistently; `handleRename` is now sync (dispatches `void doRename(...)`), and both the Enter key and the Rename button call it — matches. ✓
