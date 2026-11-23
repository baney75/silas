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
Hugging Face download, no account). A `hex service` keeps the model loaded, as whisper-server does for whisper.cpp.

Each clip is timed the way Hex's own app finishes a dictation on a CPU (bench/rivals/opencode.md, "Timing Hex"):
the CPU engine feeds nothing to the model while the key is held, and at key up it records a 200 ms tail, then
transcribes the whole clip once. So the adapter waits out the clip in real time (key down at its start, key up at
its end), waits the same 200 ms tail Silas's replay waits, and sends the clip plus that tail of quiet room to the
warm service's own local API, `POST /transcriptions`, in one request. t_final_ms is key up to the answer;
t_first_ms is the same, since a CPU engine shows no words before the end. --fast skips the real-time wait (the
model's time stays real).

The request half-closes its socket once the WAV is sent. Hex's handler checks between 100 ms waits for the result
whether the client went away, by peeking the socket under a 500 ms read timeout; a client that keeps its socket
open makes every peek wait the full 500 ms, so the answer comes in whole ~600 ms steps. That is what `hex
transcribe` measured. A half-closed socket peeks as end of file at once, so the answer comes when the model is done.

CPU is the service's; memory is the service's peak.

--cli times each clip through `hex transcribe` against the warm service, the old way (polled; kept to show the
steps). --one-shot runs each clip with no service, as `hex transcribe` does when Hex is not running: every clip
starts its own embedded service and loads the model, so its times are not dictation latency. Its text is not always
the service's (clean-0011 keeps a final period the service drops), so bench/rivals scores Hex's accuracy both ways
and keeps the better.
"""

import hashlib
import io
import json
import os
import resource
import socket
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
DISCOVERY = HOME / ".local/share/hex/local-api.json"
# Audio Hex keeps after a release (POST_RELEASE_TAIL, src/dictation.rs:113 at the pinned commit), the same 200 ms
# as Silas's TAIL (core/voice/src/core.rs). The replay fills it with quiet room, as Silas's does.
TAIL_SECONDS = 0.2
RATE = 16000


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


def frames(path):
    """A clip's 16 kHz mono 16-bit samples, as bytes."""
    import wave
    with wave.open(str(path)) as w:
        if (w.getframerate(), w.getnchannels(), w.getsampwidth()) != (RATE, 1, 2):
            raise SystemExit(f"{path}: not 16 kHz mono 16-bit")
        return w.readframes(w.getnframes())


def wav(samples):
    import wave
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(samples)
    return out.getvalue()


def with_tail(path):
    """The clip, then the tail of quiet room recorded after release, as one WAV file."""
    return wav(frames(path) + b"\0\0" * round(TAIL_SECONDS * RATE))


class Api:
    """Hex's local API, found the way Hex's own clients find it: the owner-only discovery file in its data folder
    (docs/streams.md, "Discovery and auth"). The token is a bench-only one in hex-home and is never printed."""

    def __init__(self, model):
        d = json.loads(DISCOVERY.read_text())
        self.port, self.token = d["port"], d["token"]
        self.path = f"/transcriptions?model={model}&language=en"

    def transcribe(self, wav):
        """One whole-clip transcription. The write side closes once the body is sent, so Hex's abort check sees end
        of file at once instead of waiting out its 500 ms read timeout (see the module docstring)."""
        head = (f"POST {self.path} HTTP/1.1\r\nHost: 127.0.0.1:{self.port}\r\n"
                f"Authorization: Bearer {self.token}\r\nContent-Type: audio/wav\r\n"
                f"Content-Length: {len(wav)}\r\nConnection: close\r\n\r\n").encode()
        with socket.create_connection(("127.0.0.1", self.port), timeout=600) as s:
            s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            s.sendall(head + wav)
            s.shutdown(socket.SHUT_WR)
            chunks = []
            while chunk := s.recv(65536):
                chunks.append(chunk)
        raw = b"".join(chunks)
        status_line, _, rest = raw.partition(b"\r\n")
        status = int(status_line.split()[1])
        body = rest.partition(b"\r\n\r\n")[2]
        if status != 200:
            code = body.decode(errors="replace")
            if "no-speech" in code:
                return ""
            raise SystemExit(f"hex POST /transcriptions: {status} {code}")
        return json.loads(body)["transcript"].strip()


class Usage(c.Usage):
    """The service's CPU plus every finished `hex transcribe` child's."""

    def cpu(self):
        r = resource.getrusage(resource.RUSAGE_CHILDREN)
        return super().cpu() + r.ru_utime + r.ru_stime


def main():
    p = c.args(__doc__)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--one-shot", action="store_true", help="no service: each clip loads the model itself")
    mode.add_argument("--cli", action="store_true", help="time `hex transcribe` against the service (polled)")
    p.add_argument("--fast", action="store_true", help="skip the real-time wait for each clip")
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
    DISCOVERY.unlink(missing_ok=True)
    service = subprocess.Popen([str(BIN), "service"], env=env(), cwd=HOME, stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        # The service publishes itself once its listener is bound; until then `hex transcribe` would start an
        # embedded one beside it, so wait for the discovery file Hex writes.
        deadline = time.monotonic() + 30
        while not DISCOVERY.exists():
            if service.poll() is not None or time.monotonic() > deadline:
                raise SystemExit("hex service did not start")
            time.sleep(0.05)
        if a.cli:
            t = time.perf_counter()
            transcribe(clips[0])  # loads the model; this first call is load_ms and is not counted
            load_ms = (time.perf_counter() - t) * 1000
            w = c.Writer(a, "hex", load_ms, Usage(service.pid), mode="cli")
            for clip in clips:
                w.clip(clip, lambda clip: (transcribe(clip), {"hex_version": VERSION}))
            return
        api = None
        while api is None:
            try:
                api = Api(a.model)
            except (ValueError, KeyError):  # caught mid-write
                if time.monotonic() > deadline:
                    raise SystemExit("hex service wrote no readable discovery file")
                time.sleep(0.05)
        def warm(audio):
            # The service drops a request whose body stalls past its 500 ms read timeout (local_api.rs:49), which
            # a busy machine can cause; these requests are not timed, so try again.
            for attempt in range(3):
                try:
                    return api.transcribe(audio)
                except ConnectionError:
                    if attempt == 2:
                        raise
                    time.sleep(1)

        t = time.perf_counter()
        warm(with_tail(clips[0]["_path"]))  # loads the model; load_ms, not counted
        load_ms = (time.perf_counter() - t) * 1000
        # Audio past 30 s runs on the service's own copy of the model rather than the dictation engine's
        # (SHARED_MAX_SAMPLES, src/transcription/service.rs:38), so load that copy too before any clip is timed.
        one = frames(clips[0]["_path"])
        warm(wav(one * (31 * RATE * 2 // max(len(one), 1) + 1)))
        w = c.Writer(a, "hex", load_ms, c.Usage(service.pid), mode="api")

        def dictate(clip):
            audio = with_tail(clip["_path"])
            seconds = c.wav_seconds(clip["_path"])
            if not a.fast:
                time.sleep(seconds)  # the key is held while the clip plays; a CPU engine does nothing yet
            key_up = time.perf_counter()
            if not a.fast:
                time.sleep(TAIL_SECONDS)  # the tail is recorded after release, in real time
            t = time.perf_counter()
            text = api.transcribe(audio)
            done = time.perf_counter()
            # --fast still counts the tail: the app always waits for it after key up.
            final_ms = (done - key_up) * 1000 + (TAIL_SECONDS * 1000 if a.fast else 0)
            return text, {"hex_version": VERSION, "t_first_ms": final_ms, "t_final_ms": final_ms,
                          "compute_seconds": done - t, "rtf": (done - t) / max(seconds, 1e-6),
                          "paced": not a.fast, "tail_ms": TAIL_SECONDS * 1000}

        for clip in clips:
            w.clip(clip, dictate)
    finally:
        service.terminate()
        service.wait()


if __name__ == "__main__":
    main()
