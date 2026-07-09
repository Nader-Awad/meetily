# Fork Release & Auto-Update Channel — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (recommended for this plan — it involves a secret keypair and outward-facing publish steps that must not be delegated to subagents) or superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Turn the personal fork `Nader-Awad/meetily` into its own **macOS** auto-update channel and give it a repeatable one-command local release process, then ship the current `main` (which already contains the workflows + diarization features) as **v0.5.0**. After a one-time manual cutover install of v0.5.0, every later version auto-updates in-app.

**Architecture:** Repoint the already-compiled Tauri v2 updater from upstream to the fork: replace the embedded minisign `pubkey` with a new keypair the user owns, and point `endpoints` at the fork's `releases/latest/download/latest.json`. A local `scripts/release.sh` bumps the version, builds macOS arm64 natively via `build-gpu.sh` with the signing key exported, assembles `latest.json`, tags, and publishes to the fork's GitHub Releases via `gh`. macOS-only in v0.5.0; Linux (Dockerized AppImage) is a documented follow-up.

**Tech Stack:** Tauri v2 (`tauri 2.6.2`, `tauri-plugin-updater 2.3.0`, `@tauri-apps/cli ^2.1.0`), minisign (via `tauri signer`), `gh` CLI (authed as Nader-Awad), bash. macOS Apple Silicon.

## Global Constraints

- **Personal local fork.** Releases publish to `Nader-Awad/meetily` ONLY. Never push code to a non-fork remote; never touch `Zackriya-Solutions`. The repo `Nader-Awad/meetily` is PUBLIC (so the updater endpoint needs no auth).
- **Credential safety (hard rule).** The minisign PRIVATE key and its password are SECRETS: never print them, never commit them, never paste them into a subagent prompt or report. They live OUTSIDE the repo at `~/.meetily-release/`. Only the PUBLIC key (safe) goes into `tauri.conf.json`. If a secret is ever echoed, warn the user to regenerate the key.
- **The cutover is manual and unavoidable.** The installed `v0.4.0` carries upstream's pubkey + endpoint, so it cannot auto-update to a fork-keyed build. The first fork-keyed build (v0.5.0) is installed by hand (open `.dmg` → drag to Applications → right-click Open once for the ad-hoc-signing Gatekeeper prompt). Only versions AFTER it auto-update.
- **Version lockstep.** `frontend/package.json`, `frontend/src-tauri/Cargo.toml` (`[package] version`), and `frontend/src-tauri/tauri.conf.json` (`version`) must all equal the release version. `release.sh` refuses to release on a mismatch. Tags are `vX.Y.Z`.
- **macOS signing stays ad-hoc** (`bundle.macOS.signingIdentity: "-"`). Updater signing (minisign `.sig`, via `TAURI_SIGNING_PRIVATE_KEY`) is SEPARATE from macOS code-signing and is the only signing that matters for updates.
- **Do not run the actual release or publish without explicit user go-ahead.** Building and `gh release create` are outward-facing / hard-to-reverse; confirm before executing the first real release (§ First Release).
- **Commits:** gitmoji conventional commits; no AI attribution / no `Co-Authored-By`.
- **Branch:** work on `feature/release-auto-update` (already checked out); merge to local `main` at the end (no push of code).

## Resolved facts (from investigation)
- Tauri **v2**; updater artifacts already enabled (`bundle.createUpdaterArtifacts: true`). macOS updater payload = `<app>.app.tar.gz` + `.app.tar.gz.sig`; fresh-install artifact = `.dmg`.
- `build-gpu.sh` runs `NO_STRIP=true pnpm run tauri:build` and sets **no** signing env — so `release.sh` must export `TAURI_SIGNING_PRIVATE_KEY(+_PASSWORD)` around it.
- `gh` v2.96 is authenticated as `Nader-Awad`. `gh release create` marks the release "latest" by default (so `releases/latest/download/latest.json` resolves) unless `--prerelease` is passed.
- Fork is PUBLIC. Bundle artifact paths vary by whether `--target` is passed, so `release.sh` locates them by glob under `frontend/src-tauri/target/**/release/bundle/`.
- Current version is `0.4.0` in all three files; upstream pubkey + `Zackriya-Solutions/meeting-minutes/releases/latest/download/latest.json` endpoint are in `tauri.conf.json`.

## File Structure
- Create: `scripts/release.sh` — the release orchestrator (repo root; it does git + gh + calls `frontend/build-gpu.sh`).
- Create: `RELEASING.md` (repo root) — the release runbook + cutover + key management + Linux follow-up.
- Modify: `frontend/src-tauri/tauri.conf.json` — `plugins.updater.pubkey` + `endpoints`; `version`.
- Modify: `frontend/package.json` — `version`.
- Modify: `frontend/src-tauri/Cargo.toml` — `[package] version`.
- Create: `CHANGELOG.md` (repo root) if absent — v0.5.0 entry.
- Secret (OUTSIDE repo, never committed): `~/.meetily-release/meetily_updater.key` (private) + `~/.meetily-release/meetily_updater.key.pub` (public) + `~/.meetily-release/password.txt` (or empty-password convention).

---

## Task 1: Generate the fork's updater signing keypair (CONTROLLER-ONLY — handles a secret)

**This task is executed by the controller (or the user), NEVER a subagent.** It produces the minisign keypair that the updater will trust.

**Files:** none in-repo. Writes secrets to `~/.meetily-release/`.

- [ ] **Step 1: Create the secret dir + generate the keypair**

Run (from `frontend/`, so `pnpm tauri` resolves the CLI). This writes the PRIVATE key to a file outside the repo and prints the PUBLIC key. Use an empty password for a personal single-user local key (the file itself, on the user's Mac, is the security boundary); the release script then needs no password prompt:
```bash
mkdir -p ~/.meetily-release
# -w writes the private key file; empty password (-p '') for unattended local signing
pnpm tauri signer generate -w ~/.meetily-release/meetily_updater.key -p '' 2>/dev/null
chmod 600 ~/.meetily-release/meetily_updater.key
```
Tauri writes the private key to `~/.meetily-release/meetily_updater.key` and the public key to `~/.meetily-release/meetily_updater.key.pub`.

- [ ] **Step 2: Capture ONLY the public key (never print the private key)**

```bash
cat ~/.meetily-release/meetily_updater.key.pub
```
This base64 string (an `untrusted comment: minisign public key ...` block, base64-encoded by Tauri) is what goes into `tauri.conf.json` in Task 2. **Do not** `cat` the private key file or the password.

- [ ] **Step 3: Record the password convention**

Empty password → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""` in `release.sh`. If the user chose a non-empty password, store it at `~/.meetily-release/password.txt` (chmod 600) and have `release.sh` read it from there; never inline it. Verify the private key file exists and is `chmod 600`:
```bash
ls -l ~/.meetily-release/meetily_updater.key   # expect -rw------- ; do NOT print contents
```

- [ ] **Step 4: (no commit — nothing in-repo changed).** The public key string is carried to Task 2.

---

## Task 2: Version bump to 0.5.0 + repoint the updater to the fork

**Files:**
- Modify: `frontend/src-tauri/tauri.conf.json`
- Modify: `frontend/package.json`
- Modify: `frontend/src-tauri/Cargo.toml`
- Create/Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: the PUBLIC key string from Task 1.
- Produces: an app whose next build embeds the fork's pubkey + checks the fork's release feed, versioned 0.5.0.

- [ ] **Step 1: Repoint the updater block**

In `frontend/src-tauri/tauri.conf.json`, `plugins.updater`:
- Replace `pubkey` value with the Task 1 public-key string (the whole base64 blob, single line).
- Replace `endpoints` with:
```json
"endpoints": [
    "https://github.com/Nader-Awad/meetily/releases/latest/download/latest.json"
]
```
Leave `bundle.createUpdaterArtifacts: true` and `bundle.macOS.signingIdentity: "-"` unchanged.

- [ ] **Step 2: Bump the version in all three files (lockstep)**

- `frontend/src-tauri/tauri.conf.json`: `"version": "0.4.0"` → `"0.5.0"`.
- `frontend/package.json`: `"version": "0.4.0"` → `"0.5.0"`.
- `frontend/src-tauri/Cargo.toml`: `[package]` `version = "0.4.0"` → `"0.5.0"`.

- [ ] **Step 3: Add a CHANGELOG entry**

Create `CHANGELOG.md` (or prepend if it exists):
```markdown
# Changelog

## v0.5.0 — 2026-07-09
First release of the personal fork's own update channel (auto-update now points at Nader-Awad/meetily with a fork-owned signing key). Features since v0.4.0:
- Meeting workflows: saved, named summarization recipes over OpenRouter/Ollama/Claude/OpenAI/Groq, with opt-in section-by-section export to NeoHive.
- Voice-profile speaker identification: local on-device diarization (WeSpeaker CAM++ via ort + macOS CoreML), speaker chips, rename + "remember this voice", and speaker attribution fed into summaries + workflows.

> Installing v0.5.0 requires a one-time manual install (see RELEASING.md § Cutover); every version after it auto-updates.
```

- [ ] **Step 4: Verify version lockstep + that the endpoint/pubkey are no longer upstream**

```bash
cd /Users/naderawad/PersonalProjects/meetily
grep -H '"version"' frontend/package.json frontend/src-tauri/tauri.conf.json | grep 0.5.0
grep '^version' frontend/src-tauri/Cargo.toml | head -1
grep -A3 '"updater"' frontend/src-tauri/tauri.conf.json | grep -E 'Nader-Awad|pubkey'
grep -c 'Zackriya-Solutions' frontend/src-tauri/tauri.conf.json   # expect 0
```
Expected: all three versions read 0.5.0; endpoint is the fork; zero `Zackriya-Solutions` references.

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/tauri.conf.json frontend/package.json frontend/src-tauri/Cargo.toml CHANGELOG.md
git commit -m "release(app): :bookmark: v0.5.0 + repoint updater to fork with fork-owned signing key"
```

---

## Task 3: `scripts/release.sh` — one-command macOS release

**Files:**
- Create: `scripts/release.sh` (repo root; `chmod +x`)

**Interfaces:**
- Consumes: `~/.meetily-release/meetily_updater.key` (+ optional password file), `frontend/build-gpu.sh`, `gh`.
- Produces: a tagged GitHub Release on the fork with `.dmg` + `.app.tar.gz` + `.app.tar.gz.sig` + `latest.json` (platform `darwin-aarch64`).

- [ ] **Step 1: Write the script**

Create `scripts/release.sh` with exactly this content:
```bash
#!/usr/bin/env bash
# Local macOS release for the personal fork's auto-update channel.
# Usage: scripts/release.sh [--dry-run]
# Requires: ~/.meetily-release/meetily_updater.key (minisign private key, git-ignored, outside repo).
set -euo pipefail

REPO="Nader-Awad/meetily"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FRONTEND="$ROOT/frontend"
KEY="$HOME/.meetily-release/meetily_updater.key"
PWFILE="$HOME/.meetily-release/password.txt"
DRY_RUN="${1:-}"

fail() { echo "❌ $*" >&2; exit 1; }

# --- Guards ---
[ -f "$KEY" ] || fail "Signing key not found at $KEY (run: pnpm tauri signer generate -w $KEY -p '')"
command -v gh >/dev/null || fail "gh CLI not found"
gh auth status >/dev/null 2>&1 || fail "gh not authenticated"
[ -z "$(git -C "$ROOT" status --porcelain)" ] || fail "Working tree not clean — commit or stash first"
BRANCH="$(git -C "$ROOT" branch --show-current)"
[ "$BRANCH" = "main" ] || echo "⚠️  On branch '$BRANCH' (not main) — releasing anyway per request"

# --- Version lockstep ---
PKG_V=$(grep '"version"' "$FRONTEND/package.json" | head -1 | sed -E 's/.*"version" *: *"([^"]+)".*/\1/')
CONF_V=$(grep '"version"' "$FRONTEND/src-tauri/tauri.conf.json" | head -1 | sed -E 's/.*"version" *: *"([^"]+)".*/\1/')
CARGO_V=$(grep -E '^version' "$FRONTEND/src-tauri/Cargo.toml" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
[ "$PKG_V" = "$CONF_V" ] && [ "$CONF_V" = "$CARGO_V" ] || fail "Version mismatch: package=$PKG_V conf=$CONF_V cargo=$CARGO_V"
VERSION="$PKG_V"
TAG="v$VERSION"
echo "▶ Releasing $TAG"
git -C "$ROOT" rev-parse "$TAG" >/dev/null 2>&1 && fail "Tag $TAG already exists"

# --- Build (macOS arm64) with updater signing enabled ---
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$( [ -f "$PWFILE" ] && cat "$PWFILE" || echo '' )"
echo "▶ Building (this compiles the llama-helper sidecar + the Tauri app; several minutes)…"
( cd "$FRONTEND" && ./build-gpu.sh )
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD

# --- Locate artifacts by glob (path varies with --target) ---
BUNDLE_ROOT="$FRONTEND/src-tauri/target"
APP_TARGZ=$(find "$BUNDLE_ROOT" -path '*/release/bundle/macos/*.app.tar.gz' | head -1)
APP_SIG=$(find "$BUNDLE_ROOT" -path '*/release/bundle/macos/*.app.tar.gz.sig' | head -1)
DMG=$(find "$BUNDLE_ROOT" -path '*/release/bundle/dmg/*.dmg' | head -1)
[ -f "$APP_TARGZ" ] || fail "No .app.tar.gz updater artifact found (is createUpdaterArtifacts true + signing key set?)"
[ -f "$APP_SIG" ] || fail "No .app.tar.gz.sig found — signing did not run"
[ -f "$DMG" ] || fail "No .dmg found"
APP_TARGZ_NAME=$(basename "$APP_TARGZ")
echo "▶ Artifacts: $(basename "$DMG"), $APP_TARGZ_NAME (+ .sig)"

# --- Assemble latest.json (Tauri v2 schema; signature = CONTENTS of the .sig) ---
SIG_CONTENT=$(cat "$APP_SIG")
PUB_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
DL="https://github.com/$REPO/releases/download/$TAG"
LATEST_JSON="$ROOT/latest.json"
cat > "$LATEST_JSON" <<JSON
{
  "version": "$VERSION",
  "notes": "See the release page: https://github.com/$REPO/releases/tag/$TAG",
  "pub_date": "$PUB_DATE",
  "platforms": {
    "darwin-aarch64": {
      "signature": "$SIG_CONTENT",
      "url": "$DL/$APP_TARGZ_NAME"
    }
  }
}
JSON
echo "▶ Wrote latest.json (darwin-aarch64)"

if [ "$DRY_RUN" = "--dry-run" ]; then
  echo "✅ Dry run complete. Artifacts + latest.json ready; NOT tagging or publishing."
  echo "   latest.json: $LATEST_JSON"
  exit 0
fi

# --- Tag + publish ---
git -C "$ROOT" tag "$TAG"
echo "▶ Publishing GitHub Release $TAG on $REPO…"
gh release create "$TAG" --repo "$REPO" --title "$TAG" --notes-file "$ROOT/CHANGELOG.md" \
  "$DMG" "$APP_TARGZ" "$APP_SIG" "$LATEST_JSON"
echo "✅ Released $TAG. Verify: curl -sL https://github.com/$REPO/releases/latest/download/latest.json | head"
```

- [ ] **Step 2: Make it executable + shellcheck-sanity**

```bash
chmod +x scripts/release.sh
bash -n scripts/release.sh && echo "syntax OK"
```
Expected: `syntax OK` (parse check; the script is not run here — running it performs a real build/publish, gated to the § First Release step).

- [ ] **Step 3: Commit**

```bash
git add scripts/release.sh
git commit -m "build(release): :hammer: add local macOS release script (build, sign, latest.json, gh publish)"
```

---

## Task 4: `RELEASING.md` runbook + secret-safety guard

**Files:**
- Create: `RELEASING.md` (repo root)
- Modify: `.gitignore` (root) — defensive guard

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces: the human runbook for cutting releases, the one-time cutover, key management/rotation, and the Linux follow-up.

- [ ] **Step 1: Defensive .gitignore guard**

Keys live at `~/.meetily-release/` (outside the repo), so they cannot be committed. As belt-and-suspenders, ensure the repo never accidentally tracks a key or the generated manifest. Append to the root `.gitignore` (check they aren't already present first):
```
# Updater signing material must never be committed (keys live in ~/.meetily-release/)
*.key
*.key.pub
/latest.json
```
(`latest.json` is a generated release artifact; it's uploaded to the Release, not committed.)

- [ ] **Step 2: Write `RELEASING.md`**

Create `RELEASING.md`:
```markdown
# Releasing Meetily (personal fork)

This fork ships its own **macOS** auto-update channel via GitHub Releases on
`Nader-Awad/meetily`. The Tauri updater (already compiled in) is pointed at this
fork with a fork-owned minisign key.

## One-time setup (already done)
- Signing keypair generated at `~/.meetily-release/meetily_updater.key` (+ `.pub`),
  empty password. **The private key is a secret — never commit or share it.**
  The matching public key is embedded in `frontend/src-tauri/tauri.conf.json`.
- Updater endpoint: `https://github.com/Nader-Awad/meetily/releases/latest/download/latest.json`.

## Cutting a release
1. Bump the version in lockstep: `frontend/package.json`, `frontend/src-tauri/Cargo.toml`,
   `frontend/src-tauri/tauri.conf.json` (all three must match), and add a `CHANGELOG.md` entry.
2. Commit the bump.
3. Run: `scripts/release.sh` (or `scripts/release.sh --dry-run` to build + assemble
   `latest.json` without tagging/publishing). It builds macOS arm64 via `build-gpu.sh`
   with the signing key exported, then tags `vX.Y.Z` and publishes the `.dmg`,
   `.app.tar.gz`(+`.sig`), and `latest.json` to the fork's Releases.
4. Verify: `curl -sL https://github.com/Nader-Awad/meetily/releases/latest/download/latest.json`.

## The one-time cutover (v0.5.0 only)
Your currently-installed build carries the OLD (upstream) key + endpoint, so it
**cannot** auto-update to the first fork-keyed build. Install v0.5.0 **by hand**:
open the `.dmg`, drag Meetily to Applications, then right-click → **Open** once
(the app is ad-hoc signed, so Gatekeeper asks the first time). From v0.5.0 onward,
updates arrive in-app automatically.

## Key management
- Losing `~/.meetily-release/meetily_updater.key` means installed builds can no
  longer verify updates — you'd generate a new key, embed the new pubkey, and do
  another manual cutover install. Back the key up somewhere safe (not in git).
- To rotate: `pnpm tauri signer generate -w ~/.meetily-release/meetily_updater.key -p ''`,
  update `pubkey` in `tauri.conf.json`, release, and re-install manually once.

## Follow-up: Linux
v0.5.0 is macOS-only. Linux (x86_64 AppImage) auto-update is deferred: it needs a
Dockerized build (webkit2gtk + `libappindicator3-dev librsvg2-dev patchelf
libasound2-dev libopenblas-dev libx11-dev libxtst-dev libxrandr-dev
libwebkit2gtk-4.1-dev` + `fuse libfuse2`, a Linux `llama-helper` + ONNX runtime,
and the fork's `libwayland-client` AppImage fixup from `.github/workflows/build-linux.yml`).
When added, `release.sh` gains a Docker build step and a `linux-x86_64` entry in
`latest.json` (`signature` = contents of the `.AppImage.sig`, `url` = the `.AppImage`).
```

- [ ] **Step 3: Verify no secret is staged, then commit**

```bash
cd /Users/naderawad/PersonalProjects/meetily
git add RELEASING.md .gitignore
git status --porcelain | grep -iE '\.key|password' && echo "!!! SECRET STAGED — unstage" || echo "no secrets staged"
git commit -m "docs(release): :memo: add RELEASING runbook + gitignore guard for signing keys"
```

---

## First Release (v0.5.0) — collaborative, run AFTER Tasks 1–4 + user go-ahead

This performs the real build + publish + cutover. **Do not run without explicit user confirmation** (it builds for minutes and publishes an outward-facing release).

- [ ] **Step 1: Dry run** — `scripts/release.sh --dry-run` → confirms the build produces a signed `.app.tar.gz` + `.sig` + `.dmg` and a well-formed `latest.json`, without tagging/publishing.
- [ ] **Step 2: Publish** — `scripts/release.sh` → tags `v0.5.0` and creates the GitHub Release. (User confirms; controller may run it since `gh` is authed, but treat publishing as outward-facing.)
- [ ] **Step 3: Verify the feed** — `curl -sL https://github.com/Nader-Awad/meetily/releases/latest/download/latest.json` parses and lists `darwin-aarch64` with a signature + the `.app.tar.gz` URL.
- [ ] **Step 4: Cutover install** — user opens the `.dmg` from the Release, drags to Applications, right-click → Open (Gatekeeper once). Now running a fork-keyed v0.5.0.
- [ ] **Step 5: Prove auto-update (the dry-run validation from the spec)** — bump to `v0.5.1` (trivial CHANGELOG/no-op change), run `scripts/release.sh`, and confirm the installed v0.5.0 detects + installs v0.5.1 in-app. This proves the channel end-to-end.

---

## Self-Review

**Spec coverage** (design §1–§11):
- §4 update host = fork Releases; endpoint repoint → Task 2. ✓
- §4 own minisign keypair; private key git-ignored outside repo, public in config → Tasks 1, 2. ✓
- §4/§5 local macOS build via build-gpu.sh + signing env + artifact glob + latest.json + gh publish → Task 3. ✓
- §3 the pubkey-cutover constraint + manual first install → RELEASING.md + First Release Step 4. ✓
- §6 version lockstep + tags → Task 2 + release.sh guard. ✓
- §7 cutover + §5 validation dry-run (v0.5.0 → v0.5.1) → First Release Steps 4–5. ✓
- §8 secrets never committed/printed; key outside repo; .gitignore guard; ad-hoc signing caveat → Tasks 1, 4 + Global Constraints. ✓
- §9 testing = release.sh `bash -n` + `--dry-run` + the manual end-to-end cutover/auto-update → Task 3 Step 2 + First Release. ✓
- §10 open items resolved: Tauri v2 macOS artifact names known; build-gpu.sh signing wiring (export env around it) → Task 3; gh latest semantics (default) → Task 3; endpoint URL pattern → Task 2/3. Linux artifact format + Docker recipe → explicitly DEFERRED (macOS-only v0.5.0) and documented in RELEASING.md. Gatekeeper-on-auto-update → to observe during First Release Step 5. ✓
- §11 conventions → Global Constraints. ✓

**Placeholder scan:** none — `release.sh` is complete; config edits are exact; the only value carried between tasks is the Task 1 public key (a real value produced at execution).

**Type/name consistency:** `~/.meetily-release/meetily_updater.key`(+`.pub`), the `darwin-aarch64` platform key, `TAURI_SIGNING_PRIVATE_KEY(+_PASSWORD)`, and the `releases/latest/download/latest.json` endpoint are used identically across Tasks 1–4 and the First Release section.

**Scope note:** Linux is intentionally out of v0.5.0 (user decision 2026-07-09: "macOS now, Linux later"); it is captured as a concrete follow-up in RELEASING.md rather than dropped.
```