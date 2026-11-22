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
//! Candidate models the app does not ship run through the same path with a prefixed --model:
//! `gguf:<file.gguf>` loads any GGUF through transcribe.cpp on the CPU (the engine the app runs Parakeet v3 on),
//! `whisper-server:<url>` posts each piece to a running whisper.cpp server, `parakeet-onnx:<dir>` loads a
//! full-precision Parakeet ONNX export through the app's transcribe-rs engine. `--threads` sets
//! transcribe.cpp's threads, for parakeet-v3 too (default: the app's, one per physical core up to 8), and `--usage-pid` adds a server process's CPU and memory to
//! each row.
//!
//!        silas-bench type --results <results.jsonl> --out <typing.jsonl> [--speed <setting>]
//! Types each row's `hypothesis` with the app's typer (`typer::type_text`) at a typing-speed setting
//! (default: the app's default, "fast") and writes how long each took. Run it on a display that
//! takes synthetic keys (Xvfb on Linux); it types into whatever has focus.
//!        silas-bench suite <model-id>...
//! Runs the app's bundled benchmark set (`benchmark::run_suite`, the one that picks a model for each
//! computer) on app models, downloading any that are missing, and prints each model's speed, word
//! errors and wait after you stop talking, then the report exactly as "Share results" would send it.
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
    threads: Option<i32>,
    usage_pid: Option<u32>,
}

fn run_args(args: &[String]) -> anyhow::Result<RunArgs> {
    let mut it = args.iter();
    let (mut manifest, mut set, mut subset, mut model, mut out, mut dict) = (None, None, None, None, None, None);
    let (mut fast, mut keep_fillers) = (false, false);
    let (mut threads, mut usage_pid) = (None, None);
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
            "--threads" => threads = Some(value()?.parse()?),
            "--usage-pid" => usage_pid = Some(value()?.parse()?),
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
        threads,
        usage_pid,
    })
}

/// A candidate model the app does not ship, or None for an app model id.
fn candidate(spec: &str, threads: i32) -> anyhow::Result<Option<polycarp_voice::replay::Model>> {
    if let Some(path) = spec.strip_prefix("gguf:") {
        return gguf(path, threads).map(Some);
    }
    if let Some(url) = spec.strip_prefix("whisper-server:") {
        return Ok(Some(whisper_server(url.trim_end_matches('/').to_string())));
    }
    if let Some(dir) = spec.strip_prefix("parakeet-onnx:") {
        return parakeet_onnx(dir).map(Some);
    }
    Ok(None)
}

/// A Parakeet ONNX export at full precision (encoder-model.onnx, decoder_joint-model.onnx, nemo128.onnx,
/// vocab.txt), through the same transcribe-rs engine the app uses at int8.
fn parakeet_onnx(dir: &str) -> anyhow::Result<polycarp_voice::replay::Model> {
    use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
    use transcribe_rs::onnx::Quantization;
    let mut model = ParakeetModel::load(Path::new(dir), &Quantization::FP32)?;
    Ok(Box::new(move |s: &[f32]| {
        Ok(model.transcribe_with(s, &ParakeetParams::default())?.text.trim().to_string())
    }))
}

fn gguf(path: &str, threads: i32) -> anyhow::Result<polycarp_voice::replay::Model> {
    use transcribe_cpp::{Backend, Model, ModelOptions, RunOptions, SessionOptions};
    let model = Model::load_with(path, &ModelOptions { backend: Backend::Cpu, ..Default::default() })?;
    // English for models that take a language; an English-only model is left to its default.
    let languages = model.capabilities()?.languages;
    let language = (languages.len() > 1 && languages.iter().any(|l| l == "en")).then(|| "en".to_string());
    let mut session = model.session_with(&SessionOptions { n_threads: threads, ..Default::default() })?;
    let options = RunOptions { language, ..Default::default() };
    eprintln!("{path}: {} {} on {}", model.arch(), model.variant(), model.backend());
    Ok(Box::new(move |s: &[f32]| Ok(session.run(s, &options)?.text.trim().to_string())))
}

/// whisper.cpp's server, as bench/engines/whisper_cpp.py runs it: one /inference call per piece.
fn whisper_server(url: String) -> polycarp_voice::replay::Model {
    Box::new(move |s: &[f32]| {
        let mut wav = std::io::Cursor::new(Vec::new());
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::new(&mut wav, spec)?;
        for &x in s {
            w.write_sample((x.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        }
        w.finalize()?;
        let b = "silasbenchboundary";
        let mut body = Vec::new();
        for (k, v) in [("response_format", "json"), ("temperature", "0")] {
            write!(body, "--{b}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")?;
        }
        write!(body, "--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"piece.wav\"\r\nContent-Type: audio/wav\r\n\r\n")?;
        body.extend_from_slice(wav.get_ref());
        write!(body, "\r\n--{b}--\r\n")?;
        let mut r = ureq::post(format!("{url}/inference"))
            .header("Content-Type", format!("multipart/form-data; boundary={b}"))
            .send(&body[..])?;
        let v: serde_json::Value = serde_json::from_str(&r.body_mut().read_to_string()?)?;
        Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
    })
}

/// CPU time and peak memory of another process (a model server), from /proc.
#[cfg(target_os = "linux")]
fn process_usage(pid: u32) -> Option<polycarp_voice::replay::Usage> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let f: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks: u64 = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let hwm_kb: f64 = status.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(polycarp_voice::replay::Usage {
        // SAFETY: sysconf only reads a constant.
        cpu: std::time::Duration::from_secs_f64(ticks as f64 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64),
        peak_rss_mb: hwm_kb / 1024.0,
    })
}

#[cfg(not(target_os = "linux"))]
fn process_usage(_: u32) -> Option<polycarp_voice::replay::Usage> {
    None
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
    store::set_data_dir(Some(data_dir()?));
    if models::move_legacy_parakeet()? {
        eprintln!("moved the old ONNX parakeet-v3 to {}", models::PARAKEET_V3_ONNX);
    }
    let loading = Instant::now();
    models::set_gguf_threads(a.threads.unwrap_or(0));
    let outside = candidate(&a.model, models::gguf_threads())?;
    let outside_load = loading.elapsed();
    anyhow::ensure!(outside.is_some() || models::spec(&a.model).is_some(), "unknown model {}", a.model);
    if outside.is_none() && !models::installed(&a.model) {
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
    let mut model: replay::Model = match outside {
        Some(m) => m,
        None => {
            let mut engine = models::Engine::load(&a.model)?;
            Box::new(move |s: &[f32]| engine.transcribe(s, "en"))
        }
    };
    let load_ms = (t.elapsed() + outside_load).as_millis() as u64;
    // The app warms up the same way, so its first dictation is as fast as the rest.
    let _ = model(&vec![0.0; 16_000]);
    eprintln!("{}: loaded in {load_ms} ms", a.model);
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
        let usage = || match a.usage_pid {
            Some(pid) => replay::usage().zip(process_usage(pid)).map(|(me, s)| replay::Usage {
                cpu: me.cpu + s.cpu,
                peak_rss_mb: s.peak_rss_mb,
            }),
            None => replay::usage(),
        };
        let (before, energy_before, load1) = (usage(), replay::energy_joules(), replay::load1());
        let d = if a.fast {
            replay::simulated(&samples, &mut model.lock(), finish)?
        } else {
            replay::live(samples, &model, finish)?
        };
        let after = usage();
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
            "threads": models::gguf_threads(),
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

fn data_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("SILAS_BENCH_DATA")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("silas-bench/silas-data")))
        .ok_or_else(|| anyhow::anyhow!("set SILAS_BENCH_DATA"))
}

fn suite(ids: &[String]) -> anyhow::Result<()> {
    use polycarp_voice::{benchmark, models, store};
    anyhow::ensure!(!ids.is_empty(), "usage: silas-bench suite <model-id>...");
    store::set_data_dir(Some(data_dir()?));
    let mut results = vec![];
    for id in ids {
        anyhow::ensure!(models::spec(id).is_some(), "unknown model {id}");
        if !models::installed(id) {
            eprintln!("{id}: downloading");
            models::download(id, &|_, _, _| {}, &|| false)?;
        }
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
            &|done, total| eprint!("\r{id}: {done}/{total}"),
        )?;
        eprintln!(
            "\n{id}: {:.1}x real time, {:.1}% word errors, {:.2}s wait after you stop talking, {}",
            r.speed,
            r.wer * 100.0,
            r.wait_seconds,
            if benchmark::passes(&r) { "passes both bars" } else { "misses a bar" }
        );
        results.push(r);
    }
    let report = benchmark::Report::new(benchmark::detect(), results);
    println!("{}", report.share_payload());
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
        Some("suite") => suite(&args[2..]),
        _ => anyhow::bail!(
            "usage: silas-bench run --manifest <file> --model <id> --out <results.jsonl> [options]\n       silas-bench type --results <results.jsonl> --out <typing.jsonl> [--speed <setting>]\n       silas-bench suite <model-id>..."
        ),
    }
}
