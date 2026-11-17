#!/usr/bin/env bash
# Proves scripts/check-channels.sh passes a good repo and fails each bad workflow.
#   scripts/check-channels-test.sh
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
check=$here/check-channels.sh
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
status=0

good=$tmp/good bad=$tmp/bad
mkdir -p "$good/.github/workflows" "$good/scripts" "$bad/.github/workflows" "$bad/scripts"
cat > "$good/.github/workflows/nightly.yml" <<'EOF'
on:
  schedule:
    - cron: "0 7 * * *"
  workflow_dispatch:
jobs:
  build:
    uses: ./.github/workflows/channel-build.yml
    secrets: inherit
  publish:
    steps:
      - run: gh api -X PATCH "repos/x/releases/1" -F draft=false -f make_latest=false
EOF
cat > "$good/.github/workflows/promote.yml" <<'EOF'
on:
  workflow_dispatch:
    inputs:
      nightly: { type: string }
jobs:
  promote:
    if: github.ref == 'refs/heads/main'
EOF
cat > "$good/scripts/release.sh" <<'EOF'
gh release create v1 --draft --title x
gh release edit v1 --draft=false --latest
gh release create bench-1 --latest=false --notes x
EOF

cat > "$bad/.github/workflows/release.yml" <<'EOF'
on:
  push:
    tags: ["v*"]
EOF
cat > "$bad/.github/workflows/stable.yml" <<'EOF'
on:
  push:
    branches: [stable]
jobs:
  build:
    environment: stable
    env:
      TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_STABLE_SIGNING_KEY }}
EOF
cat > "$bad/.github/workflows/promote.yml" <<'EOF'
on:
  workflow_dispatch:
  schedule:
    - cron: "0 0 * * *"
jobs:
  promote:
    runs-on: ubuntu-latest
    steps:
      - run: gh api -X PATCH "repos/x/releases/1" -F draft=false
      - run: git push origin "$sha:refs/heads/stable"
EOF
cat > "$bad/scripts/models.sh" <<'EOF'
gh release create models-1 --notes "bench set"
gh release edit v1 --draft=false
EOF

out=$("$check" "$good" 2>&1) || { echo "good repo failed: $out"; status=1; }
out=$("$check" "$bad" 2>&1) && { echo "bad repo passed"; status=1; }
for want in \
  "release.yml starts on a pushed tag" \
  "stable.yml uses the stable environment but also runs on push" \
  "stable.yml names a stable signing secret" \
  "promote.yml runs on schedule" \
  "promote.yml does not check that it runs on main" \
  "promote.yml pushes a branch or tag" \
  "promote.yml publishes a release without make_latest" \
  "models.sh creates a release without --latest or --latest=false" \
  "models.sh publishes a release without --latest or --latest=false"; do
  grep -qF "$want" <<<"$out" || { echo "bad repo did not report: $want"; status=1; }
done
[ "$status" = 0 ] && echo "check-channels: good repo passes, every bad case fails"
exit $status
