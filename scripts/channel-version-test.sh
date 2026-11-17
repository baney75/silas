#!/usr/bin/env bash
# Exercises scripts/channel-version.sh: the version format and how new versions sort.
#   scripts/channel-version-test.sh
set -euo pipefail
cd "$(dirname "$0")/.."
v=scripts/channel-version.sh
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

base=$(jq -r .version src-tauri/tauri.conf.json)
IFS=. read -r ma mi pa <<<"$base"
sha=$(git rev-parse HEAD)
got=$($v next nightly HEAD 20261117070000)
[ "$got" = "$ma.$mi.$((pa + 1))-nightly.20261117070000+${sha:0:7}" ] || fail "nightly version is $got"
got=$($v next dev HEAD 20261117131042)
[ "$got" = "$ma.$mi.$((pa + 1))-dev.20261117131042+${sha:0:7}" ] || fail "dev version is $got"
[[ "$($v next nightly)" =~ ^[0-9]+\.[0-9]+\.[0-9]+-nightly\.[0-9]{14}\+[0-9a-f]{7}$ ]] || fail "default nightly version has the wrong shape"
! $v next stable >/dev/null 2>&1 || fail "next accepted the stable channel"
! $v next nightly HEAD 202611170700 >/dev/null 2>&1 || fail "next accepted a 12-digit timestamp"

# The last nightly published before seconds and +sha7 were added (2026-11-08).
last=0.1.7-nightly.202611081355
$v newer 0.1.7-nightly.20261117070000+abc1234 "$last" || fail "the new nightly format does not sort above $last"
$v newer 0.1.7-nightly.20261118070000+abc1234 0.1.7-nightly.20261117070000+def5678 || fail "a later nightly is not newer"
! $v newer 0.1.7-nightly.20261117070000+abc1234 0.1.7-nightly.20261117070000+def5678 || fail "the same timestamp counts as newer"
! $v newer 0.1.7-nightly.20261116070000+abc1234 0.1.7-nightly.20261117070000+def5678 || fail "an older nightly counts as newer"
$v newer 0.1.8-dev.20261101000000+abc1234 0.1.7-dev.20261117131042+def5678 || fail "a higher base is not newer"
$v newer 0.1.7 0.1.7-nightly.20261117070000+abc1234 || fail "stable does not sort above its nightlies"

[ "$failures" = 0 ] && echo "channel-version: all checks passed" || exit 1
