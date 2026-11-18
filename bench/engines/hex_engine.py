# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Hex (anomalyco/hex, the OpenCode team's dictation app) on the Silas speech test set.

  uv run bench/engines/hex_engine.py --manifest bench/manifest.jsonl --set ~/.cache/silas-bench/bench-set-v1 --model parakeet_unified_en --out bench/out/hex.jsonl

OpenCode's own voice (a GPT-Live call) needs a paid OpenAI key or an OpenCode Console account, so it can't run
free; bench/rivals/opencode.md says why. Hex is the same team's local dictation app and runs free and headless
on Linux, so it stands in. --model is a Hex model id (parakeet_unified_en is Hex's default on an English system).

The pinned Hex 3.4.0 release binary is downloaded and hash-checked into ~/.cache/silas-bench/engines/hex-3.4.0.
Hex runs with HOME and the XDG folders inside ~/.cache/silas-bench/hex-home, so it never reads or writes the
user's own Hex settings. Setup there: `hex labs on transcribe-files`, then `hex model install <model>` (a free
Hugging Face download, no account). A `hex service` keeps the model loaded, as whisper-server does for whisper.cpp;
each clip is one `hex transcribe <wav>` against it, which finds the service on its own.

This is Hex's file path: the whole clip after key up, so t_first_ms = t_final_ms. Hex's app streams partial text
while you talk with this model (the /streams API), which needs the running desktop app; that path is not replayed
here. CPU is the service's plus the `hex transcribe` processes'; memory is the service's peak.

--one-shot runs each clip with no service, as `hex transcribe` does when Hex is not running: every clip starts its
own embedded service and loads the model, so its times are not dictation latency. Its text is not always the
service's (clean-0011 keeps a final period the service drops), so bench/rivals scores Hex's accuracy both ways and
keeps the better.
"""

import hashlib
import os
import resource
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _common as c

VERSION = "3.4.0"
URL = f"https://github.com/anomalyco/hex/releases/download/v{VERSION}/hex-{VERSION}-linux-x64"
SHA256 = "7f8132a2202338c1b52c36113d23d7403cf91f220bcdbd9eec82bc726f5f8029"
BIN = c.CACHE / f"engines/hex-{VERSION}/hex-{VERSION}-linux-x64"
HOME = c.CACHE / "hex-home"


def env():
    e = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(HOME),
        "XDG_DATA_HOME": str(HOME / ".local/share"),
        "XDG_CONFIG_HOME": str(HOME / ".config"),
        "XDG_CACHE_HOME": str(HOME / ".cache"),
        "XDG_STATE_HOME": str(HOME / ".local/state"),
        "XDG_RUNTIME_DIR": str(HOME / "run"),
    }
    Path(e["XDG_RUNTIME_DIR"]).mkdir(parents=True, exist_ok=True, mode=0o700)
    return e


def fetch():
    if BIN.exists() and hashlib.sha256(BIN.read_bytes()).hexdigest() == SHA256:
        return
    BIN.parent.mkdir(parents=True, exist_ok=True)
    tmp = BIN.with_suffix(".part")
    with urllib.request.urlopen(URL, timeout=600) as r, open(tmp, "wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
    got = hashlib.sha256(tmp.read_bytes()).hexdigest()
    if got != SHA256:
        tmp.unlink()
        raise SystemExit(f"{URL}: sha256 {got}, expected {SHA256}")
    tmp.chmod(0o755)
    tmp.rename(BIN)


def hex_cmd(*args, check=True):
    r = subprocess.run([str(BIN), *args], env=env(), cwd=HOME, stdin=subprocess.DEVNULL,
                       capture_output=True, text=True, timeout=1800)
    if check and r.returncode != 0:
        raise SystemExit(f"hex {' '.join(args)}: {r.stderr.strip() or r.stdout.strip()}")
    return r


class Usage(c.Usage):
    """The service's CPU plus every finished `hex transcribe` child's."""

    def cpu(self):
        r = resource.getrusage(resource.RUSAGE_CHILDREN)
        return super().cpu() + r.ru_utime + r.ru_stime


def main():
    p = c.args(__doc__)
    p.add_argument("--one-shot", action="store_true", help="no service: each clip loads the model itself")
    a = p.parse_args()
    clips = c.clips(a)
    fetch()
    HOME.mkdir(parents=True, exist_ok=True)
    hex_cmd("labs", "on", "transcribe-files")
    hex_cmd("model", "install", a.model)

    def transcribe(clip):
        r = hex_cmd("transcribe", str(Path(clip["_path"]).resolve()), "--model", a.model, check=False)
        if r.returncode != 0 and "no-speech" not in r.stderr + r.stdout:
            raise SystemExit(f"hex transcribe {clip['id']}: {r.stderr.strip() or r.stdout.strip()}")
        return r.stdout.strip() if r.returncode == 0 else ""

    if a.one_shot:
        w = c.Writer(a, "hex", None, Usage(os.getpid()), mode="one-shot")
        for clip in clips:
            w.clip(clip, lambda clip: (transcribe(clip), {"hex_version": VERSION}))
        return
    # A file left by a killed run would end the wait below before this service is listening.
    (HOME / ".local/share/hex/local-api.json").unlink(missing_ok=True)
    service = subprocess.Popen([str(BIN), "service"], env=env(), cwd=HOME, stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        # The service publishes itself once its listener is bound; until then `hex transcribe` would start an
        # embedded one beside it, so wait for the discovery file Hex writes (its contents are never read here).
        found = HOME / ".local/share/hex/local-api.json"
        deadline = time.monotonic() + 30
        while not found.exists():
            if service.poll() is not None or time.monotonic() > deadline:
                raise SystemExit("hex service did not start")
            time.sleep(0.05)
        t = time.perf_counter()
        transcribe(clips[0])  # loads the model; this first call is load_ms and is not counted
        load_ms = (time.perf_counter() - t) * 1000
        w = c.Writer(a, "hex", load_ms, Usage(service.pid))
        for clip in clips:
            w.clip(clip, lambda clip: (transcribe(clip), {"hex_version": VERSION}))
    finally:
        service.terminate()
        service.wait()


if __name__ == "__main__":
    main()
