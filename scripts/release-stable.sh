#!/usr/bin/env bash
# Finish a stable Silas release on this Mac (docs/channels.md).
# ONNX Runtime ships no prebuilt x86_64-apple-darwin library, so there is no Intel build.
#
#   gh workflow run promote.yml --ref main -f nightly=nightly-YYYYMMDDHHMMSS   # first, anywhere
#   git fetch origin && git checkout --detach <the commit promote.yml names>
#   scripts/release-stable.sh 0.2.0 "What changed, in a sentence or two."
#
# promote.yml checks the promotion and builds the Windows and Linux bundles into a draft release
# for vX.Y.Z whose target is the promoted commit, unsigned for the updater. This script builds and
# signs the Mac app from that commit, signs every updater bundle with the stable key, writes one
# latest.json for every platform and uploads it all into the draft. Then it fast-forwards
# `stable` to the commit, tags vX.Y.Z there and publishes the draft as the latest release. It
# pushes with your own credentials: the workflow token can't move `stable` over workflow changes.
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
# Pin the draft by id: a promotion re-run could replace it, and the files signed here must be the
# ones built from the commit this Mac builds.
drafts=$(gh api "repos/$REPO/releases" --paginate -q ".[] | select(.draft and .tag_name == \"$tag\") | [.id, .target_commitish] | @tsv")
[ -n "$drafts" ] || { echo "no draft release for $tag; promote.yml makes it (or $tag is already published)" >&2; exit 1; }
[ "$(wc -l <<<"$drafts" | tr -d ' ')" = 1 ] || { echo "more than one draft for $tag; delete the stale ones" >&2; exit 1; }
IFS=$'\t' read -r draft sha <<<"$drafts"
# shellcheck source=scripts/release-draft.sh
. scripts/release-draft.sh
same_draft() { # the draft is still there, still a draft, still for this tag and commit
  [ "$(gh api "repos/$REPO/releases/$draft" -q '[.draft, .tag_name, .target_commitish] | @tsv' 2>/dev/null)" = "$(printf 'true\t%s\t%s' "$tag" "$sha")" ] ||
    { echo "the $tag draft ($draft) changed or went away (promote.yml re-run?); start again" >&2; exit 1; }
}
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo "the $tag draft targets '$sha', not a commit; re-run promote.yml" >&2; exit 1; }
[ "$(git rev-parse HEAD)" = "$sha" ] || { echo "check out the promoted commit first: git checkout --detach $sha" >&2; exit 1; }
# A promotion fast-forwards stable; a hotfix is already stable's head.
git merge-base --is-ancestor origin/stable "$sha" || { echo "$sha does not contain stable's head; moving stable there would drop a hotfix" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  [ "$(git rev-parse "$tag^{commit}")" = "$sha" ] || { echo "$tag already exists on another commit" >&2; exit 1; }
fi

# The default speech engine (transcribe.cpp) builds from source with CMake.
command -v cmake >/dev/null || { echo "cmake is missing: the speech engine builds with it (brew install cmake)" >&2; exit 1; }

identity="${APPLE_SIGNING_IDENTITY:-$(find_identity "Developer ID Application:")}"
if [ -z "$identity" ] && [ "${SILAS_LOCAL_SIGNING:-1}" != 0 ]; then
  # Self-signed, so `find-identity -v` hides it; codesign takes it by SHA-1.
  identity=$(bash scripts/signing-identity.sh ensure)
fi
identity="${identity:-$(find_identity "Apple Development:")}"
[ -n "$identity" ] || { echo "no code-signing identity found" >&2; exit 1; }
echo "signing with $identity"

# The promoted commit holds main's base version; the release version is stamped for the build
# only and put back afterwards, since `stable` moves only by promotion or a hotfix pull request.
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
same_draft
draft_download "$dir/ci" '*.AppImage' '*.deb' '*.rpm' '*-setup.exe'
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
same_draft
draft_edit "Silas $version" "$notes"
for f in "$dir"/Silas_aarch64.dmg "$dir"/Silas_aarch64.app.tar.gz "$dir"/Silas_aarch64.app.tar.gz.sig "$dir"/ci/*.sig "$dir"/latest.json; do
  for attempt in 1 2 3 4 5; do
    draft_upload "$f" && break
    [ "$attempt" -lt 5 ] || { echo "upload of $f failed; the draft $tag is still there" >&2; exit 1; }
    sleep $((attempt * 5))
  done
done
same_draft
have=$(gh api "repos/$REPO/releases/$draft" -q '.assets[].name')
for name in $(jq -r '.platforms[].url | split("/") | last' "$dir/latest.json") latest.json; do
  grep -qxF "$name" <<<"$have" || { echo "the $tag draft lacks $name, which latest.json needs" >&2; exit 1; }
done
# Move stable and tag it only now, with every file in the draft. Through the API, not git push:
# GitHub's email-privacy check refuses a push of main's commits to another branch when they carry
# a private author address (v0.1.7, 2026-11-22). force=false still refuses anything but a
# fast-forward, and a re-run finds both already there.
gh api -X PATCH "repos/$REPO/git/refs/heads/stable" -f sha="$sha" -F force=false >/dev/null
gh api "repos/$REPO/git/refs/tags/$tag" >/dev/null 2>&1 ||
  gh api "repos/$REPO/git/refs" -f ref="refs/tags/$tag" -f sha="$sha" >/dev/null
git fetch -q origin stable --tags --force
[ "$(git rev-parse "$tag^{commit}")" = "$(git rev-parse origin/stable)" ] || { echo "$tag is not stable's head; the draft is unpublished" >&2; exit 1; }
gh api -X PATCH "repos/$REPO/releases/$draft" -f tag_name="$tag" -F draft=false -f make_latest=true >/dev/null
echo "Released Silas $version (stable: macOS, Windows, Linux)."
echo "Next: bump main's base version in a pull request (docs/channels.md)."
git diff --quiet -- scripts/release-requirement.txt ||
  echo "scripts/release-requirement.txt changed (ALLOW_NEW_REQUIREMENT=1): land it on main in a pull request."
