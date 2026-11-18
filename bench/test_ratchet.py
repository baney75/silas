"""The ratchet rejects a worse result, accepts an equal one, and only moves when nothing got worse.

  python3 -I bench/test_ratchet.py
"""

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "bench"))
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

    def test_memory_missing_from_a_run_fails_and_never_updates(self):
        self.assertTrue(ratchet.check(BASE, worse(peak_rss_mb=None)))
        self.assertFalse(ratchet.no_worse(BASE, worse(peak_rss_mb=None)))

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

    def results(self, name, hyps, ids="ab", t_final_ms=300):
        p = self.dir / f"{name}.jsonl"
        rows = [{"id": i, "engine": "silas", "model": "m", "hypothesis": h, "t_final_ms": t_final_ms, "audio_seconds": 2,
                 "peak_rss_mb": 500, "paced": True} for i, h in zip(ids, hyps)]
        p.write_text("".join(json.dumps(r) + "\n" for r in rows))
        return p

    def main(self, *args):
        return ratchet.main([*args, "--ratchet", str(self.cfg)])

    def test_round_trip(self):
        self.assertEqual(self.main("--results", str(self.good), "--update"), 0)
        recorded = json.loads(self.cfg.read_text())["baselines"]["local"]["unknown"]
        self.assertEqual(recorded["wer"], {"clean": 0.0})
        self.assertEqual(self.main("--results", str(self.good)), 0)
        self.assertEqual(self.main("--results", str(self.bad)), 1)
        self.assertEqual(self.main("--results", str(self.bad), "--update"), 1)
        self.assertEqual(json.loads(self.cfg.read_text())["baselines"]["local"]["unknown"], recorded)

    def test_each_cpu_has_its_own_baseline(self):
        self.assertEqual(self.main("--results", str(self.good), "--update"), 0)
        self.assertEqual(self.main("--results", str(self.good), "--baseline", "ci"), 1)  # none recorded yet
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "ci", "--cpu", "Intel", "--update"), 0)
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "ci", "--cpu", "Intel"), 0)
        self.assertEqual(self.main("--results", str(self.bad)), 1)
        # A CPU not recorded yet passes against any recorded one, and --update then records it.
        self.assertEqual(self.main("--results", str(self.good), "--baseline", "ci", "--cpu", "AMD", "--update"), 0)
        ci = json.loads(self.cfg.read_text())["baselines"]["ci"]
        self.assertEqual(set(ci), {"Intel", "AMD"})
        # On its own CPU a run is held to that CPU's numbers.
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "ci", "--cpu", "AMD"), 1)
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "ci", "--cpu", "Other"), 0)
        # A CPU whose text fails every recorded one is seeded explicitly, once.
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "local", "--cpu", "Slow", "--update"), 1)
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "local", "--cpu", "Slow", "--new-cpu"), 0)
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "local", "--cpu", "Slow", "--new-cpu"), 1)
        self.assertEqual(self.main("--results", str(self.bad), "--baseline", "local", "--cpu", "Slow"), 0)

    def test_results_must_hold_every_clip_once(self):
        self.assertEqual(self.main("--results", str(self.good), "--update"), 0)
        recorded = self.cfg.read_text()
        # Dropping the failing clip leaves its category measured on the rest; that must not pass or update.
        dropped = self.results("dropped", ["A dog ran."], ids="b")
        self.assertEqual(self.main("--results", str(dropped)), 1)
        self.assertEqual(self.main("--results", str(dropped), "--update"), 1)
        twice = self.results("twice", ["The cat sat on the mat.", "A dog ran.", "A dog ran."], ids="abb")
        self.assertEqual(self.main("--results", str(twice)), 1)
        extra = self.results("extra", ["The cat sat on the mat.", "A dog ran.", "Hi."], ids="abz")
        self.assertEqual(self.main("--results", str(extra)), 1)
        self.assertEqual(self.cfg.read_text(), recorded)

    def test_an_unrecorded_cpu_records_only_when_no_worse(self):
        self.assertEqual(self.main("--results", str(self.good), "--baseline", "ci", "--cpu", "Intel", "--update"), 0)
        slower = self.results("slower", ["The cat sat on the mat.", "A dog ran."], t_final_ms=330)  # within +15%
        self.assertEqual(self.main("--results", str(slower), "--baseline", "ci", "--cpu", "AMD", "--update"), 0)
        self.assertEqual(set(json.loads(self.cfg.read_text())["baselines"]["ci"]), {"Intel"})

    def test_a_reviewed_loss_records_only_with_a_reason_and_resets_the_group(self):
        self.assertEqual(self.main("--results", str(self.good), "--cpu", "Intel", "--update"), 0)
        self.assertEqual(self.main("--results", str(self.good), "--cpu", "AMD", "--new-cpu"), 0)
        before = self.cfg.read_text()
        with self.assertRaises(SystemExit):
            self.main("--results", str(self.bad), "--cpu", "Intel", "--allow-worse", "new engine")  # no --update
        with self.assertRaises(SystemExit):
            self.main("--results", str(self.bad), "--cpu", "Intel", "--update", "--allow-worse", " ")
        self.assertEqual(self.cfg.read_text(), before)
        self.assertEqual(self.main("--results", str(self.bad), "--cpu", "Intel", "--update", "--allow-worse", "new engine"), 0)
        local = json.loads(self.cfg.read_text())["baselines"]["local"]
        self.assertEqual(set(local), {"Intel"})
        self.assertEqual(local["Intel"]["accepted"], "new engine")
        self.assertEqual(self.main("--results", str(self.bad), "--cpu", "Intel"), 0)
        # The dropped CPU now compares with the new numbers and reseeds itself.
        self.assertEqual(self.main("--results", str(self.bad), "--cpu", "AMD", "--update"), 0)
        self.assertEqual(set(json.loads(self.cfg.read_text())["baselines"]["local"]), {"Intel", "AMD"})


class Scoring(unittest.TestCase):
    def test_normalization_matches_benchmark_rs(self):
        # The cases in core/voice/src/benchmark.rs normalize_follows_the_spec.
        self.assertEqual(score.normalize("  Hello,   World!  "), "hello world")
        self.assertEqual(score.normalize("Well-known \u2014 and don't"), "well known and don't")
        self.assertEqual(score.normalize("It cost $5.50 (really)"), "it cost 550 really")
        self.assertEqual(score.normalize("A\tB\nC"), "a b c")
        self.assertEqual(score.normalize("...!?"), "")

    def test_whisper_norm_matches_numbers_and_spellings(self):
        self.assertEqual(score.word_errors("twenty five colours", "25 colors", score.normalize_whisper), (0, 2))
        self.assertEqual(score.word_errors("one two three", "1 2 3", score.normalize_whisper), (0, 3))

    def test_formatted_error_counts_case_and_punctuation(self):
        self.assertEqual(score.format_errors("Paris, France.", "paris france"), (2, 2))
        self.assertEqual(score.format_errors("Dear Ann,\nThanks.", "Dear Ann, Thanks."), (1, 4))

    def test_accents_fold_and_wer_scores_the_raw_model_text(self):
        self.assertEqual(score.normalize("Caf\u00e9 Na\u00efve"), "cafe naive")
        row = {"hypothesis": "So the plan.", "hypothesis_raw": "Um, so the plan."}
        self.assertEqual(score.word_errors("um so the plan", score.verbatim(row)), (0, 4))

    def test_numbers_rule_applies_to_every_side(self):
        # bench/README.md: 10 and above in digits, zero to nine in words. Books spell the numbers out.
        for ref, hyp in [("a girl of sixteen", "a girl of 16"), ("this beauty of forty", "this beauty of 40"),
                         ("twenty five yet", "25 yet"), ("a hundred serpents", "100 serpents"),
                         ("april sixteenth eighteen sixty one", "April 16th, 1861"), ("three books", "3 books")]:
            self.assertEqual(score.word_errors(ref, hyp), (0, len(score.normalize_silas(ref).split())), hyp)
            self.assertEqual(score.word_errors(hyp, ref)[0], 0, ref)
        self.assertEqual(score.normalize_silas("no one came, one by one"), "no one came one by one")
        self.assertEqual(score.word_errors("a girl of sixteen", "a girl of 60"), (1, 4))
        self.assertEqual(score.format_errors("a girl of sixteen, to whom", "a girl of 16, to whom"), (0, 6))
        self.assertEqual(score.format_errors("appearance of twenty-five, yet", "appearance of 25, yet"), (0, 4))
        self.assertEqual(score.format_errors("$47.50 at 3:30, 12%", "$47.50 at 3:30, 12%"), (0, 4))
        self.assertEqual(score.format_errors("A man and a dog.", "a man and a dog."), (1, 5))

    def test_published_longform_clips_typed_with_digits_score_clean(self):
        manifest = score.load_manifest(ROOT / "bench" / "manifest.jsonl")
        typed = {"sixteen": "16", "forty": "40", "twenty-five": "25", "a hundred": "100"}
        for cid in ("longform-0004", "longform-0005"):
            ref = manifest[cid]["ref_formatted"]
            hyp = ref
            for words, digits in typed.items():
                hyp = hyp.replace(f" {words}", f" {digits}")
            self.assertNotEqual(hyp, ref)
            self.assertEqual(score.format_errors(ref, hyp)[0], 0, cid)
            self.assertEqual(score.word_errors(manifest[cid]["ref_verbatim"], hyp.replace("-", " "))[0],
                             score.word_errors(manifest[cid]["ref_verbatim"], ref.replace("-", " "))[0], cid)

    def test_quote_marks_are_not_words(self):
        # whisper.cpp large-v3-turbo types LibriSpeech dialogue in quotes.
        hyp = "\"'Come, therefore, and let us fling mud at them.'\n"
        self.assertEqual(score.word_errors("come therefore and let us fling mud at them", hyp), (0, 9))
        self.assertEqual(score.normalize_silas("don't 'twas the students'"), "don't twas the students")

    def test_written_symbols_read_as_spoken(self):
        # Parakeet's own text on dictation clips, against what ref_verbatim says was spoken.
        cases = [
            ("the flight was three hundred eighty nine dollars the hotel was two forty dollars a night",
             "The flight was$389, the hotel was$240 a night"),
            ("version three point two fixed most of the memory leaks", "version 3.2 fixed most of the memory leaks"),
            ("around nine a m", "around 9 a.m."),
            ("the new office is at one twenty five west elm street", "The new office is at 125 West Elm Street"),
            ("silas dot app slash download", "silas.app/download"),
            ("twelve percent of jane dot porter at example dot com", "12% of jane.porter@example.com"),
        ]
        for ref, hyp in cases:
            self.assertEqual(score.word_errors(ref, hyp)[0], 0, (score.normalize_silas(ref), score.normalize_silas(hyp)))
        self.assertEqual(score.normalize_silas("one two three, Mr. Smith"), "one two three mr smith")

    def test_commands_must_match_exactly(self):
        ref = json.dumps([{"action": "new_line"}])
        self.assertTrue(score.command_match(ref, '[{"action": "new_line"}]'))
        self.assertFalse(score.command_match(ref, "new line"))


if __name__ == "__main__":
    unittest.main()
