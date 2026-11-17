#!/usr/bin/env bash
# Checks the release channel rules a machine can check (docs/channels.md). From
# baney75/local-studio scripts/check-channels.sh, adapted to Silas.
#   scripts/check-channels.sh [repo root]
set -euo pipefail
root=${1:-$(git rev-parse --show-toplevel)}
fail=0
err() { echo "check-channels: $*" >&2; fail=1; }

triggers() {
  awk '/^on:/{on=1;next} on&&/^[^ #]/{on=0} on&&/^  [a-z_]+:/{sub(/^  /,"");sub(/:.*/,"");print}' "$1"
}

shopt -s nullglob
for wf in "$root"/.github/workflows/*.yml "$root"/.github/workflows/*.yaml; do
  name=${wf#"$root"/}
  if awk '/^on:/{on=1;next} on&&/^[^ #]/{on=0} on&&/^  push:/{p=1;next} on&&/^  [a-z_]+:/{p=0} on&&p&&/^    tags:/{f=1} END{exit !f}' "$wf"; then
    err "$name starts on a pushed tag; stable builds start only from promote.yml"
  fi
  # A push trigger with no branch filter also runs on every pushed tag.
  if grep -qE '^on: *(push|\[[^]]*push)' "$wf" ||
    awk '/^on:/{on=1;next} on&&/^[^ #]/{on=0}
      on&&/^  push:/{seen=1; p=1; v=$0; sub(/^  push: */,"",v); if (v!="" && v!~/^#/) bad=1; next}
      on&&p&&/^    branches(-ignore)?:/{br=1} on&&/^  [a-z_]+:/{p=0}
      END{exit !(bad || (seen && !br))}' "$wf"; then
    err "$name runs on every push, pushed tags included; give push a branches filter"
  fi
  if awk '/^ +environment: *["'"'"']?stable["'"'"']? *$/{f=1} /^ +environment: *$/{e=1;next} e&&/^ +name: *["'"'"']?stable["'"'"']? *$/{f=1} {e=0} END{exit !f}' "$wf"; then
    for t in $(triggers "$wf"); do
      case $t in workflow_call|workflow_dispatch) ;; *) err "$name uses the stable environment but also runs on $t" ;; esac
    done
  fi
  # Silas's stable updater key never leaves the release Mac, so no workflow may name one.
  if grep -qiE 'secrets\.[A-Z0-9_]*STABLE|silas-stable\.key' "$wf"; then
    err "$name names a stable signing secret; the stable key never leaves the release Mac"
  fi
done

if [ -f "$root/.github/workflows/promote.yml" ]; then
  for t in $(triggers "$root/.github/workflows/promote.yml"); do
    [ "$t" = workflow_dispatch ] || err ".github/workflows/promote.yml runs on $t; it runs only by hand on main"
  done
  grep -q "github.ref == 'refs/heads/main'" "$root/.github/workflows/promote.yml" ||
    err ".github/workflows/promote.yml does not check that it runs on main"
  # The job's token can't move stable over workflow changes; release-stable.sh pushes on the Mac.
  grep -qE 'git push|git/refs' "$root/.github/workflows/promote.yml" &&
    err ".github/workflows/promote.yml pushes a branch or tag; release-stable.sh moves stable and tags it"
fi

# Only the stable app release may be marked latest: the stable feed is releases/latest. A release
# that is published must say which it is (--latest, --latest=false or make_latest).
for f in "$root"/.github/workflows/*.yml "$root"/scripts/*.sh; do
  name=${f#"$root"/}
  # Tests hold bad examples on purpose.
  case "$name" in *-test.sh) continue ;; esac
  while IFS= read -r line; do
    case "$line" in
      *'gh release create'*) case "$line" in *--draft*|*--latest*) ;; *) err "$name creates a release without --latest or --latest=false: $line" ;; esac ;;
      *'gh release edit'*--draft=false*) case "$line" in *--latest*) ;; *) err "$name publishes a release without --latest or --latest=false: $line" ;; esac ;;
      *'draft=false'*) case "$line" in *make_latest*) ;; *) err "$name publishes a release without make_latest: $line" ;; esac ;;
    esac
  done < <(grep -v '^ *#' "$f")
done

# stable < hotfix < dev < nightly < next stable for one base, and the nightlies published before
# seconds and +sha7 (2026-11-08) still sort below the new ones.
order=(0.2.0 0.2.1 0.3.0-dev.20261117131042 0.3.0-dev.20261118090500 0.3.0-nightly.202611160700 0.3.0-nightly.20261117070000 0.3.0)
if [ "$(npx --loglevel=error -y semver@7.6.3 "${order[@]}" 2>/dev/null)" != "$(printf '%s\n' "${order[@]}")" ]; then
  err "versions do not sort stable < hotfix < dev < nightly < next stable"
fi
if [ "$(npx --loglevel=error -y semver@7.6.3 0.1.7-nightly.20261117070000+abc1234 0.1.7-nightly.202611081355 2>/dev/null)" != "$(printf '%s\n' 0.1.7-nightly.202611081355 0.1.7-nightly.20261117070000)" ]; then
  err "the new nightly 0.1.7-nightly.20261117070000+abc1234 does not sort above the last old one, 0.1.7-nightly.202611081355"
fi

exit $fail
