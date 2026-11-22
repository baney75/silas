"""Fails when Silas gets worse on the ratchet subset; locks in gains with --update.

  python3 -I bench/ratchet.py                 run Silas on the subset (`silas-bench run --fast`) and compare
  python3 -I bench/ratchet.py --real-time     pace each clip in real time instead
  python3 -I bench/ratchet.py --gate accuracy WER and formatted error only (CI: shared runners make timing noise)
  python3 -I bench/ratchet.py --results x.jsonl   compare existing results instead of running
  python3 -I bench/ratchet.py --update        rewrite the baseline, only if every metric is equal or better
  python3 -I bench/ratchet.py --baseline ci   compare against the numbers recorded on GitHub's runner (CI does)
  python3 -I bench/ratchet.py --update --allow-worse "why"   record a reviewed loss (an engine switch) anyway

The ratchet runs --fast by default because it repeats word for word: the model's text depends a little on how
much silence trails the clip, and in real time that tail varies by a few milliseconds of thread timing, enough
to change a word on a handful of clips between two runs. --fast pads exactly 200 ms of quiet every time.

The same model gives slightly different text on a different CPU (the engine picks different kernels): 10 of
the 40 clips differed by a word or a comma between abbey1's Ryzen and an Intel runner, enough to move a category
by a few points, and GitHub's runners are not all one CPU. So ratchet.json keeps baselines in groups, "local"
(abbey1) and "ci" (GitHub's ubuntu-latest runners), one per CPU model. A run compares with the baseline for its own
CPU. On a CPU not yet recorded it passes if it is within the limits of any recorded one, and says so; --update
then records that CPU. A new CPU whose text differs enough to fail is seeded with --new-cpu, from a run of code that
already passed on another CPU. Pass --cpu with --results to name the CPU they came from.

--allow-worse REASON, with --update, records a run that fails, for a change reviewed as a trade (a new engine that
is far better on accents and worse on code). It writes the reason into the baseline and drops the group's other
CPUs, whose numbers came from before the trade; each of them reseeds with --update on its next passing run.

Fails (exit 1) when any category's WER or the formatted error rises more than 0.5 points absolute, the median
t_final_ms rises more than 15%, or peak memory rises more than 10%. Latency is compared only between runs with
the same timing (real time or --fast). The silas-bench binary comes from $SILAS_BENCH_BIN or --bin; build it with
  cd core/voice && cargo build --release --features bench --bin silas-bench
The audio comes from bench/tools/fetch.py (default ~/.cache/silas-bench/bench-set-v1).
"""

import argparse
import json
import os
import platform
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
    s = score.summarize(rows, manifest, score.normalize_silas, random.Random(score.SEED))
    return {
        "wer": {c: round(b["wer"], 6) for c, b in s["categories"].items() if b["wer"] is not None},
        "formatted_error": round(s["overall"]["formatted_error"], 6),
        "t_final_ms_median": round(s["overall"]["t_final_ms_median"], 1),
        "peak_rss_mb": round(s["overall"]["peak_rss_mb"], 1) if s["overall"]["peak_rss_mb"] is not None else None,
        "timing": "real time" if s["overall"]["paced"] else "fast",
    }


def incomplete(rows, manifest):
    """Why `rows` are not exactly one result per expected clip, as readable lines. Empty means complete.
    A results file that drops a failing clip could otherwise pass on the clips left in its category."""
    ids = [r.get("id") for r in rows]
    bad = []
    dup = sorted({i for i in ids if ids.count(i) > 1}, key=str)
    missing = sorted(set(manifest) - set(ids))
    extra = sorted({i for i in ids if i not in manifest}, key=str)
    if missing:
        bad.append(f"results miss {len(missing)} clip(s): {', '.join(missing[:5])}")
    if dup:
        bad.append(f"results repeat {len(dup)} clip(s): {', '.join(map(str, dup[:5]))}")
    if extra:
        bad.append(f"results hold {len(extra)} clip(s) outside the subset: {', '.join(map(str, extra[:5]))}")
    return bad


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
    if gate == "all" and base.get("peak_rss_mb") and not new.get("peak_rss_mb"):
        bad.append("peak memory: measured in the baseline, missing from this run")
    elif gate == "all" and base.get("peak_rss_mb"):
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
        and (base.get("peak_rss_mb") is None or (new.get("peak_rss_mb") is not None and new["peak_rss_mb"] <= base["peak_rss_mb"]))
    )


def cpu_model():
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or None


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
    p.add_argument("--real-time", action="store_true", help="pace clips in real time (repeats less exactly)")
    p.add_argument("--gate", choices=["all", "accuracy"], default="all")
    p.add_argument("--results", type=Path, help="score these results instead of running Silas")
    p.add_argument("--update", action="store_true")
    p.add_argument("--bin", default=os.environ.get("SILAS_BENCH_BIN"))
    p.add_argument("--ratchet", type=Path, default=RATCHET)
    p.add_argument("--baseline", default="local", help="which machine's numbers to compare against (local, ci)")
    p.add_argument("--cpu", help="the CPU the --results came from (default: this machine's when it runs Silas)")
    p.add_argument("--new-cpu", action="store_true",
                   help="record a first baseline for this CPU without comparing; only for a run of code that passed on another CPU")
    p.add_argument("--allow-worse", metavar="REASON",
                   help="with --update: record this run even where it is worse, for a reviewed trade; drops the group's other CPUs")
    a = p.parse_args(argv)
    if a.allow_worse is not None and (not a.update or not a.allow_worse.strip()):
        p.error("--allow-worse needs --update and a reason")

    cfg = json.loads(a.ratchet.read_text())
    manifest = score.load_manifest(ROOT / cfg["manifest"])
    if cfg.get("subset"):
        keep = {l.strip() for l in (ROOT / cfg["subset"]).read_text().splitlines() if l.strip() and not l.startswith("#")}
        manifest = {k: v for k, v in manifest.items() if k in keep}
    results, cpu = a.results, a.cpu
    if results is None:
        cpu = cpu or cpu_model()
        binary = a.bin or str(ROOT / "core/voice/target/release/silas-bench")
        results = HERE / "out" / "ratchet-silas.jsonl"
        results.parent.mkdir(exist_ok=True)
        run_silas(cfg, binary, not a.real_time, results)
    rows = score.read_rows(results)
    gaps = incomplete(rows, manifest)
    for g in gaps:
        print("INCOMPLETE", g)
    if gaps:
        return 1
    new = metrics(rows, manifest)
    group = cfg.get("baselines", {}).get(a.baseline, {})
    key = cpu or "unknown"
    base = group.get(key)
    candidates = {key: base} if base is not None else group

    if a.allow_worse and not cpu:
        p.error("--allow-worse records one CPU's baseline: name it with --cpu")
    print(json.dumps({**new, "cpu": cpu}, indent=2))
    if a.new_cpu:
        if base is not None or not cpu:
            print(f"--new-cpu only records a CPU the {a.baseline!r} group has no baseline for, named by --cpu or detected")
            return 1
        cfg.setdefault("baselines", {}).setdefault(a.baseline, {})[key] = {**new, "updated": date.today().isoformat()}
        a.ratchet.write_text(json.dumps(cfg, indent=2) + "\n")
        print(f"recorded a first baseline for {key} in {a.baseline!r}")
        return 0
    if not candidates:
        if not a.update:
            print(f"ratchet.json has no {a.baseline!r} baseline yet; run with --update to record it")
            return 1
        bad = []
    else:
        if base is None:
            print(f"note: no {a.baseline!r} baseline for {key}; comparing with the CPUs recorded ({', '.join(group)}). "
                  "Text differs by a few words across CPUs, so passing against any one of them passes.")
            if os.environ.get("GITHUB_ACTIONS"):
                print(f"::warning title=ratchet: unrecorded CPU::{key} has no {a.baseline!r} baseline; "
                      "compared with the closest recorded CPU, which is looser than its own baseline would be")
        found = {k: check(b, new, a.gate) for k, b in candidates.items()}
        nearest = min(found, key=lambda k: len(found[k]))
        bad = found[nearest]
        if bad and base is None:
            print(f"closest baseline: {nearest}")
        if candidates[nearest].get("timing") != new["timing"] and a.gate == "all":
            print(f"latency not compared: the ratchet holds {candidates[nearest].get('timing')} timing, this run is {new['timing']}")
    for b in bad:
        print("REGRESSION", b)
    if a.allow_worse:
        dropped = sorted(k for k in group if k != key)
        cfg.setdefault("baselines", {})[a.baseline] = {
            key: {**new, "updated": date.today().isoformat(), "accepted": a.allow_worse.strip()}}
        a.ratchet.write_text(json.dumps(cfg, indent=2) + "\n")
        print(f"recorded {key} in {a.baseline!r} with {len(bad)} accepted regression(s): {a.allow_worse.strip()}")
        if dropped:
            print(f"dropped {', '.join(dropped)}: recorded before this trade; each reseeds with --update on its next passing run")
        return 0
    if bad:
        return 1
    print("ratchet passes")
    if a.update:
        if base is not None and not no_worse(base, new):
            print("not updated: some metric is worse than the ratchet (within the slack), so it stays put")
            return 0
        if base is None and candidates and not no_worse(candidates[nearest], new):
            print(f"not recorded: {key} is worse than {nearest} on some metric (within the slack); "
                  "seed it with --new-cpu from a run of code that already passed")
            return 0
        cfg.setdefault("baselines", {}).setdefault(a.baseline, {})[key] = {**new, "updated": date.today().isoformat()}
        a.ratchet.write_text(json.dumps(cfg, indent=2) + "\n")
        print(f"updated {a.ratchet} ({a.baseline}, {key})")
    return 0

if __name__ == "__main__":
    sys.exit(main())
