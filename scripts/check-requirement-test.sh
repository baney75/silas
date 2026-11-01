#!/usr/bin/env bash
# Exercises scripts/check-requirement.sh with a stand-in codesign, so it runs on any CI host.
#   scripts/check-requirement-test.sh
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/repo/scripts" "$tmp/bin" "$tmp/Silas.app"
cp scripts/check-requirement.sh "$tmp/repo/scripts/"
pin=$(cat scripts/release-requirement.txt)
cat > "$tmp/bin/codesign" <<'EOF'
#!/usr/bin/env bash
echo "Executable=$3/Contents/MacOS/silas"
echo "designated => $FAKE_REQ"
EOF
chmod +x "$tmp/bin/codesign"
export PATH="$tmp/bin:$PATH"
check() { "$tmp/repo/scripts/check-requirement.sh" "$@" >"$tmp/out" 2>&1; }
failures=0
expect() { # pass|fail, description, args...
  local want=$1 what=$2; shift 2
  if check "$@"; then got=pass; else got=fail; fi
  if [ "$got" = "$want" ]; then echo "ok: $what"; else echo "FAIL: $what (wanted $want)"; cat "$tmp/out"; failures=$((failures + 1)); fi
}
set_pin() { printf '%s\n' "$1" > "$tmp/repo/scripts/release-requirement.txt"; }
other='identifier "com.baney.silas" and anchor apple generic and certificate leaf[subject.OU] = "TEAMID"'

set_pin "$pin"
FAKE_REQ="$pin" expect pass "a build matching the pin passes" "$tmp/Silas.app"
FAKE_REQ='cdhash H"9dd2baebd941b5cfe914bac60b1e3b7a0575ae51"' expect fail "an ad-hoc build fails" "$tmp/Silas.app"
FAKE_REQ='cdhash H"9dd2baebd941b5cfe914bac60b1e3b7a0575ae51"' ALLOW_NEW_REQUIREMENT=1 expect fail "an ad-hoc build fails even when allowed" --update "$tmp/Silas.app"
FAKE_REQ='' expect fail "an unsigned build fails" "$tmp/Silas.app"
FAKE_REQ="$other" expect fail "a changed requirement fails" "$tmp/Silas.app"
FAKE_REQ="$other" ALLOW_NEW_REQUIREMENT=1 expect fail "a nightly cannot move the pin" "$tmp/Silas.app"
FAKE_REQ="$other" expect fail "a release cannot move the pin without ALLOW_NEW_REQUIREMENT" --update "$tmp/Silas.app"
[ "$(cat "$tmp/repo/scripts/release-requirement.txt")" = "$pin" ] && echo "ok: a refused change leaves the pin alone" \
  || { echo "FAIL: a refused change rewrote the pin"; failures=$((failures + 1)); }
FAKE_REQ="$other" ALLOW_NEW_REQUIREMENT=1 expect pass "an allowed release rewrites the pin" --update "$tmp/Silas.app"
[ "$(cat "$tmp/repo/scripts/release-requirement.txt")" = "$other" ] && echo "ok: the pin holds the new requirement" \
  || { echo "FAIL: the pin was not rewritten"; failures=$((failures + 1)); }

set_pin '# bootstrap: no stable release signed with a real identity yet'
expect pass "a bootstrap placeholder passes the pin check" --pin
FAKE_REQ="$pin" expect fail "a bootstrap placeholder needs ALLOW_NEW_REQUIREMENT for the first pin" --update "$tmp/Silas.app"
FAKE_REQ="$pin" ALLOW_NEW_REQUIREMENT=1 expect pass "the first stable release writes the pin" --update "$tmp/Silas.app"
set_pin 'cdhash H"9dd2baebd941b5cfe914bac60b1e3b7a0575ae51"'
expect fail "an ad-hoc pin fails the pin check" --pin
printf '%s\n%s\n' "$pin" "$other" > "$tmp/repo/scripts/release-requirement.txt"
expect fail "a pin with two requirements fails" --pin

[ "$failures" = 0 ] && echo "check-requirement: all passed" || { echo "check-requirement: $failures failed"; exit 1; }
