#!/usr/bin/env bash
# Checks a Mac build's designated requirement against the pin in scripts/release-requirement.txt.
# macOS ties Silas's Microphone and Accessibility grants to that requirement (docs/signing.md),
# so a build whose requirement differs from the last stable release makes every user grant again.
#
#   scripts/check-requirement.sh <Silas.app>           fail on ad-hoc (cdhash) or a mismatch
#   scripts/check-requirement.sh --update <Silas.app>  same, but with ALLOW_NEW_REQUIREMENT=1 it
#                                                      rewrites the pin instead (release-stable.sh)
#   scripts/check-requirement.sh --pin                 check only the pin file (CI, no codesign)
#
# The pin file holds one line: the requirement exactly as `codesign -d -r-` prints it after
# "designated => ". A file with only blank or # lines is a bootstrap placeholder: there is no
# stable release to match yet, and the first one needs ALLOW_NEW_REQUIREMENT=1 to write the pin.
set -euo pipefail
cd "$(dirname "$0")/.."
pin_file=scripts/release-requirement.txt
fail() { echo "check-requirement.sh: $*" >&2; exit 1; }

read_pin() {
  [ -f "$pin_file" ] || fail "$pin_file is missing"
  local lines
  lines=$(grep -v -e '^#' -e '^[[:space:]]*$' "$pin_file" || true)
  [ "$(printf '%s' "$lines" | grep -c '')" -le 1 ] || fail "$pin_file must hold one requirement line"
  printf '%s' "$lines"
}

stable_or_fail() { # requirement, what
  case "$1" in
    '') fail "$2 has no designated requirement" ;;
    *'cdhash '*) fail "$2 is ad-hoc signed (cdhash): a new app to macOS on every build" ;;
    'identifier "com.baney.silas" '*) ;;
    *) fail "$2 does not name com.baney.silas: $1" ;;
  esac
}

update=0
case "${1:-}" in
  --pin)
    pin=$(read_pin)
    if [ -z "$pin" ]; then echo "$pin_file is a bootstrap placeholder; the next stable release writes it"; exit 0; fi
    stable_or_fail "$pin" "$pin_file"
    echo "pinned requirement: $pin"
    exit 0 ;;
  --update) update=1; shift ;;
esac
app="${1:?usage: check-requirement.sh [--update] <Silas.app> | --pin}"
[ -d "$app" ] || fail "$app is not an app bundle"

new_req=$(codesign -d -r- "$app" 2>&1 | sed -n 's/^designated => //p')
echo "designated requirement: $new_req"
stable_or_fail "$new_req" "$app"
pin=$(read_pin)
[ "$new_req" = "$pin" ] && { echo "matches $pin_file"; exit 0; }

if [ "$update" = 1 ] && [ "${ALLOW_NEW_REQUIREMENT:-}" = 1 ]; then
  printf '%s\n' "$new_req" > "$pin_file"
  echo "ALLOW_NEW_REQUIREMENT=1: wrote the new requirement to $pin_file (was: ${pin:-bootstrap placeholder})"
  exit 0
fi
echo "check-requirement.sh: the designated requirement differs from $pin_file:" >&2
echo "  pinned: ${pin:-(bootstrap placeholder, no stable requirement yet)}" >&2
echo "  build:  $new_req" >&2
if [ "$update" = 1 ]; then
  echo "users would be asked for permissions again; set ALLOW_NEW_REQUIREMENT=1 if intended" >&2
else
  echo "only a stable release (scripts/release-stable.sh with ALLOW_NEW_REQUIREMENT=1) moves the pin" >&2
fi
exit 1
