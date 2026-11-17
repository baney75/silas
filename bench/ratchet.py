"""Fails when Silas gets worse on the ratchet subset; locks in gains with --update.

  python3 -I bench/ratchet.py                 run Silas on the subset in real time and compare
  python3 -I bench/ratchet.py --fast          simulated waiting (see `silas-bench run --fast`)
  python3 -I bench/ratchet.py --gate accuracy WER and formatted error only (CI: shared runners make timing noise)
  python3 -I bench/ratchet.py --results x.jsonl   compare existing results instead of running
  python3 -I bench/ratchet.py --update        rewrite bench/ratchet.json, only if every metric is equal or better

Fails (exit 1) when any category's WER or the formatted error rises more than 0.5 points absolute, the median
t_final_ms rises more than 15%, or peak memory rises more than 10%. Latency is compared only between runs with
the same timing (real time or --fast). The silas-bench binary comes from $SILAS_BENCH_BIN or --bin; build it with
  cd src-tauri && cargo build --release --features bench --bin silas-bench
"""

import argparse
import json
import os
import random
import subprocess
import sys
from datetime import date
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import score  # noqa: E402

RATCHET = HERE / "ratchet.json"
WER_SLACK = 0.005
LATENCY_SLACK = 0.15
MEMORY_SLACK = 0.10


def metrics(rows, manifest):
    s = score.summarize(rows, manifest, score.normalize, random.Random(score.SEED))
    return {
        "wer": {c: round(b["wer"], 6) for c, b in s["categories"].items() if b["wer"] is not None},
        "formatted_error": round(s["overall"]["formatted_error"], 6),
        "t_final_ms_median": round(s["overall"]["t_final_ms_median"], 1),
        "peak_rss_mb": round(s["overall"]["peak_rss_mb"], 1) if s["overall"]["peak_rss_mb"] is not None else None,
        "timing": "real time" if s["overall"]["paced"] else "fast",
    }


def check(base, new, gate="all"):
    """Regressions of `new` against `base` beyond the slack, as readable lines. Empty means it passes."""
    bad = []
    for cat, old in base["wer"].items():
        got = new["wer"].get(cat)
        if got is None:
            bad.append(f"WER {cat}: no longer measured")
        elif got - old > WER_SLACK:
            bad.append(f"WER {cat}: {old * 100:.2f}% -> {got * 100:.2f}% (+{(got - old) * 100:.2f} pts, limit +0.5)")
    if new["formatted_error"] - base["formatted_error"] > WER_SLACK:
        bad.append(f"formatted error: {base['formatted_error'] * 100:.2f}% -> {new['formatted_error'] * 100:.2f}% (limit +0.5 pts)")
    if gate == "all" and base.get("timing") == new.get("timing"):
        if new["t_final_ms_median"] > base["t_final_ms_median"] * (1 + LATENCY_SLACK):
            bad.append(f"median t_final: {base['t_final_ms_median']:.0f} ms -> {new['t_final_ms_median']:.0f} ms (limit +15%)")
    if gate == "all" and base.get("peak_rss_mb") and new.get("peak_rss_mb"):
        if new["peak_rss_mb"] > base["peak_rss_mb"] * (1 + MEMORY_SLACK):
            bad.append(f"peak memory: {base['peak_rss_mb']:.0f} MB -> {new['peak_rss_mb']:.0f} MB (limit +10%)")
    return bad


def no_worse(base, new):
    """Every metric equal or better: the condition for --update."""
    if base.get("timing") != new.get("timing"):
        return False
    if set(base["wer"]) - set(new["wer"]):
        return False
    return (
        all(new["wer"][c] <= v for c, v in base["wer"].items())
        and new["formatted_error"] <= base["formatted_error"]
        and new["t_final_ms_median"] <= base["t_final_ms_median"]
        and (base.get("peak_rss_mb") is None or (new.get("peak_rss_mb") or 0) <= base["peak_rss_mb"])
    )


def run_silas(cfg, binary, fast, out):
    cmd = [binary, "run", "--manifest", str(ROOT / cfg["manifest"]), "--model", cfg["model"], "--out", str(out)]
    if cfg.get("subset"):
        cmd += ["--subset", str(ROOT / cfg["subset"])]
    if cfg.get("set"):
        cmd += ["--set", os.path.expanduser(cfg["set"])]
    if fast:
        cmd.append("--fast")
    subprocess.run(cmd, check=True)


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--fast", action="store_true")
    p.add_argument("--gate", choices=["all", "accuracy"], default="all")
    p.add_argument("--results", type=Path, help="score these results instead of running Silas")
    p.add_argument("--update", action="store_true")
    p.add_argument("--bin", default=os.environ.get("SILAS_BENCH_BIN"))
    p.add_argument("--ratchet", type=Path, default=RATCHET)
    a = p.parse_args(argv)

    cfg = json.loads(a.ratchet.read_text())
    manifest = score.load_manifest(ROOT / cfg["manifest"])
    if cfg.get("subset"):
        keep = {l.strip() for l in (ROOT / cfg["subset"]).read_text().splitlines() if l.strip() and not l.startswith("#")}
        manifest = {k: v for k, v in manifest.items() if k in keep}
    results = a.results
    if results is None:
        binary = a.bin or str(ROOT / "src-tauri/target/release/silas-bench")
        results = HERE / "out" / "ratchet-silas.jsonl"
        results.parent.mkdir(exist_ok=True)
        run_silas(cfg, binary, a.fast, results)
    rows = [json.loads(l) for l in results.read_text().splitlines() if l.strip()]
    new = metrics(rows, manifest)
    base = cfg.get("metrics")

    print(json.dumps(new, indent=2))
    if base is None:
        if not a.update:
            print("ratchet.json has no numbers yet; run with --update to record them")
            return 1
        bad = []
    else:
        bad = check(base, new, a.gate)
        if base.get("timing") != new["timing"] and a.gate == "all":
            print(f"latency not compared: the ratchet holds {base.get('timing')} timing, this run is {new['timing']}")
    for b in bad:
        print("REGRESSION", b)
    if bad:
        return 1
    print("ratchet passes")
    if a.update:
        if base is not None and not no_worse(base, new):
            print("not updated: some metric is worse than the ratchet (within the slack), so it stays put")
            return 0
        cfg["metrics"] = new
        cfg["updated"] = date.today().isoformat()
        a.ratchet.write_text(json.dumps(cfg, indent=2) + "\n")
        print(f"updated {a.ratchet}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
