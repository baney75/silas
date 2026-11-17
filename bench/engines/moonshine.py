# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["useful-moonshine-onnx==20251121", "numpy<2.3"]
# ///
"""Moonshine (Useful Sensors' ONNX release) on the Silas speech test set.

  uv run bench/engines/moonshine.py --manifest bench/manifest.jsonl --set ~/.cache/silas-bench/bench-set-v1 --model base --out bench/out/moonshine-base.jsonl

--model is tiny or base. Weights come from the Hugging Face repo UsefulSensors/moonshine into
~/.cache/silas-bench/hf. Each clip is decoded whole after the key comes up.
"""

import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _common as c

os.environ.setdefault("HF_HOME", str(c.CACHE / "hf"))


def main():
    a = c.args(__doc__).parse_args()
    import numpy as np
    from moonshine_onnx import MoonshineOnnxModel, load_tokenizer
    from moonshine_onnx.model import _get_onnx_weights

    clips = c.clips(a)
    _get_onnx_weights(a.model)  # download first, so load_ms is only the load
    t = time.perf_counter()
    model = MoonshineOnnxModel(model_name=f"moonshine/{a.model}")
    tokenizer = load_tokenizer()
    load_ms = (time.perf_counter() - t) * 1000

    def decode(clip):
        audio = np.asarray(c.read_wav(clip["_path"]), dtype=np.float32)[np.newaxis, :]
        return tokenizer.decode_batch(model.generate(audio))[0], None

    decode(clips[0])  # warm-up, not counted
    w = c.Writer(a, "moonshine", load_ms, c.Usage())
    for clip in clips:
        w.clip(clip, decode)


if __name__ == "__main__":
    main()
