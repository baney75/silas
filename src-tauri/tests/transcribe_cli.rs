//! `silas transcribe` on real speech: each bench clip, and all of them joined into one long file
//! with pauses between, which goes through in pieces. It needs Parakeet v3 (about 470 MB, downloaded
//! once into the cache), so it only runs when asked:
//!
//!     cargo test --release --test transcribe_cli -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::process::Command;

use polycarp_voice::{benchmark, models, store};

const MODEL: &str = "parakeet-v3";
/// Parakeet v3 scores about 2% on these clips through the dictation path.
const MAX_WER: f32 = 0.08;

fn data_dir() -> PathBuf {
    dirs::cache_dir().expect("a cache folder").join("silas-bench/silas-data")
}

fn bench_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/voice/bench")
}

fn silas(args: &[&str]) -> std::process::Output {
    let data = data_dir();
    let out = Command::new(env!("CARGO_BIN_EXE_silas"))
        .arg("transcribe")
        .args(args)
        .args(["--model", MODEL, "--data-dir"])
        .arg(&data)
        .output()
        .expect("silas runs");
    assert!(
        out.status.success(),
        "silas transcribe {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// The bench clips, each followed by 0.8 s of silence, as one 16 kHz WAV.
fn joined(path: &Path, clips: &[benchmark::Clip]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for clip in clips {
        for &s in clip.samples.iter().chain(std::iter::repeat_n(&0.0f32, 12_800)) {
            w.write_sample((s * 32767.0) as i16).unwrap();
        }
    }
    w.finalize().unwrap();
}

#[test]
#[ignore = "downloads a 470 MB speech model; run with --ignored"]
fn transcribes_the_bench_clips_within_a_small_error_rate() {
    store::set_data_dir(Some(data_dir()));
    if !models::installed(MODEL) {
        eprintln!("downloading {MODEL} into {}", data_dir().display());
        models::download(MODEL, &|_, _, _| {}, &|| false).expect("model download");
    }
    let manifest = benchmark::manifest().unwrap();
    let (mut errors, mut words) = (0.0f32, 0usize);
    for clip in &manifest.clips {
        let file = bench_dir().join(&clip.file);
        let text = String::from_utf8(silas(&[file.to_str().unwrap()]).stdout).unwrap();
        let n = benchmark::normalize(&clip.text).split(' ').count();
        let wer = benchmark::wer(&clip.text, &text);
        eprintln!("{} {:.3} {}", clip.file, wer, text.trim());
        errors += wer * n as f32;
        words += n;
    }
    let pooled = errors / words as f32;
    eprintln!("pooled WER over {} clips: {pooled:.3}", manifest.clips.len());
    assert!(pooled <= MAX_WER, "pooled WER {pooled:.3} is over {MAX_WER}");

    // All of them in one file: longer than a piece, so it is cut at the pauses.
    let clips = benchmark::load_clips().unwrap();
    let long = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bench-joined.wav");
    joined(&long, &clips);
    let out = silas(&[long.to_str().unwrap(), "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("--json prints JSON");
    let reference = clips.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" ");
    let wer = benchmark::wer(&reference, json["text"].as_str().unwrap());
    let seconds = json["audioSeconds"].as_f64().unwrap();
    let pieces = json["pieces"].as_u64().unwrap();
    eprintln!("joined: {seconds:.1} s in {pieces} pieces, WER {wer:.3}, timings {}", json["timings"]);
    assert!(seconds > 60.0, "the joined file is {seconds} s");
    assert!(pieces >= 3, "{seconds} s should take at least 3 pieces of at most 30 s, took {pieces}");
    assert!(wer <= MAX_WER, "WER {wer:.3} on the joined file is over {MAX_WER}");
    assert_eq!(json["model"], MODEL);
    for key in ["raw", "file"] {
        assert!(json[key].is_string(), "--json has no {key}");
    }
    for key in ["loadMs", "decodeMs", "transcribeMs", "totalMs"] {
        assert!(json["timings"][key].is_u64(), "--json timings have no {key}");
    }

    // --raw prints the model's own words, which score the same here (the dictionary is empty).
    let raw = String::from_utf8(silas(&[bench_dir().join(&manifest.clips[0].file).to_str().unwrap(), "--raw"]).stdout).unwrap();
    assert!(benchmark::wer(&manifest.clips[0].text, &raw) <= MAX_WER, "--raw: {raw}");
}

#[test]
fn a_file_that_is_not_media_fails_with_a_plain_message() {
    let out = Command::new(env!("CARGO_BIN_EXE_silas"))
        .args(["transcribe", file!()])
        .output()
        .expect("silas runs");
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Silas can't read that file"), "{err}");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_usage_mistake_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_silas")).args(["transcribe"]).output().expect("silas runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage: silas transcribe"));
}
