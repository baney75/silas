#!/usr/bin/env python3
"""Check a fetched bench set against the manifest. Standard library only; run with python3 -I.

  python3 -I bench/tools/check.py [--dest DIR]

Prints clip counts and minutes per category, then confirms that every manifest row has
exactly the expected keys and that its audio exists, is 16 kHz mono 16-bit PCM WAV, and
lasts `seconds` within 0.05 s. Also checks bench/subsets/*.txt. Exits 1 on any failure.
"""

import argparse
import json
import sys
import wave
from collections import Counter, defaultdict
from pathlib import Path

BENCH = Path(__file__).resolve().parents[1]
KEYS = ["id", "category", "audio", "seconds", "ref_verbatim", "ref_formatted", "speaker", "accent",
        "snr_db", "noise", "source", "license", "tts", "notes"]
CATEGORIES = ["clean", "noise", "accent", "dictation", "code", "command", "longform"]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dest", type=Path, default=Path.home() / ".cache" / "silas-bench" / "bench-set-v1")
    ap.add_argument("--manifest", type=Path, default=BENCH / "manifest.jsonl")
    args = ap.parse_args()
    dest = args.dest.expanduser().resolve()
    rows = [json.loads(line) for line in args.manifest.read_text().splitlines() if line.strip()]
    errors = []

    ids = Counter(r["id"] for r in rows)
    errors += [f"duplicate id {i}" for i, n in ids.items() if n > 1]
    for r in rows:
        if list(r) != KEYS:
            errors.append(f"{r.get('id')}: keys {list(r)}")
        if r["category"] not in CATEGORIES:
            errors.append(f"{r['id']}: category {r['category']}")
        if r["category"] == "command":
            try:
                json.loads(r["ref_formatted"])
            except ValueError:
                errors.append(f"{r['id']}: ref_formatted is not a JSON action list")
        p = dest / r["audio"]
        if not p.is_file():
            errors.append(f"{r['id']}: missing {p}")
            continue
        try:
            with wave.open(str(p)) as w:
                fmt = (w.getframerate(), w.getnchannels(), w.getsampwidth(), w.getcomptype())
                secs = w.getnframes() / w.getframerate()
        except wave.Error as e:
            errors.append(f"{r['id']}: not a PCM WAV ({e})")
            continue
        if fmt != (16000, 1, 2, "NONE"):
            errors.append(f"{r['id']}: format {fmt}, want 16 kHz mono 16-bit PCM")
        if abs(secs - r["seconds"]) > 0.05:
            errors.append(f"{r['id']}: {secs:.3f} s on disk, manifest says {r['seconds']}")

    by_cat = defaultdict(list)
    for r in rows:
        by_cat[r["category"]].append(r["seconds"])
    print(f"{'category':<10} {'clips':>5} {'minutes':>8}")
    for c in CATEGORIES:
        print(f"{c:<10} {len(by_cat[c]):>5} {sum(by_cat[c]) / 60:>8.2f}")
    total = sum(r["seconds"] for r in rows)
    print(f"{'total':<10} {len(rows):>5} {total / 60:>8.2f}")

    secs_by_id = {r["id"]: r["seconds"] for r in rows}
    cat_by_id = {r["id"]: r["category"] for r in rows}
    for sub in sorted((BENCH / "subsets").glob("*.txt")):
        sub_ids = [line.strip() for line in sub.read_text().splitlines() if line.strip() and not line.startswith("#")]
        unknown = [i for i in sub_ids if i not in secs_by_id]
        errors += [f"{sub.name}: unknown id {i}" for i in unknown]
        known = [i for i in sub_ids if i in secs_by_id]
        cats = Counter(cat_by_id[i] for i in known)
        print(f"subset {sub.stem}: {len(sub_ids)} clips, {sum(secs_by_id[i] for i in known) / 60:.2f} min, "
              + ", ".join(f"{c} {cats[c]}" for c in CATEGORIES))
        if sub.stem == "ratchet":
            if len(sub_ids) != 40:
                errors.append(f"ratchet has {len(sub_ids)} ids, want 40")
            if sum(secs_by_id[i] for i in known) > 360:
                errors.append("ratchet is over 6 minutes")
            if set(cats) != set(CATEGORIES):
                errors.append(f"ratchet misses categories {set(CATEGORIES) - set(cats)}")

    if errors:
        print(f"\nFAIL: {len(errors)} problems")
        for e in errors[:50]:
            print("  " + e)
        sys.exit(1)
    print(f"\nOK: {len(rows)} clips, every audio file present, 16 kHz mono 16-bit, durations within 0.05 s")


if __name__ == "__main__":
    main()
