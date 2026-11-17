# /// script
# requires-python = ">=3.12"
# dependencies = ["numpy", "soundfile", "pyarrow", "fsspec", "aiohttp", "requests"]
# ///
"""One-off selection of source clips for bench set v1. Writes bench/sources/selection.json.

This ran once to freeze the set. The build reads selection.json, never this script, so a
rerun only matters when making a new set version. Selection is seeded and deterministic.

  uv run bench/tools/select_sources.py
"""

import json
import random
import re
from collections import Counter, defaultdict
from pathlib import Path

import requests

import sources as S

ROOT = Path(__file__).resolve().parents[1]
SEED = 20261117

# EdAcc accent label -> manifest accent code
ACCENTS = {
    "Nigerian English": "ng",
    "Indian English": "in",
    "Scottish English": "scot",
    "Irish English": "ie",
    "Chinese": "zh",
    "Spanish": "es",
    "Kenyan English": "ke",
    "Jamaican English": "jm",
    "Vietnamese": "vi",
    "Southern British English": "uk",
}
PER_ACCENT = 5
NOISES = ["cafe", "street", "keyboard", "fan"]
# Noise sources: DEMAND environments (16 kHz, 16 channels of 300 s) and CC0 FSD50K clips.
NOISE_SOURCES = {
    "cafe": [("demand", "PCAFETER", 1), ("demand", "PCAFETER", 9)],
    "street": [("demand", "STRAFFIC", 1), ("demand", "STRAFFIC", 9)],
    "keyboard": [("fsd50k", "dev", "155910"), ("fsd50k", "dev", "378389")],
    "fan": [("fsd50k", "eval", "118767"), ("fsd50k", "eval", "395734")],
}
LONGFORM_TARGETS = [130, 180, 215, 270, 290]  # seconds of LibriSpeech test-other per passage
# Chapters skipped for longform: 5764-299665 is a polemical essay against religion; any other
# chapter tests the model as well without putting that text in the owner's test set.
LONGFORM_SKIP = {("5764", "299665")}


def words(text: str) -> list[str]:
    return re.sub(r"[^a-z' ]", " ", text.lower().replace("-", " ")).split()


def pick_clean(rng: random.Random, pc: dict, trans: dict) -> list[dict]:
    by_spk = defaultdict(list)
    for utt, row in pc.items():
        # Only utterances where the punctuated text has exactly the official words.
        if words(row["text"]) == words(trans[utt]):
            by_spk[utt.split("-")[0]].append((utt, row["duration"]))
    speakers = sorted(by_spk)
    rng.shuffle(speakers)
    out = []
    for spk in sorted(speakers[:30], key=int):
        utts = sorted(by_spk[spk])
        short = [u for u, d in utts if 3.0 <= d < 7.0]
        medium = [u for u, d in utts if 7.0 <= d <= 15.0]
        out.append(rng.choice(short))
        out.append(rng.choice(medium))
    return [{"id": f"clean-{i + 1:04d}", "split": "test-clean", "utt": u} for i, u in enumerate(out)]


def pick_noise(rng: random.Random, clean: list[dict], pc: dict) -> list[dict]:
    base = clean[1::2]  # 30 clips, one per speaker, alternating short and medium
    out = []
    for i, c in enumerate(base):
        kind = NOISES[i % len(NOISES)]
        src = NOISE_SOURCES[kind][(i // len(NOISES)) % 2]
        for snr in (10, 0):
            out.append({
                "id": f"noise-{len(out) + 1:04d}",
                "base": c["id"],
                "noise": kind,
                "noise_src": list(src),
                "snr_db": snr,
                # Offset as a fraction of the room left in the noise file, resolved at build time.
                "offset_frac": round(rng.random(), 6),
            })
    return out


def pick_accent(rng: random.Random) -> list[dict]:
    files = sorted(
        x["path"]
        for x in requests.get("https://huggingface.co/api/datasets/edinburghcstr/edacc/tree/main/data", timeout=60).json()
        if "/test-" in x["path"]
    )
    rows = []
    import fsspec
    import pyarrow.parquet as pq

    fs = fsspec.filesystem("https")
    for f in files:
        with fs.open(S.EDACC.format(file=f), block_size=1 << 20) as fh:
            t = pq.ParquetFile(fh).read(columns=["speaker", "text", "accent", "l1", "gender"])
        for i in range(t.num_rows):
            r = {k: t.column(k)[i].as_py() for k in t.column_names}
            rows.append({**r, "file": f, "row": i})
    out = []
    for label, code in ACCENTS.items():
        cands = [
            r for r in rows
            if r["accent"] == label and not re.search(r"[<\[{_-]", r["text"]) and 12 <= len(r["text"].split()) <= 35
        ]
        spk = [s for s, _ in sorted(Counter(r["speaker"] for r in cands).items(), key=lambda kv: (-kv[1], kv[0]))][:2]
        per_spk = []
        for s in spk:
            mine = [r for r in cands if r["speaker"] == s]
            rng.shuffle(mine)
            per_spk.append(mine)
        # Alternate speakers so each accent gets a 3 and 2 split when it has two speakers.
        order = [r for group in zip(*per_spk) for r in group] + [r for g in per_spk for r in g[min(map(len, per_spk)):]]
        picked = []
        for r in order:
            if len(picked) == PER_ACCENT:
                break
            x, sr = S.read_audio(S.edacc_audio(r["file"], r["row"]))
            if 3.0 <= len(x) / sr <= 20.0:
                picked.append(r)
        for r in picked:
            out.append({
                "id": f"accent-{len(out) + 1:04d}",
                "file": r["file"], "row": r["row"], "speaker": r["speaker"], "accent": code,
                "edacc_accent": label, "l1": r["l1"], "gender": r["gender"], "text": r["text"],
            })
    return out


def pick_longform(rng: random.Random, pc: dict, trans: dict) -> list[dict]:
    by_chap = defaultdict(list)
    for utt in trans:
        by_chap[tuple(utt.split("-")[:2])].append(utt)
    chapters = sorted(by_chap)
    rng.shuffle(chapters)
    out, used_spk = [], set()
    for target in LONGFORM_TARGETS:
        for chap in chapters:
            if chap[0] in used_spk or chap in LONGFORM_SKIP:
                continue
            run, total = [], 0.0
            for u in sorted(by_chap[chap]):
                if u not in pc or words(pc[u]["text"]) != words(trans[u]):
                    run, total = [], 0.0  # need punctuated text for every utterance in the run
                    continue
                run.append(u)
                total += pc[u]["duration"] + 0.35
                if total >= target:
                    break
            if target <= total <= 360:
                used_spk.add(chap[0])
                out.append({"id": f"longform-{len(out) + 1:04d}", "split": "test-other", "utts": run})
                break
    return out


def main() -> None:
    rng = random.Random(SEED)
    pc_clean, tr_clean = S.librispeech_pc("test-clean"), S.librispeech_transcripts("test-clean")
    pc_other, tr_other = S.librispeech_pc("test-other"), S.librispeech_transcripts("test-other")
    clean = pick_clean(rng, pc_clean, tr_clean)
    sel = {
        "seed": SEED,
        "clean": clean,
        "noise": pick_noise(rng, clean, pc_clean),
        "accent": pick_accent(rng),
        "longform_librispeech": pick_longform(rng, pc_other, tr_other),
    }
    (ROOT / "sources").mkdir(exist_ok=True)
    (ROOT / "sources" / "selection.json").write_text(json.dumps(sel, indent=1) + "\n")
    for k, v in sel.items():
        if isinstance(v, list):
            print(k, len(v))


if __name__ == "__main__":
    main()
