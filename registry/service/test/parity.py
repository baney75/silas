"""Ask registry/ingest.py what it makes of every shared vector and of mutations of the base
report. vitest.config.ts runs this and hands the outcomes to the service tests, which require
the Worker to give the same decision, reason and cleaned report for each body.

The service takes raw JSON, not an issue body, so json.loads replaces extract()'s brace slicing;
the reasons for unparseable and non-object bodies are extract()'s."""

import copy
import json
import sys
from pathlib import Path

REGISTRY = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REGISTRY))
import ingest  # noqa: E402
import test_ingest  # noqa: E402

BASE = test_ingest.GOOD


def outcome(name, body):
    try:
        try:
            data = json.loads(body)
        except (ValueError, RecursionError):
            raise ingest.Reject("The report isn't valid JSON. Please submit it from Silas without editing it.")
        if not isinstance(data, dict):
            raise ingest.Reject("The report isn't a JSON object.")
        return {"name": name, "body": body, "ok": True, "report": ingest.validate(data)}
    except ingest.Reject as e:
        return {"name": name, "body": body, "ok": False, "error": str(e), "stale": isinstance(e, ingest.Stale)}


ODD = [True, False, None, 0, -1, 1, 0.5, 10.0, 1e-9, 1e9, 2**53 + 1, "", "x", "10", [], {}, [1], {"a": 1}]
LITERALS = ["NaN", "Infinity", "-Infinity", "1e999", "-0", "10.0", "1" + "0" * 400, "-NaN", "NaN1", "Infinityx", "1NaN"]


def leaves(node, path=()):
    if isinstance(node, dict):
        for k, v in node.items():
            yield from leaves(v, path + (k,))
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield from leaves(v, path + (i,))
    yield path


def setting(path, value):
    rep = copy.deepcopy(BASE)
    node = rep
    for k in path[:-1]:
        node = node[k]
    node[path[-1]] = value
    return rep


def mutations():
    out = []
    paths = [p for p in leaves(BASE) if p]
    for p in paths:
        label = ".".join(map(str, p))
        for v in ODD:
            out.append(outcome(f"{label} = {json.dumps(v)}", json.dumps(setting(p, v), indent=2)))
        # Bare tokens Python's json reads (or refuses) that JSON.parse does not know.
        marker = "__LITERAL__"
        text = json.dumps(setting(p, marker), indent=2)
        for lit in LITERALS:
            out.append(outcome(f"{label} = {lit}", text.replace(f'"{marker}"', lit)))
        rep = copy.deepcopy(BASE)
        node = rep
        for k in p[:-1]:
            node = node[k]
        del node[p[-1]]
        out.append(outcome(f"remove {label}", json.dumps(rep, indent=2)))
    for lit in LITERALS:
        out.append(outcome(f"extra = {lit}", json.dumps(BASE, indent=2).replace("{", '{"extra": ' + lit + ",", 1)))
    out.append(outcome("BOM", "﻿" + json.dumps(BASE)))
    # Deep nesting is left out: Python 3.12's json gives up near depth 1000, 3.14's doesn't.
    out.append(outcome("bare NaN", "NaN"))
    out.append(outcome("empty", ""))
    return out


vectors = {
    kind: [outcome(c["name"], test_ingest.apply(c)) for c in test_ingest.VECTORS[kind]]
    for kind in ("good", "bad", "stale")
}
json.dump({"vectors": vectors, "mutations": mutations()}, sys.stdout)
