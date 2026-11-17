# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""whisper.cpp on the Silas speech test set, through whisper-server so the model loads once.

  bench/engines/build-whisper-cpp.sh
  uv run bench/engines/whisper_cpp.py --manifest bench/manifest.jsonl --set ~/.cache/silas-bench/bench-set-v1 --model small.en --out bench/out/whisper-small.en.jsonl

--model is a ggml model name under ~/.cache/silas-bench/models/ggml (base.en, small.en, large-v3-turbo-q5_0).
The server keeps its own defaults (beam search, 4 threads unless --threads says otherwise). CPU and memory
are the server process's.
"""

import json
import socket
import subprocess
import sys
import time
import urllib.request
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _common as c

SERVER = c.CACHE / "engines/whisper.cpp-v1.9.5/build/bin/whisper-server"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def post(url, path):
    boundary = uuid.uuid4().hex
    parts = []
    for k, v in {"response_format": "json", "temperature": "0"}.items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
    parts.append(
        f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="clip.wav"\r\n'
        f"Content-Type: audio/wav\r\n\r\n".encode() + Path(path).read_bytes() + b"\r\n"
    )
    parts.append(f"--{boundary}--\r\n".encode())
    req = urllib.request.Request(url, data=b"".join(parts), headers={"Content-Type": f"multipart/form-data; boundary={boundary}"})
    with urllib.request.urlopen(req, timeout=600) as r:
        return json.loads(r.read())["text"]


def main():
    p = c.args(__doc__)
    p.add_argument("--threads", type=int, default=4)
    a = p.parse_args()
    model = c.CACHE / f"models/ggml/ggml-{a.model}.bin"
    if not SERVER.exists() or not model.exists():
        raise SystemExit("run bench/engines/build-whisper-cpp.sh first")
    clips = c.clips(a)
    port = free_port()
    t = time.perf_counter()
    server = subprocess.Popen(
        [str(SERVER), "-m", str(model), "-t", str(a.threads), "--host", "127.0.0.1", "--port", str(port), "-l", "en"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        while True:
            if server.poll() is not None:
                raise SystemExit("whisper-server exited")
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1).read()
                break
            except OSError:
                time.sleep(0.05)
        load_ms = (time.perf_counter() - t) * 1000
        url = f"http://127.0.0.1:{port}/inference"
        post(url, clips[0]["_path"])  # warm-up, not counted
        w = c.Writer(a, "whisper.cpp", load_ms, c.Usage(server.pid))
        for clip in clips:
            w.clip(clip, lambda clip: (post(url, clip["_path"]), {"threads": a.threads}))
    finally:
        server.terminate()
        server.wait()


if __name__ == "__main__":
    main()
