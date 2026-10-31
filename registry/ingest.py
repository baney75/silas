#!/usr/bin/env python3
"""Turn a device-report issue into a stored report and rebuild registry/devices.json.

    ISSUE_NUMBER=12 ISSUE_BODY="$(cat body.md)" python3 registry/ingest.py   # store reports/12.json
    python3 registry/ingest.py --rebuild                                      # rewrite devices.json

The issue body is untrusted. It is only ever parsed as JSON and checked field by field;
nothing from it is executed or written except the validated values. Exit 0 with the report
stored, 2 when the report is rejected (the reason is printed for the issue comment).

The bars and the pick rule must match src-tauri/src/benchmark.rs.
"""

import json
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
    except ValueError:
        raise Reject("The report isn't valid JSON. Please submit it from Silas without editing it.")
    if not isinstance(data, dict):
        raise Reject("The report isn't a JSON object.")
    return data


def num(v, lo, hi, what, integer=False):
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise Reject(f"{what} must be a number.")
    if integer and int(v) != v:
        raise Reject(f"{what} must be a whole number.")
    if not lo <= v <= hi:
        raise Reject(f"{what} is out of range.")
    return int(v) if integer else float(v)


def text(v, what, pattern=TEXT_RE):
    if not isinstance(v, str) or not pattern.match(v):
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
        if model not in KNOWN_MODELS:
            raise Reject("The report names a model Silas doesn't offer.")
        if model in seen:
            raise Reject("The report lists a model twice.")
        seen.add(model)
        clean.append(
            {
                "model": model,
                "speed": num(r.get("speed"), 0.01, 10_000, f"{model} speed"),
                "wer": num(r.get("wer"), 0, 1, f"{model} wer"),
                "clips": num(r.get("clips"), 1, 1000, f"{model} clips", integer=True),
                "audio_seconds": num(r.get("audio_seconds"), 1, 100_000, f"{model} audio_seconds"),
                "compute_seconds": num(r.get("compute_seconds"), 0.0001, 100_000, f"{model} compute_seconds"),
            }
        )
    return {"schema": 1, "silas": silas, "device": device, "results": clean}


def pick(results):
    """Same rule as benchmark::pick in Silas."""
    if not results:
        return None
    passing = [r for r in results if r["speed"] >= MIN_SPEED and r["wer"] <= MAX_WER]
    if passing:
        best = min(r["wer"] for r in passing)
        close = [r for r in passing if r["wer"] - best <= TIE_WER]
        return max(close, key=lambda r: r["speed"])["model"]
    accurate = [r for r in results if r["wer"] <= MAX_WER]
    return max(accurate or results, key=lambda r: r["speed"])["model"]


def rebuild(write: bool = True) -> dict:
    """Median speed and WER per device and model across every stored report."""
    by_device: dict = {}
    for f in sorted(REPORTS.glob("*.json")):
        rep = json.loads(f.read_text())
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
            "speed": round(chosen["speed"], 2),
            "wer": round(chosen["wer"], 4),
            "reports": dev["reports"],
        }
    out = {"schema": 1, "devices": devices}
    if write:
        DEVICES.write_text(json.dumps(out, indent=2) + "\n")
    return out


def main() -> int:
    if sys.argv[1:] == ["--rebuild"]:
        rebuild()
        return 0
    number = os.environ.get("ISSUE_NUMBER", "")
    if not number.isdigit():
        print("ISSUE_NUMBER must be the issue number.", file=sys.stderr)
        return 1
    try:
        report = validate(extract(os.environ.get("ISSUE_BODY", "")))
    except Reject as e:
        print(e)
        return 2
    REPORTS.mkdir(exist_ok=True)
    (REPORTS / f"{int(number)}.json").write_text(json.dumps(report, indent=2) + "\n")
    # The report PR adds only this file; devices.json is rebuilt on main after a maintainer
    # merges it, so report PRs never conflict with each other.
    entry = rebuild(write=False)["devices"][report["device"]["key"]]
    print(
        f"Thanks! Once this is merged, {report['device']['cpu']} ({report['device']['os']}) "
        f"will get {entry['model']}, from {entry['reports']} report(s)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
