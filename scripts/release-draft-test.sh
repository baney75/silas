#!/usr/bin/env bash
# Proves scripts/release-draft.sh addresses the stable draft only by id: with a fake `gh` it checks
# no call looks the draft up by its tag, that every edit restates tag_name (GitHub otherwise turns
# the draft "untagged-...", v0.1.7), and that once the pinned draft is replaced, an upload fails
# without writing anything (the replacement draft is never touched).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin" "$t/dl"
cat > "$t/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_LOG"
url=""
for a in "$@"; do case "$a" in repos/*|https://*) url=$a ;; esac; done
# The pinned draft 111 is gone once FAKE_GONE is set: a promotion re-run made draft 222.
if [ -n "${FAKE_GONE:-}" ] && [[ "$url" == *releases/111* || "$url" == *releases/assets/5* ]]; then
  echo "HTTP 404" >&2; exit 1
fi
case "$url" in
  */releases/111/assets) printf 'Silas_0.2.1_amd64.deb\t51\nSilas_0.2.1_x64-setup.exe\t52\nlatest.json\t53\n' ;;
  */releases/assets/5*) echo "bytes of $url" ;;
esac
EOF
chmod +x "$t/bin/gh"
export PATH="$t/bin:$PATH" FAKE_LOG="$t/log" REPO=baney75/silas draft=111 tag=v0.2.1
# shellcheck source=scripts/release-draft.sh
. "$here/release-draft.sh"
status=0
fail() { echo "release-draft: $*"; status=1; }

echo new > "$t/latest.json"
draft_download "$t/dl" '*.deb' '*-setup.exe' || fail "download failed"
if [ ! -f "$t/dl/Silas_0.2.1_amd64.deb" ] || [ ! -f "$t/dl/Silas_0.2.1_x64-setup.exe" ]; then fail "download missed a file"; fi
[ ! -e "$t/dl/latest.json" ] || fail "download took a file no glob named"
draft_upload "$t/latest.json" || fail "upload failed"
grep -q 'DELETE repos/baney75/silas/releases/assets/53' "$t/log" || fail "upload did not replace the old latest.json"
grep -q 'uploads.github.com/repos/baney75/silas/releases/111/assets?name=latest.json' "$t/log" || fail "upload did not go to draft 111"
draft_edit "Silas 0.2.1" "notes" || fail "edit failed"
grep -q 'PATCH repos/baney75/silas/releases/111 -f tag_name=v0.2.1 ' "$t/log" || fail "edit did not restate tag_name"
if sed 's/ -f tag_name=v0\.2\.1 / /' "$t/log" | grep -qE 'releases/tags|release (upload|download|edit|view)|v0\.2\.1'; then
  fail "a call named the tag:"; cat "$t/log"
fi

: > "$t/log"
export FAKE_GONE=1
if draft_upload "$t/latest.json"; then fail "upload succeeded after the draft was replaced"; fi
if grep -qE 'POST|DELETE' "$t/log"; then fail "upload wrote after the draft was replaced:"; cat "$t/log"; fi
if draft_download "$t/dl" '*.deb'; then fail "download succeeded after the draft was replaced"; fi

[ "$status" = 0 ] && echo "release-draft: every call names draft 111 by id; a replaced draft is never written"
exit $status
