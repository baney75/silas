//! Which speech model suits this computer: device identity, the bundled benchmark, the
//! pick rule and the shared device registry.
//!
//! The bars, the pick rule and the text normalization are mirrored in `registry/ingest.py`.
//! Change them in both places or not at all.

use std::collections::HashMap;
use std::io::Cursor;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::models;
use crate::store;

/// Seconds of audio per second of compute: 10x real time.
pub const MIN_SPEED: f64 = 10.0;
/// Word error rate.
pub const MAX_WER: f64 = 0.08;
/// Two passing models closer than this in WER count as tied; the faster one wins.
pub const TIE_WER: f32 = 0.005;

/// Names the bundled speech set. Results from different sets are not comparable, so a new
/// set gets a new id.
pub const SUITE_ID: &str = "librispeech-test-clean-9clips-v1";

pub const REGISTRY_URL: &str = "https://raw.githubusercontent.com/baney75/silas/main/registry/devices.json";
const ISSUE_URL: &str = "https://github.com/baney75/silas/issues/new";

// ---------------------------------------------------------------- device

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub key: String,
    pub os: String,
    pub os_version: String,
    pub cpu: String,
    pub cores: u32,
    pub memory_gb: u32,
    pub hw_model: Option<String>,
}

/// Lowercase a-z and 0-9; everything else (non-ASCII included) becomes `-`; runs collapse
/// and the ends are trimmed. `registry/ingest.py` computes the same.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    out
}

pub fn device_key(os: &str, cpu: &str, cores: u32) -> String {
    format!("{}/{}/{}", slug(os), slug(cpu), cores)
}

/// Text that goes into a shared report: ASCII letters, digits and ` ()@.,_+/:-` only, other
/// characters become spaces, spaces collapse, at most 120 characters, "unknown" if nothing is left.
pub fn sanitize_text(s: &str) -> String {
    let kept: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || " ()@.,_+/:-".contains(c) { c } else { ' ' })
        .collect();
    let mut out = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    out.truncate(120);
    let out = out.trim().to_string();
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}

/// The hardware model as the registry accepts it: `[A-Za-z0-9,._ -]`, 1 to 40 characters.
fn clean_hw_model(s: Option<String>) -> Option<String> {
    s.filter(|m| {
        (1..=40).contains(&m.len()) && m.chars().all(|c| c.is_ascii_alphanumeric() || ",._ -".contains(c))
    })
}

/// Assemble the device from raw readings. The key comes from the sanitized CPU name.
pub fn build_device(os: &str, cpu: &str, os_version: &str, cores: u32, memory_gb: u32, hw_model: Option<String>) -> Device {
    let cpu = sanitize_text(cpu);
    Device {
        key: device_key(os, &cpu, cores),
        os: os.into(),
        os_version: sanitize_text(os_version),
        cpu,
        cores,
        memory_gb,
        hw_model: clean_hw_model(hw_model),
    }
}

fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

#[cfg(target_os = "macos")]
fn hardware_model() -> Option<String> {
    let out = std::process::Command::new("sysctl").args(["-n", "hw.model"]).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

#[cfg(not(target_os = "macos"))]
fn hardware_model() -> Option<String> {
    None
}

/// Identify this computer. Reads the CPU list, so call it off the UI path.
pub fn detect() -> Device {
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let cpu = sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default();
    let cores = if sys.cpus().is_empty() {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        sys.cpus().len()
    } as u32;
    let os = os_name();
    build_device(
        os,
        &cpu,
        &System::long_os_version().unwrap_or_else(|| os.into()),
        cores,
        (sys.total_memory() as f64 / (1u64 << 30) as f64).round() as u32,
        hardware_model(),
    )
}

/// The device, detected once per run of the app.
pub fn device() -> &'static Device {
    static DEVICE: std::sync::OnceLock<Device> = std::sync::OnceLock::new();
    DEVICE.get_or_init(detect)
}

// ---------------------------------------------------------------- bundled speech set

#[derive(Debug, Deserialize)]
pub struct ManifestClip {
    pub file: String,
    pub text: String,
    pub seconds: f32,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub source: String,
    pub license: String,
    pub clips: Vec<ManifestClip>,
}

const MANIFEST: &str = include_str!("../bench/manifest.json");

const WAVS: &[(&str, &[u8])] = &[
    ("6930-75918-0002.wav", include_bytes!("../bench/6930-75918-0002.wav") as &[u8]),
    ("7729-102255-0039.wav", include_bytes!("../bench/7729-102255-0039.wav") as &[u8]),
    ("8463-294828-0027.wav", include_bytes!("../bench/8463-294828-0027.wav") as &[u8]),
    ("5142-33396-0024.wav", include_bytes!("../bench/5142-33396-0024.wav") as &[u8]),
    ("4446-2275-0007.wav", include_bytes!("../bench/4446-2275-0007.wav") as &[u8]),
    ("4507-16021-0033.wav", include_bytes!("../bench/4507-16021-0033.wav") as &[u8]),
    ("1188-133604-0032.wav", include_bytes!("../bench/1188-133604-0032.wav") as &[u8]),
    ("4970-29093-0007.wav", include_bytes!("../bench/4970-29093-0007.wav") as &[u8]),
    ("237-134493-0015.wav", include_bytes!("../bench/237-134493-0015.wav") as &[u8]),
];

pub fn manifest() -> anyhow::Result<Manifest> {
    Ok(serde_json::from_str(MANIFEST)?)
}

pub struct Clip {
    pub text: String,
    pub seconds: f32,
    pub samples: Vec<f32>,
}

/// Decode a 16 kHz mono 16-bit WAV into f32 samples in [-1, 1].
pub fn decode_wav(bytes: &[u8]) -> anyhow::Result<Vec<f32>> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes))?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.sample_rate == 16_000 && spec.channels == 1 && spec.bits_per_sample == 16 && spec.sample_format == hound::SampleFormat::Int,
        "benchmark clip is not 16 kHz mono 16-bit"
    );
    reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.0).map_err(Into::into))
        .collect()
}

pub fn load_clips() -> anyhow::Result<Vec<Clip>> {
    let m = manifest()?;
    let clips = m
        .clips
        .iter()
        .map(|c| {
            let bytes = WAVS
                .iter()
                .find(|(f, _)| *f == c.file)
                .map(|(_, b)| *b)
                .ok_or_else(|| anyhow::anyhow!("clip {} is not bundled", c.file))?;
            Ok(Clip { text: c.text.clone(), seconds: c.seconds, samples: decode_wav(bytes)? })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    anyhow::ensure!(!clips.is_empty(), "the benchmark set is empty");
    Ok(clips)
}

// ---------------------------------------------------------------- scoring

/// Lowercase; `-` and `—` become spaces; keep letters, digits, apostrophes and whitespace;
/// collapse whitespace; trim.
pub fn normalize(s: &str) -> String {
    let kept: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c == '-' || c == '—' { ' ' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '\'' || c.is_whitespace())
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Word-level Levenshtein distance (substitution, insertion and deletion each cost 1)
/// and the number of reference words, after normalizing both sides.
fn word_errors(reference: &str, hypothesis: &str) -> (usize, usize) {
    let (r, h) = (normalize(reference), normalize(hypothesis));
    let r: Vec<&str> = r.split(' ').filter(|w| !w.is_empty()).collect();
    let h: Vec<&str> = h.split(' ').filter(|w| !w.is_empty()).collect();
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, hw) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(rw != hw);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    (prev[h.len()], r.len())
}

/// Word error rate of one hypothesis against one reference.
pub fn wer(reference: &str, hypothesis: &str) -> f32 {
    let (errors, words) = word_errors(reference, hypothesis);
    // Capped at 1: a model that repeats itself can make more errors than there are words.
    if words == 0 {
        return if errors == 0 { 0.0 } else { 1.0 };
    }
    (errors as f32 / words as f32).min(1.0)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelResult {
    pub model: String,
    /// Seconds of audio per second of compute.
    pub speed: f32,
    pub wer: f32,
    pub audio_seconds: f32,
    pub compute_seconds: f32,
    pub clips: u32,
}

/// Fast enough and accurate enough: both bars cleared.
pub fn passes_bars(speed: f64, wer: f64) -> bool {
    speed >= MIN_SPEED && wer <= MAX_WER
}

pub fn passes(r: &ModelResult) -> bool {
    passes_bars(f64::from(r.speed), f64::from(r.wer))
}

/// The model this device should use. Fast and accurate enough wins on accuracy (a tie
/// within `TIE_WER` goes to the faster); else the fastest accurate one; else the fastest.
/// Exactly equal speeds go to the lower model id, in every branch.
pub fn pick(results: &[ModelResult]) -> Option<String> {
    let fastest = |rs: &[&ModelResult]| {
        rs.iter()
            .copied()
            .max_by(|a, b| a.speed.total_cmp(&b.speed).then_with(|| b.model.cmp(&a.model)))
            .map(|r| r.model.clone())
    };
    let passing: Vec<&ModelResult> = results.iter().filter(|r| passes(r)).collect();
    if !passing.is_empty() {
        let best = passing.iter().map(|r| r.wer).fold(f32::INFINITY, f32::min);
        let close: Vec<&ModelResult> = passing.iter().copied().filter(|r| r.wer - best <= TIE_WER).collect();
        return fastest(&close);
    }
    let accurate: Vec<&ModelResult> = results.iter().filter(|r| f64::from(r.wer) <= MAX_WER).collect();
    if !accurate.is_empty() {
        return fastest(&accurate);
    }
    fastest(&results.iter().collect::<Vec<_>>())
}

/// A transcript and the compute time it took.
pub type Timed = anyhow::Result<(String, Duration)>;

/// Run the bundled set through `transcribe`, which returns the text and its own compute time
/// (so waits for a lock or a quiet moment stay out of the timing). The first clip runs once
/// as a warm-up and is not counted. `progress(done, total)` is called before each clip starts
/// and once more when the set is finished.
pub fn run_suite(
    model_id: &str,
    transcribe: &mut dyn FnMut(&[f32]) -> Timed,
    progress: &dyn Fn(usize, usize),
) -> anyhow::Result<ModelResult> {
    let clips = load_clips()?;
    let total = clips.len();
    progress(0, total);
    transcribe(&clips[0].samples)?;
    let (mut audio, mut compute) = (0f64, 0f64);
    let (mut errors, mut words) = (0usize, 0usize);
    for (i, clip) in clips.iter().enumerate() {
        progress(i, total);
        let (text, took) = transcribe(&clip.samples)?;
        compute += took.as_secs_f64();
        audio += clip.samples.len() as f64 / 16_000.0;
        let (e, w) = word_errors(&clip.text, &text);
        errors += e;
        words += w;
    }
    progress(total, total);
    Ok(ModelResult {
        model: model_id.into(),
        speed: (audio / compute.max(1e-6)) as f32,
        wer: if words == 0 { 1.0 } else { (errors as f32 / words as f32).min(1.0) },
        audio_seconds: audio as f32,
        compute_seconds: compute as f32,
        clips: total as u32,
    })
}

// ---------------------------------------------------------------- report

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub schema: u32,
    pub silas: String,
    /// Which speech set produced the numbers. Reports saved before this field are v1.
    #[serde(default = "default_suite")]
    pub suite: String,
    pub device: Device,
    pub results: Vec<ModelResult>,
    pub pick: Option<String>,
    pub ran_at: String,
}

pub const REPORT_FILE: &str = "benchmark.json";

fn default_suite() -> String {
    SUITE_ID.into()
}

impl Report {
    pub fn new(device: Device, results: Vec<ModelResult>) -> Self {
        Report {
            schema: 1,
            silas: env!("CARGO_PKG_VERSION").into(),
            suite: SUITE_ID.into(),
            device,
            pick: pick(&results),
            results,
            ran_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// The report if it is a real one measured on the computer with this device key. A data
/// folder carried over from another computer says nothing about this one.
fn report_for(r: Report, key: &str) -> Option<Report> {
    (!r.ran_at.is_empty() && r.device.key == key).then_some(r)
}

pub fn load_report() -> Option<Report> {
    if !store::exists(REPORT_FILE) {
        return None;
    }
    report_for(store::load(REPORT_FILE), &device().key)
}

pub fn save_report(r: &Report) {
    store::save(REPORT_FILE, r);
}

fn utf8_percent(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The prefilled GitHub issue the user reviews and submits.
pub fn report_url(r: &Report) -> String {
    let title = format!("Device report: {} ({})", r.device.cpu, r.device.os);
    let body = serde_json::to_string_pretty(r).unwrap_or_default();
    format!(
        "{ISSUE_URL}?template=device-report.yml&title={}&report={}",
        utf8_percent(&title),
        utf8_percent(&body)
    )
}

/// Open a link in the default browser.
pub fn open_url(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // Not cmd.exe: it would expand % and split on &. The URL is one argument.
        let mut c = std::process::Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn()?;
    Ok(())
}

// ---------------------------------------------------------------- registry

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub model: String,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub wer: f64,
    #[serde(default)]
    pub reports: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub schema: u32,
    pub devices: HashMap<String, Entry>,
}

pub const REGISTRY_FILE: &str = "registry.json";

impl Registry {
    /// Parse the published file. Entries naming a model this build doesn't offer are dropped.
    pub fn parse(text: &str) -> anyhow::Result<Registry> {
        let mut r: Registry = serde_json::from_str(text)?;
        anyhow::ensure!(r.schema == 1, "unsupported registry schema {}", r.schema);
        r.devices.retain(|_, e| models::spec(&e.model).is_some());
        Ok(r)
    }

    pub fn lookup(&self, key: &str) -> Option<&Entry> {
        self.devices
            .get(key)
            .filter(|e| models::spec(&e.model).is_some() && passes_bars(e.speed, e.wer))
    }
}

static REGISTRY_TRIED: (parking_lot::Mutex<bool>, parking_lot::Condvar) =
    (parking_lot::Mutex::new(false), parking_lot::Condvar::new());

/// Wait (at most `timeout`) until this run has asked GitHub for the registry once, so
/// first-run setup downloads the model the registry picks rather than the static default.
pub fn wait_for_registry(timeout: Duration) {
    let (done, cv) = &REGISTRY_TRIED;
    let mut d = done.lock();
    if !*d {
        let _ = cv.wait_for(&mut d, timeout);
    }
}

fn mark_registry_tried() {
    let (done, cv) = &REGISTRY_TRIED;
    *done.lock() = true;
    cv.notify_all();
}

/// The registry from GitHub (5 s timeout), cached on success. Offline, the cached copy;
/// with neither, an empty registry.
pub fn fetch_registry() -> Registry {
    let r = fetch_registry_inner();
    mark_registry_tried();
    r
}

fn fetch_registry_inner() -> Registry {
    let fetched = (|| -> anyhow::Result<Registry> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        let text = agent.get(REGISTRY_URL).call()?.body_mut().read_to_string()?;
        Registry::parse(&text)
    })();
    match fetched {
        Ok(r) => {
            store::save(REGISTRY_FILE, &r);
            r
        }
        Err(e) => {
            log::info!("device registry unavailable ({e}); using the cached copy");
            cached_registry()
        }
    }
}

pub fn cached_registry() -> Registry {
    if !store::exists(REGISTRY_FILE) {
        return Registry::default();
    }
    let r: Registry = store::load(REGISTRY_FILE);
    Registry::parse(&serde_json::to_string(&r).unwrap_or_default()).unwrap_or_default()
}

/// The model this device is known to run well: one that clears both bars, by this computer's
/// own measurement if it has one, else the registry's entry for this device type.
pub fn best_for_device() -> Option<String> {
    best_for_device_from(load_report().as_ref())
}

/// Same, for a report the caller already loaded.
pub fn best_for_device_from(report: Option<&Report>) -> Option<String> {
    match report {
        Some(r) => r
            .pick
            .clone()
            .filter(|p| r.results.iter().any(|x| &x.model == p && passes(x)) && models::spec(p).is_some()),
        None => cached_registry().lookup(&device().key).map(|e| e.model.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(model: &str, speed: f32, wer: f32) -> ModelResult {
        ModelResult { model: model.into(), speed, wer, ..Default::default() }
    }

    #[test]
    fn normalize_follows_the_spec() {
        assert_eq!(normalize("  Hello,   World!  "), "hello world");
        assert_eq!(normalize("Well-known — and don't"), "well known and don't");
        assert_eq!(normalize("It cost $5.50 (really)"), "it cost 550 really");
        assert_eq!(normalize("A\tB\nC"), "a b c");
        assert_eq!(normalize("...!?"), "");
    }

    #[test]
    fn wer_identical_is_zero() {
        assert_eq!(wer("the cat sat", "The cat, sat."), 0.0);
    }

    #[test]
    fn wer_counts_substitution_insertion_deletion() {
        assert!((wer("the cat sat", "the dog sat") - 1.0 / 3.0).abs() < 1e-6);
        assert!((wer("the cat sat", "the big cat sat") - 1.0 / 3.0).abs() < 1e-6);
        assert!((wer("the cat sat", "the sat") - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn wer_empty_hypothesis_is_total_loss() {
        assert_eq!(wer("the cat sat", ""), 1.0);
        assert_eq!(wer("", ""), 0.0);
    }

    #[test]
    fn pick_prefers_passing_then_accuracy() {
        let rs = [r("fast", 40.0, 0.07), r("best", 12.0, 0.03), r("slow", 2.0, 0.01)];
        assert_eq!(pick(&rs).as_deref(), Some("best"));
    }

    #[test]
    fn pick_tie_within_half_a_point_goes_to_the_faster() {
        let rs = [r("a", 12.0, 0.030), r("b", 30.0, 0.034)];
        assert_eq!(pick(&rs).as_deref(), Some("b"));
        let rs = [r("a", 12.0, 0.030), r("b", 30.0, 0.040)];
        assert_eq!(pick(&rs).as_deref(), Some("a"));
    }

    #[test]
    fn pick_falls_back_to_fastest_accurate_then_fastest() {
        // Nothing is fast enough: the fastest of the accurate ones.
        let rs = [r("a", 4.0, 0.02), r("b", 6.0, 0.05), r("c", 9.0, 0.30)];
        assert_eq!(pick(&rs).as_deref(), Some("b"));
        // Nothing is accurate enough either: the fastest overall.
        let rs = [r("a", 4.0, 0.20), r("b", 6.0, 0.15)];
        assert_eq!(pick(&rs).as_deref(), Some("b"));
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn passes_uses_both_bars_inclusively() {
        assert!(passes(&r("a", 10.0, 0.08)));
        assert!(!passes(&r("a", 9.9, 0.01)));
        assert!(!passes(&r("a", 50.0, 0.081)));
    }

    #[test]
    fn device_key_slugs() {
        assert_eq!(device_key("macos", "Apple M5", 10), "macos/apple-m5/10");
        assert_eq!(
            device_key("windows", "Intel(R) Core(TM) i7-12700H @ 2.30GHz", 20),
            "windows/intel-r-core-tm-i7-12700h-2-30ghz/20"
        );
        assert_eq!(slug("  --Weird__Name!!  "), "weird-name");
    }

    #[test]
    fn registry_parse_ignores_unknown_models() {
        let json = r#"{"schema":1,"devices":{
            "macos/apple-m5/10":{"model":"parakeet-v3","speed":23.5,"wer":0.04,"reports":3},
            "linux/mystery/4":{"model":"not-a-model","speed":9.0,"wer":0.1,"reports":1}}}"#;
        let reg = Registry::parse(json).unwrap();
        assert_eq!(reg.lookup("macos/apple-m5/10").unwrap().model, "parakeet-v3");
        assert!(reg.lookup("linux/mystery/4").is_none());
        assert_eq!(reg.devices.len(), 1);
        assert!(Registry::parse(r#"{"schema":2,"devices":{}}"#).is_err());
    }

    #[test]
    fn registry_entries_must_clear_both_bars() {
        let json = r#"{"schema":1,"devices":{
            "a/slow/1":{"model":"parakeet-v3","speed":9.9,"wer":0.01,"reports":3},
            "a/sloppy/1":{"model":"parakeet-v3","speed":30.0,"wer":0.081,"reports":3},
            "a/edge/1":{"model":"parakeet-v3","speed":10.0,"wer":0.08,"reports":3}}}"#;
        let reg = Registry::parse(json).unwrap();
        assert!(reg.lookup("a/slow/1").is_none());
        assert!(reg.lookup("a/sloppy/1").is_none());
        assert!(reg.lookup("a/edge/1").is_some());
        assert!(passes_bars(10.0, 0.08) && !passes_bars(9.99, 0.0) && !passes_bars(99.0, 0.0801));
        // Compared exactly as published, not squeezed into f32 first.
        let json = r#"{"schema":1,"devices":{
            "a/slow/1":{"model":"parakeet-v3","speed":9.9999999,"wer":0.08,"reports":1},
            "a/sloppy/1":{"model":"parakeet-v3","speed":10.0,"wer":0.0800000001,"reports":1}}}"#;
        let reg = Registry::parse(json).unwrap();
        assert!(reg.lookup("a/slow/1").is_none());
        assert!(reg.lookup("a/sloppy/1").is_none());
    }

    #[test]
    fn pick_equal_speeds_go_to_the_lower_id() {
        // Passing branch (WER tie, same speed).
        let rs = [r("b", 20.0, 0.03), r("a", 20.0, 0.03)];
        assert_eq!(pick(&rs).as_deref(), Some("a"));
        // Accurate-but-slow branch.
        let rs = [r("z", 5.0, 0.05), r("m", 5.0, 0.06)];
        assert_eq!(pick(&rs).as_deref(), Some("m"));
        // Nothing accurate: fastest overall.
        let rs = [r("y", 5.0, 0.5), r("x", 5.0, 0.4)];
        assert_eq!(pick(&rs).as_deref(), Some("x"));
    }

    #[test]
    fn reports_carry_the_suite_id_and_old_files_read_as_v1() {
        let rep = Report::new(build_device("macos", "Apple M5", "x", 10, 32, None), vec![r("parakeet-v3", 20.0, 0.03)]);
        assert_eq!(rep.suite, SUITE_ID);
        let mut v: serde_json::Value = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["suite"], SUITE_ID);
        v.as_object_mut().unwrap().remove("suite");
        let old: Report = serde_json::from_value(v).unwrap();
        assert_eq!(old.suite, "librispeech-test-clean-9clips-v1");
    }

    #[test]
    fn a_report_from_another_computer_is_ignored() {
        let rep = Report::new(build_device("macos", "Apple M5", "macOS 27", 10, 32, None), vec![r("parakeet-v3", 20.0, 0.03)]);
        assert!(report_for(rep.clone(), "macos/apple-m5/10").is_some());
        assert!(report_for(rep.clone(), "macos/apple-m4/10").is_none());
        let mut blank = rep;
        blank.ran_at.clear();
        assert!(report_for(blank, "macos/apple-m5/10").is_none());
    }

    #[test]
    fn sanitized_device_text_and_key_agree_with_the_registry() {
        let d = build_device("windows", "Intel(R) Core(TM) i7-1165G7 @ 2.80GHz", "Windows 11 (22631)", 8, 16, None);
        assert_eq!(d.key, "windows/intel-r-core-tm-i7-1165g7-2-80ghz/8");
        assert_eq!(d.cpu, "Intel(R) Core(TM) i7-1165G7 @ 2.80GHz");

        let d = build_device("windows", "Intel® Core™ Ultra 7", "Windows 11", 16, 32, None);
        assert_eq!(d.cpu, "Intel Core Ultra 7");
        assert_eq!(d.key, "windows/intel-core-ultra-7/16");
        // The key is always os/slug(cpu)/cores of what is reported.
        assert_eq!(d.key, format!("{}/{}/{}", d.os, slug(&d.cpu), d.cores));

        let d = build_device("linux", "AMD Ryzen", "Ubuntu 24.04!; rm -rf $HOME\n \"x\"", 4, 8, Some("Mac17,2; drop".into()));
        assert_eq!(d.os_version, "Ubuntu 24.04 rm -rf HOME x");
        assert!(d.os_version.chars().all(|c| c.is_ascii_alphanumeric() || " ()@.,_+/:-".contains(c)));
        assert_eq!(d.hw_model, None);

        assert_eq!(build_device("linux", "", "", 1, 1, None).cpu, "unknown");
        assert_eq!(build_device("linux", "™™", "", 1, 1, None).key, "linux/unknown/1");
        assert_eq!(sanitize_text(&"a".repeat(300)).len(), 120);
        assert_eq!(slug("İstanbul Ünï"), "stanbul-n");
        assert_eq!(build_device("macos", "Apple M5", "x", 10, 32, Some("Mac17,2".into())).hw_model.as_deref(), Some("Mac17,2"));
    }

    #[test]
    fn manifest_loads_and_every_wav_decodes_to_its_length() {
        let m = manifest().unwrap();
        assert!(!m.clips.is_empty());
        assert!(!m.source.is_empty() && !m.license.is_empty());
        for c in &m.clips {
            let bytes = WAVS.iter().find(|(f, _)| *f == c.file).unwrap_or_else(|| panic!("{} not bundled", c.file)).1;
            let samples = decode_wav(bytes).unwrap();
            let secs = samples.len() as f32 / 16_000.0;
            assert!((secs - c.seconds).abs() < 0.1, "{}: {secs} vs {}", c.file, c.seconds);
            assert!(samples.iter().all(|s| (-1.0..=1.0).contains(s)));
            assert!(!c.text.is_empty());
        }
        assert_eq!(load_clips().unwrap().len(), m.clips.len());
    }

    #[test]
    fn run_suite_scores_a_perfect_transcriber() {
        let texts: Vec<String> = manifest().unwrap().clips.iter().map(|c| c.text.clone()).collect();
        let mut calls = 0usize;
        let mut transcribe = |_: &[f32]| -> anyhow::Result<(String, Duration)> {
            // Call 0 is the warm-up; calls 1.. are the clips in order.
            let t = texts[calls.saturating_sub(1)].clone();
            calls += 1;
            Ok((t, Duration::from_millis(100)))
        };
        let res = run_suite("m", &mut transcribe, &|_, _| {}).unwrap();
        assert_eq!(res.wer, 0.0);
        assert_eq!(res.clips as usize, texts.len());
        assert!((res.audio_seconds - 62.2).abs() < 0.5);
        assert_eq!(calls, texts.len() + 1);
        // Only the returned compute time counts: 9 clips at 100 ms, warm-up excluded.
        assert!((res.compute_seconds - 0.9).abs() < 1e-3);
        assert!((res.speed - res.audio_seconds / 0.9).abs() < 0.1);
    }

    #[test]
    fn report_url_percent_encodes() {
        let mut rep = Report::new(
            Device { cpu: "Weird & Co #1 + é\nCPU".into(), os: "linux".into(), ..Default::default() },
            vec![r("parakeet-v3", 20.0, 0.03)],
        );
        rep.device.os_version = "a&b#c+d\ne é".into();
        let url = report_url(&rep);
        let (head, query) = url.split_once('?').unwrap();
        assert_eq!(head, "https://github.com/baney75/silas/issues/new");
        // Exactly three parameters: nothing in the values leaked a separator.
        assert_eq!(query.matches('&').count(), 2);
        assert_eq!(query.matches('#').count(), 0);
        assert_eq!(query.matches('+').count(), 0);
        assert!(!query.contains('\n') && !query.contains(' ') && query.is_ascii());
        assert!(query.starts_with("template=device-report.yml&title=Device%20report%3A%20Weird%20%26%20Co%20%231%20%2B%20%C3%A9%0ACPU%20%28linux%29&report="));
        let report = query.split("&report=").nth(1).unwrap();
        assert!(report.contains("%26") && report.contains("%23") && report.contains("%2B") && report.contains("%0A"));
        assert!(report.contains("%C3%A9"));
        assert!(report.contains("parakeet-v3"));
    }

    #[test]
    fn detect_finds_this_computer() {
        let d = detect();
        assert!(d.cores >= 1);
        assert!(!d.cpu.is_empty());
        assert_eq!(d.key, device_key(&d.os, &d.cpu, d.cores));
        assert!(d.cpu.len() <= 120 && d.os_version.len() <= 120);
    }
}
