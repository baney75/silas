#!/usr/bin/env bash
# Checks which release install.sh reads on Linux: stable by default, nightly when asked, and
# nightly when a nightly .deb or .rpm is installed and no channel was named (apt and dnf would
# refuse to "downgrade" it to stable). Stand-in tools only; the fake API lists no assets, so
# install.sh stops before any download or package change.
#   scripts/install-channel-test.sh
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"
fake() { printf '#!/bin/sh\n%s\n' "$2" > "$tmp/bin/$1"; chmod +x "$tmp/bin/$1"; }
fake uname 'if [ "$1" = -m ]; then echo x86_64; else echo Linux; fi'
fake getconf 'echo "glibc 2.39"'
fake curl 'for a; do case "$a" in https://api.github.com/*) echo "$a" >> "$FAKE_CALLS" ;; esac; done; echo "{}"'
fake sudo 'echo "sudo $*" >> "$FAKE_CALLS"; exit 1'
fake dpkg-query 'case "$*" in *Status*) printf "%s" "${FAKE_DEB_STATUS:-}" ;; *Version*) printf "%s" "${FAKE_DEB_VERSION:-}" ;; esac'
fake rpm 'case "$*" in *--qf*) printf "%s" "$FAKE_RPM_VERSION" ;; *) [ -n "${FAKE_RPM_VERSION:-}" ] ;; esac'
export PATH="$tmp/bin:/usr/bin:/bin" FAKE_CALLS="$tmp/calls"
status=0

# expect <label> <package manager> <tail of the API URL> [VAR=value ...]
expect() {
  label=$1 pm=$2 want=$3; shift 3
  rm -f "$tmp/bin/apt-get" "$tmp/bin/dnf" "$FAKE_CALLS"
  fake "$pm" 'exit 0'
  out=$(env -u SILAS_CHANNEL -u SILAS_LINUX_FORMAT "$@" sh install.sh 2>&1) && { echo "FAIL $label: install.sh did not stop"; status=1; return; }
  got=$(cat "$FAKE_CALLS" 2>/dev/null || true)
  case "$got" in
  *"releases/$want") echo "ok   $label" ;;
  *) echo "FAIL $label: read '$got', wanted releases/$want"; echo "$out" | sed 's/^/     /'; status=1 ;;
  esac
  if grep -q '^sudo' "$FAKE_CALLS" 2>/dev/null; then echo "FAIL $label: ran sudo"; status=1; fi
}

ok='install ok installed'
expect "fresh Debian gets stable" apt-get latest
expect "SILAS_CHANNEL=nightly gets nightly" apt-get tags/nightly SILAS_CHANNEL=nightly
expect "stable .deb stays stable" apt-get latest FAKE_DEB_STATUS="$ok" FAKE_DEB_VERSION=0.1.7
expect "nightly .deb stays nightly" apt-get tags/nightly FAKE_DEB_STATUS="$ok" FAKE_DEB_VERSION=0.1.7-nightly.20261117070000
expect "nightly .deb with SILAS_CHANNEL=stable gets stable" apt-get latest FAKE_DEB_STATUS="$ok" FAKE_DEB_VERSION=0.1.7-nightly.20261117070000 SILAS_CHANNEL=stable
expect "removed .deb (config-files) gets stable" apt-get latest FAKE_DEB_STATUS="deinstall ok config-files" FAKE_DEB_VERSION=0.1.7-nightly.1
expect "nightly .rpm stays nightly" dnf tags/nightly FAKE_RPM_VERSION=0.1.7~nightly.20261117070000-1
exit $status
