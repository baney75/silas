#!/usr/bin/env bash
# Release a stable Silas for macOS from this Mac.
#
#   scripts/release-stable.sh 0.2.0 "What changed, in a sentence or two."
#
# Stable is macOS only: it is the build Donovan tests. It is signed with the local Apple
# Development identity, so macOS keeps Silas's Microphone and Accessibility grants across
# updates (CI nightlies are ad-hoc signed and must be re-granted after each update).
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:?usage: release-stable.sh <version> [notes]}"
notes="${2:-Silas $version}"
key="${TAURI_SIGNING_KEY_FILE:-$HOME/.tauri/silas.key}"
identity="${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development|Developer ID Application/ {print $2; exit}')}"
[ -f "$key" ] || { echo "missing updater key $key" >&2; exit 1; }
[ -n "$identity" ] || { echo "no code-signing identity found" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit your changes first" >&2; exit 1; }

jq --arg v "$version" '.version = $v' src-tauri/tauri.conf.json > /tmp/silas-conf.json && mv /tmp/silas-conf.json src-tauri/tauri.conf.json
sed -i '' -E "s/^version = \".*\"/version = \"$version\"/" src-tauri/Cargo.toml
(cd src-tauri && cargo test --lib --quiet)

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$key")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" APPLE_SIGNING_IDENTITY="$identity"
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
bunx tauri build --target universal-apple-darwin --bundles app,dmg

out=src-tauri/target/universal-apple-darwin/release/bundle
tarball="$out/macos/Silas.app.tar.gz"
dmg=$(ls "$out"/dmg/*.dmg | head -1)
sig=$(cat "$tarball.sig")
url="https://github.com/baney75/silas/releases/download/v$version/Silas_universal.app.tar.gz"
cp "$tarball" "$out/macos/Silas_universal.app.tar.gz"
jq -n --arg v "$version" --arg n "$notes" --arg d "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg s "$sig" --arg u "$url" \
  '{version:$v, notes:$n, pub_date:$d, platforms:{"darwin-aarch64":{signature:$s,url:$u}, "darwin-x86_64":{signature:$s,url:$u}}}' > "$out/latest.json"

git commit -qam "Release Silas $version"
git tag "v$version"
git push -q origin HEAD "v$version"
gh release create "v$version" --latest --title "Silas $version" --notes "$notes" \
  "$dmg#Silas_${version}_universal.dmg" "$out/macos/Silas_universal.app.tar.gz" "$out/latest.json"
echo "Released Silas $version (stable, macOS)."
