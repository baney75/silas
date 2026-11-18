"""The rival check fails when Silas falls behind a rival, holds when it doesn't, and scores punctuation by its marks.

  python3 -I bench/test_rivals.py
"""

import contextlib
import copy
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "bench"))
import rivals  # noqa: E402

RESULTS = json.loads((ROOT / "bench/rivals/opencode.json").read_text())
ROWS = ROOT / "bench/rivals/opencode-chapel1"
CPU = RESULTS["machine"]["cpu"]


def quiet(f, *args, **kwargs):
    with contextlib.redirect_stdout(io.StringIO()):
        return f(*args, **kwargs)


def recorded():
    return {k: m["silas"] for k, m in RESULTS["metrics"].items()}


class Punctuation(unittest.TestCase):
    def test_matching_marks_are_hits(self):
        self.assertEqual(rivals.punctuation("Come, therefore, and fling mud!", "Come, therefore, and fling mud."), (3, 3, 3))

    def test_a_missing_mark_is_a_miss_and_a_wrong_one_a_false_mark(self):
        # "darling?" typed as "darling." is the wrong class; the final stop is missing.
        self.assertEqual(rivals.punctuation("Is it, my darling? Bless you.", "Is it, my darling. Bless you"), (1, 2, 3))

    def test_a_mark_survives_a_misheard_word(self):
        self.assertEqual(rivals.punctuation("Send it to Tom.", "Send it to Thom."), (1, 1, 1))

    def test_numbers_and_symbols_align_as_spoken(self):
        hits, marked, wanted = rivals.punctuation("It costs $389, not more.", "It costs 389 dollars, not more.")
        self.assertEqual((hits, marked, wanted), (2, 2, 2))


class Check(unittest.TestCase):
    def test_the_recorded_numbers_pass(self):
        self.assertEqual(quiet(rivals.check, RESULTS, recorded(), set(rivals.GATES), CPU), [])

    def test_a_shrinking_lead_fails(self):
        now = recorded()
        now["punctuation_f1"] -= RESULTS["metrics"]["punctuation_f1"]["slack"] + 0.01
        fails = quiet(rivals.check, RESULTS, now, {"accuracy"}, CPU)
        self.assertEqual(len(fails), 1)
        self.assertIn("punctuation_f1", fails[0])

    def test_a_growing_loss_fails(self):
        now = recorded()
        now["wer"] += RESULTS["metrics"]["wer"]["slack"] + 0.001
        self.assertEqual(len(quiet(rivals.check, RESULTS, now, {"accuracy"}, CPU)), 1)

    def test_a_loss_within_its_slack_passes(self):
        now = recorded()
        now["wer"] += RESULTS["metrics"]["wer"]["slack"] / 2
        self.assertEqual(quiet(rivals.check, RESULTS, now, {"accuracy"}, CPU), [])

    def test_slower_typing_fails_on_the_recorded_machine_only(self):
        now = recorded()
        now["typing_ms"] += RESULTS["metrics"]["typing_ms"]["slack"] + 1
        self.assertEqual(len(quiet(rivals.check, RESULTS, now, {"typing"}, CPU)), 1)
        self.assertEqual(quiet(rivals.check, RESULTS, now, {"typing"}, "another CPU"), [])

    def test_latency_is_compared_on_the_recorded_cpu_only(self):
        now = recorded()
        now["peak_rss_mb"] += 10_000
        self.assertEqual(len(quiet(rivals.check, RESULTS, now, {"latency"}, CPU)), 1)
        self.assertEqual(quiet(rivals.check, RESULTS, now, {"latency"}, "another CPU"), [])

    def test_a_gated_metric_the_rows_dont_measure_fails(self):
        now = recorded()
        del now["wer"]
        fails = quiet(rivals.check, RESULTS, now, {"accuracy"}, CPU)
        self.assertTrue(any("wer" in f and "not measured" in f for f in fails))
        self.assertTrue(quiet(rivals.check, RESULTS, {"setup_steps": 1}, {"accuracy"}, CPU))
        now = recorded()
        del now["typing_ms"]
        self.assertTrue(quiet(rivals.check, RESULTS, now, {"typing"}, CPU))

    def test_record_holds_silas_to_the_rivals_new_numbers(self):
        m = RESULTS["metrics"]["wer"]
        better_rival = {"wer": m["rival"] - m["slack"] - 0.01}  # the rival improved; Silas did not
        self.assertEqual(quiet(rivals.check, RESULTS, recorded(), {"accuracy"}, CPU), [])
        fails = quiet(rivals.check, RESULTS, recorded(), {"accuracy"}, CPU, new_rival=better_rival)
        self.assertTrue(any("wer" in f for f in fails))

    def test_cli_polled_rival_times_are_unmeasured(self):
        load = lambda f: [json.loads(l) for l in (ROWS / f).read_text().splitlines() if l.strip()]  # noqa: E731
        self.assertTrue(rivals.polled(load("hex-service.jsonl")))
        self.assertFalse(rivals.polled(load("silas-real-time.jsonl")))
        for k in rivals.POLLED:
            self.assertIsNone(RESULTS["metrics"][k]["rival"], k)
            self.assertEqual(RESULTS["metrics"][k]["winner"], "not scored", k)

    def test_the_workflow_runs_when_the_readme_changes(self):
        self.assertIn("- README.md", (ROOT / ".github/workflows/bench-ratchet.yml").read_text())

    def test_a_metric_outside_the_gates_is_not_checked(self):
        now = recorded()
        now["wer"] = 1.0
        self.assertEqual(quiet(rivals.check, RESULTS, now, {"setup"}, CPU), [])


class Setup(unittest.TestCase):
    def readme(self, text):
        f = tempfile.NamedTemporaryFile("w", suffix=".md", delete=False)
        f.write(text)
        f.close()
        return Path(f.name)

    def test_the_real_readme_matches_the_recorded_steps(self):
        steps, problems = rivals.setup_steps(RESULTS["setup"], ROOT / "README.md")
        self.assertEqual((steps, problems), (RESULTS["metrics"]["setup_steps"]["silas"], []))

    def test_a_new_first_run_step_fails(self):
        text = (ROOT / "README.md").read_text().replace("3. Try it", "3. Pick a microphone.\n4. Try it", 1)
        _, problems = rivals.setup_steps(RESULTS["setup"], self.readme(text))
        self.assertTrue(any("First run has 4 items" in p for p in problems))

    def test_a_step_the_readme_no_longer_documents_fails(self):
        entry = copy.deepcopy(RESULTS["setup"])
        quote = entry["silas_steps"][0]["quote"]
        text = (ROOT / "README.md").read_text().replace(quote, "Download Silas, then install it.")
        _, problems = rivals.setup_steps(entry, self.readme(text))
        self.assertTrue(any("no longer says" in p for p in problems))


class EndToEnd(unittest.TestCase):
    def run_check(self, *args):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            code = rivals.main(["opencode", *args])
        return code, out.getvalue()

    def test_the_committed_rows_reproduce_the_results_file(self):
        code, out = self.run_check("--silas", str(ROWS / "silas-fast.jsonl"), "--silas", str(ROWS / "silas-real-time.jsonl"),
                                   "--typing", str(ROWS / "silas-typing.jsonl"), "--cpu", CPU)
        self.assertEqual(code, 0, out)
        for key, m in RESULTS["metrics"].items():
            if m["rival"] is not None:
                self.assertIn(f"ok   {key}:", out)

    def test_worse_silas_text_fails_the_accuracy_gate(self):
        rows = [json.loads(l) for l in (ROWS / "silas-fast.jsonl").read_text().splitlines() if l.strip()]
        for r in rows:
            words = r["hypothesis_raw"].split()
            r["hypothesis_raw"] = r["hypothesis"] = " ".join(words[: len(words) * 3 // 4])
        f = tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False)
        f.write("".join(json.dumps(r) + "\n" for r in rows))
        f.close()
        code, out = self.run_check("--silas", f.name, "--gate", "accuracy", "--cpu", "another CPU")
        self.assertEqual(code, 1, out)
        self.assertIn("FAIL wer", out)


if __name__ == "__main__":
    unittest.main()
