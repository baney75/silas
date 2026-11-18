#!/usr/bin/env bash
# Runs candidate models through `silas-bench run --fast` on all 250 clips, one at a time, pinned to 3 cores
# (CPUS) at nice 10 (BIN picks another silas-bench build), holding while the 1-minute load is 8 or more. bench/models/README.md has the setup.
#   bench/models/run.sh <id>=<model> ...
# <model> is an app model id (parakeet-v3), gguf:<file> (transcribe.cpp), or whisper:<ggml name> (starts a
# whisper-server with 3 threads on the same cores and adds its CPU and memory to each row). Rows go to
# bench/models/$RUN/<id>.jsonl; a finished file (250 rows) is skipped.
set -u
WT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${BIN:-$WT/core/voice/target/release/silas-bench}
SET=~/.cache/silas-bench/bench-set-v1
OUT=$WT/bench/models/${RUN:-accent-2026-11}
LOG=~/.cache/silas-bench/logs/accent
CPUS=${CPUS:-1,2,3}
mkdir -p "$OUT" "$LOG"

hold() {
  while :; do
    l=$(cut -d' ' -f1 /proc/loadavg)
    awk -v l="$l" 'BEGIN{exit !(l < 8)}' && return
    echo "$(date +%T) load $l, holding" >> "$LOG/queue.log"; sleep 60
  done
}

for item in "$@"; do
  id=${item%%=*}; model=${item#*=}
  [ -s "$OUT/$id.jsonl" ] && [ "$(wc -l < "$OUT/$id.jsonl")" -ge 250 ] && continue
  if [[ $model == gguf:* ]]; then
    until [ -s "${model#gguf:}" ]; do sleep 30; done  # still downloading
  fi
  hold
  echo "$(date +%T) start $id ($model)" >> "$LOG/queue.log"
  extra=()
  server=
  if [[ $model == whisper:* ]]; then
    name=${model#whisper:}
    port=$((20000 + RANDOM % 20000))
    nice -n 10 taskset -c $CPUS ~/.cache/silas-bench/engines/whisper.cpp-v1.9.5/build/bin/whisper-server \
      -m ~/.cache/silas-bench/models/ggml/ggml-$name.bin -t 3 --host 127.0.0.1 --port $port -l en \
      > "$LOG/$id.server.log" 2>&1 &
    server=$!
    until curl -fs "http://127.0.0.1:$port/health" >/dev/null 2>&1; do sleep 0.2; kill -0 $server || break; done
    model=whisper-server:http://127.0.0.1:$port
    extra=(--usage-pid $server)
  fi
  nice -n 10 taskset -c $CPUS "$BIN" run --fast --threads 3 --manifest $WT/bench/manifest.jsonl --set $SET \
    --model "$model" --out "$OUT/$id.jsonl" "${extra[@]}" > "$LOG/$id.log" 2>&1
  echo "$(date +%T) end $id exit $?" >> "$LOG/queue.log"
  [ -n "$server" ] && kill $server && wait $server 2>/dev/null
done
echo "$(date +%T) queue done" >> "$LOG/queue.log"
