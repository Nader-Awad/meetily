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

# --- Locate artifacts by glob (Cargo WORKSPACE → bundles land under repo-root target/;
#     path also varies with --target, hence the glob) ---
BUNDLE_ROOT="$ROOT/target"
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
# Annotated + messaged so it works under tag.gpgsign/forceSignAnnotated (a plain
# lightweight tag errors "no tag message?" when signing is forced).
git -C "$ROOT" tag -a "$TAG" -m "Release $TAG"
# Push the tag (and the commits it references) so the release's commit exists on
# GitHub — origin/main is intentionally behind (personal fork), so gh release create
# needs the tag ref present remotely first.
echo "▶ Pushing tag $TAG to ${REPO}..."
git -C "$ROOT" push origin "$TAG"
echo "▶ Publishing GitHub Release $TAG on ${REPO}..."
gh release create "$TAG" --repo "$REPO" --title "$TAG" --notes-file "$ROOT/CHANGELOG.md" \
  "$DMG" "$APP_TARGZ" "$APP_SIG" "$LATEST_JSON"
echo "✅ Released $TAG. Verify: curl -sL https://github.com/$REPO/releases/latest/download/latest.json | head"
