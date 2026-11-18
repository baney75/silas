//! Scores Silas on a speech test set through the dictation path (bench/HARNESS.md).
//!
//! Usage: silas-bench run --manifest <file> --model <id> --out <results.jsonl> [--set <dir>]
//!                        [--subset <ids.txt>] [--fast] [--dict <dictionary.json>] [--keep-fillers]
//! Replays each clip of the manifest through the dictation path, pieces cut at pauses and all,
//! and writes one JSON line per clip. Real time by default; `--fast` keeps the model's time real
//! and simulates the waiting. Models live under $SILAS_BENCH_DATA (default
//! ~/.cache/silas-bench/silas-data) and are downloaded if missing. Lives here rather than in
//! src-tauri so it builds without a UI toolkit.
//!
//!        silas-bench type --results <results.jsonl> --out <typing.jsonl> [--speed <setting>]
//! Types each row's `hypothesis` with the app's typer (`typer::type_text`) at a typing-speed setting
//! (default: the app's default, "fast") and writes how long each took. Run it on a display that
//! takes synthetic keys (Xvfb on Linux); it types into whatever has focus.
//! Build: cargo build --release --features bench --bin silas-bench

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

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
    use polycarp_voice::{benchmark, dictionary::Dictionary, models, replay, store};
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
        let (before, energy_before, load1) = (replay::usage(), replay::energy_joules(), replay::load1());
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
            "load1": load1,
            "pieces": d.pieces,
            "paced": !a.fast,
        });
        writeln!(out, "{row}")?;
        out.flush()?;
        eprintln!(
            "{}/{} {id}: {:.1}s audio, final {:.0} ms after release, {} piece(s), load {}",
            i + 1,
            clips.len(),
            seconds,
            d.after_release.as_secs_f64() * 1000.0,
            d.pieces,
            load1.map_or("n/a".into(), |l| format!("{l:.1}"))
        );
    }
    Ok(())
}

/// Times the typer on each row's text: key events from the first to the last, as a dictation types
/// them once its text is final.
fn type_rows(args: &[String]) -> anyhow::Result<()> {
    let mut it = args.iter();
    let (mut results, mut out, mut speed) = (None, None, "fast".to_string());
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| anyhow::anyhow!("{flag} needs a value"));
        match flag.as_str() {
            "--results" => results = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--speed" => speed = value()?,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let results = results.ok_or_else(|| anyhow::anyhow!("--results is required"))?;
    let out = out.ok_or_else(|| anyhow::anyhow!("--out is required"))?;
    let pace = polycarp_voice::typer::Pace::from_setting(&speed);
    let mut f = std::io::BufWriter::new(std::fs::File::create(&out)?);
    for line in std::fs::read_to_string(&results)?.lines().filter(|l| !l.trim().is_empty()) {
        let row: serde_json::Value = serde_json::from_str(line)?;
        let text = row["hypothesis"].as_str().unwrap_or_default();
        let t = Instant::now();
        let typed = polycarp_voice::typer::type_text(text, pace, false, &|_| true)?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        anyhow::ensure!(typed, "the typer stopped early");
        writeln!(
            f,
            "{}",
            serde_json::json!({"id": row["id"], "chars": text.chars().count(), "type_ms": ms, "speed": speed})
        )?;
        println!("{}: {} chars in {ms:.0} ms", row["id"].as_str().unwrap_or("?"), text.chars().count());
    }
    f.flush()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => run(&args[2..]),
        Some("type") => type_rows(&args[2..]),
        _ => anyhow::bail!(
            "usage: silas-bench run --manifest <file> --model <id> --out <results.jsonl> [options]\n       silas-bench type --results <results.jsonl> --out <typing.jsonl> [--speed <setting>]"
        ),
    }
}
