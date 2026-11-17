# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "numpy",
#   "soundfile",
#   "soxr",
#   "pyarrow",
#   "fsspec",
#   "aiohttp",
#   "kokoro-onnx==0.4.9",
#   "onnxruntime",
# ]
# ///
"""Build bench set v1 from its sources.

Reads bench/sources/{selection.json, refs.json, synth.toml}, writes the audio to --dest
(default ~/.cache/silas-bench/bench-set-v1), and rewrites bench/manifest.jsonl and
bench/checksums.sha256. With --pack it also writes bench-set-v1.tar.zst next to --dest.

  OMP_NUM_THREADS=3 nice uv run bench/tools/build.py [--dest DIR] [--only CATEGORY] [--pack]

Everything is deterministic: fixed seeds, fixed offsets, fixed resampler. TTS clips match
bit for bit only with the same onnxruntime build and CPU family; their durations match.
"""

import argparse
import hashlib
import io
import json
import os
import random
import re
import socket
import tarfile
import tomllib
import unicodedata
from pathlib import Path

import numpy as np

import sources as S

ROOT = Path(__file__).resolve().parents[1]
SET = "bench-set-v1"
SEED = 20261117
THREADS = int(os.environ.get("OMP_NUM_THREADS", "3"))

# tts field: engine, voice and the host that rendered it. Kokoro v1.0 weights via kokoro-onnx 0.4.9.
KOKORO_TAG = "kokoro-82m"
HOST = socket.gethostname().split(".")[0]
# Voice rotation for synthesized clips: US and UK, female and male.
VOICES = ["af_heart", "am_michael", "bf_emma", "bm_george", "af_bella", "am_fenrir", "bf_isabella", "bm_fable"]
SYNTH_RMS_DBFS = -22.0
GAP_S = 0.35  # silence between LibriSpeech utterances in a longform passage
LICENSES = {
    "librispeech": "CC BY 4.0",
    "edacc": "CC BY-SA 4.0",
    "synth": "CC BY 4.0",
}


def verbatim(text: str) -> str:
    """Spoken words: lowercase, accents folded, hyphens split, punctuation dropped."""
    t = unicodedata.normalize("NFKD", text)
    t = "".join(c for c in t if not unicodedata.combining(c)).lower()
    t = t.replace("-", " ").replace("’", "'")
    t = re.sub(r"[^a-z0-9' ]", " ", t)
    words = [w.strip("'") if w not in ("'cause",) else w for w in t.split()]
    out = " ".join(w for w in words if w)
    assert not re.search(r"\d", out), f"digits in verbatim: {out}"
    return out


def rng_for(clip_id: str) -> random.Random:
    return random.Random(f"{SEED}-{clip_id}")


def power(x: np.ndarray) -> float:
    return float(np.mean(x * x)) + 1e-12


def row(clip_id, category, audio, seconds, ref_v, ref_f, speaker, accent, source, license, *, snr=None, noise=None, tts=None, notes=None):
    return {
        "id": clip_id, "category": category, "audio": audio, "seconds": round(seconds, 3),
        "ref_verbatim": ref_v, "ref_formatted": ref_f, "speaker": speaker, "accent": accent,
        "snr_db": snr, "noise": noise, "source": source, "license": license, "tts": tts, "notes": notes,
    }


# ---------------------------------------------------------------------------------- categories


def build_clean(sel, refs, dest):
    trans = S.librispeech_transcripts("test-clean")
    out = []
    for c in sel["clean"]:
        x, sr = S.read_audio(S.librispeech_flac("test-clean", c["utt"]))
        assert sr == S.SR
        rel = f"clean/{c['id']}.wav"
        secs = S.write_wav(dest / rel, S.to_int16(x))
        out.append(row(c["id"], "clean", rel, secs, verbatim(trans[c["utt"]]), refs[c["id"]],
                       f"librispeech-{c['utt'].split('-')[0]}", None,
                       f"librispeech test-clean {c['utt']}", LICENSES["librispeech"]))
    return out


def load_noise(src: list) -> tuple[np.ndarray, str, str]:
    if src[0] == "demand":
        _, env, ch = src
        x, sr = S.read_audio(S.demand_wav(env, ch))
        return S.to_16k(x, sr), f"DEMAND {env} ch{ch:02d}", "CC BY 4.0"
    _, split, clip = src
    x, sr = S.read_audio(S.fsd50k_clip(split, clip))
    return S.to_16k(x, sr), f"FSD50K {split} {clip}", "CC0 1.0"


def build_noise(sel, refs, dest, clean_rows):
    base = {r["id"]: r for r in clean_rows}
    trans_ids = {c["id"]: c["utt"] for c in sel["clean"]}
    cache = {}
    out = []
    for n in sel["noise"]:
        b = base[n["base"]]
        speech, _ = S.read_audio(dest / b["audio"])
        key = tuple(n["noise_src"])
        if key not in cache:
            cache[key] = load_noise(n["noise_src"])
        noise, desc, _ = cache[key]
        if len(noise) < len(speech):  # short FSD50K clip: loop it
            noise = np.tile(noise, int(np.ceil(len(speech) / len(noise))) + 1)
        start = int(round(n["offset_frac"] * (len(noise) - len(speech))))
        seg = noise[start:start + len(speech)]
        gain = np.sqrt(power(speech) / (power(seg) * 10 ** (n["snr_db"] / 10)))
        mix = speech + gain * seg
        peak = float(np.max(np.abs(mix)))
        if peak > 0.98:  # scale the whole mix; SNR is unchanged
            mix *= 0.98 / peak
        rel = f"noise/{n['id']}.wav"
        secs = S.write_wav(dest / rel, S.to_int16(mix))
        out.append(row(n["id"], "noise", rel, secs, b["ref_verbatim"], b["ref_formatted"], b["speaker"], None,
                       f"librispeech test-clean {trans_ids[n['base']]} ({n['base']}) + {desc} at {start / S.SR:.3f} s",
                       "CC BY 4.0", snr=n["snr_db"], noise=n["noise"]))
    return out


def build_accent(sel, refs, dest):
    out = []
    for a in sel["accent"]:
        x, sr = S.read_audio(S.edacc_audio(a["file"], a["row"]))
        rel = f"accent/{a['id']}.wav"
        secs = S.write_wav(dest / rel, S.to_int16(S.to_16k(x, sr)))
        out.append(row(a["id"], "accent", rel, secs, verbatim(a["text"]), refs[a["id"]], a["speaker"], a["accent"],
                       f"edacc test {a['file']} row {a['row']}", LICENSES["edacc"],
                       notes=f"EdAcc accent: {a['edacc_accent']}; L1: {a['l1']}; conversational speech"))
    return out


def build_longform_ls(sel, refs, dest):
    trans = S.librispeech_transcripts("test-other")
    out = []
    for lf in sel["longform_librispeech"]:
        gap = np.zeros(int(GAP_S * S.SR))
        parts = []
        for i, u in enumerate(lf["utts"]):
            x, sr = S.read_audio(S.librispeech_flac("test-other", u))
            assert sr == S.SR
            parts += ([gap] if i else []) + [x]
        rel = f"longform/{lf['id']}.wav"
        secs = S.write_wav(dest / rel, S.to_int16(np.concatenate(parts)))
        u0, u1 = lf["utts"][0], lf["utts"][-1]
        out.append(row(lf["id"], "longform", rel, secs, " ".join(verbatim(trans[u]) for u in lf["utts"]), refs[lf["id"]],
                       f"librispeech-{u0.split('-')[0]}", None,
                       f"librispeech test-other {u0} to {u1} ({len(lf['utts'])} utterances, {GAP_S} s gaps)",
                       LICENSES["librispeech"], notes="read audiobook passage"))
    return out


class Tts:
    def __init__(self):
        import onnxruntime as ort
        from kokoro_onnx import Kokoro

        mdir = S.CACHE / "models" / "kokoro"
        model = S.download(S.KOKORO.format(name="kokoro-v1.0.onnx"), mdir / "kokoro-v1.0.onnx")
        voices = S.download(S.KOKORO.format(name="voices-v1.0.bin"), mdir / "voices-v1.0.bin")
        so = ort.SessionOptions()
        so.intra_op_num_threads = THREADS
        so.inter_op_num_threads = 1
        sess = ort.InferenceSession(str(model), so, providers=["CPUExecutionProvider"])
        self.k = Kokoro.from_session(sess, str(voices))

    def say(self, text: str, voice: str, rng: random.Random) -> np.ndarray:
        lang = "en-gb" if voice.startswith("b") else "en-us"
        # One call per sentence, joined by short natural pauses.
        sentences = [s for s in re.split(r"(?<=[.?!])\s+", text.strip()) if s]
        parts = [np.zeros(int(0.25 * S.SR))]
        for i, s in enumerate(sentences):
            x, sr = self.k.create(s, voice=voice, speed=1.0, lang=lang)
            parts.append(S.to_16k(np.asarray(x, dtype=np.float64), sr))
            if i + 1 < len(sentences):
                parts.append(np.zeros(int(rng.uniform(0.25, 0.5) * S.SR)))
        parts.append(np.zeros(int(0.35 * S.SR)))
        y = np.concatenate(parts)
        y *= 10 ** (SYNTH_RMS_DBFS / 20) / np.sqrt(power(y[np.abs(y) > 1e-4]))
        peak = float(np.max(np.abs(y)))
        return y * (0.95 / peak) if peak > 0.95 else y


def build_synth(dest, only=None):
    spec = tomllib.loads((ROOT / "sources" / "synth.toml").read_text())
    respell = spec["respell"]
    tts = Tts()
    out = []
    counters = {}
    for it in spec["item"]:
        cat = it["id"].rsplit("-", 1)[0]
        if only and cat != only:
            continue
        n = counters[cat] = counters.get(cat, -1) + 1
        voice = VOICES[(n + len(cat)) % len(VOICES)]
        text = it["say"]
        for k, v in respell.items():
            text = re.sub(rf"\b{re.escape(k)}\b", v, text)
        y = tts.say(text, voice, rng_for(it["id"]))
        rel = f"{cat}/{it['id']}.wav"
        secs = S.write_wav(dest / rel, S.to_int16(y))
        print(f"{it['id']} {voice} {secs:.2f}s", flush=True)
        out.append(row(it["id"], cat, rel, secs, verbatim(it["say"]), it["formatted"], f"kokoro-{voice}",
                       "uk" if voice.startswith("b") else "us", "synthesized from bench/sources/synth.toml",
                       LICENSES["synth"], tts=f"{KOKORO_TAG} {voice} {HOST}", notes=it.get("notes")))
    return out


# ------------------------------------------------------------------------------------- output

ORDER = ["clean", "noise", "accent", "dictation", "code", "command", "longform"]


def write_outputs(rows, dest, out_dir):
    rows.sort(key=lambda r: (ORDER.index(r["category"]), r["id"]))
    with open(out_dir / "manifest.jsonl", "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    with open(out_dir / "checksums.sha256", "w") as f:
        for r in rows:
            f.write(f"{S.file_digest(dest / r['audio'])}  {r['audio']}\n")


def pack(rows, dest) -> Path:
    """Deterministic tar (sorted, mtime 0, root owner) of the audio plus manifest, then zstd -19."""
    tar_path = dest.parent / f"{SET}.tar"
    with tarfile.open(tar_path, "w", format=tarfile.PAX_FORMAT) as t:
        extra = [("manifest.jsonl", ROOT / "manifest.jsonl"), ("checksums.sha256", ROOT / "checksums.sha256")]
        for name, path in sorted([(r["audio"], dest / r["audio"]) for r in rows] + extra):
            data = path.read_bytes()
            ti = tarfile.TarInfo(f"{SET}/{name}")
            ti.size, ti.mtime, ti.mode, ti.uid, ti.gid, ti.uname, ti.gname = len(data), 0, 0o644, 0, 0, "root", "root"
            t.addfile(ti, io.BytesIO(data))
    out = dest.parent / f"{SET}.tar.zst"
    os.system(f"nice zstd -q -f -19 -T{THREADS} '{tar_path}' -o '{out}' && rm '{tar_path}'")
    print(f"packed {out} {out.stat().st_size} bytes sha256 {hashlib.sha256(out.read_bytes()).hexdigest()}")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dest", type=Path, default=S.CACHE / SET)
    ap.add_argument("--only", choices=["real", "synth"],
                    help="rebuild only recorded (real) or synthesized clips, keeping the other rows of the manifest")
    ap.add_argument("--manifest-out", type=Path, default=ROOT,
                    help="where to write manifest.jsonl and checksums.sha256 (fetch.py --rebuild points this at --dest)")
    ap.add_argument("--pack", action="store_true")
    ap.add_argument("--pack-only", action="store_true", help="pack the audio already in --dest using bench/manifest.jsonl")
    args = ap.parse_args()
    dest = args.dest.expanduser().resolve()
    if args.pack_only:
        pack([json.loads(line) for line in (ROOT / "manifest.jsonl").read_text().splitlines()], dest)
        return
    sel = json.loads((ROOT / "sources" / "selection.json").read_text())
    refs = json.loads((ROOT / "sources" / "refs.json").read_text())
    old_path = args.manifest_out.expanduser().resolve() / "manifest.jsonl"
    keep = [json.loads(line) for line in old_path.read_text().splitlines()] if args.only and old_path.exists() else []
    rows = []
    if args.only != "synth":
        clean = build_clean(sel, refs, dest)
        rows += clean + build_noise(sel, refs, dest, clean) + build_accent(sel, refs, dest) + build_longform_ls(sel, refs, dest)
    else:
        rows += [r for r in keep if r["tts"] is None]
    if args.only != "real":
        rows += build_synth(dest)
    else:
        rows += [r for r in keep if r["tts"]]
    write_outputs(rows, dest, args.manifest_out.expanduser().resolve())
    print(f"{len(rows)} clips, {sum(r['seconds'] for r in rows) / 60:.1f} min")
    if args.pack:
        pack(rows, dest)


if __name__ == "__main__":
    main()
