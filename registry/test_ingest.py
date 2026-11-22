import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import ingest  # noqa: E402

# Shared with registry/service: every validator runs the same good and bad reports.
VECTORS = json.loads((HERE / "vectors.json").read_text())
GOOD = VECTORS["base"]
SECONDS = GOOD["results"][0]["audio_seconds"]
SHARED = ("suite.json", "models.json")


def apply(case) -> str:
    """The request body a vector describes (the same rules as registry/service/test/vectors.ts)."""
    if "body" in case:
        return case["body"]
    rep = copy.deepcopy(GOOD)
    for path, value in case.get("set", {}).items():
        *parents, last = path.split(".")
        node = rep
        for k in parents:
            node = node[int(k)] if isinstance(node, list) else node[k]
        if isinstance(node, list):
            i = int(last)
            node[i:i + 1] = [value]
        else:
            node[last] = value
    for path in case.get("remove", []):
        *parents, last = path.split(".")
        node = rep
        for k in parents:
            node = node[int(k)] if isinstance(node, list) else node[k]
        del node[int(last) if isinstance(node, list) else last]
    text = json.dumps(rep, indent=2)
    if "replace" in case:
        old, new = case["replace"]
        assert old in text, case["name"]
        text = text.replace(old, new, 1)
    return text


def body(report) -> str:
    return "### Report\n\n```json\n" + json.dumps(report, indent=2) + "\n```\n"


class Pick(unittest.TestCase):
    def r(self, model, speed, wer):
        return {"model": model, "speed": speed, "wer": wer}

    def test_most_accurate_passing(self):
        self.assertEqual(ingest.pick([self.r("a", 30, 0.05), self.r("b", 12, 0.02)]), "b")

    def test_close_wer_prefers_speed(self):
        self.assertEqual(ingest.pick([self.r("a", 30, 0.024), self.r("b", 12, 0.02)]), "a")

    def test_none_pass_fastest_accurate(self):
        self.assertEqual(ingest.pick([self.r("a", 4, 0.05), self.r("b", 2, 0.03), self.r("c", 50, 0.3)]), "a")

    def test_exact_speed_tie_goes_to_lower_id(self):
        self.assertEqual(ingest.pick([self.r("b", 20, 0.02), self.r("a", 20, 0.02)]), "a")

    def test_none_accurate_fastest(self):
        self.assertEqual(ingest.pick([self.r("a", 4, 0.2), self.r("b", 8, 0.3)]), "b")


class Vectors(unittest.TestCase):
    def test_good_pass(self):
        for case in VECTORS["good"]:
            with self.subTest(case["name"]):
                out = ingest.validate(ingest.extract(apply(case)))
                self.assertNotIn("pick", out)

    def test_bad_rejected_not_stale(self):
        for case in VECTORS["bad"]:
            with self.subTest(case["name"]):
                with self.assertRaises(ingest.Reject) as cm:
                    ingest.validate(ingest.extract(apply(case)))
                self.assertNotIsInstance(cm.exception, ingest.Stale)

    def test_stale_rejected_as_stale(self):
        for case in VECTORS["stale"]:
            with self.subTest(case["name"]):
                with self.assertRaises(ingest.Stale):
                    ingest.validate(ingest.extract(apply(case)))

    def test_vectors_measure_the_current_suite(self):
        self.assertEqual(GOOD["suite"], ingest.SUITE_ID)
        self.assertEqual(GOOD["results"][0]["clips"], ingest.SUITE_CLIPS)
        self.assertEqual(SECONDS, ingest.SUITE_SECONDS)


class Suite(unittest.TestCase):
    def test_suite_json_matches_the_bundled_set(self):
        """suite.json is what Silas builds in (benchmark.rs reads it with include_str!); the bundled
        manifest must hold exactly that many clips and seconds."""
        manifest = json.loads((HERE.parent / "core/voice/bench/manifest.json").read_text())
        clips = manifest["clips"]
        self.assertEqual(len(clips), ingest.SUITE_CLIPS)
        self.assertAlmostEqual(sum(c["seconds"] for c in clips), ingest.SUITE_SECONDS, delta=0.05)
        self.assertGreaterEqual(sum(1 for c in clips if 5 <= c["seconds"] <= 10), 5)

    def test_models_json_lists_known_ids(self):
        self.assertIn("parakeet-v3", ingest.KNOWN_MODELS)
        self.assertTrue(all(isinstance(m, str) and m for m in ingest.KNOWN_MODELS))


class Validate(unittest.TestCase):
    def test_real_cpu_names_pass(self):
        for cpu in ("Intel(R) Core(TM) i7-1165G7 @ 2.80GHz", "AMD Ryzen 7 5800X 8-Core Processor", "Apple M5"):
            self.assertRegex(cpu, ingest.TEXT_RE)

    def test_good(self):
        out = ingest.validate(ingest.extract(body(GOOD)))
        self.assertEqual(out["device"]["key"], "macos/apple-m5/10")
        self.assertNotIn("pick", out)  # recomputed, never trusted

    def rejects(self, mutate):
        bad = copy.deepcopy(GOOD)
        mutate(bad)
        with self.assertRaises(ingest.Reject):
            ingest.validate(ingest.extract(body(bad)))

    def test_rejects_markup_in_long_text(self):
        self.rejects(lambda b: b["device"].update(os_version="a" * 500))

    def test_issue_body_in_a_code_fence(self):
        self.assertEqual(ingest.validate(ingest.extract(body(GOOD)))["device"]["key"], "macos/apple-m5/10")

    def test_rejects_deep_nesting(self):
        with self.assertRaises(ingest.Reject):
            ingest.extract("```json\n" + "[" * 100000 + "]" * 100000 + "\n```")

    def test_rejects_not_json(self):
        with self.assertRaises(ingest.Reject):
            ingest.extract("```json\n{not json\n```")


class EndToEnd(unittest.TestCase):
    def run_ingest(self, reg, *args, number="1", body_text=""):
        return subprocess.run(
            [sys.executable, "-I", str(reg / "ingest.py"), *args],
            env={**os.environ, "ISSUE_NUMBER": number, "ISSUE_BODY": body_text},
            capture_output=True,
            text=True,
        )

    def registry(self, tmp):
        reg = Path(tmp)
        for name in ("ingest.py", *SHARED):
            (reg / name).write_text((HERE / name).read_text())
        return reg

    def test_check_stores_nothing_and_approve_stores_and_rebuilds_median(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            p = self.run_ingest(reg, "--check", body_text=body(GOOD))
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertFalse((reg / "reports").exists())
            for n, speed in ((1, 20.0), (2, 30.0), (3, 25.0)):
                rep = copy.deepcopy(GOOD)
                rep["results"][0]["speed"] = speed
                rep["results"][0]["compute_seconds"] = SECONDS / speed
                rep["results"][0]["wait_seconds"] = n / 10
                p = self.run_ingest(reg, number=str(n), body_text=body(rep))
                self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
                self.assertNotIn("Apple M5", p.stdout)  # comments carry the slug key, not free text
            dev = json.loads((reg / "devices.json").read_text())["devices"]["macos/apple-m5/10"]
            self.assertEqual(dev, {"model": "parakeet-v3", "speed": 25.0, "wer": 0.061, "wait": 0.2, "reports": 3})

    def test_precision_is_kept(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            rep = copy.deepcopy(GOOD)
            rep["results"][0].update(speed=9.999, compute_seconds=SECONDS / 9.999)
            self.assertEqual(self.run_ingest(reg, body_text=body(rep)).returncode, 0)
            dev = json.loads((reg / "devices.json").read_text())["devices"]["macos/apple-m5/10"]
            self.assertLess(dev["speed"], 10.0)  # never rounded up past the bar

    def test_rejected_report_exits_2_and_stores_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            p = self.run_ingest(reg, number="9", body_text="hello")
            self.assertEqual(p.returncode, 2)
            self.assertFalse((reg / "reports").exists())

    def test_hand_edited_devices_json_fails_verify(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            self.assertEqual(self.run_ingest(reg, body_text=body(GOOD)).returncode, 0)
            self.assertEqual(self.run_ingest(reg, "--verify").returncode, 0)
            d = json.loads((reg / "devices.json").read_text())
            d["devices"]["macos/apple-m5/10"]["model"] = "cohere-int4"
            (reg / "devices.json").write_text(json.dumps(d))
            self.assertNotEqual(self.run_ingest(reg, "--verify").returncode, 0)

    def test_old_suite_and_retired_model_are_skipped_not_fatal(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            self.assertEqual(self.run_ingest(reg, number="1", body_text=body(GOOD)).returncode, 0)
            old = copy.deepcopy(GOOD)
            old["suite"] = "older-set"
            (reg / "reports" / "2.json").write_text(json.dumps(old))
            retired = copy.deepcopy(GOOD)
            retired["results"][0]["model"] = "retired-model"
            (reg / "reports" / "3.json").write_text(json.dumps(retired))
            p = self.run_ingest(reg, "--rebuild")
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertIn("skipping 2.json", p.stderr)
            self.assertEqual(json.loads((reg / "devices.json").read_text())["devices"]["macos/apple-m5/10"]["reports"], 1)
            # A new report from an old set is rejected with a reason.
            self.assertEqual(self.run_ingest(reg, "--check", body_text=body(old)).returncode, 2)

    def test_hand_made_report_fails_verify_and_rebuild(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            (reg / "reports").mkdir()
            bad = copy.deepcopy(GOOD)
            bad["results"][0].update(speed=10000, clips=1)
            (reg / "reports" / "x.json").write_text(json.dumps(bad))
            self.assertNotEqual(self.run_ingest(reg, "--verify").returncode, 0)
            self.assertNotEqual(self.run_ingest(reg, "--rebuild").returncode, 0)
            self.assertFalse((reg / "devices.json").exists())


if __name__ == "__main__":
    unittest.main()
