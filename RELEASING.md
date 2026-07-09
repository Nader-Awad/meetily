# Releasing Meetily (personal fork)

This fork ships its own **macOS** auto-update channel via GitHub Releases on
`Nader-Awad/meetily`. The Tauri v2 updater (already compiled into the app) is pointed
at this fork with a fork-owned minisign key.

## One-time setup (already done)

- Signing keypair generated at `~/.meetily-release/meetily_updater.key` (private) and
  `…/meetily_updater.key.pub` (public), empty password.
  **The private key is a secret — never commit or share it.** Back it up somewhere safe
  (outside git). The matching public key is embedded in
  `frontend/src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.
- Updater endpoint: `https://github.com/Nader-Awad/meetily/releases/latest/download/latest.json`.

## Cutting a release

1. Bump the version in lockstep across all three files (they must match):
   `frontend/package.json`, `frontend/src-tauri/Cargo.toml` (`[package] version`),
   `frontend/src-tauri/tauri.conf.json` (`version`). Then sync the lockfile
   (`cd frontend/src-tauri && cargo update -p meetily --precise <version> --offline`)
   and add a `CHANGELOG.md` entry.
2. Commit the bump.
3. Run `scripts/release.sh` (or `scripts/release.sh --dry-run` to build + assemble
   `latest.json` **without** tagging/publishing). It builds macOS arm64 via
   `frontend/build-gpu.sh` with the signing key exported, locates the bundle artifacts,
   writes `latest.json` (`darwin-aarch64`), tags `vX.Y.Z`, and publishes the `.dmg`,
   `.app.tar.gz` (+ `.sig`), and `latest.json` to the fork's Releases via `gh`.
4. Verify the feed: `curl -sL https://github.com/Nader-Awad/meetily/releases/latest/download/latest.json`.

## The one-time cutover (v0.5.0 only)

Your currently-installed build carries the OLD (upstream) signing key + endpoint, so it
**cannot** auto-update to the first fork-keyed build. Install **v0.5.0 by hand**: download
the `.dmg` from the Release, open it, drag Meetily to Applications, then right-click →
**Open** once (the app is ad-hoc signed, so Gatekeeper prompts the first time). From
v0.5.0 onward, updates arrive in-app automatically.

## Key management

- Losing `~/.meetily-release/meetily_updater.key` means installed builds can no longer
  verify updates — you'd generate a new key, embed the new pubkey, release, and do
  another manual cutover install. **Back the key up** (not in git).
- To rotate:
  `pnpm tauri signer generate -w ~/.meetily-release/meetily_updater.key -p '' -f`,
  update `pubkey` in `tauri.conf.json`, release, and re-install manually once.
- If a non-empty password is used, store it at `~/.meetily-release/password.txt`
  (chmod 600); `release.sh` reads it from there. Never inline it.

## Follow-up: Linux

v0.5.0 is **macOS-only**. Linux (x86_64 AppImage) auto-update is deferred: it needs a
Dockerized build with webkit2gtk + `libappindicator3-dev librsvg2-dev patchelf
libasound2-dev libopenblas-dev libx11-dev libxtst-dev libxrandr-dev
libwebkit2gtk-4.1-dev` and `fuse libfuse2`, a Linux build of the `llama-helper` sidecar +
ONNX runtime, and the fork's `libwayland-client` AppImage fixup (see
`.github/workflows/build-linux.yml`). When added, `release.sh` gains a Docker build step
and a `linux-x86_64` entry in `latest.json` (`signature` = contents of the `.AppImage.sig`,
`url` = the `.AppImage`). Confirm the exact Tauri v2 Linux updater artifact name against
the actual `bundle/appimage/` output at that time.
