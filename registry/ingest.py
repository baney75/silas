#!/usr/bin/env python3
"""Turn a device-report issue into a stored report and rebuild registry/devices.json.

    ISSUE_NUMBER=12 ISSUE_BODY="$(cat body.md)" python3 registry/ingest.py --check   # validate only
    ISSUE_NUMBER=12 ISSUE_BODY="$(cat body.md)" python3 registry/ingest.py           # store + rebuild
    python3 registry/ingest.py --rebuild    # rewrite devices.json from every stored report
    python3 registry/ingest.py --verify     # validate every stored report (CI)

The issue body is untrusted. It is only ever parsed as JSON and checked field by field;
nothing from it is executed or written except the validated values. Exit 0 when the report is
valid (and stored), 2 when it is rejected (the reason is printed for the issue comment).

The bars and the pick rule must match src-tauri/src/benchmark.rs.
"""

import json
import math
import os
import re
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPORTS = ROOT / "reports"
DEVICES = ROOT / "devices.json"

MIN_SPEED = 10.0
MAX_WER = 0.08
TIE_WER = 0.005
# Models Silas can install. Add new ids here when the catalog in models.rs grows.
KNOWN_MODELS = {"parakeet-v3", "cohere-int4"}
MAX_BODY = 20_000
# The bundled set in src-tauri/bench (manifest.json). Every honest report measured exactly this
# much audio; update these when the set changes.
SUITE_CLIPS = 9
SUITE_SECONDS = 62.2
# No CPU transcribes this set 500x faster than real time; a bigger number is made up.
MAX_SPEED = 500.0

KEY_RE = re.compile(r"^(macos|windows|linux)/[a-z0-9]+(?:-[a-z0-9]+)*/[0-9]{1,4}$")
# CPU names and OS versions need only these; no brackets, backticks or markup reach a comment.
TEXT_RE = re.compile(r"^[A-Za-z0-9 ()@.,_+/:-]{1,120}$")
HW_RE = re.compile(r"^[A-Za-z0-9,._ -]{1,40}$")
VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]{1,40})?$")


class Reject(Exception):
    pass


def slug(s: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")


def extract(body: str) -> dict:
    if len(body) > MAX_BODY:
        raise Reject("The report is too long.")
    m = re.search(r"```(?:json)?\s*\n(.*?)\n```", body, re.S)
    text = m.group(1) if m else body[body.find("{") : body.rfind("}") + 1]
    try:
        data = json.loads(text)
    except (ValueError, RecursionError):
        raise Reject("The report isn't valid JSON. Please submit it from Silas without editing it.")
    if not isinstance(data, dict):
        raise Reject("The report isn't a JSON object.")
    return data


def num(v, lo, hi, what, integer=False):
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise Reject(f"{what} must be a number.")
    # Compare before isfinite: a 400-digit int can't convert to float.
    if isinstance(v, float) and not math.isfinite(v):
        raise Reject(f"{what} is out of range.")
    if not lo <= v <= hi:
        raise Reject(f"{what} is out of range.")
    if integer and int(v) != v:
        raise Reject(f"{what} must be a whole number.")
    return int(v) if integer else float(v)


def text(v, what, pattern=TEXT_RE):
    if not isinstance(v, str) or not pattern.fullmatch(v):
        raise Reject(f"{what} has characters or a length Silas doesn't produce.")
    return v


def validate(data: dict) -> dict:
    if data.get("schema") != 1:
        raise Reject("Unknown report schema.")
    silas = text(data.get("silas"), "silas", VERSION_RE)
    d = data.get("device")
    if not isinstance(d, dict):
        raise Reject("The report has no device.")
    os_name = d.get("os")
    if os_name not in ("macos", "windows", "linux"):
        raise Reject("Unknown operating system.")
    cpu = text(d.get("cpu"), "device.cpu")
    cores = num(d.get("cores"), 1, 1024, "device.cores", integer=True)
    key = text(d.get("key"), "device.key", KEY_RE)
    if key != f"{os_name}/{slug(cpu)}/{cores}":
        raise Reject("device.key doesn't match the device's OS, CPU and core count.")
    device = {
        "key": key,
        "os": os_name,
        "os_version": text(d.get("os_version"), "device.os_version"),
        "cpu": cpu,
        "cores": cores,
        "memory_gb": num(d.get("memory_gb"), 0, 65536, "device.memory_gb", integer=True),
    }
    if d.get("hw_model") is not None:
        device["hw_model"] = text(d["hw_model"], "device.hw_model", HW_RE)

    results = data.get("results")
    if not isinstance(results, list) or not 1 <= len(results) <= 10:
        raise Reject("The report needs between 1 and 10 model results.")
    clean, seen = [], set()
    for r in results:
        if not isinstance(r, dict):
            raise Reject("Each result must be an object.")
        model = r.get("model")
        if not isinstance(model, str) or model not in KNOWN_MODELS:
            raise Reject("The report names a model Silas doesn't offer.")
        if model in seen:
            raise Reject("The report lists a model twice.")
        seen.add(model)
        res = {
            "model": model,
            "speed": num(r.get("speed"), 0.01, MAX_SPEED, f"{model} speed"),
            "wer": num(r.get("wer"), 0, 1, f"{model} wer"),
            "clips": num(r.get("clips"), 1, 1000, f"{model} clips", integer=True),
            "audio_seconds": num(r.get("audio_seconds"), 1, 100_000, f"{model} audio_seconds"),
            "compute_seconds": num(r.get("compute_seconds"), 0.0001, 100_000, f"{model} compute_seconds"),
        }
        # The numbers have to come from Silas's own set and agree with each other.
        if res["clips"] != SUITE_CLIPS or abs(res["audio_seconds"] - SUITE_SECONDS) > 1.0:
            raise Reject(f"{model} wasn't measured on Silas's benchmark set.")
        if abs(res["audio_seconds"] / res["compute_seconds"] - res["speed"]) > 0.02 * res["speed"]:
            raise Reject(f"{model} speed doesn't match its audio and compute seconds.")
        clean.append(res)
    return {"schema": 1, "silas": silas, "device": device, "results": clean}


def fastest(rs):
    """The fastest; on an exact tie the lower model id, as in Silas."""
    return min(rs, key=lambda r: (-r["speed"], r["model"]))["model"]


def pick(results):
    """Same rule as benchmark::pick in Silas."""
    if not results:
        return None
    passing = [r for r in results if r["speed"] >= MIN_SPEED and r["wer"] <= MAX_WER]
    if passing:
        best = min(r["wer"] for r in passing)
        return fastest([r for r in passing if r["wer"] - best <= TIE_WER])
    accurate = [r for r in results if r["wer"] <= MAX_WER]
    return fastest(accurate or results)


def stored():
    """Every stored report, each validated again: hand-made files get no free pass."""
    for f in sorted(REPORTS.glob("*.json")):
        try:
            yield validate(json.loads(f.read_text()))
        except (Reject, ValueError) as e:
            raise SystemExit(f"{f.name}: {e}")


def rebuild(write: bool = True) -> dict:
    """Median speed and WER per device and model across every stored report. Full precision is
    kept: Silas compares these numbers with the bars, so rounding could pass a model that missed."""
    by_device: dict = {}
    for rep in stored():
        dev = by_device.setdefault(rep["device"]["key"], {"reports": 0, "models": {}})
        dev["reports"] += 1
        for r in rep["results"]:
            m = dev["models"].setdefault(r["model"], {"speed": [], "wer": []})
            m["speed"].append(r["speed"])
            m["wer"].append(r["wer"])
    devices = {}
    for key, dev in sorted(by_device.items()):
        meds = [
            {"model": m, "speed": statistics.median(v["speed"]), "wer": statistics.median(v["wer"])}
            for m, v in sorted(dev["models"].items())
        ]
        best = pick(meds)
        chosen = next(r for r in meds if r["model"] == best)
        devices[key] = {
            "model": best,
            "speed": chosen["speed"],
            "wer": chosen["wer"],
            "reports": dev["reports"],
        }
    out = {"schema": 1, "devices": devices}
    if write:
        DEVICES.write_text(json.dumps(out, indent=2) + "\n")
    return out


def main() -> int:
    args = sys.argv[1:]
    if args == ["--rebuild"]:
        rebuild()
        return 0
    if args == ["--verify"]:
        n = sum(1 for _ in stored())
        print(f"{n} stored report(s) valid.")
        return 0
    check_only = args == ["--check"]
    if args and not check_only:
        print(__doc__, file=sys.stderr)
        return 1
    number = os.environ.get("ISSUE_NUMBER", "")
    if not number.isdigit():
        print("ISSUE_NUMBER must be the issue number.", file=sys.stderr)
        return 1
    try:
        report = validate(extract(os.environ.get("ISSUE_BODY", "")))
    except Reject as e:
        print(e)
        return 2
    except Exception:  # anything unexpected is still a rejection with a reason, never a crash
        print("The report couldn't be read. Please submit it from Silas without editing it.")
        return 2
    # Only validated slug and id text goes into a comment, never free text from the issue.
    key = report["device"]["key"]
    if check_only:
        print(f"This report is valid (`{key}`). A maintainer will review it and add it to the registry.")
        return 0
    REPORTS.mkdir(exist_ok=True)
    (REPORTS / f"{int(number)}.json").write_text(json.dumps(report, indent=2) + "\n")
    entry = rebuild()["devices"][key]
    print(f"Added to the registry. `{key}` now gets `{entry['model']}`, from {entry['reports']} report(s). Thank you!")
    return 0


if __name__ == "__main__":
    sys.exit(main())
