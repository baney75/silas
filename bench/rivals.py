"""Silas against a rival, metric by metric, with a check that fails when Silas falls further behind.

  python3 -I bench/rivals.py opencode --silas bench/out/ratchet-silas.jsonl --gate accuracy,setup
  python3 -I bench/rivals.py opencode --silas rt.jsonl --typing typing.jsonl          every metric
  python3 -I bench/rivals.py opencode --silas rt.jsonl --rival hex.jsonl --typing typing.jsonl --record

bench/rivals/<name>.json is the results file: for each metric, Silas's number and the rival's on the same machine
and clips, who wins, and the margin (positive when Silas leads). The check recomputes Silas's number from the
rows given and fails (exit 1) when Silas's margin is worse than the recorded one by more than the metric's slack:
a lead that shrinks, or a loss that grows. Losses stay in the file as losses.

Gates: accuracy (WER, punctuation F1, formatted error; any CPU), typing (`silas-bench type` on a display, the
recorded machine's CPU only), latency (a real-time run, the recorded machine's CPU only), setup (README.md's install
steps, no run at all). Typing and latency are skipped on any other CPU. --record rewrites the numbers from the rows given (and --rival rows), and refuses when
any margin got worse unless --allow-worse says the loss is known.

Punctuation F1: each typed token's trailing mark is one of three classes (`,` `;` `:` / `.` `!` / `?`). The
reference's and the engine's words are aligned on normalized words (score.py); an aligned pair whose classes
match is a hit, a mark on the engine side alone a false mark, on the reference side alone a miss.
"""

import argparse
import json
import random
import re
import statistics
import sys
from datetime import date
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import ratchet  # noqa: E402
import score  # noqa: E402

MARKS = {",": "comma", ";": "comma", ":": "comma", ".": "stop", "!": "stop", "?": "question"}
GATES = ("accuracy", "typing", "latency", "setup")


# ---------------------------------------------------------------- punctuation


def marked_words(text):
    """(normalized word, mark class or None) for each typed word; a token that normalizes to several words
    carries its mark on the last."""
    out = []
    for tok in score.typed_tokens(text):
        stripped = tok.rstrip("\"')]”’")
        mark = MARKS.get(stripped[-1]) if stripped else None
        ws = score.words(tok)
        for i, w in enumerate(ws):
            out.append((w, mark if i == len(ws) - 1 else None))
    return out


def align(r, h):
    """Levenshtein alignment of two word lists: (i, j) pairs, None on the side that has no word."""
    n, m = len(r), len(h)
    d = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1):
        d[i][0] = i
    for j in range(m + 1):
        d[0][j] = j
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            d[i][j] = min(d[i - 1][j - 1] + (r[i - 1] != h[j - 1]), d[i - 1][j] + 1, d[i][j - 1] + 1)
    pairs, i, j = [], n, m
    while i or j:
        if i and j and d[i][j] == d[i - 1][j - 1] + (r[i - 1] != h[j - 1]):
            pairs.append((i - 1, j - 1))
            i, j = i - 1, j - 1
        elif i and d[i][j] == d[i - 1][j] + 1:
            pairs.append((i - 1, None))
            i -= 1
        else:
            pairs.append((None, j - 1))
            j -= 1
    return pairs[::-1]


def punctuation(ref, hyp):
    """(hits, engine marks, reference marks) for one clip."""
    r, h = marked_words(ref), marked_words(hyp)
    hits = 0
    for i, j in align([w for w, _ in r], [w for w, _ in h]):
        if i is not None and j is not None and r[i][1] and r[i][1] == h[j][1]:
            hits += 1
    return hits, sum(1 for _, m in h if m), sum(1 for _, m in r if m)


def punctuation_f1(rows, manifest):
    hits = marked = wanted = 0
    for row in rows:
        clip = manifest.get(row["id"])
        if clip is None or clip.get("category") == "command":
            continue
        a, b, c = punctuation(clip["ref_formatted"], row.get("hypothesis", ""))
        hits, marked, wanted = hits + a, marked + b, wanted + c
    p = hits / marked if marked else 0.0
    r = hits / wanted if wanted else 0.0
    return 2 * p * r / (p + r) if p + r else 0.0


# ---------------------------------------------------------------- metrics


def median(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def first_word_ms(row):
    """Key down to the first text: Silas's t_first_ms; a batch engine sees the clip only after key up."""
    if row.get("engine") == "silas":
        return row.get("t_first_ms")
    return row["audio_seconds"] * 1000 + row["t_final_ms"]


def timed(rows):
    """Rows whose times mean dictation latency: Silas paced in real time, or a rival's warm engine (not one-shot)."""
    return bool(rows) and all(r.get("paced") if r.get("engine") == "silas" else r.get("mode") != "one-shot"
                              for r in rows)


POLLED = ("time_to_first_word_ms", "wait_after_key_up_ms", "wait_after_key_up_p95_ms")


def polled(rows):
    """True when a rival's times come in whole steps of one poll interval, whatever the clip length: then they
    measure how often a CLI polls its service, not how long the engine takes (Hex's `hex transcribe`, ~612 ms)."""
    times = [r["t_final_ms"] for r in rows if r.get("engine") != "silas" and r.get("t_final_ms")]
    if len(times) < 10:
        return False
    step = min(times)
    if step < 200:
        return False
    on_step = sum(abs(t / step - round(t / step)) < 0.05 for t in times)
    return on_step >= 0.9 * len(times)


def measure(rows, manifest, typing=None):
    """Every metric these rows can give, keyed as in the results file. Times only from timed rows."""
    s = score.summarize(rows, manifest, score.normalize_silas, random.Random(score.SEED))["overall"]
    out = {
        "wer": round(s["wer"], 6),
        "formatted_error": round(s["formatted_error"], 6),
        "punctuation_f1": round(punctuation_f1(rows, manifest), 6),
    }
    if timed(rows):
        out["time_to_first_word_ms"] = round(median(first_word_ms(r) for r in rows), 1)
        out["wait_after_key_up_ms"] = round(s["t_final_ms_median"], 1)
        out["wait_after_key_up_p95_ms"] = round(s["t_final_ms_p95"], 1)
        out["peak_rss_mb"] = round(s["peak_rss_mb"], 1)
        out["cpu_s_per_audio_min"] = round(s["cpu_s_per_audio_min"], 2)
        if polled(rows):
            out.update(dict.fromkeys(POLLED))  # unmeasured, not a win or a loss
    if typing:
        out["typing_ms"] = round(median(t["type_ms"] for t in typing), 1)
    return out


def silas_metrics(sets):
    """Silas's metrics from several runs: accuracy from a --fast run when there is one (CI compares --fast runs),
    times from a real-time run."""
    out = {}
    for m in sorted(sets, key=lambda m: "time_to_first_word_ms" in m):
        for k, v in m.items():
            out.setdefault(k, v)
    return out


def best(sets, metrics):
    """The rival's best number per metric across its runs."""
    out = {}
    for m in sets:
        for k, v in m.items():
            if v is None:
                out.setdefault(k, None)
                continue
            if out.get(k) is not None and k in metrics:
                v = min(out[k], v) if metrics[k]["better"] == "lower" else max(out[k], v)
            out[k] = v
    return out


def readme_first_run_items(readme):
    """Numbered items under README.md's "### First run"."""
    text = readme.read_text()
    m = re.search(r"^### First run\n(.*?)(?=^#)", text, re.S | re.M)
    return len(re.findall(r"^\d+\. ", m.group(1), re.M)) if m else 0


def setup_steps(entry, readme):
    """Silas's setup steps, each still documented in README.md, and the README's First run list unchanged."""
    text = readme.read_text()
    problems = [f"README.md no longer says {s['quote']!r} ({s.get('step') or s['what']!r})"
                for s in entry["silas_steps"] + entry.get("silas_no_step", []) if s["quote"] not in text]
    items = readme_first_run_items(readme)
    if items != entry["readme_first_run_items"]:
        problems.append(f"README.md's First run has {items} items; bench/rivals has {entry['readme_first_run_items']}:"
                        " count any new step in silas_steps")
    return len(entry["silas_steps"]), problems


def margin(better, silas, rival):
    """Positive when Silas leads."""
    return (rival - silas) if better == "lower" else (silas - rival)


def winner(better, silas, rival, tie=0.0):
    m = margin(better, silas, rival)
    return "tie" if abs(m) <= tie else ("silas" if m > 0 else "rival")


def check(results, current, gates, cpu, new_rival=None):
    """Messages for every metric where Silas's margin over the scored rival got worse than its slack allows.
    A gated metric these rows don't measure fails: leaving out --silas or --typing must not pass. With
    `new_rival` (--record), the margin now is against the rival's new numbers and is held to the recorded one."""
    fails = []
    for key, m in results["metrics"].items():
        if m["gate"] not in gates:
            continue
        if m["gate"] in ("latency", "typing") and cpu != results["machine"]["cpu"]:
            print(f"skip {key}: recorded on {results['machine']['cpu']}, this is {cpu}")
            continue
        rival = m["rival"]
        if rival is None or m["silas"] is None:
            continue
        if current.get(key) is None:
            fails.append(f"{key}: not measured by these rows (gate {m['gate']!r} asked for it)")
            continue
        if new_rival and m.get("rival_source", "measured") == "measured" and new_rival.get(key) is not None:
            rival = new_rival[key]
        now, then = margin(m["better"], current[key], rival), margin(m["better"], m["silas"], m["rival"])
        verdict = winner(m["better"], current[key], rival, m.get("tie", 0.0))
        line = f"{key}: Silas {current[key]} vs {results['scored']} {rival} ({verdict}; margin {now:+.4g}, recorded {then:+.4g})"
        if now < then - m["slack"]:
            fails.append(line + f": Silas fell behind by more than {m['slack']}")
        else:
            print("ok   " + line)
    return fails


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("rival", help="bench/rivals/<rival>.json")
    p.add_argument("--silas", type=Path, action="append", default=[],
                   help="Silas's rows (silas-bench run); repeat for a --fast run and a real-time run")
    p.add_argument("--rival-rows", "--rival", dest="rival_rows", type=Path, action="append", default=[],
                   help="the scored rival's rows (with --record); repeat to keep its best run per metric")
    p.add_argument("--typing", type=Path, help="silas-bench type output for the same rows")
    p.add_argument("--manifest", type=Path, default=HERE / "manifest.jsonl")
    p.add_argument("--gate", default=",".join(GATES), help="comma-separated: " + ", ".join(GATES))
    p.add_argument("--cpu", help="the CPU the rows came from (default: this machine's)")
    p.add_argument("--record", action="store_true", help="rewrite the recorded numbers from these rows")
    p.add_argument("--allow-worse", action="store_true", help="with --record: write even where Silas fell behind")
    a = p.parse_args(argv)

    path = HERE / "rivals" / f"{a.rival}.json"
    results = json.loads(path.read_text())
    gates = set(a.gate.split(","))
    if unknown := gates - set(GATES):
        p.error(f"unknown gate {', '.join(sorted(unknown))}")
    manifest = score.load_manifest(a.manifest)
    load = score.read_rows
    # Every row set must cover the recorded subset exactly, or easy clips alone could pass against the full baseline.
    keep = {l.strip() for l in (ROOT / results["subset"]).read_text().splitlines() if l.strip() and not l.startswith("#")}
    want = {k: v for k, v in manifest.items() if k in keep}
    for f in [*a.silas, *a.rival_rows, *([a.typing] if a.typing else [])]:
        if gaps := ratchet.incomplete(load(f), want):
            print("\n".join(f"INCOMPLETE {f.name}: {g}" for g in gaps))
            return 1
    typing = load(a.typing) if a.typing else None
    current = silas_metrics([measure(load(f), manifest, typing) for f in a.silas])
    steps, problems = setup_steps(results["setup"], ROOT / "README.md")
    current["setup_steps"] = steps
    cpu = a.cpu or ratchet.cpu_model()

    if a.record:
        if not (a.silas and a.rival_rows):
            p.error("--record needs --silas and --rival")
        if gates & {"latency", "typing"} and cpu != results["machine"]["cpu"]:
            p.error(f"--record of latency or typing needs rows from {results['machine']['cpu']}, where the rival's"
                    f" times were measured; this is {cpu}. Record --gate accuracy,setup, or re-measure both sides")
        rival = best([measure(load(f), manifest) for f in a.rival_rows], results["metrics"])
        fails = [] if a.allow_worse else check(results, current, gates, cpu, new_rival=rival)
        if fails or problems:
            print("\n".join(fails + problems))
            print("not recorded: Silas fell behind (pass --allow-worse to record a known loss)")
            return 1
        for key, m in results["metrics"].items():
            if key not in current or m["gate"] not in gates:
                continue
            if m.get("rival_source", "measured") in ("measured", "unmeasured"):
                m["rival"] = rival.get(key, m["rival"])
                m["rival_source"] = "measured" if m["rival"] is not None else "unmeasured"
            m["silas"] = current[key]
            m["winner"] = winner(m["better"], m["silas"], m["rival"], m.get("tie", 0.0)) if m["rival"] is not None else "not scored"
            m["margin"] = round(margin(m["better"], m["silas"], m["rival"]), 6) if m["rival"] is not None else None
        if gates & {"latency", "typing"}:  # an accuracy-only record keeps the CPU the timings came from
            results["machine"]["cpu"] = cpu
        results["recorded"] = date.today().isoformat()
        path.write_text(json.dumps(results, indent=2) + "\n")
        print(f"recorded {path.relative_to(ROOT)}")
        return 0

    fails = check(results, current, gates, cpu) + (problems if "setup" in gates else [])
    if fails:
        print("\n".join("FAIL " + f for f in fails))
        return 1
    print(f"Silas held its ground against {results['rival']} on every gated metric ({', '.join(sorted(gates))})")
    return 0



if __name__ == "__main__":
    sys.exit(main())
