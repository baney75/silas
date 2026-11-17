"""Shared helpers for the bench set build: source downloads, safe extraction and WAV IO.

Downloaded archives are untrusted data. They are extracted into their own folders with
tarfile's "data" filter or zipfile name checks, and nothing inside them is ever executed.
"""

import hashlib
import io
import os
import tarfile
import urllib.request
import wave
import zipfile
from pathlib import Path

import numpy as np

CACHE = Path(os.environ.get("SILAS_BENCH_CACHE", Path.home() / ".cache" / "silas-bench"))
SR = 16000

LIBRISPEECH = {
    "test-clean": ("https://www.openslr.org/resources/12/test-clean.tar.gz", "32fa31d27d2e1cad72775fee3f4849a9"),
    "test-other": ("https://www.openslr.org/resources/12/test-other.tar.gz", "fb5a50374b501bb3bac4815ee91d3135"),
}
LIBRISPEECH_PC = ("https://www.openslr.org/resources/145/manifests.tar.gz", None)
DEMAND = "https://zenodo.org/records/1227121/files/{env}_16k.zip?download=1"
FSD50K = "https://huggingface.co/datasets/Fhrozen/FSD50k/resolve/main/clips/{split}/{clip}.wav"
EDACC = "https://huggingface.co/datasets/edinburghcstr/edacc/resolve/main/{file}"
KOKORO = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/{name}"

# sha256 of every downloaded file, pinned after the first build so a changed upstream fails loudly.
PINNED_SHA256: dict[str, str] = {}


def file_digest(path: Path, algo: str = "sha256") -> str:
    h = hashlib.new(algo)
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url: str, dest: Path, md5: str | None = None) -> Path:
    """Fetch url into dest once. Checks md5 (LibriSpeech publishes md5) and the pinned sha256."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    if not dest.exists():
        tmp = dest.with_suffix(dest.suffix + ".part")
        print(f"download {url}", flush=True)
        req = urllib.request.Request(url, headers={"User-Agent": "silas-bench/1"})
        with urllib.request.urlopen(req, timeout=120) as r, open(tmp, "wb") as f:
            while chunk := r.read(1 << 20):
                f.write(chunk)
        tmp.rename(dest)
    if md5 and file_digest(dest, "md5") != md5:
        raise SystemExit(f"md5 mismatch for {dest}")
    pinned = PINNED_SHA256.get(dest.name)
    if pinned and file_digest(dest) != pinned:
        raise SystemExit(f"sha256 mismatch for {dest}")
    return dest


def safe_untar(archive: Path, out: Path) -> Path:
    stamp = out / ".extracted"
    if stamp.exists():
        return out
    out.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive) as t:
        t.extractall(out, filter="data")
    stamp.touch()
    return out


def safe_unzip_member(archive: Path, member: str, dest: Path) -> Path:
    if dest.exists():
        return dest
    if member.startswith("/") or ".." in Path(member).parts:
        raise SystemExit(f"refusing zip member {member}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as z, z.open(member) as src, open(dest, "wb") as f:
        f.write(src.read())
    return dest


def librispeech_root(split: str) -> Path:
    url, md5 = LIBRISPEECH[split]
    arc = download(url, CACHE / "dl" / "librispeech" / f"{split}.tar.gz", md5)
    return safe_untar(arc, CACHE / "src" / "librispeech" / split) / "LibriSpeech" / split


def librispeech_flac(split: str, utt: str) -> Path:
    spk, chap, _ = utt.split("-")
    return librispeech_root(split) / spk / chap / f"{utt}.flac"


def librispeech_transcripts(split: str) -> dict[str, str]:
    out = {}
    for p in sorted(librispeech_root(split).glob("*/*/*.trans.txt")):
        for line in p.read_text().splitlines():
            utt, text = line.split(" ", 1)
            out[utt] = text
    return out


def librispeech_pc(split: str) -> dict[str, dict]:
    import json

    arc = download(LIBRISPEECH_PC[0], CACHE / "dl" / "librispeech-pc" / "manifests.tar.gz")
    root = safe_untar(arc, CACHE / "src" / "librispeech-pc")
    out = {}
    for line in (root / f"{split}.json").read_text().splitlines():
        row = json.loads(line)
        out[Path(row["audio_filepath"]).stem] = row
    return out


def demand_wav(env: str, channel: int) -> Path:
    arc = download(DEMAND.format(env=env), CACHE / "dl" / "demand" / f"{env}_16k.zip")
    return safe_unzip_member(arc, f"{env}/ch{channel:02d}.wav", CACHE / "src" / "demand" / env / f"ch{channel:02d}.wav")


def fsd50k_clip(split: str, clip: str) -> Path:
    return download(FSD50K.format(split=split, clip=clip), CACHE / "dl" / "fsd50k" / "clips" / split / f"{clip}.wav")


def edacc_audio(file: str, row: int) -> Path:
    """Audio bytes of one EdAcc row. Reads only the row group that holds it, over HTTP ranges."""
    dest = CACHE / "src" / "edacc" / f"{Path(file).stem}-{row:05d}.wav"
    if dest.exists():
        return dest
    import fsspec
    import pyarrow.parquet as pq

    fs = fsspec.filesystem("https")
    with fs.open(EDACC.format(file=file), block_size=1 << 22) as fh:
        pf = pq.ParquetFile(fh)
        start = 0
        for g in range(pf.metadata.num_row_groups):
            n = pf.metadata.row_group(g).num_rows
            if start <= row < start + n:
                t = pf.read_row_group(g, columns=["audio"])
                dest.parent.mkdir(parents=True, exist_ok=True)
                # Cache every row in the group; neighbours are often selected too.
                for i in range(n):
                    p = CACHE / "src" / "edacc" / f"{Path(file).stem}-{start + i:05d}.wav"
                    if not p.exists():
                        p.write_bytes(t.column("audio")[i].as_py()["bytes"])
                return dest
            start += n
    raise SystemExit(f"row {row} not in {file}")


def read_audio(path_or_bytes) -> tuple[np.ndarray, int]:
    """Float mono audio from any file soundfile can read."""
    import soundfile as sf

    src = io.BytesIO(path_or_bytes) if isinstance(path_or_bytes, bytes) else str(path_or_bytes)
    x, sr = sf.read(src, dtype="float64", always_2d=True)
    return x.mean(axis=1), sr


def to_16k(x: np.ndarray, sr: int) -> np.ndarray:
    if sr == SR:
        return x
    import soxr

    return soxr.resample(x, sr, SR, quality="VHQ")


def to_int16(x: np.ndarray) -> np.ndarray:
    return np.clip(np.round(x * 32767.0), -32768, 32767).astype("<i2")


def write_wav(path: Path, pcm: np.ndarray) -> float:
    """Write 16 kHz mono 16-bit PCM. Returns the duration in seconds."""
    assert pcm.dtype == np.dtype("<i2") and pcm.ndim == 1
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())
    return len(pcm) / SR
