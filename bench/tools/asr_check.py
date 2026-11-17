# /// script
# requires-python = ">=3.12"
# dependencies = ["faster-whisper", "whisper-normalizer", "numpy"]
# ///
"""Proxy listen check: transcribe clips with a free local ASR and report WER against
ref_verbatim. A synthesized clip over 15% WER means the TTS or the reference is wrong.

  OMP_NUM_THREADS=3 nice uv run bench/tools/asr_check.py [--dest DIR] [--n 15] [--all] [--category accent]

Both sides go through Whisper's English normalizer (numbers to digits, fillers dropped,
spelling variants unified). The hypothesis first has code symbols read back as words
("-m" becomes "dash m"), since the references spell them out.
"""

import argparse
import json
import random
import re
import wave
from pathlib import Path

import numpy as np

BENCH = Path(__file__).resolve().parents[1]

SYMBOLS = [
    (r"\+=", " plus equals "), (r">=", " greater than or equal to "), (r"=>", " arrow "), (r"==", " equals equals "),
    (r"--", " dash dash "), (r"(?<![\w])-(?=\w)", " dash "), (r"(?<=\w)-(?=\w)", " dash "),
    (r"@", " at "), (r"~", " tilde "), (r"/", " slash "), (r"(?<=\w)\.(?=[a-zA-Z])", " dot "),
    (r"_", " underscore "), (r"\+", " plus "), (r"=", " equals "), (r"\*", " times "),
    (r"\(", " open paren "), (r"\)", " close paren "), (r"\[", " open bracket "), (r"\]", " close bracket "),
    (r"\{", " open brace "), (r"\}", " close brace "), (r";", " semicolon "),
]


def spoken_symbols(text: str) -> str:
    for pat, rep in SYMBOLS:
        text = re.sub(pat, rep, text)
    return text


def load(path: Path) -> np.ndarray:
    """16 kHz mono 16-bit WAV as float32; skips PyAV, whose newer releases break faster-whisper."""
    with wave.open(str(path)) as w:
        assert (w.getframerate(), w.getnchannels(), w.getsampwidth()) == (16000, 1, 2)
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32) / 32768.0


def wer(ref: list[str], hyp: list[str]) -> float:
    d = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        prev, d[0] = d[0], i
        for j, h in enumerate(hyp, 1):
            prev, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, prev + (r != h))
    return d[len(hyp)] / max(1, len(ref))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dest", type=Path, default=Path.home() / ".cache" / "silas-bench" / "bench-set-v1")
    ap.add_argument("--n", type=int, default=15)
    ap.add_argument("--seed", type=int, default=20261117)
    ap.add_argument("--all", action="store_true", help="every selected clip, not a random sample")
    ap.add_argument("--category", help="limit to one category (default: synthesized clips)")
    ap.add_argument("--model", default="small.en")
    args = ap.parse_args()

    from faster_whisper import WhisperModel
    from whisper_normalizer.english import EnglishTextNormalizer

    norm = EnglishTextNormalizer()
    rows = [json.loads(line) for line in (BENCH / "manifest.jsonl").read_text().splitlines()]
    pool = [r for r in rows if (r["category"] == args.category if args.category else r["tts"])]
    picked = pool if args.all else random.Random(args.seed).sample(pool, min(args.n, len(pool)))
    model = WhisperModel(args.model, device="cpu", compute_type="int8", cpu_threads=3)
    total_err = total_words = over = 0
    for r in sorted(picked, key=lambda r: r["id"]):
        segs, _ = model.transcribe(load(args.dest / r["audio"]), language="en", beam_size=5, vad_filter=False)
        hyp_raw = " ".join(s.text.strip() for s in segs)
        ref = norm(r["ref_verbatim"]).split()
        hyp = norm(spoken_symbols(hyp_raw)).split()
        w = wer(ref, hyp)
        total_err += w * len(ref)
        total_words += len(ref)
        flag = "  OVER 15%" if w > 0.15 else ""
        over += w > 0.15
        print(f"{r['id']:<15} WER {w * 100:5.1f}%{flag}\n   ref: {r['ref_verbatim']}\n   hyp: {hyp_raw}")
    print(f"\n{len(picked)} clips, pooled WER {total_err / max(1, total_words) * 100:.1f}%, {over} over 15%")


if __name__ == "__main__":
    main()
