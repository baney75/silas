# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["vosk==0.3.45"]
# ///
"""Vosk on the Silas speech test set, fed the way nerd-dictation feeds it: audio in small blocks as it is
spoken, text taken at the end.

  uv run bench/engines/vosk_engine.py --manifest bench/manifest.jsonl --set ~/.cache/silas-bench/bench-set-v1 --model small-en-us-0.15 --out bench/out/vosk.jsonl

--model names vosk-model-<model>.zip under ~/.cache/silas-bench/models/vosk (fetched from alphacephei.com if
missing). Vosk streams, so the clip is replayed on a simulated clock like `silas-bench run --fast`: block i
arrives at its time in the clip and waits for the recognizer to finish the block before it. t_first_ms is
when the first words showed up from key down; t_final_ms is key up to the final text.
"""

import json
import sys
import time
import urllib.request
import wave
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _common as c

BLOCK_SECONDS = 0.1


def unpack(model):
    root = c.CACHE / "models/vosk"
    folder = root / f"vosk-model-{model}"
    if folder.is_dir():
        return folder
    archive = root / f"vosk-model-{model}.zip"
    if not archive.exists():
        root.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(f"https://alphacephei.com/vosk/models/vosk-model-{model}.zip", archive)
    with zipfile.ZipFile(archive) as z:
        for n in z.namelist():
            if n.startswith("/") or ".." in Path(n).parts:
                raise SystemExit(f"{archive}: unsafe path {n}")
        z.extractall(root)
    return folder


def main():
    a = c.args(__doc__).parse_args()
    from vosk import KaldiRecognizer, Model, SetLogLevel

    SetLogLevel(-1)
    clips = c.clips(a)
    folder = unpack(a.model)
    t = time.perf_counter()
    model = Model(str(folder))
    load_ms = (time.perf_counter() - t) * 1000

    def stream(clip):
        with wave.open(str(clip["_path"])) as wf:
            raw = wf.readframes(wf.getnframes())
        rec = KaldiRecognizer(model, 16000)
        block = int(16000 * BLOCK_SECONDS) * 2
        busy = 0.0
        first = None
        words = []
        compute = 0.0
        for i in range(0, len(raw), block):
            arrives = min(i + block, len(raw)) / 2 / 16000  # a short last block arrives when the clip ends
            start = max(arrives, busy)
            t = time.perf_counter()
            if rec.AcceptWaveform(raw[i:i + block]):
                text = json.loads(rec.Result()).get("text", "")
                if text:
                    words.append(text)
            else:
                text = json.loads(rec.PartialResult()).get("partial", "")
            took = time.perf_counter() - t
            compute += took
            busy = start + took
            if text and first is None:
                first = busy
        key_up = len(raw) / 2 / 16000
        t = time.perf_counter()
        last = json.loads(rec.FinalResult()).get("text", "")
        took = time.perf_counter() - t
        compute += took
        if last:
            words.append(last)
        done = max(busy, key_up) + took
        return " ".join(words), {
            "t_first_ms": (first if first is not None else done) * 1000,
            "t_final_ms": (done - key_up) * 1000,
            "compute_seconds": compute,
            "rtf": compute / max(key_up, 1e-6),
        }

    stream(clips[0])  # warm-up, not counted
    w = c.Writer(a, "vosk", load_ms, c.Usage(), mode="streaming, simulated clock")
    for clip in clips:
        w.clip(clip, stream)


if __name__ == "__main__":
    main()
