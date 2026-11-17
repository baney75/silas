"""The ratchet rejects a worse result, accepts an equal one, and only moves when nothing got worse.

  python3 -I bench/test_ratchet.py
"""

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ratchet  # noqa: E402
import score  # noqa: E402

BASE = {
    "wer": {"clean": 0.020, "longform": 0.030},
    "formatted_error": 0.100,
    "t_final_ms_median": 400.0,
    "peak_rss_mb": 1000.0,
    "timing": "real time",
}


def worse(**changes):
    m = copy.deepcopy(BASE)
    for k, v in changes.items():
        if k.startswith("wer_"):
            m["wer"][k[4:]] = v
        else:
            m[k] = v
    return m


class Ratchet(unittest.TestCase):
    def test_an_equal_result_passes(self):
        self.assertEqual(ratchet.check(BASE, copy.deepcopy(BASE)), [])
        self.assertTrue(ratchet.no_worse(BASE, copy.deepcopy(BASE)))

    def test_wer_up_more_than_half_a_point_fails(self):
        self.assertTrue(ratchet.check(BASE, worse(wer_clean=0.026)))
        self.assertEqual(ratchet.check(BASE, worse(wer_clean=0.024)), [])

    def test_a_category_that_disappears_fails(self):
        m = worse()
        del m["wer"]["longform"]
        self.assertTrue(ratchet.check(BASE, m))

    def test_formatted_error_latency_and_memory_have_their_own_limits(self):
        self.assertTrue(ratchet.check(BASE, worse(formatted_error=0.106)))
        self.assertTrue(ratchet.check(BASE, worse(t_final_ms_median=461.0)))
        self.assertEqual(ratchet.check(BASE, worse(t_final_ms_median=459.0)), [])
        self.assertTrue(ratchet.check(BASE, worse(peak_rss_mb=1101.0)))
        self.assertEqual(ratchet.check(BASE, worse(peak_rss_mb=1099.0)), [])

    def test_the_accuracy_gate_ignores_timing_and_memory(self):
        self.assertEqual(ratchet.check(BASE, worse(t_final_ms_median=9000.0, peak_rss_mb=9000.0), gate="accuracy"), [])
        self.assertTrue(ratchet.check(BASE, worse(wer_clean=0.5), gate="accuracy"))

    def test_latency_is_not_compared_across_timing_modes(self):
        self.assertEqual(ratchet.check(BASE, worse(t_final_ms_median=9000.0, timing="fast")), [])

    def test_update_needs_every_metric_equal_or_better(self):
        self.assertFalse(ratchet.no_worse(BASE, worse(wer_clean=0.021)))
        self.assertFalse(ratchet.no_worse(BASE, worse(t_final_ms_median=401.0)))
        self.assertTrue(ratchet.no_worse(BASE, worse(wer_clean=0.010, peak_rss_mb=900.0)))


class EndToEnd(unittest.TestCase):
    """main() on result files: exit 1 on a worse run, 0 on the same run, and --update leaves a worse file alone."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        manifest = self.dir / "manifest.jsonl"
        clips = [
            {"id": "a", "category": "clean", "ref_verbatim": "the cat sat on the mat", "ref_formatted": "The cat sat on the mat."},
            {"id": "b", "category": "clean", "ref_verbatim": "a dog ran", "ref_formatted": "A dog ran."},
        ]
        manifest.write_text("".join(json.dumps(c) + "\n" for c in clips))
        self.good = self.results("good", ["The cat sat on the mat.", "A dog ran."])
        self.bad = self.results("bad", ["The cat sat on a hat.", "A dog ran."])
        self.cfg = self.dir / "ratchet.json"
        self.cfg.write_text(json.dumps({"model": "m", "manifest": str(manifest)}))

    def results(self, name, hyps):
        p = self.dir / f"{name}.jsonl"
        rows = [{"id": i, "engine": "silas", "model": "m", "hypothesis": h, "t_final_ms": 300, "audio_seconds": 2,
                 "peak_rss_mb": 500, "paced": True} for i, h in zip("ab", hyps)]
        p.write_text("".join(json.dumps(r) + "\n" for r in rows))
        return p

    def main(self, *args):
        return ratchet.main([*args, "--ratchet", str(self.cfg)])

    def test_round_trip(self):
        self.assertEqual(self.main("--results", str(self.good), "--update"), 0)
        recorded = json.loads(self.cfg.read_text())["metrics"]
        self.assertEqual(recorded["wer"], {"clean": 0.0})
        self.assertEqual(self.main("--results", str(self.good)), 0)
        self.assertEqual(self.main("--results", str(self.bad)), 1)
        self.assertEqual(self.main("--results", str(self.bad), "--update"), 1)
        self.assertEqual(json.loads(self.cfg.read_text())["metrics"], recorded)


class Scoring(unittest.TestCase):
    def test_normalization_matches_benchmark_rs(self):
        # The cases in core/voice/src/benchmark.rs normalize_follows_the_spec.
        self.assertEqual(score.normalize("  Hello,   World!  "), "hello world")
        self.assertEqual(score.normalize("Well-known — and don't"), "well known and don't")
        self.assertEqual(score.normalize("It cost $5.50 (really)"), "it cost 550 really")
        self.assertEqual(score.normalize("A\tB\nC"), "a b c")
        self.assertEqual(score.normalize("...!?"), "")

    def test_whisper_norm_matches_numbers_and_spellings(self):
        self.assertEqual(score.word_errors("twenty five colours", "25 colors", score.normalize_whisper), (0, 2))
        self.assertEqual(score.word_errors("one two three", "1 2 3", score.normalize_whisper), (0, 3))

    def test_formatted_error_counts_case_and_punctuation(self):
        self.assertEqual(score.format_errors("Paris, France.", "paris france"), (2, 2))

    def test_commands_must_match_exactly(self):
        ref = json.dumps([{"action": "new_line"}])
        self.assertTrue(score.command_match(ref, '[{"action": "new_line"}]'))
        self.assertFalse(score.command_match(ref, "new line"))


if __name__ == "__main__":
    unittest.main()
