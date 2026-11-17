#!/usr/bin/env bash
# Finish a stable Silas release on this Mac (docs/channels.md).
# ONNX Runtime ships no prebuilt x86_64-apple-darwin library, so there is no Intel build.
#
#   gh workflow run promote.yml --ref stable -f nightly=nightly-YYYYMMDDHHMMSS   # first, anywhere
#   git fetch origin --tags && git checkout v0.2.0
#   scripts/release-stable.sh 0.2.0 "What changed, in a sentence or two."
#
# promote.yml moves `stable`, tags vX.Y.Z and builds the Windows and Linux bundles into a draft
# release for that tag, unsigned for the updater. This script builds and signs the Mac app from the
# same tag, signs every updater bundle with the stable key, writes one latest.json for every
# platform, uploads, and publishes the draft as the latest release.
#
# The Mac app is signed with a stable identity, so macOS keeps Silas's Microphone and Accessibility
# grants across updates as long as the designated requirement stays the same (docs/signing.md).
# It picks, in order:
#   1. APPLE_SIGNING_IDENTITY (an explicit override)
#   2. a Developer ID Application identity in the keychain
#   3. Silas Local Signing (scripts/signing-identity.sh; SILAS_LOCAL_SIGNING=0 skips it)
#   4. an Apple Development identity
# Updates are signed with a stable-only key that never leaves this Mac.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:?usage: release-stable.sh <version> [notes]}"
notes="${2:-Silas $version}"
tag="v$version"
key="${TAURI_SIGNING_KEY_FILE:-$HOME/.tauri/silas-stable.key}"
REPO=baney75/silas
find_identity() { security find-identity -v -p codesigning | awk -F'"' -v kind="$1" 'index($2, kind) == 1 {print $2; exit}'; }

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 1.2.3" >&2; exit 1; }
[ -f "$key" ] || { echo "missing stable updater key $key" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit your changes first" >&2; exit 1; }
git fetch -q origin stable --tags
git rev-parse -q --verify "refs/tags/$tag" >/dev/null || { echo "$tag does not exist; run promote.yml on the stable ref first" >&2; exit 1; }
[ "$(git rev-parse "$tag^{commit}")" = "$(git rev-parse origin/stable)" ] || { echo "$tag is not stable's head" >&2; exit 1; }
[ "$(git rev-parse HEAD)" = "$(git rev-parse "$tag^{commit}")" ] || { echo "check out $tag first: git checkout $tag" >&2; exit 1; }
[ "$(gh release view "$tag" -R "$REPO" --json isDraft -q .isDraft 2>/dev/null)" = true ] ||
  { echo "no draft release for $tag; promote.yml makes it (or $tag is already published)" >&2; exit 1; }

identity="${APPLE_SIGNING_IDENTITY:-$(find_identity "Developer ID Application:")}"
if [ -z "$identity" ] && [ "${SILAS_LOCAL_SIGNING:-1}" != 0 ]; then
  # Self-signed, so `find-identity -v` hides it; codesign takes it by SHA-1.
  identity=$(bash scripts/signing-identity.sh ensure)
fi
identity="${identity:-$(find_identity "Apple Development:")}"
[ -n "$identity" ] || { echo "no code-signing identity found" >&2; exit 1; }
echo "signing with $identity"

# The tag holds main's base version; the release version is stamped for the build only and
# put back afterwards, since `stable` moves only by promotion or a hotfix pull request.
restore() { git checkout -q -- src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock 2>/dev/null || true; }
trap 'status=$?; restore; [ $status -eq 0 ] || git checkout -q -- scripts/release-requirement.txt 2>/dev/null || true' EXIT

jq --arg v "$version" '.version = $v' src-tauri/tauri.conf.json > /tmp/silas-conf.json && mv /tmp/silas-conf.json src-tauri/tauri.conf.json
sed -i '' -E "1,10s/^version = \".*\"/version = \"$version\"/" src-tauri/Cargo.toml
(cd src-tauri && cargo test --lib --quiet && cargo clippy --lib --quiet -- -D warnings)

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$key")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" APPLE_SIGNING_IDENTITY="$identity"
rustup target add aarch64-apple-darwin >/dev/null
bunx tauri build --target aarch64-apple-darwin --bundles app

out=src-tauri/target/aarch64-apple-darwin/release/bundle
# macOS ties the grants to the designated requirement. If it changed since the last stable
# release (pinned in scripts/release-requirement.txt) or the installed copy, every user is asked
# again; stop unless that is intended (ALLOW_NEW_REQUIREMENT=1, which also rewrites the pin; land
# that change on main in a pull request after the release).
scripts/check-requirement.sh --update "$out/macos/Silas.app"
requirement() { codesign -d -r- "$1" 2>&1 | sed -n 's/^designated => //p'; }
new_req=$(requirement "$out/macos/Silas.app")
for installed in /Applications/Silas.app "$HOME/Applications/Silas.app"; do
  [ -d "$installed" ] || continue
  old_req=$(requirement "$installed")
  # An ad-hoc copy (a local or nightly build) holds no grants a release could keep; the pin decides.
  case "$old_req" in *'cdhash '*) echo "installed $installed is ad-hoc; checked against the pin only"; break ;; esac
  if [ "$old_req" != "$new_req" ] && [ "${ALLOW_NEW_REQUIREMENT:-}" != 1 ]; then
    echo "designated requirement changed from the installed $installed:" >&2
    echo "  was: $old_req" >&2
    echo "  now: $new_req" >&2
    echo "users would be asked for permissions again; set ALLOW_NEW_REQUIREMENT=1 if intended" >&2
    exit 1
  fi
  break
done
dir=$(mktemp -d)
cp "$out/macos/Silas.app.tar.gz" "$dir/Silas_aarch64.app.tar.gz"
cp "$out/macos/Silas.app.tar.gz.sig" "$dir/Silas_aarch64.app.tar.gz.sig"
# A plain disk image: Tauri's styled DMG drives Finder through AppleScript, which fails headless.
mkdir -p "$dir/dmg" && cp -R "$out/macos/Silas.app" "$dir/dmg/" && ln -s /Applications "$dir/dmg/Applications"
hdiutil create -quiet -volname Silas -srcfolder "$dir/dmg" -ov -format UDZO "$dir/Silas_aarch64.dmg"

# The Windows and Linux bundles CI built from the same tag. Each installer is its own updater
# bundle; sign it here with the stable key.
mkdir -p "$dir/ci"
gh release download "$tag" -R "$REPO" --dir "$dir/ci" --pattern '*.AppImage' --pattern '*.deb' --pattern '*.rpm' --pattern '*-setup.exe'
for pattern in '*.AppImage' '*.deb' '*.rpm' '*-setup.exe'; do
  [ "$(find "$dir/ci" -name "$pattern" | wc -l | tr -d ' ')" = 1 ] || { echo "the $tag draft needs exactly one $pattern; re-run promote.yml's build" >&2; exit 1; }
done
for f in "$dir"/ci/*; do
  bunx tauri signer sign "$f" >/dev/null
done

base="https://github.com/$REPO/releases/download/$tag"
platforms='{}'
add() { # platform-key file
  local name; name=$(basename "$2")
  platforms=$(jq --arg k "$1" --arg s "$(cat "$2.sig")" --arg u "$base/$name" --arg h "$(shasum -a 256 "$2" | cut -d' ' -f1)" \
    '. + {($k): {signature: $s, url: $u, sha256: $h}}' <<<"$platforms")
}
add darwin-aarch64 "$dir/Silas_aarch64.app.tar.gz"
for f in "$dir"/ci/*; do
  case "$f" in
    *.sig) ;;
    *.AppImage) add linux-x86_64 "$f"; add linux-x86_64-appimage "$f" ;;
    *.deb) add linux-x86_64-deb "$f" ;;
    *.rpm) add linux-x86_64-rpm "$f" ;;
    *-setup.exe) add windows-x86_64 "$f"; add windows-x86_64-nsis "$f" ;;
  esac
done
jq -n --arg v "$version" --arg n "$notes" --arg d "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg sha "$(git rev-parse HEAD)" --argjson p "$platforms" \
  '{product: "silas", channel: "stable", version: $v, commit: $sha, notes: $n, pub_date: $d, platforms: $p}' > "$dir/latest.json"

# Upload into the draft, retrying slow uploads, and publish only once every file is there, so the
# update feed never points at a missing file.
gh release edit "$tag" -R "$REPO" --title "Silas $version" --notes "$notes" >/dev/null
for f in "$dir"/Silas_aarch64.dmg "$dir"/Silas_aarch64.app.tar.gz "$dir"/Silas_aarch64.app.tar.gz.sig "$dir"/ci/*.sig "$dir"/latest.json; do
  for attempt in 1 2 3 4 5; do
    gh release upload "$tag" -R "$REPO" "$f" --clobber && break
    [ "$attempt" -lt 5 ] || { echo "upload of $f failed; the draft $tag is still there" >&2; exit 1; }
    sleep $((attempt * 5))
  done
done
gh release edit "$tag" -R "$REPO" --draft=false --latest >/dev/null
echo "Released Silas $version (stable: macOS, Windows, Linux)."
echo "Next: bump main's base version in a pull request (docs/channels.md)."
git diff --quiet -- scripts/release-requirement.txt ||
  echo "scripts/release-requirement.txt changed (ALLOW_NEW_REQUIREMENT=1): land it on main in a pull request."
