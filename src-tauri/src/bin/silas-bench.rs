//! Benchmark the speech engines Silas can ship on real recordings.
//!
//! Usage: silas-bench <parakeet|cohere> <model-dir> <wav>...
//! Set SILAS_DICT=<dictionary.json> to also print the text after the dictionary.
//! Prints load time, then per-file seconds of audio, seconds to transcribe and the text.

use std::path::PathBuf;
use std::time::Instant;

use transcribe_rs::onnx::cohere::{CohereModel, CohereParams};
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::onnx::Quantization;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        anyhow::bail!("usage: silas-bench <parakeet|cohere> <model-dir> <wav>...");
    }
    let engine = args[1].as_str();
    let dir = PathBuf::from(&args[2]);
    let wavs = &args[3..];

    let t = Instant::now();
    let mut run: Box<dyn FnMut(&[f32]) -> anyhow::Result<String>> = match engine {
        "parakeet" => {
            let mut m = ParakeetModel::load(&dir, &Quantization::Int8)?;
            Box::new(move |s| Ok(m.transcribe_with(s, &ParakeetParams::default())?.text))
        }
        "cohere" => {
            let mut m = CohereModel::load(&dir, &Quantization::Int4)?;
            Box::new(move |s| {
                let p = CohereParams {
                    language: Some("en".into()),
                    ..Default::default()
                };
                Ok(m.transcribe_with(s, &p)?.text)
            })
        }
        other => anyhow::bail!("unknown engine {other}"),
    };
    println!("load {:.2}s", t.elapsed().as_secs_f32());

    let mut dict: Option<silas_lib::dictionary::Dictionary> = std::env::var("SILAS_DICT")
        .ok()
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).expect("read dict")).expect("parse dict"));
    let mut audio_total = 0.0;
    let mut work_total = 0.0;
    for w in wavs {
        let samples = transcribe_rs::audio::read_wav_samples(&PathBuf::from(w))?;
        let secs = samples.len() as f32 / 16000.0;
        let t = Instant::now();
        let text = run(&samples)?;
        let took = t.elapsed().as_secs_f32();
        let text = match &mut dict {
            Some(d) => format!("{}\n  => {}", text.trim(), d.apply(&text, true)),
            None => text,
        };
        audio_total += secs;
        work_total += took;
        println!(
            "--- {:.1}s audio in {:.2}s  {}\n{}",
            secs,
            took,
            w.rsplit('/').next().unwrap_or(w),
            text.trim()
        );
    }
    println!(
        "TOTAL {:.1}s audio in {:.2}s ({:.0}x realtime)",
        audio_total,
        work_total,
        audio_total / work_total.max(0.001)
    );
    Ok(())
}
