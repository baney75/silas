//! Settings timings on this computer, with no window. Run with
//! `SILAS_BENCH_DIR=<a folder on disk> cargo test --lib settings_bench -- --ignored --nocapture`. It lists the microphones (no sound is
//! recorded), keeps its data in a temp folder and never loads a model or starts the key listener.

use std::sync::Arc;
use std::time::{Duration, Instant};

use polycarp_voice::core::{Core, NullHost};
use polycarp_voice::store::{self, HistoryItem, Settings};
use polycarp_voice::{audio, benchmark, models, permissions};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn spread(mut v: Vec<Duration>) -> String {
    v.sort();
    let at = |q: f64| ms(v[((v.len() - 1) as f64 * q).round() as usize]);
    format!("median {:.2} ms, p90 {:.2} ms, max {:.2} ms (n={})", at(0.5), at(0.9), at(1.0), v.len())
}

#[test]
#[ignore]
fn settings_bench() {
    // SILAS_BENCH_DIR puts the data on the disk the real app uses; the temp folder may be in memory.
    let dir = std::env::var_os("SILAS_BENCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("silas-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    store::set_data_dir(Some(dir.clone()));
    let history: Vec<HistoryItem> = (0..store::HISTORY_KEEP)
        .map(|i| HistoryItem {
            id: i.to_string(),
            ts: "2026-11-13T12:00:00Z".into(),
            text: "A sentence of about the usual length, said into the microphone and typed out.".into(),
            raw: "a sentence of about the usual length said into the microphone and typed out".into(),
            duration_s: 4.0,
            latency_ms: 140,
            words: 14,
            corrected: None,
        })
        .collect();
    store::save("history.json", &history);
    store::save("settings.json", &Settings { onboarded: true, ..Settings::default() });

    // Cold start without a model: the core, then the reads the window's first paint waits on.
    let t = Instant::now();
    let core = Core::new(Arc::new(NullHost));
    let core_new = t.elapsed();
    let active = core.settings.lock().model.clone();
    let mut reads = Vec::new();
    let mut read = |name: &'static str, f: &dyn Fn()| {
        let t = Instant::now();
        f();
        reads.push((name, t.elapsed()));
    };
    read("permissions", &|| drop(permissions::check()));
    read("device", &|| drop(benchmark::device()));
    read("models", &|| drop(models::list(Some(&active))));
    read("history", &|| drop(serde_json::to_string(&core.history.lock().iter().take(200).collect::<Vec<_>>())));
    read("dictionary", &|| drop(serde_json::to_string(&*core.dict.lock())));
    read("benchmark", &|| drop(benchmark::load_report()));
    let boot_reads: Duration = reads.iter().map(|r| r.1).sum();
    let each: Vec<String> = reads.iter().map(|(n, d)| format!("{n} {:.2}", ms(*d))).collect();

    // Opening Settings: the microphone list it asked for.
    let mics: Vec<Duration> = (0..6)
        .map(|_| {
            let t = Instant::now();
            let _ = audio::microphones();
            t.elapsed()
        })
        .collect();

    // A change to its saved state: apply it, then read the file back until it holds the change.
    let saves: Vec<Duration> = (0..50)
        .map(|i| {
            let mut s = core.settings.lock().clone();
            s.sounds = i % 2 == 0;
            let t = Instant::now();
            core.apply_settings(s).expect("saves");
            let back: Settings = serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
            assert_eq!(back.sounds, i % 2 == 0);
            t.elapsed()
        })
        .collect();

    store::set_data_dir(None);
    let _ = std::fs::remove_dir_all(&dir);
    println!("settings_bench: core start {:.2} ms, first-paint reads {:.2} ms ({})", ms(core_new), ms(boot_reads), each.join(", "));
    println!("settings_bench: list microphones first {:.2} ms, then {} ({} found)", ms(mics[0]), spread(mics[1..].to_vec()), audio::microphones().len());
    println!("settings_bench: change to saved {}", spread(saves));
}
