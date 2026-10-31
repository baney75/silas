import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ingest  # noqa: E402

GOOD = {
    "schema": 1,
    "silas": "0.1.7",
    "device": {
        "key": "macos/apple-m5/10",
        "os": "macos",
        "os_version": "macOS 27.2",
        "cpu": "Apple M5",
        "cores": 10,
        "memory_gb": 24,
        "hw_model": "Mac17,2",
    },
    "results": [
        {"model": "parakeet-v3", "speed": 23.0, "wer": 0.021, "clips": 9, "audio_seconds": 62.2, "compute_seconds": 2.7043}
    ],
    "pick": "parakeet-v3",
    "ran_at": "2026-10-31T20:00:00Z",
}


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

    def test_rejects_key_mismatch(self):
        self.rejects(lambda b: b["device"].update(key="macos/apple-m4/10"))

    def test_rejects_unknown_model(self):
        self.rejects(lambda b: b["results"][0].update(model="evil"))

    def test_rejects_out_of_range(self):
        self.rejects(lambda b: b["results"][0].update(wer=1.5))
        self.rejects(lambda b: b["results"][0].update(speed="fast"))
        self.rejects(lambda b: b["results"][0].update(speed=True))

    def test_rejects_markup_in_text(self):
        self.rejects(lambda b: b["device"].update(cpu="Apple M5 <script>", key="macos/apple-m5-script/10"))
        self.rejects(lambda b: b["device"].update(cpu="[click](https://x)", key="macos/click-https-x/10"))
        self.rejects(lambda b: b["device"].update(cpu="x\ny", key="macos/x-y/10"))
        self.rejects(lambda b: b["device"].update(os_version="a" * 500))

    def test_rejects_trailing_newline(self):
        self.rejects(lambda b: b.update(silas="0.1.7\n"))
        self.rejects(lambda b: b["device"].update(os_version="macOS 27.2\n"))

    def test_rejects_non_finite(self):
        for bad in ("Infinity", "NaN", "1e999"):
            with self.assertRaises(ingest.Reject):
                ingest.validate(ingest.extract(body(GOOD).replace('"cores": 10', f'"cores": {bad}')))

    def test_rejects_huge_integer_and_non_string_model(self):
        with self.assertRaises(ingest.Reject):
            ingest.validate(ingest.extract(body(GOOD).replace('"cores": 10', '"cores": 1' + "0" * 400)))
        self.rejects(lambda b: b["results"][0].update(model=[]))
        self.rejects(lambda b: b["results"][0].update(model={}))

    def test_rejects_deep_nesting(self):
        with self.assertRaises(ingest.Reject):
            ingest.extract("```json\n" + "[" * 100000 + "]" * 100000 + "\n```")

    def test_rejects_numbers_not_from_the_suite(self):
        self.rejects(lambda b: b["results"][0].update(clips=3))
        self.rejects(lambda b: b["results"][0].update(audio_seconds=600.0, compute_seconds=26.0))
        self.rejects(lambda b: b["results"][0].update(speed=90.0))  # disagrees with 62.2 / 2.7043
        self.rejects(lambda b: b["results"][0].update(speed=900.0, compute_seconds=0.0691))

    def test_rejects_duplicate_model(self):
        self.rejects(lambda b: b["results"].append(dict(b["results"][0])))

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
        (reg / "ingest.py").write_text(Path(ingest.__file__).read_text())
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
                rep["results"][0]["compute_seconds"] = 62.2 / speed
                p = self.run_ingest(reg, number=str(n), body_text=body(rep))
                self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
                self.assertNotIn("Apple M5", p.stdout)  # comments carry the slug key, not free text
            dev = json.loads((reg / "devices.json").read_text())["devices"]["macos/apple-m5/10"]
            self.assertEqual(dev, {"model": "parakeet-v3", "speed": 25.0, "wer": 0.021, "reports": 3})

    def test_precision_is_kept(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            rep = copy.deepcopy(GOOD)
            rep["results"][0].update(speed=9.999, compute_seconds=62.2 / 9.999)
            self.assertEqual(self.run_ingest(reg, body_text=body(rep)).returncode, 0)
            dev = json.loads((reg / "devices.json").read_text())["devices"]["macos/apple-m5/10"]
            self.assertLess(dev["speed"], 10.0)  # never rounded up past the bar

    def test_rejected_report_exits_2_and_stores_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            reg = self.registry(tmp)
            p = self.run_ingest(reg, number="9", body_text="hello")
            self.assertEqual(p.returncode, 2)
            self.assertFalse((reg / "reports").exists())

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
