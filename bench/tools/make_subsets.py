#!/usr/bin/env python3
"""Write bench/subsets/ratchet.txt: 40 clips across every category, under 6 minutes.

  python3 -I bench/tools/make_subsets.py

Seeded and deterministic. Within each category it samples from the shorter half of the
clips so the whole subset stays fast; longform contributes its shortest passage.
"""

import json
import random
from pathlib import Path

BENCH = Path(__file__).resolve().parents[1]
SEED = 20261117
COUNTS = {"clean": 7, "noise": 7, "accent": 7, "dictation": 7, "code": 6, "command": 5, "longform": 1}


def main() -> None:
    rows = [json.loads(line) for line in (BENCH / "manifest.jsonl").read_text().splitlines()]
    rng = random.Random(SEED)
    picked = []
    for cat, n in COUNTS.items():
        pool = sorted((r for r in rows if r["category"] == cat), key=lambda r: (r["seconds"], r["id"]))
        if cat == "longform":
            picked += pool[:n]
            continue
        if cat == "noise":  # keep both SNRs and every noise type in play
            pool = pool[: max(n * 2, len(pool) // 2)]
            picked += sorted(rng.sample(pool, n), key=lambda r: r["id"])
            continue
        picked += sorted(rng.sample(pool[: max(n, len(pool) // 2)], n), key=lambda r: r["id"])
    total = sum(r["seconds"] for r in picked)
    assert len(picked) == 40 and total < 360, (len(picked), total)
    out = BENCH / "subsets" / "ratchet.txt"
    out.parent.mkdir(exist_ok=True)
    header = f"# Fast regression subset of bench set v1: {len(picked)} clips, {total / 60:.2f} min. Made by bench/tools/make_subsets.py.\n"
    out.write_text(header + "".join(r["id"] + "\n" for r in picked))
    print(f"{out}: {len(picked)} clips, {total / 60:.2f} min")


if __name__ == "__main__":
    main()
