# Changelog

## v0.6.0 — 2026-07-14

- **See which voices are saved.** Speaker Identification settings now shows an empty-state
  when no voices are remembered yet, so it's clear whether the app has anything to recognize
  you by (alongside the existing rename/forget list).
- **Confirmable speaker suggestions.** When you retranscribe or import a meeting and a
  speaker *nearly* matches a saved voice — close, but not confident enough to auto-name —
  the transcript shows a subtle hint like **"Speaker 2 · Alice?"**. Click it to open the
  rename dialog pre-filled with that name; one confirm applies it and reinforces the voice.
  The app still never auto-labels on a weak match (no silent mis-attribution), and confirming
  one suggestion no longer clears the others.

## v0.5.9 — 2026-07-14

- **Fix: summaries now know who said what.** The built-in summary was sending the LLM the
  transcript without speaker labels, so summaries referred to "unnamed speaker." The
  speaker names are now included in the transcript handed to the model (matching the
  Workflows behavior), so summaries can attribute points and action items to the right people.

## v0.5.8 — 2026-07-14

- **Fix: re-attributing a speaker no longer loses the voice.** Correcting a speaker you'd
  already named (or the app had auto-named) used to warn "Voice data was not available for
  this meeting" and skip saving the voice, because the meeting's stored voice data wasn't
  kept in sync with the renamed labels. Renaming now updates that stored voice data in
  lockstep, so corrections keep working and the voice is remembered. (Meetings you renamed
  before this update stay as-is; a fresh retranscribe re-syncs them.)

## v0.5.7 — 2026-07-14

- **"Remember this voice" is on by default.** When you name a speaker, the app now
  remembers that voice for future meetings unless you deliberately turn it off — so you
  no longer have to remember to tick the box for recognition to build up.
- **No accidental duplicate/merge when you type an existing name.** If you type a name
  that already exists (a saved voice or someone already named in this meeting), the app
  asks whether it's the same person: choose "use existing" to merge/reinforce that voice,
  or go back and pick a more specific name for a different person. Selecting someone from
  the picklist still works as before.

## v0.5.6 — 2026-07-11

- **Speakers are only auto-named when we're sure.** Cross-meeting voice recognition no
  longer guesses: a detected speaker is auto-labeled with a saved voice only when that
  voice is a clear, confident match (well ahead of any other candidate). When it's weak
  or ambiguous, the speaker stays "Speaker N" for you to name — so a returning voice is
  recognized, but the app won't confidently mislabel someone.
- **Naming a voice strengthens it over time.** Each time you rename a speaker to a name
  you've already saved (with "Remember this voice"), that profile is reinforced with the
  new sample instead of creating a duplicate — so recognition improves the more you use it.

## v0.5.5 — 2026-07-10

- **Renaming a speaker no longer mislabels everyone else.** Naming a speaker now
  relabels only that speaker; it no longer auto-sweeps other detected speakers into
  the same name (which could cascade and was hard to correct). To combine two detected
  speakers you know are the same person, use the new picklist (below).
- **Pick from people you've already named.** The rename dialog now lets you select a
  person you've already added — a saved voice profile or someone already named in this
  meeting — instead of only typing into an empty box. Picking an existing name is the
  deliberate, safe way to say "this speaker is the same person as that one."

## v0.5.4 — 2026-07-10

- **Much better speaker separation on Retranscribe & Import.** Speaker labeling on the
  batch paths no longer collapses a back-and-forth conversation into one speaker. When
  speaker identification is enabled, an on-device neural diarizer (pyannote segmentation
  via a bundled `speakrs` sidecar) detects real speaker turns, and each turn is
  transcribed and labeled separately — so two people talking now show as distinct
  speakers. Cross-meeting voice profiles ("Me" and named voices) still apply, and per-meeting
  centroids are saved so rename + "remember this voice" works on retranscribed/imported
  meetings. Fully local (audio never leaves your machine); the first run downloads the
  segmentation models once. Best-effort: with the feature off or the sidecar unavailable,
  Retranscribe/Import behave exactly as before. (Live recording is unchanged for now.)
- **Rename cleans up over-splitting.** Naming a speaker now also re-checks the meeting's
  other detected speakers and merges any that match that voice — so a person accidentally
  split across two labels becomes one when you name them.

## v0.5.3 — 2026-07-10

- **Retroactive speaker diarization.** Speaker identification now runs on the two
  batch paths, not just live recordings: **Retranscribe** an existing meeting or
  **Import** an audio file and — when speaker identification is enabled and its model
  is downloaded — each segment is labeled (`Speaker N`, or a matched saved-profile name
  like "Me"), and per-meeting voice centroids are saved so the rename + "remember this
  voice" flow works on those meetings too. Meetings recorded before the feature existed
  can now be labeled by simply retranscribing them. Fully best-effort: with the feature
  off or the model absent, retranscribe/import behave exactly as before.

## v0.5.2 — 2026-07-09

- **Selectable NeoHive connection auth.** Beyond the existing Cloudflare Access service
  token, you can now connect a NeoHive instance via Bearer token / API key, Basic auth,
  a custom header, or "None (network-level)" — the last for instances reached over
  Tailscale / LAN / VPN with no app credentials. Choose the method under
  Settings → Workflows → NeoHive. Existing Cloudflare setups are migrated automatically.

## v0.5.1 — 2026-07-09

Fix: the **Speaker identification** toggle was mounted in an unused settings component and
never appeared on the actual Settings screen. It now shows under **Settings → Transcription**.
(Also validates the fork's in-app auto-update path from v0.5.0.)

## v0.5.0 — 2026-07-09

First release of the personal fork's own update channel — auto-update now points at
`Nader-Awad/meetily` with a fork-owned signing key. Features since v0.4.0:

- **Meeting workflows:** saved, named summarization recipes that run over OpenRouter /
  Ollama / Claude / OpenAI / Groq, with opt-in section-by-section export to a NeoHive instance.
- **Voice-profile speaker identification:** local, on-device diarization (WeSpeaker CAM++
  embeddings via ONNX Runtime, with a macOS CoreML execution provider), speaker chips in the
  live and saved transcript views, rename + "remember this voice" profiles (including self-labeling
  "Me"), and speaker attribution fed into both the built-in summary and workflow runs. Off by
  default; enabling downloads the ~28 MB model on demand.

> Installing v0.5.0 requires a one-time manual install (see `RELEASING.md` § "The one-time
> cutover"). Every version after it auto-updates in-app.
