#!/usr/bin/env bash
# Version strings for Silas's release channels (docs/channels.md).
#
#   scripts/channel-version.sh next <nightly|dev> [sha] [YYYYMMDDHHMMSS]
#       X.Y.Z-<channel>.<UTC timestamp>+<sha7>, where X.Y.Z is the base in
#       src-tauri/tauri.conf.json with the patch bumped. sha defaults to HEAD, the timestamp to now.
#   scripts/channel-version.sh newer <candidate> <published>
#       exit 0 when candidate sorts strictly above published (SemVer 2.0, build metadata ignored).
#       A channel refuses to publish a build that is not newer than its feed.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:-}" in
  next)
    channel="${2:?usage: channel-version.sh next <nightly|dev> [sha] [timestamp]}"
    case "$channel" in nightly | dev) ;; *) echo "channel must be nightly or dev, not $channel" >&2; exit 1 ;; esac
    sha=$(git rev-parse "${3:-HEAD}")
    stamp="${4:-$(date -u +%Y%m%d%H%M%S)}"
    [[ "$stamp" =~ ^[0-9]{14}$ ]] || { echo "timestamp must be YYYYMMDDHHMMSS, not $stamp" >&2; exit 1; }
    base=$(jq -r .version src-tauri/tauri.conf.json)
    [[ "$base" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || { echo "base version $base is not X.Y.Z" >&2; exit 1; }
    echo "${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((BASH_REMATCH[3] + 1))-$channel.$stamp+${sha:0:7}"
    ;;
  newer)
    candidate="${2:?usage: channel-version.sh newer <candidate> <published>}"
    published="${3:?usage: channel-version.sh newer <candidate> <published>}"
    # The semver CLI drops build metadata when it prints, so compare without it.
    a=${candidate%%+*} b=${published%%+*}
    [ "$a" != "$b" ] || exit 1
    sorted=$(npx --loglevel=error -y semver@7.6.3 "$b" "$a")
    [ "$(tail -1 <<<"$sorted")" = "$a" ]
    ;;
  *)
    echo "usage: channel-version.sh next <nightly|dev> [sha] [timestamp] | newer <candidate> <published>" >&2
    exit 1
    ;;
esac
