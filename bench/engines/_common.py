"""Shared by the rival-engine adapters: the same arguments as `silas-bench run`, the same clips and the
same results.jsonl rows, so bench/score.py can set any engine beside Silas.

Batch engines see the whole clip after the key comes up, so t_first_ms = t_final_ms = decode time.
Model loading is timed once as load_ms and kept out of every other number.
"""

import argparse
import json
import resource
import struct
import time
import wave
from pathlib import Path

CACHE = Path.home() / ".cache/silas-bench"
RAPL = Path("/sys/class/powercap/intel-rapl:0/energy_uj")


def args(engine_help):
    p = argparse.ArgumentParser(description=engine_help)
    p.add_argument("--manifest", required=True, type=Path)
    p.add_argument("--set", type=Path, help="audio paths are relative to this (default: the manifest's folder)")
    p.add_argument("--subset", type=Path, help="file of clip ids, one per line")
    p.add_argument("--model", required=True)
    p.add_argument("--out", required=True, type=Path)
    return p


def clips(a):
    keep = None
    if a.subset:
        keep = {l.strip() for l in a.subset.read_text().splitlines() if l.strip() and not l.startswith("#")}
    rows = [json.loads(l) for l in a.manifest.read_text().splitlines() if l.strip()]
    rows = [r for r in rows if keep is None or r["id"] in keep]
    if not rows:
        raise SystemExit("no clips to run")
    base = a.set or a.manifest.parent
    for r in rows:
        r["_path"] = base / r["audio"]
    return rows


def read_wav(path):
    """16 kHz mono 16-bit PCM as floats in [-1, 1]."""
    with wave.open(str(path)) as w:
        if (w.getframerate(), w.getnchannels(), w.getsampwidth()) != (16000, 1, 2):
            raise SystemExit(f"{path}: not 16 kHz mono 16-bit")
        raw = w.readframes(w.getnframes())
    return [s / 32768 for s in struct.unpack(f"<{len(raw) // 2}h", raw)]


def wav_seconds(path):
    with wave.open(str(path)) as w:
        return w.getnframes() / w.getframerate()


def load1():
    try:
        return float(Path("/proc/loadavg").read_text().split()[0])
    except (OSError, ValueError, IndexError):
        return None


def energy():
    try:
        return int(RAPL.read_text()) / 1e6
    except OSError:
        return None


class Usage:
    """CPU seconds and peak memory of this process, or of `pid` when the engine runs in a server process."""

    def __init__(self, pid=None):
        self.pid = pid

    def cpu(self):
        if self.pid is None:
            r = resource.getrusage(resource.RUSAGE_SELF)
            return r.ru_utime + r.ru_stime
        fields = Path(f"/proc/{self.pid}/stat").read_text().rsplit(")", 1)[1].split()
        import os
        return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")

    def peak_mb(self):
        if self.pid is None:
            return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
        for line in Path(f"/proc/{self.pid}/status").read_text().splitlines():
            if line.startswith("VmHWM:"):
                return int(line.split()[1]) / 1024
        return None


class Writer:
    def __init__(self, a, engine, load_ms, usage, mode="batch"):
        self.f = open(a.out, "w")
        self.a, self.engine, self.load_ms, self.usage, self.mode = a, engine, load_ms, usage, mode

    def clip(self, clip, decode):
        """Runs decode(clip) -> (text, extra dict or None) and writes its row."""
        seconds = wav_seconds(clip["_path"])
        cpu0, e0, load = self.usage.cpu(), energy(), load1()
        t = time.perf_counter()
        text, extra = decode(clip)
        took = time.perf_counter() - t
        cpu1, e1 = self.usage.cpu(), energy()
        row = {
            "id": clip["id"], "engine": self.engine, "model": self.a.model, "category": clip.get("category"),
            "hypothesis": text.strip(), "hypothesis_raw": text,
            "t_first_ms": took * 1000, "t_final_ms": took * 1000,
            "audio_seconds": seconds, "compute_seconds": took, "rtf": took / max(seconds, 1e-6),
            "peak_rss_mb": self.usage.peak_mb(), "cpu_seconds": cpu1 - cpu0,
            "energy_j": (e1 - e0) if e0 is not None and e1 is not None and e1 >= e0 else None,
            "load_ms": self.load_ms, "load1": load, "paced": False, "mode": self.mode,
        }
        row.update(extra or {})
        self.f.write(json.dumps(row) + "\n")
        self.f.flush()
        print(f"{clip['id']}: {seconds:.1f}s audio, final {row['t_final_ms']:.0f} ms, load {load}", flush=True)
        return row
