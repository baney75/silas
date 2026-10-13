#!/usr/bin/env bash
# Release a stable Silas for macOS (Apple silicon) from this Mac.
# ONNX Runtime ships no prebuilt x86_64-apple-darwin library, so there is no Intel build.
#
#   scripts/release-stable.sh 0.2.0 "What changed, in a sentence or two."
#
# Stable is macOS only: it is the build Donovan tests. It is signed with the local Apple
# Development identity, so macOS keeps Silas's Microphone and Accessibility grants across
# updates, and its updates are signed with a stable-only key that never leaves this Mac.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:?usage: release-stable.sh <version> [notes]}"
notes="${2:-Silas $version}"
key="${TAURI_SIGNING_KEY_FILE:-$HOME/.tauri/silas-stable.key}"
identity="${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development|Developer ID Application/ {print $2; exit}')}"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 1.2.3" >&2; exit 1; }
[ -f "$key" ] || { echo "missing stable updater key $key" >&2; exit 1; }
[ -n "$identity" ] || { echo "no code-signing identity found" >&2; exit 1; }
[ "$(git branch --show-current)" = main ] || { echo "release from main" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit your changes first" >&2; exit 1; }
git fetch -q origin main --tags
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "main is not in sync with origin" >&2; exit 1; }
! git rev-parse -q --verify "refs/tags/v$version" >/dev/null || { echo "v$version already exists" >&2; exit 1; }

# Put the version files back if anything below fails.
restore() { git checkout -q -- src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock 2>/dev/null || true; }
trap 'status=$?; [ $status -eq 0 ] || restore' EXIT

jq --arg v "$version" '.version = $v' src-tauri/tauri.conf.json > /tmp/silas-conf.json && mv /tmp/silas-conf.json src-tauri/tauri.conf.json
sed -i '' -E "1,10s/^version = \".*\"/version = \"$version\"/" src-tauri/Cargo.toml
(cd src-tauri && cargo test --lib --quiet && cargo clippy --lib --quiet -- -D warnings)

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$key")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" APPLE_SIGNING_IDENTITY="$identity"
rustup target add aarch64-apple-darwin >/dev/null
bunx tauri build --target aarch64-apple-darwin --bundles app

out=src-tauri/target/aarch64-apple-darwin/release/bundle
dir=$(mktemp -d)
cp "$out/macos/Silas.app.tar.gz" "$dir/Silas_aarch64.app.tar.gz"
cp "$out/macos/Silas.app.tar.gz.sig" "$dir/Silas_aarch64.app.tar.gz.sig"
# A plain disk image: Tauri's styled DMG drives Finder through AppleScript, which fails headless.
mkdir -p "$dir/dmg" && cp -R "$out/macos/Silas.app" "$dir/dmg/" && ln -s /Applications "$dir/dmg/Applications"
hdiutil create -quiet -volname Silas -srcfolder "$dir/dmg" -ov -format UDZO "$dir/Silas_aarch64.dmg"
url="https://github.com/baney75/silas/releases/download/v$version/Silas_aarch64.app.tar.gz"
jq -n --arg v "$version" --arg n "$notes" --arg d "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg s "$(cat "$dir/Silas_aarch64.app.tar.gz.sig")" --arg u "$url" \
  '{version:$v, notes:$n, pub_date:$d, platforms:{"darwin-aarch64":{signature:$s,url:$u}}}' > "$dir/latest.json"

git diff --quiet || git commit -qam "Release Silas $version"
git push -q origin main
# Upload into a draft (which creates no tag), retrying slow uploads, and publish only
# once every file is there, so the update feed never points at a missing file.
gh release create "v$version" --draft --target "$(git rev-parse HEAD)" --title "Silas $version" --notes "$notes" >/dev/null
for f in Silas_aarch64.dmg Silas_aarch64.app.tar.gz Silas_aarch64.app.tar.gz.sig latest.json; do
  for attempt in 1 2 3 4 5; do
    gh release upload "v$version" "$dir/$f" --clobber && break
    [ "$attempt" -lt 5 ] || { echo "upload of $f failed; the draft v$version is still there" >&2; exit 1; }
    sleep $((attempt * 5))
  done
done
gh release edit "v$version" --draft=false --latest >/dev/null
git fetch -q origin --tags
echo "Released Silas $version (stable, macOS)."
