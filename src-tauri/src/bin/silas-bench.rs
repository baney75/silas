//! Benchmark the speech engines Silas can ship on real recordings.
//!
//! Usage: silas-bench <parakeet|cohere> <model-dir> <wav>...
//!        silas-bench suite [model-id...]
//!        silas-bench run --manifest <file> --model <id> --out <results.jsonl> [--set <dir>]
//!                        [--subset <ids.txt>] [--fast] [--dict <dictionary.json>] [--keep-fillers]
//! `suite` runs the bundled speech set (the same one the app uses to pick a model for each
//! computer) through the installed models, default all of them, and prints the Report JSON
//! and the link that would share it. Progress goes to stderr.
//! `run` replays each clip of a speech test set (bench/README.md) through the dictation path,
//! pieces cut at pauses and all, and writes one JSON line per clip. Real time by default;
//! `--fast` keeps the model's time real and simulates the waiting. Models live under
//! $SILAS_BENCH_DATA (default ~/.cache/silas-bench/silas-data) and are downloaded if missing.
//! Set SILAS_DICT=<dictionary.json> to also print the text after the dictionary.
//! Prints load time, then per-file seconds of audio, seconds to transcribe and the text.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use transcribe_rs::onnx::cohere::{CohereModel, CohereParams};
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::onnx::Quantization;

fn suite(ids: &[String]) -> anyhow::Result<()> {
    use silas_lib::{benchmark, models};
    let ids: Vec<String> = if ids.is_empty() {
        models::MODELS.iter().filter(|m| models::installed(m.id)).map(|m| m.id.to_string()).collect()
    } else {
        ids.to_vec()
    };
    anyhow::ensure!(!ids.is_empty(), "no model is installed");
    let mut results = vec![];
    for id in &ids {
        anyhow::ensure!(models::spec(id).is_some(), "unknown model {id}");
        let t = Instant::now();
        let mut engine = models::Engine::load(id)?;
        eprintln!("{id}: loaded in {:.2}s", t.elapsed().as_secs_f32());
        let r = benchmark::run_suite(
            id,
            &mut |s: &[f32]| {
                let t = Instant::now();
                let text = engine.transcribe(s, "en")?;
                Ok((text, t.elapsed()))
            },
            &|done, total| eprintln!("{id}: {done}/{total}"),
        )?;
        eprintln!(
            "{id}: {:.1}x real time, {:.1}% word errors, {}",
            r.speed,
            r.wer * 100.0,
            if benchmark::passes(&r) { "passes both bars" } else { "misses a bar" }
        );
        results.push(r);
    }
    let report = benchmark::Report::new(benchmark::detect(), results);
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("report_url: {}", benchmark::report_url(&report));
    Ok(())
}

struct RunArgs {
    manifest: PathBuf,
    set: Option<PathBuf>,
    subset: Option<PathBuf>,
    model: String,
    out: PathBuf,
    fast: bool,
    dict: Option<PathBuf>,
    keep_fillers: bool,
}

fn run_args(args: &[String]) -> anyhow::Result<RunArgs> {
    let mut it = args.iter();
    let (mut manifest, mut set, mut subset, mut model, mut out, mut dict) = (None, None, None, None, None, None);
    let (mut fast, mut keep_fillers) = (false, false);
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| anyhow::anyhow!("{a} needs a value"));
        match a.as_str() {
            "--manifest" => manifest = Some(PathBuf::from(value()?)),
            "--set" => set = Some(PathBuf::from(value()?)),
            "--subset" => subset = Some(PathBuf::from(value()?)),
            "--model" => model = Some(value()?),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--dict" => dict = Some(PathBuf::from(value()?)),
            "--fast" => fast = true,
            "--keep-fillers" => keep_fillers = true,
            other => anyhow::bail!("unknown option {other}"),
        }
    }
    Ok(RunArgs {
        manifest: manifest.ok_or_else(|| anyhow::anyhow!("--manifest is required"))?,
        set,
        subset,
        model: model.ok_or_else(|| anyhow::anyhow!("--model is required"))?,
        out: out.ok_or_else(|| anyhow::anyhow!("--out is required"))?,
        fast,
        dict,
        keep_fillers,
    })
}

/// The manifest's clips, in its order, kept to the ids in `subset` if one is given.
fn clips(manifest: &Path, subset: Option<&Path>) -> anyhow::Result<Vec<serde_json::Value>> {
    let keep: Option<std::collections::HashSet<String>> = match subset {
        Some(p) => Some(
            std::fs::read_to_string(p)?
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect(),
        ),
        None => None,
    };
    let mut out = vec![];
    for (n, line) in std::fs::read_to_string(manifest)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let clip: serde_json::Value =
            serde_json::from_str(line).map_err(|e| anyhow::anyhow!("{}:{}: {e}", manifest.display(), n + 1))?;
        let id = clip["id"].as_str().ok_or_else(|| anyhow::anyhow!("{}:{}: no id", manifest.display(), n + 1))?;
        if keep.as_ref().is_none_or(|k| k.contains(id)) {
            out.push(clip);
        }
    }
    anyhow::ensure!(!out.is_empty(), "no clips to run");
    Ok(out)
}

fn run(args: &[String]) -> anyhow::Result<()> {
    use silas_lib::{benchmark, dictionary::Dictionary, models, replay, store};
    let a = run_args(args)?;
    let data = std::env::var_os("SILAS_BENCH_DATA")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("silas-bench/silas-data")))
        .ok_or_else(|| anyhow::anyhow!("set SILAS_BENCH_DATA"))?;
    store::set_data_dir(Some(data));
    anyhow::ensure!(models::spec(&a.model).is_some(), "unknown model {}", a.model);
    if !models::installed(&a.model) {
        eprintln!("{}: downloading", a.model);
        models::download(&a.model, &|_, _, _| {}, &|| false)?;
    }
    let set = a.set.clone().unwrap_or_else(|| a.manifest.parent().unwrap_or(Path::new(".")).to_path_buf());
    let clips = clips(&a.manifest, a.subset.as_deref())?;
    let dict: Dictionary = match &a.dict {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
        None => Dictionary::default(),
    };
    let remove_fillers = !a.keep_fillers;

    let t = Instant::now();
    let mut engine = models::Engine::load(&a.model)?;
    let load_ms = t.elapsed().as_millis() as u64;
    // The app warms up the same way, so its first dictation is as fast as the rest.
    let _ = engine.transcribe(&vec![0.0; 16_000], "en");
    eprintln!("{}: loaded in {load_ms} ms", a.model);
    let model: replay::Model = Box::new(move |s: &[f32]| engine.transcribe(s, "en"));
    let model = Arc::new(parking_lot::Mutex::new(model));

    let mut out = std::fs::File::create(&a.out)?;
    for (i, clip) in clips.iter().enumerate() {
        let id = clip["id"].as_str().unwrap_or_default();
        let audio_path = set.join(clip["audio"].as_str().ok_or_else(|| anyhow::anyhow!("{id}: no audio"))?);
        let samples = std::fs::read(&audio_path)
            .map_err(anyhow::Error::from)
            .and_then(|b| benchmark::decode_wav(&b))
            .map_err(|e| anyhow::anyhow!("{}: {e}", audio_path.display()))?;
        let seconds = samples.len() as f64 / 16_000.0;
        let finish = |raw: &str| dict.clone().apply(raw, remove_fillers);
        let (before, energy_before) = (replay::usage(), replay::energy_joules());
        let d = if a.fast {
            replay::simulated(&samples, &mut model.lock(), finish)?
        } else {
            replay::live(samples, &model, finish)?
        };
        let after = replay::usage();
        let energy = match (energy_before, replay::energy_joules()) {
            (Some(b), Some(e)) if e >= b => Some(e - b),
            _ => None,
        };
        let row = serde_json::json!({
            "id": id,
            "engine": "silas",
            "model": a.model,
            "category": clip["category"],
            "hypothesis": d.text.trim(),
            "hypothesis_raw": d.raw,
            "t_first_ms": d.first.as_secs_f64() * 1000.0,
            "t_final_ms": d.after_release.as_secs_f64() * 1000.0,
            "audio_seconds": seconds,
            "compute_seconds": d.compute.as_secs_f64(),
            "rtf": d.compute.as_secs_f64() / seconds.max(1e-6),
            "peak_rss_mb": after.as_ref().map(|u| u.peak_rss_mb),
            "cpu_seconds": before.zip(after).map(|(b, e)| (e.cpu - b.cpu).as_secs_f64()),
            "energy_j": energy,
            "load_ms": load_ms,
            "pieces": d.pieces,
            "paced": !a.fast,
        });
        writeln!(out, "{row}")?;
        out.flush()?;
        eprintln!(
            "{}/{} {id}: {:.1}s audio, final {:.0} ms after release, {} piece(s)",
            i + 1,
            clips.len(),
            seconds,
            d.after_release.as_secs_f64() * 1000.0,
            d.pieces
        );
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("suite") {
        return suite(&args[2..]);
    }
    if args.get(1).map(String::as_str) == Some("run") {
        return run(&args[2..]);
    }
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
