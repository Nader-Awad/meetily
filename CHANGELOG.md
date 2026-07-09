# Changelog

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
