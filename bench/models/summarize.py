"""Candidate models for Silas's default engine, scored on the same clips, machine and dictation path.

  python3 -I bench/models/summarize.py bench/models/accent-2026-11.candidates.json --out bench/models/accent-2026-11.json

The candidates file lists each model with its results rows (from `silas-bench run`, bench/models/README.md) and
the facts a run can't measure: size on disk, license, runtime and how Silas would embed it. This prints a
Markdown table and writes the machine-readable results: WER per category (score.py's default normalization,
on the model's own text, and again with fillers dropped from both sides), formatted error and punctuation F1 (rivals.py), the wait after key up (median and p95),
peak memory, CPU seconds per audio minute and model load time.
"""

import argparse
import json
import random
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import rivals  # noqa: E402
import score  # noqa: E402

CATEGORIES = ("clean", "noise", "accent", "dictation", "code", "command", "longform")


def command_wer(rows, manifest):
    """score.py scores command clips only as action lists; their spoken words still say how well a model hears them."""
    pairs = [score.word_errors(manifest[r["id"]]["ref_verbatim"], score.verbatim(r)) for r in rows
             if manifest[r["id"]]["category"] == "command"]
    return score.pooled(pairs) if pairs else None


def no_fillers(s):
    """score.py's default normalization without fillers: the app removes them before typing, so a model that
    leaves out "uh" loses nothing a user sees, yet the default WER counts each one as a deletion."""
    return " ".join(w for w in score.normalize_silas(s).split() if w not in score.FILLERS)


def wer_without_fillers(rows, manifest):
    """Pooled WER per category (speech clips, as score.py) and overall, fillers dropped from both sides."""
    by = {}
    for r in rows:
        cat = manifest[r["id"]]["category"]
        by.setdefault(cat, []).append(score.word_errors(manifest[r["id"]]["ref_verbatim"], score.verbatim(r), no_fillers))
    out = {k: round(score.pooled(v), 4) for k, v in by.items()}
    out["all"] = round(score.pooled([p for k, v in by.items() if k != "command" for p in v]), 4)
    return out


def rounded(x, n=4):
    return None if x is None else round(x, n)


def candidate(c, manifest, base):
    rows = [json.loads(l) for l in (base / c["rows"]).read_text().splitlines() if l.strip()]
    s = score.summarize(rows, manifest, score.normalize_silas, random.Random(score.SEED))
    o, cats = s["overall"], s["categories"]
    wer = {k: rounded(cats[k]["wer"]) for k in CATEGORIES if k in cats and k != "command"}
    wer["command"] = rounded(command_wer(rows, manifest))
    return {
        **{k: v for k, v in c.items() if k != "rows"},
        "rows": c["rows"],
        "clips": o["clips"],
        "missing": s["missing"],
        "wer": rounded(o["wer"]),
        "wer_ci": [rounded(x) for x in o["wer_ci"]],
        "wer_by_category": wer,
        "wer_ci_by_category": {k: [rounded(x) for x in cats[k]["wer_ci"]] for k in CATEGORIES
                               if k in cats and cats[k]["wer_ci"]},
        "wer_without_fillers": wer_without_fillers(rows, manifest),
        "formatted_error": rounded(o["formatted_error"]),
        "punctuation_f1": rounded(rivals.punctuation_f1(rows, manifest)),
        "wait_after_key_up_ms": {"median": rounded(o["t_final_ms_median"], 1), "p95": rounded(o["t_final_ms_p95"], 1)},
        "wait_after_key_up_ms_by_category": {k: {"median": rounded(cats[k]["t_final_ms_median"], 1),
                                                 "p95": rounded(cats[k]["t_final_ms_p95"], 1)} for k in cats},
        "time_to_first_text_ms_median": rounded(o["t_first_ms_median"], 1),
        "rtf_median": rounded(o["rtf_median"]),
        "peak_rss_mb": rounded(o["peak_rss_mb"], 1),
        "cpu_s_per_audio_min": rounded(o["cpu_s_per_audio_min"], 2),
        "load_ms": o["load_ms"],
        "paced": o["paced"],
        "load1_median": rounded(o["load1_median"], 2),
        "load1_max": rounded(o["load1_max"], 2),
    }


def pct(x):
    return "n/a" if x is None else f"{x * 100:.1f}"


def table(results):
    head = ["Candidate", "All", *CATEGORIES, "Fmt err", "Punct F1", "Wait p50", "Wait p95", "Peak MB", "CPU s/min", "Size MB"]
    lines = ["| " + " | ".join(head) + " |", "|" + " --- |" + " ---: |" * (len(head) - 1)]
    for r in results:
        w = r["wer_by_category"]
        cells = [r["label"], pct(r["wer"]), *(pct(w.get(k)) for k in CATEGORIES), pct(r["formatted_error"]),
                 f"{r['punctuation_f1']:.3f}", f"{r['wait_after_key_up_ms']['median']:.0f}",
                 f"{r['wait_after_key_up_ms']['p95']:.0f}", f"{r['peak_rss_mb']:.0f}",
                 f"{r['cpu_s_per_audio_min']:.1f}", str(r.get("size_mb", ""))]
        lines.append("| " + " | ".join(cells) + " |")
    return "\n".join(lines)


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("candidates", type=Path)
    p.add_argument("--manifest", type=Path, default=HERE.parent / "manifest.jsonl")
    p.add_argument("--out", type=Path)
    a = p.parse_args(argv)
    spec = json.loads(a.candidates.read_text())
    manifest = score.load_manifest(a.manifest)
    base = a.candidates.parent
    results = [candidate(c, manifest, base) for c in spec["candidates"] if (base / c["rows"]).exists()]
    if a.out:
        out = {k: v for k, v in spec.items() if k != "candidates"}
        out["candidates"] = results
        a.out.write_text(json.dumps(out, indent=2) + "\n")
    print(table(results))


if __name__ == "__main__":
    main()
