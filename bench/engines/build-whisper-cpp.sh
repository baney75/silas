#!/usr/bin/env bash
# Builds whisper.cpp from a pinned release tag into ~/.cache/silas-bench/engines and fetches the
# three ggml models the bench compares. Rerunning skips what is already there.
# cmake comes from PyPI through uvx when the machine has none. JOBS defaults to 3.
set -euo pipefail
TAG=v1.9.5
ROOT="$HOME/.cache/silas-bench"
SRC="$ROOT/engines/whisper.cpp-$TAG"
MODELS="$ROOT/models/ggml"
JOBS="${JOBS:-3}"

if [ ! -d "$SRC" ]; then
  git clone -q --depth 1 --branch "$TAG" https://github.com/ggml-org/whisper.cpp "$SRC"
fi
test "$(git -C "$SRC" describe --tags --exact-match)" = "$TAG"

# An array, not a shell function: `nice` runs only executables.
if command -v cmake >/dev/null; then CMAKE=(cmake); else CMAKE=(uvx --from 'cmake==3.31.6' cmake); fi
if [ ! -x "$SRC/build/bin/whisper-server" ]; then
  "${CMAKE[@]}" -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release -DWHISPER_BUILD_SERVER=ON -DWHISPER_BUILD_TESTS=OFF -DBUILD_SHARED_LIBS=OFF
  nice -n 10 "${CMAKE[@]}" --build "$SRC/build" -j "$JOBS" --target whisper-server whisper-cli
fi

mkdir -p "$MODELS"
for m in base.en small.en large-v3-turbo-q5_0; do
  f="$MODELS/ggml-$m.bin"
  [ -s "$f" ] || { curl -fsSL -o "$f.part" "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$m.bin" && mv "$f.part" "$f"; }
done
echo "$SRC/build/bin/whisper-server"
