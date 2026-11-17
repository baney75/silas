#!/usr/bin/env python3
"""Fetch bench set v1 and verify every file. Standard library only; run with python3 -I.

  python3 -I bench/tools/fetch.py [--dest DIR]      download the release tarball
  python3 -I bench/tools/fetch.py --rebuild [--dest DIR]
                                                    regenerate from the original sources
                                                    (needs uv; slow; downloads about 2 GB)

The default DIR is ~/.cache/silas-bench/bench-set-v1. Files are checked against
bench/checksums.sha256 from this checkout, so a tampered tarball cannot vouch for itself.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

BENCH = Path(__file__).resolve().parents[1]
SET = "bench-set-v1"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def expected() -> dict[str, str]:
    out = {}
    for line in (BENCH / "checksums.sha256").read_text().splitlines():
        digest, path = line.split("  ", 1)
        out[path] = digest
    return out


def download(url: str, dest: Path, size: int) -> None:
    print(f"downloading {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "silas-bench-fetch/1"})
    done = 0
    with urllib.request.urlopen(req, timeout=120) as r, open(dest, "wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
            done += len(chunk)
            print(f"\r  {done / 1e6:.0f} / {size / 1e6:.0f} MB", end="", flush=True)
    print()


def zstd_reader(path: Path):
    """A readable stream of the decompressed tarball: stdlib zstd on 3.14+, else the zstd CLI."""
    try:
        from compression import zstd  # Python 3.14+

        return zstd.open(path, "rb"), None
    except ImportError:
        if not shutil.which("zstd"):
            sys.exit("need Python 3.14+ or the zstd command to unpack the tarball")
        proc = subprocess.Popen(["zstd", "-dc", str(path)], stdout=subprocess.PIPE)
        return proc.stdout, proc


def extract(archive: Path, dest: Path) -> None:
    """Unpack only regular files under bench-set-v1/, never links, absolute or parent paths."""
    stream, proc = zstd_reader(archive)
    with tarfile.open(fileobj=stream, mode="r|") as t:
        for m in t:
            name = m.name
            if not m.isfile() or not name.startswith(SET + "/") or ".." in Path(name).parts:
                sys.exit(f"refusing archive member {name!r}")
            out = dest / name[len(SET) + 1:]
            out.parent.mkdir(parents=True, exist_ok=True)
            with t.extractfile(m) as src, open(out, "wb") as f:
                shutil.copyfileobj(src, f)
    stream.close()
    if proc and proc.wait() != 0:
        sys.exit("zstd failed")


def verify(dest: Path, synth_ok: bool = False) -> bool:
    want = expected()
    tts = set()
    if synth_ok:
        for line in (BENCH / "manifest.jsonl").read_text().splitlines():
            r = json.loads(line)
            if r["tts"]:
                tts.add(r["audio"])
    bad = missing = warn = 0
    for path, digest in want.items():
        p = dest / path
        if not p.is_file():
            print(f"MISSING {path}")
            missing += 1
        elif sha256(p) != digest:
            if path in tts:
                warn += 1
            else:
                print(f"BAD     {path}")
                bad += 1
    print(f"verified {len(want) - bad - missing - warn}/{len(want)} files in {dest}")
    if warn:
        print(f"note: {warn} synthesized clips differ in bytes from the release (TTS output depends on CPU and onnxruntime build); run check.py to confirm durations")
    return bad == 0 and missing == 0


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dest", type=Path, default=Path.home() / ".cache" / "silas-bench" / SET)
    ap.add_argument("--rebuild", action="store_true", help="regenerate from sources instead of downloading")
    args = ap.parse_args()
    dest = args.dest.expanduser().resolve()
    dest.mkdir(parents=True, exist_ok=True)

    if args.rebuild:
        env = dict(os.environ, OMP_NUM_THREADS=os.environ.get("OMP_NUM_THREADS", "3"))
        cmd = ["nice", "uv", "run", "-q", str(BENCH / "tools" / "build.py"), "--dest", str(dest), "--manifest-out", str(dest)]
        print(" ".join(cmd))
        if subprocess.run(cmd, env=env).returncode:
            sys.exit("build failed")
        sys.exit(0 if verify(dest, synth_ok=True) else 1)

    rel = json.loads((BENCH / "release.json").read_text())
    if verify_quiet(dest):
        print(f"already present and verified: {dest}")
        return
    with tempfile.TemporaryDirectory(dir=dest.parent) as tmp:
        archive = Path(tmp) / rel["asset"]
        download(rel["url"], archive, rel["bytes"])
        got = sha256(archive)
        if got != rel["sha256"]:
            sys.exit(f"tarball sha256 {got} does not match {rel['sha256']}")
        extract(archive, dest)
    sys.exit(0 if verify(dest) else 1)


def verify_quiet(dest: Path) -> bool:
    want = expected()
    return all((dest / p).is_file() and sha256(dest / p) == d for p, d in want.items())


if __name__ == "__main__":
    main()
