"""Scores results.jsonl files from `silas-bench run` and the rival adapters against a manifest, and writes
bench/out/report.json and a self-contained bench/out/report.html.

  python3 -I bench/score.py --manifest bench/manifest.jsonl bench/out/*.jsonl

Metrics, per engine and per category:
  wer             word error rate on ref_verbatim, pooled over clips (errors / reference words), with a
                  bootstrap 95% interval over clips. By default text is normalized like core/voice/src/benchmark.rs,
                  then numbers are written by the bench set's numbers rule (10 and above in digits, zero to nine
                  in words) on both sides; --whisper-norm uses Whisper-style English normalization (numbers,
                  spellings, fillers, currency and percent) instead.
  formatted_error token error on ref_formatted, case and punctuation sensitive: what the app types. Numbers
                  follow the numbers rule on both sides here too.
  command_acc     command clips only: the hypothesis must parse as JSON and equal the reference action list.
                  An engine with no command mode scores 0, and the report says so.
  t_first / t_final  median and p95 milliseconds; median RTF; peak memory; CPU seconds per audio minute (a
                  battery proxy); energy per audio minute when RAPL was readable.
Each engine is compared with Silas per category by a paired bootstrap of the WER difference; a difference
whose interval includes zero is a tie, not a win.
Python standard library only.
"""

import argparse
import html
import json
import math
import random
import re
import statistics
import sys
import unicodedata
from datetime import datetime, timezone
from pathlib import Path

BOOTSTRAP = 1000
SEED = 20261117

# ---------------------------------------------------------------- normalization


def fold(s):
    """Accents off ("caf\u00e9" -> "cafe"), as bench set v1 writes ref_verbatim."""
    return "".join(c for c in unicodedata.normalize("NFKD", s) if not unicodedata.combining(c))


def normalize(s):
    """benchmark.rs `normalize` with accents folded first: lowercase; `-` and the em dash become
    spaces; keep letters, digits, apostrophes and whitespace; collapse whitespace."""
    s = fold(s.lower()).replace("-", " ").replace("\u2014", " ")
    kept = "".join(c for c in s if c.isalnum() or c == "'" or c.isspace())
    return " ".join(kept.split())


ONES = {
    "zero": 0, "oh": None, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7,
    "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12, "thirteen": 13, "fourteen": 14,
    "fifteen": 15, "sixteen": 16, "seventeen": 17, "eighteen": 18, "nineteen": 19,
}
TENS = {"twenty": 20, "thirty": 30, "forty": 40, "fifty": 50, "sixty": 60, "seventy": 70, "eighty": 80, "ninety": 90}
SCALES = {"hundred": 100, "thousand": 1000, "million": 10**6, "billion": 10**9}
ORDINALS = {
    "first": 1, "second": 2, "third": 3, "fourth": 4, "fifth": 5, "sixth": 6, "seventh": 7, "eighth": 8,
    "ninth": 9, "tenth": 10, "eleventh": 11, "twelfth": 12, "thirteenth": 13, "fourteenth": 14,
    "fifteenth": 15, "sixteenth": 16, "seventeenth": 17, "eighteenth": 18, "nineteenth": 19,
    "twentieth": 20, "thirtieth": 30, "fortieth": 40, "fiftieth": 50, "sixtieth": 60, "seventieth": 70,
    "eightieth": 80, "ninetieth": 90, "hundredth": 100, "thousandth": 1000,
}
SPELLINGS = {
    "colour": "color", "favour": "favor", "favourite": "favorite", "honour": "honor", "labour": "labor",
    "neighbour": "neighbor", "behaviour": "behavior", "humour": "humor", "centre": "center", "theatre": "theater",
    "metre": "meter", "litre": "liter", "organise": "organize", "organised": "organized", "realise": "realize",
    "realised": "realized", "recognise": "recognize", "analyse": "analyze", "catalogue": "catalog",
    "grey": "gray", "travelled": "traveled", "travelling": "traveling", "cancelled": "canceled",
    "programme": "program", "licence": "license", "defence": "defense", "offence": "offense", "practise": "practice",
    "jewellery": "jewelry", "okay": "ok", "alright": "all right",
}
FILLERS = {"hmm", "mm", "mhm", "mmm", "uh", "um", "umm", "uhm", "er", "erm", "ah"}


def _number_words(words):
    """Replaces runs of spelled-out numbers with digits: "twenty five" -> "25", "one hundred and two" -> "102",
    "twenty first" -> "21st". "one two three" stays three numbers, and a lone "first" or "second" stays a word."""
    out, i = [], 0
    while i < len(words):
        j, total, current, last, suffix = i, 0, 0, None, ""
        while j < len(words):
            w = words[j]
            if w in ONES and ONES[w] is not None:
                if last not in (None, "scale", "and") and not (last == "tens" and current % 10 == 0):
                    break
                current += ONES[w]
                last = "ones"
            elif w in TENS:
                # Years and similar pairs: "nineteen ninety", "twenty twenty".
                if last in ("ones", "tens") and 10 <= current < 100 and total == 0:
                    current = current * 100 + TENS[w]
                elif last not in (None, "scale", "and"):
                    break
                else:
                    current += TENS[w]
                last = "tens"
            elif w in SCALES and last in ("ones", "tens", "scale"):
                if SCALES[w] == 100:
                    current = (current or 1) * 100
                else:
                    total += (current or 1) * SCALES[w]
                    current = 0
                last = "scale"
            elif w in ORDINALS and (last is not None or w not in ("first", "second")):
                n = ORDINALS[w]
                if n >= 100 and last in ("ones", "tens", "scale"):
                    current = (current or 1) * n
                elif last == "ones" or (last == "tens" and current % 10):
                    break
                else:
                    current += n
                suffix = {1: "st", 2: "nd", 3: "rd"}.get(0 if n % 100 in (11, 12, 13) else n % 10, "th")
                last = "ordinal"
                j += 1
                break
            elif w == "and" and last == "scale" and j + 1 < len(words) and (words[j + 1] in ONES or words[j + 1] in TENS):
                last = "and"
            else:
                break
            j += 1
        if last is not None:
            out.append(f"{total + current}{suffix}")
            i = j
        else:
            out.append(words[i])
            i += 1
    return out


def _spelling(w):
    if w in SPELLINGS:
        return SPELLINGS[w]
    if w.endswith("s") and w[:-1] in SPELLINGS:
        return SPELLINGS[w[:-1]] + "s"
    return w


WORDS_UNDER_TEN = {n: w for w, n in ONES.items() if n is not None and n < 10}
ORDINALS_UNDER_TEN = {n: w for w, n in ORDINALS.items() if n < 10}


def numbers_rule(words):
    """The bench set's numbers rule (bench/README.md), applied the same way to the reference and every engine:
    10 and above in digits, zero to nine in words. Books spell "a girl of sixteen" and an app types "a girl of 16";
    both come out "16", and "3" and "three" both come out "three". `words` are lowercase tokens."""
    words = [w.replace(",", "") if re.fullmatch(r"\d{1,3}(,\d{3})+", w) else w for w in words]
    # "a hundred" and "a thousand" are numbers too.
    words = ["one" if w == "a" and i + 1 < len(words) and words[i + 1] in SCALES else w for i, w in enumerate(words)]
    out = []
    for w in _number_words(words):
        m = re.fullmatch(r"(\d)(st|nd|rd|th)?", w)
        if m:
            n = int(m.group(1))
            w = ORDINALS_UNDER_TEN.get(n, w) if m.group(2) else WORDS_UNDER_TEN[n]
        out.append(w)
    return out


def normalize_silas(s):
    """The default for WER: `normalize`, then the numbers rule."""
    return " ".join(numbers_rule(normalize(s).split()))


def normalize_whisper(s):
    """Whisper-style English normalization, then `normalize`. Applied to both sides, so "5" and "five",
    "colour" and "color", "10%" and "ten percent" all match."""
    s = s.lower()
    s = re.sub(r"[\[(<][^\])>]*[\])>]", " ", s)
    s = re.sub(r"(\d),(\d{3})", r"\1\2", s)
    s = re.sub(r"\$\s?(\d+(?:\.\d+)?)", r"\1 dollars", s)
    s = re.sub(r"(\d)\s?%", r"\1 percent", s)
    s = s.replace("&", " and ")
    words = normalize(s).split()
    words = [_spelling(w) for w in words if w not in FILLERS]
    words = _number_words(words)
    return " ".join(words)


# ---------------------------------------------------------------- errors


def edits(r, h):
    """Levenshtein distance between two token lists (substitution, insertion and deletion each cost 1)."""
    prev = list(range(len(h) + 1))
    for i, rw in enumerate(r):
        cur = [i + 1]
        for j, hw in enumerate(h):
            cur.append(min(prev[j] + (rw != hw), prev[j + 1] + 1, cur[j] + 1))
        prev = cur
    return prev[len(h)]


def word_errors(ref, hyp, norm=normalize_silas):
    r, h = norm(ref).split(), norm(hyp).split()
    return edits(r, h), len(r)


def typed_tokens(s):
    """Whitespace-separated tokens of typed text; a line break is a token of its own."""
    return s.replace("\n", " \u00b6 ").split()


def typed_numbers(tokens):
    """The numbers rule on typed tokens, keeping the punctuation around them: "sixteen," and "16," are both "16,",
    "twenty-five" and "25" both "25". A number the rule rewrites loses its capital ("Sixteen" at the start of a
    sentence); everything else is left as typed."""
    out, run = [], []

    def flush():
        words = [w for _, c, _, _ in run for w in c]
        core = numbers_rule(words)
        if core != words:
            out.append(run[0][0] + " ".join(core) + run[-1][2])
        else:
            out.extend(t for *_, t in run)
        run.clear()

    for t in tokens:
        m = re.fullmatch(r"([^\w$]*)([\w',-]*?\w)([^\w]*)", t)
        parts = m.group(2).lower().split("-") if m else []
        number = m and all(p in ONES or p in TENS or p in SCALES or p in ORDINALS or p == "and" or p == "a"
                           or re.fullmatch(r"[\d,]+(st|nd|rd|th)?", p) for p in parts)
        if number:
            if run and (run[-1][2] or m.group(1)):
                flush()
            run.append((m.group(1), parts, m.group(3), t))
        else:
            flush()
            out.append(t)
    flush()
    return " ".join(out).split()


def format_errors(ref, hyp):
    """Token errors on the text as typed: case, punctuation and line breaks count. Numbers follow the numbers rule
    on both sides, so writing "16" where a book spells "sixteen" is not an error."""
    r, h = typed_numbers(typed_tokens(ref)), typed_numbers(typed_tokens(hyp))
    return edits(r, h), len(r)


def command_match(ref, hyp):
    try:
        want = json.loads(ref) if isinstance(ref, str) else ref
        got = json.loads(hyp)
    except (ValueError, TypeError):
        return False
    return isinstance(got, list) and got == want


def is_command_output(hyp):
    try:
        return isinstance(json.loads(hyp), list)
    except (ValueError, TypeError):
        return False


# ---------------------------------------------------------------- stats


def pooled(pairs):
    errors = sum(e for e, _ in pairs)
    words = sum(w for _, w in pairs)
    if words == 0:
        return 0.0 if errors == 0 else 1.0
    return errors / words


def bootstrap(pairs, rng):
    """95% interval of the pooled rate, resampling clips."""
    if len(pairs) < 2:
        r = pooled(pairs)
        return [r, r]
    rates = sorted(pooled([pairs[rng.randrange(len(pairs))] for _ in pairs]) for _ in range(BOOTSTRAP))
    return [rates[int(0.025 * BOOTSTRAP)], rates[int(0.975 * BOOTSTRAP) - 1]]


def paired_diff(a, b, rng):
    """Pooled WER of b minus a over the same clips, and its bootstrap 95% interval."""
    diff = pooled(b) - pooled(a)
    if len(a) < 2:
        return diff, [diff, diff]
    ds = []
    for _ in range(BOOTSTRAP):
        idx = [rng.randrange(len(a)) for _ in a]
        ds.append(pooled([b[i] for i in idx]) - pooled([a[i] for i in idx]))
    ds.sort()
    return diff, [ds[int(0.025 * BOOTSTRAP)], ds[int(0.975 * BOOTSTRAP) - 1]]


def pct(xs, q):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    k = (len(xs) - 1) * q
    lo, hi = math.floor(k), math.ceil(k)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def median(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


# ---------------------------------------------------------------- scoring


def load_manifest(path):
    return {c["id"]: c for c in (json.loads(l) for l in Path(path).read_text().splitlines() if l.strip())}


def load_results(paths):
    """{label: [rows]}. The label is "engine model"; a second file with the same label gets its file name."""
    runs = {}
    for p in paths:
        rows = [json.loads(l) for l in Path(p).read_text().splitlines() if l.strip()]
        if not rows:
            continue
        label = f"{rows[0]['engine']} {rows[0]['model']}"
        if label in runs:
            label = f"{label} ({Path(p).stem})"
        runs[label] = rows
    return runs


def verbatim(row):
    """The text WER is scored on: the model's own words (`hypothesis_raw`), before any cleanup removes the
    fillers ref_verbatim keeps. What the app types is scored by formatted error."""
    return row.get("hypothesis_raw") or row.get("hypothesis", "")


def summarize(rows, manifest, norm=normalize_silas, rng=None):
    """Metrics for one engine's rows: overall and per category."""
    rng = rng or random.Random(SEED)
    rows = [r for r in rows if r["id"] in manifest]
    has_commands = any(is_command_output(r.get("hypothesis", "")) for r in rows
                       if manifest[r["id"]].get("category") == "command")

    def block(rs):
        clips = [manifest[r["id"]] for r in rs]
        speech = [(r, c) for r, c in zip(rs, clips) if c.get("category") != "command"]
        commands = [(r, c) for r, c in zip(rs, clips) if c.get("category") == "command"]
        werp = [word_errors(c["ref_verbatim"], verbatim(r), norm) for r, c in speech]
        fmtp = [format_errors(c["ref_formatted"], r.get("hypothesis", "")) for r, c in speech]
        audio = sum(r.get("audio_seconds") or 0 for r in rs)
        cpu = [r.get("cpu_seconds") for r in rs]
        energy = [r.get("energy_j") for r in rs]
        out = {
            "clips": len(rs),
            "wer": pooled(werp) if werp else None,
            "wer_ci": bootstrap(werp, rng) if werp else None,
            "formatted_error": pooled(fmtp) if fmtp else None,
            "command_acc": (sum(command_match(c["ref_formatted"], r.get("hypothesis", "")) for r, c in commands)
                            / len(commands)) if commands else None,
            "t_first_ms_median": median(r.get("t_first_ms") for r in rs),
            "t_first_ms_p95": pct([r.get("t_first_ms") for r in rs], 0.95),
            "t_final_ms_median": median(r.get("t_final_ms") for r in rs),
            "t_final_ms_p95": pct([r.get("t_final_ms") for r in rs], 0.95),
            "rtf_median": median(r.get("rtf") for r in rs),
            "peak_rss_mb": max((r["peak_rss_mb"] for r in rs if r.get("peak_rss_mb") is not None), default=None),
            "cpu_s_per_audio_min": (sum(cpu) / audio * 60) if audio and all(x is not None for x in cpu) else None,
            "load1_median": median(r.get("load1") for r in rs),
            "load1_max": max((r["load1"] for r in rs if r.get("load1") is not None), default=None),
            "energy_j_per_audio_min": (sum(energy) / audio * 60) if audio and all(x is not None for x in energy) else None,
            "audio_seconds": audio,
        }
        return out, werp

    overall, _ = block(rows)
    cats = {}
    for cat in sorted({manifest[r["id"]].get("category") or "uncategorized" for r in rows}):
        cats[cat], _ = block([r for r in rows if (manifest[r["id"]].get("category") or "uncategorized") == cat])
    load = [r.get("load_ms") for r in rows if r.get("load_ms") is not None]
    overall["load_ms"] = load[0] if load else None
    overall["paced"] = all(r.get("paced") for r in rows) if rows else None
    overall["mode"] = rows[0].get("mode", "dictation path" if rows and rows[0].get("engine") == "silas" else "batch") if rows else None
    return {"overall": overall, "categories": cats, "has_commands": has_commands,
            "missing": sorted(set(manifest) - {r["id"] for r in rows})}


def compare(base_rows, rows, manifest, norm, rng):
    """Per category: WER of `rows` minus WER of `base_rows` on the clips both ran, with a paired interval."""
    base = {r["id"]: r for r in base_rows}
    out = {}
    cats = {}
    for r in rows:
        c = manifest.get(r["id"])
        if c is None or r["id"] not in base or c.get("category") == "command":
            continue
        cats.setdefault(c.get("category") or "uncategorized", []).append(r)
        cats.setdefault("all", []).append(r)
    for cat, rs in sorted(cats.items()):
        a = [word_errors(manifest[r["id"]]["ref_verbatim"], verbatim(base[r["id"]]), norm) for r in rs]
        b = [word_errors(manifest[r["id"]]["ref_verbatim"], verbatim(r), norm) for r in rs]
        d, ci = paired_diff(a, b, rng)
        verdict = "tie" if ci[0] <= 0 <= ci[1] else ("worse" if d > 0 else "better")
        out[cat] = {"clips": len(rs), "wer_diff": d, "ci": ci, "verdict": verdict}
    return out


def score(manifest_path, result_paths, whisper_norm=False):
    manifest = load_manifest(manifest_path)
    runs = load_results(result_paths)
    norm = normalize_whisper if whisper_norm else normalize_silas
    engines = {label: summarize(rows, manifest, norm, random.Random(SEED)) for label, rows in runs.items()}
    base = next((l for l, rows in runs.items() if rows[0]["engine"] == "silas"), None)
    comparisons = {}
    if base:
        for label, rows in runs.items():
            if label != base:
                comparisons[label] = compare(runs[base], rows, manifest, norm, random.Random(SEED))
    notes = [
        "WER is on each engine's raw model text against ref_verbatim. Normalization: " + (
            "Whisper-style English (numbers, spellings, fillers) on top of benchmark.rs normalize with accents folded"
            if whisper_norm else "benchmark.rs normalize with accents folded, then the numbers rule (10 and above in digits, zero to nine in words) on reference and hypothesis alike (the default; --whisper-norm uses Whisper-style English normalization)"),
        "Latency figures carry the machine's 1-minute load (load1) when each clip ran; compare them only at similar load.",
        "Formatted error compares the text as typed, token by token; case and punctuation count.",
        "Batch engines see the whole clip after key up: t_first = t_final = decode time. Model load is reported apart as load_ms.",
    ]
    if any(c.get("category") == "command" for c in manifest.values()):
        for label, e in engines.items():
            if not e["has_commands"]:
                notes.append(f"{label} has no command mode: its command accuracy is 0 by construction.")
    if all(all(r.get("energy_j") is None for r in rows) for rows in runs.values()):
        notes.append("Energy: not available (RAPL /sys/class/powercap needs root on this machine). CPU seconds per audio minute stands in.")
    return {
        "generated": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "manifest": str(manifest_path),
        "clips": len(manifest),
        "normalization": "whisper" if whisper_norm else "silas",
        "baseline": base,
        "engines": engines,
        "comparisons": comparisons,
        "notes": notes,
        "rows": {label: rows for label, rows in runs.items()},
    }


# ---------------------------------------------------------------- html


def fmt(v, kind):
    if v is None:
        return '<span class="na">n/a</span>'
    if kind == "pct":
        return f"{v * 100:.1f}%"
    if kind == "ms":
        return f"{v:,.0f}"
    if kind == "x":
        return f"{v:.3f}"
    if kind == "mb":
        return f"{v:,.0f}"
    return f"{v:.1f}"


def ci_text(ci):
    return "" if not ci else f'<span class="ci">{ci[0] * 100:.1f} to {ci[1] * 100:.1f}</span>'


def render(report):
    e = report["engines"]
    labels = sorted(e, key=lambda l: (l != report["baseline"], e[l]["overall"]["wer"] if e[l]["overall"]["wer"] is not None else 9))
    cats = sorted({c for l in labels for c in e[l]["categories"]})
    best = min((e[l]["overall"]["wer"] for l in labels if e[l]["overall"]["wer"] is not None), default=None)
    h = []
    h.append("<table><thead><tr><th>Engine</th><th>WER</th><th>Formatted error</th><th>Command acc.</th>"
             "<th>Final wait median (ms)</th><th>Final wait p95 (ms)</th><th>First text median (ms)</th>"
             "<th>RTF median</th><th>Peak memory (MB)</th><th>CPU s / audio min</th><th>Energy J / audio min</th><th>Model load (ms)</th><th>load1 median / max</th></tr></thead><tbody>")
    for l in labels:
        o = e[l]["overall"]
        bar = "" if o["wer"] is None else f'<span class="bar" style="width:{min(100, o["wer"] * 400):.0f}%"></span>'
        mark = ' class="base"' if l == report["baseline"] else ""
        cmd = fmt(o["command_acc"], "pct") + ("" if e[l]["has_commands"] or o["command_acc"] is None else ' <span class="na">no command mode</span>')
        missing = f", missing {len(e[l]['missing'])} clips" if e[l]["missing"] else ""
        h.append(f"<tr{mark}><td>{html.escape(l)}<div class=\"sub\">{html.escape(str(o.get('mode')))}{missing}</div></td>"
                 f"<td class=\"num\">{fmt(o['wer'], 'pct')} {ci_text(o['wer_ci'])}<div class=\"track\">{bar}</div></td>"
                 f"<td class=\"num\">{fmt(o['formatted_error'], 'pct')}</td><td class=\"num\">{cmd}</td>"
                 f"<td class=\"num\">{fmt(o['t_final_ms_median'], 'ms')}</td><td class=\"num\">{fmt(o['t_final_ms_p95'], 'ms')}</td>"
                 f"<td class=\"num\">{fmt(o['t_first_ms_median'], 'ms')}</td><td class=\"num\">{fmt(o['rtf_median'], 'x')}</td>"
                 f"<td class=\"num\">{fmt(o['peak_rss_mb'], 'mb')}</td><td class=\"num\">{fmt(o['cpu_s_per_audio_min'], '')}</td>"
                 f"<td class=\"num\">{fmt(o['energy_j_per_audio_min'], '')}</td><td class=\"num\">{fmt(o['load_ms'], 'ms')}</td>"
                 f"<td class=\"num\">{fmt(o['load1_median'], '')} / {fmt(o['load1_max'], '')}</td></tr>")
    h.append("</tbody></table>")
    summary = "".join(h)

    h = ["<table><thead><tr><th>Engine</th>" + "".join(f"<th>{html.escape(c)}</th>" for c in cats) + "</tr></thead><tbody>"]
    for l in labels:
        cells = []
        for c in cats:
            b = e[l]["categories"].get(c)
            if not b:
                cells.append('<td class="num"><span class="na">not run</span></td>')
            elif c == "command":
                cells.append(f'<td class="num">acc {fmt(b["command_acc"], "pct")}</td>')
            else:
                cells.append(f'<td class="num">{fmt(b["wer"], "pct")} {ci_text(b["wer_ci"])}<div class="sub">fmt {fmt(b["formatted_error"], "pct")} · {b["clips"]} clips</div></td>')
        h.append(f"<tr><td>{html.escape(l)}</td>{''.join(cells)}</tr>")
    h.append("</tbody></table>")
    by_cat = "".join(h)

    comp = ""
    if report["comparisons"]:
        h = ["<table><thead><tr><th>Engine vs " + html.escape(report["baseline"]) + "</th>" +
             "".join(f"<th>{html.escape(c)}</th>" for c in ["all"] + [c for c in cats if c not in ("all", "command")]) + "</tr></thead><tbody>"]
        for l, cs in report["comparisons"].items():
            cells = []
            for c in ["all"] + [c for c in cats if c not in ("all", "command")]:
                x = cs.get(c)
                if not x:
                    cells.append('<td><span class="na">n/a</span></td>')
                    continue
                cls = {"better": "good", "worse": "bad", "tie": "tie"}[x["verdict"]]
                cells.append(f'<td><span class="pill {cls}">{x["verdict"]}</span> {x["wer_diff"] * 100:+.1f} pts'
                             f'<div class="sub">95% {x["ci"][0] * 100:+.1f} to {x["ci"][1] * 100:+.1f}</div></td>')
            h.append(f"<tr><td>{html.escape(l)}</td>{''.join(cells)}</tr>")
        h.append("</tbody></table>")
        comp = "<h2>Against Silas</h2><p class=\"lede\">WER difference on the clips both ran (positive is worse than Silas). A tie means the paired 95% interval includes zero: not a win either way.</p>" + "".join(h)

    clips = []
    for l in labels:
        rows = report["rows"][l]
        clips.append(f"<details><summary>{html.escape(l)}: {len(rows)} clips</summary><table><thead><tr><th>Clip</th><th>Hypothesis</th><th>Final wait (ms)</th><th>load1</th></tr></thead><tbody>")
        for r in rows:
            clips.append(f"<tr><td>{html.escape(r['id'])}</td><td>{html.escape(r.get('hypothesis', ''))}</td><td class=\"num\">{fmt(r.get('t_final_ms'), 'ms')}</td><td class=\"num\">{fmt(r.get('load1'), '')}</td></tr>")
        clips.append("</tbody></table></details>")

    notes = "".join(f"<li>{html.escape(n)}</li>" for n in report["notes"])
    best_line = "" if best is None else f"Best overall WER {best * 100:.1f}%. "
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Silas Speech Bench</title>
<style>
:root {{ --bg:#fbfaf7; --fg:#1d1c1a; --muted:#6b6862; --line:#e4e0d8; --accent:#2f6f5e; --good:#2f6f5e; --bad:#a33b2b; --tie:#7a7366; --track:#efebe3; --base:#f3efe6; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --bg:#161614; --fg:#ecebe7; --muted:#a19d95; --line:#2e2c28; --accent:#6fbfa6; --good:#6fbfa6; --bad:#e2735f; --tie:#a19d95; --track:#24221f; --base:#1f1e1b; }} }}
:root[data-theme="dark"] {{ --bg:#161614; --fg:#ecebe7; --muted:#a19d95; --line:#2e2c28; --accent:#6fbfa6; --good:#6fbfa6; --bad:#e2735f; --tie:#a19d95; --track:#24221f; --base:#1f1e1b; }}
* {{ box-sizing:border-box; }}
body {{ margin:0; background:var(--bg); color:var(--fg); font:15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }}
main {{ max-width:1180px; margin:0 auto; padding:32px 16px 64px; }}
h1 {{ font-size:26px; margin:0 0 4px; letter-spacing:-0.01em; }}
h2 {{ font-size:18px; margin:36px 0 8px; }}
.lede, .meta {{ color:var(--muted); margin:0 0 12px; }}
.scroll {{ overflow-x:auto; }}
table {{ border-collapse:collapse; width:100%; font-variant-numeric:tabular-nums; }}
th, td {{ text-align:left; padding:8px 10px; border-bottom:1px solid var(--line); vertical-align:top; }}
th {{ font-size:12px; font-weight:600; color:var(--muted); text-transform:uppercase; letter-spacing:.04em; }}
td.num {{ white-space:nowrap; }}
tr.base td {{ background:var(--base); }}
.sub, .ci {{ color:var(--muted); font-size:12px; }}
.ci::before {{ content:"95% "; }}
.na {{ color:var(--muted); font-size:12px; }}
.track {{ height:4px; background:var(--track); border-radius:2px; margin-top:4px; }}
.bar {{ display:block; height:4px; background:var(--accent); border-radius:2px; }}
.pill {{ font-size:12px; font-weight:600; padding:1px 8px; border-radius:999px; border:1px solid currentColor; }}
.good {{ color:var(--good); }} .bad {{ color:var(--bad); }} .tie {{ color:var(--tie); }}
details {{ margin:8px 0; }} summary {{ cursor:pointer; font-weight:600; }}
ul {{ padding-left:20px; color:var(--muted); }}
</style></head><body><main>
<h1>Silas speech bench</h1>
<p class="meta">{html.escape(report['manifest'])} · {report['clips']} clips · generated {html.escape(report['generated'])}</p>
<p class="lede">{best_line}Final wait is key up to final text, the number users feel. Lower is better everywhere.</p>
<h2>Engines</h2><div class="scroll">{summary}</div>
<h2>WER by category</h2><div class="scroll">{by_cat}</div>
<div class="scroll">{comp}</div>
<h2>Notes</h2><ul>{notes}</ul>
<h2>Clips</h2>{''.join(clips)}
</main></body></html>
"""


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--manifest", required=True, type=Path)
    p.add_argument("--whisper-norm", action="store_true", help="add Whisper-style English normalization to WER")
    p.add_argument("--out-dir", type=Path, default=Path(__file__).parent / "out")
    p.add_argument("results", nargs="+", type=Path)
    a = p.parse_args(argv)
    report = score(a.manifest, a.results, a.whisper_norm)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / "report.json").write_text(json.dumps({k: v for k, v in report.items() if k != "rows"}, indent=2) + "\n")
    (a.out_dir / "report.html").write_text(render(report))
    for label, e in report["engines"].items():
        o = e["overall"]
        wer = "n/a" if o["wer"] is None else f"{o['wer'] * 100:.1f}%"
        wait = "n/a" if o["t_final_ms_median"] is None else f"{o['t_final_ms_median']:.0f} ms"
        print(f"{label}: WER {wer}, final wait median {wait}, peak {o['peak_rss_mb'] or 0:.0f} MB")
    print(f"wrote {a.out_dir / 'report.json'} and {a.out_dir / 'report.html'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
