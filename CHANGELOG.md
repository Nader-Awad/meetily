# Changelog

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
