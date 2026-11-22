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

/// The bundled speech set's definition: its id, clip count and total seconds. `registry/ingest.py`
/// and `registry/service` read the same file. Results from different sets are not comparable, so a
/// new set gets a new id there.
const SUITE_JSON: &str = include_str!("../../../registry/suite.json");

#[derive(Debug, Deserialize)]
pub struct Suite {
    pub id: String,
    pub clips: u32,
    pub seconds: f64,
    /// "The wait after you stop talking" is the median compute time of clips this long.
    pub wait_clip_min_seconds: f32,
    pub wait_clip_max_seconds: f32,
    /// Earlier sets, oldest first. A report saved before reports named their set was made on the first.
    pub legacy_ids: Vec<String>,
}

pub fn suite() -> &'static Suite {
    static SUITE: std::sync::OnceLock<Suite> = std::sync::OnceLock::new();
    SUITE.get_or_init(|| serde_json::from_str(SUITE_JSON).expect("registry/suite.json is valid"))
}

/// Names the bundled speech set (`suite().id`).
pub fn suite_id() -> &'static str {
    &suite().id
}

pub const REGISTRY_URL: &str = "https://raw.githubusercontent.com/baney75/silas/main/registry/devices.json";
/// The anonymous report service (`registry/service`): one-tap sharing and the community results.
pub const SERVICE_URL: &str = "https://silas-registry.magnus-b37.workers.dev";
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
    pub id: String,
    pub category: String,
    /// The reference transcript.
    pub text: String,
    /// Another accepted reference: the spoken form of code and numbers ("dash m" for `-m`), or an
    /// accented clip without its fillers. A clip is scored on whichever reference fits better.
    #[serde(default)]
    pub alt: Option<String>,
    pub seconds: f32,
    pub source: String,
    pub license: String,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub about: String,
    pub clips: Vec<ManifestClip>,
}

const MANIFEST: &str = include_str!("../bench/manifest.json");

macro_rules! wav {
    ($name:literal) => {
        ($name, include_bytes!(concat!("../bench/", $name)) as &[u8])
    };
}

const WAVS: &[(&str, &[u8])] = &[
    wav!("clean-0053.wav"),
    wav!("clean-0059.wav"),
    wav!("noise-0044.wav"),
    wav!("noise-0054.wav"),
    wav!("noise-0058.wav"),
    wav!("accent-0026.wav"),
    wav!("accent-0036.wav"),
    wav!("accent-0041.wav"),
    wav!("accent-0042.wav"),
    wav!("dictation-0019.wav"),
    wav!("dictation-0021.wav"),
    wav!("command-0005.wav"),
    wav!("code-0001.wav"),
    wav!("code-0003.wav"),
];

pub fn manifest() -> anyhow::Result<Manifest> {
    Ok(serde_json::from_str(MANIFEST)?)
}

pub struct Clip {
    pub text: String,
    pub alt: Option<String>,
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
            Ok(Clip { text: c.text.clone(), alt: c.alt.clone(), seconds: c.seconds, samples: decode_wav(bytes)? })
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

/// Errors and reference words of a clip, on whichever of its references the hypothesis fits better
/// (the lower rate; the main reference on a tie).
fn clip_errors(clip: &Clip, hypothesis: &str) -> (usize, usize) {
    let main = word_errors(&clip.text, hypothesis);
    let rate = |(e, w): (usize, usize)| if w == 0 { f64::from(u8::from(e > 0)) } else { e as f64 / w as f64 };
    match &clip.alt {
        Some(alt) => {
            let other = word_errors(alt, hypothesis);
            if rate(other) < rate(main) {
                other
            } else {
                main
            }
        }
        None => main,
    }
}

/// The median; the mean of the middle two for an even count. `None` when empty.
pub fn median(values: &[f32]) -> Option<f32> {
    let mut v: Vec<f32> = values.to_vec();
    v.sort_by(f32::total_cmp);
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]) / 2.0),
    }
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
    /// The wait after you stop talking: the median compute seconds of the set's 5 to 10 s clips.
    #[serde(default)]
    pub wait_seconds: f32,
    /// Compute seconds of each clip, in the set's order. Kept on this computer; never shared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip_seconds: Vec<f32>,
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
    let (mut clip_seconds, mut waits) = (vec![], vec![]);
    let (lo, hi) = (suite().wait_clip_min_seconds, suite().wait_clip_max_seconds);
    for (i, clip) in clips.iter().enumerate() {
        progress(i, total);
        let (text, took) = transcribe(&clip.samples)?;
        let took = took.as_secs_f64();
        let length = clip.samples.len() as f64 / 16_000.0;
        compute += took;
        audio += length;
        clip_seconds.push(took as f32);
        if (f64::from(lo)..=f64::from(hi)).contains(&length) {
            waits.push(took as f32);
        }
        let (e, w) = clip_errors(clip, &text);
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
        // Every set has clips of that length (a test checks); the whole set is the fallback.
        wait_seconds: median(&waits).or_else(|| median(&clip_seconds)).unwrap_or(0.0),
        clip_seconds,
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

/// A report saved before reports named their set was made on the first one.
fn default_suite() -> String {
    suite().legacy_ids.first().cloned().unwrap_or_default()
}

impl Report {
    pub fn new(device: Device, results: Vec<ModelResult>) -> Self {
        Report {
            schema: 1,
            silas: env!("CARGO_PKG_VERSION").into(),
            suite: suite_id().into(),
            device,
            pick: pick(&results),
            results,
            ran_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// What leaves this computer when the user shares: the report without the per-clip timings.
    /// No audio and no transcript is ever in a report.
    pub fn shared(&self) -> Report {
        let mut r = self.clone();
        for x in &mut r.results {
            x.clip_seconds.clear();
        }
        r
    }

    /// The exact text sent: shown to the user first, then posted unchanged.
    pub fn share_payload(&self) -> String {
        serde_json::to_string_pretty(&self.shared()).unwrap_or_default()
    }
}

/// The report if it is a real one measured on the computer with this device key, on the current
/// speech set. A data folder carried over from another computer says nothing about this one, and
/// numbers from an older set aren't comparable: with no report, the benchmark runs again.
fn report_for(r: Report, key: &str) -> Option<Report> {
    (!r.ran_at.is_empty() && r.device.key == key && r.suite == suite_id()).then_some(r)
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
    let body = r.share_payload();
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
    /// Median wait after you stop talking, in seconds.
    #[serde(default)]
    pub wait: f64,
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
/// with neither, an empty registry. Call `mark_registry_tried` once the lookups are done.
pub fn fetch_registry() -> Registry {
    fetch_registry_inner()
}

/// Marks this run's device lookup (registry, then community) as tried when dropped, so
/// `wait_for_registry` never waits out its timeout on a path that returned early.
pub struct LookupDone;

impl Drop for LookupDone {
    fn drop(&mut self) {
        mark_registry_tried();
    }
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

/// The model this device is known to run well: one that clears both bars. Trust order: this
/// computer's own measurement; else the maintainer-approved registry; else the community results,
/// only when at least `MIN_AGREE` reports agree.
pub fn best_for_device() -> Option<String> {
    best_for_device_from(load_report().as_ref())
}

/// Same, for a report the caller already loaded.
pub fn best_for_device_from(report: Option<&Report>) -> Option<String> {
    trusted_best_for_device_from(report)
        .or_else(|| report.is_none().then(|| cached_community().lookup(&device().key)).flatten())
}

/// The pick that may choose a download: this computer's own measurement, else the
/// maintainer-approved registry. Never the community results, which anyone can post.
pub fn trusted_best_for_device_from(report: Option<&Report>) -> Option<String> {
    match report {
        Some(r) => r
            .pick
            .clone()
            .filter(|p| r.results.iter().any(|x| &x.model == p && passes(x)) && models::spec(p).is_some()),
        None => cached_registry().lookup(&device().key).map(|e| e.model.clone()),
    }
}

// ---------------------------------------------------------------- community results and sharing

/// A community pick switches models only when at least this many reports for the device key
/// picked the same model. Fewer are shown as information.
pub const MIN_AGREE: u32 = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CommunityModel {
    pub reports: u32,
    /// Medians over the reports for this device key.
    pub speed: f64,
    pub wer: f64,
    pub wait: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CommunityDevice {
    pub reports: u32,
    /// The pick rule over the medians, as the service computed it.
    pub pick: Option<String>,
    /// Reports whose own pick is `pick`.
    pub agree: u32,
    pub models: HashMap<String, CommunityModel>,
}

/// The service's `GET /v1/devices` answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Community {
    pub schema: u32,
    pub suite: String,
    pub devices: HashMap<String, CommunityDevice>,
}

pub const COMMUNITY_FILE: &str = "community.json";

impl Community {
    /// Parse an answer. Results from another speech set aren't comparable, so they are dropped.
    pub fn parse(text: &str) -> anyhow::Result<Community> {
        let mut c: Community = serde_json::from_str(text)?;
        anyhow::ensure!(c.schema == 1, "unsupported community schema {}", c.schema);
        if c.suite != suite_id() {
            c.devices.clear();
        }
        Ok(c)
    }

    /// The model to use on this device key, if the community settles it: the pick rule recomputed
    /// here over the medians agrees with the service's, at least `MIN_AGREE` reports picked it,
    /// this build offers it and its medians clear both bars.
    pub fn lookup(&self, key: &str) -> Option<String> {
        let d = self.devices.get(key)?;
        let meds: Vec<ModelResult> = d
            .models
            .iter()
            .map(|(m, x)| ModelResult { model: m.clone(), speed: x.speed as f32, wer: x.wer as f32, ..Default::default() })
            .collect();
        let pick = d.pick.clone().filter(|p| pick(&meds).as_ref() == Some(p))?;
        let m = d.models.get(&pick)?;
        (d.agree >= MIN_AGREE && models::spec(&pick).is_some() && passes_bars(m.speed, m.wer)).then_some(pick)
    }
}

/// This device's community results from the service (3 s timeout), cached on success. Offline,
/// the cached copy; with neither, nothing.
pub fn fetch_community(key: &str) -> Community {
    let fetched = (|| -> anyhow::Result<Community> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(3)))
            .build()
            .into();
        let url = format!("{SERVICE_URL}/v1/devices?key={}", utf8_percent(key));
        let text = agent.get(&url).call()?.body_mut().read_to_string()?;
        Community::parse(&text)
    })();
    match fetched {
        Ok(c) => {
            store::save(COMMUNITY_FILE, &c);
            c
        }
        Err(e) => {
            log::info!("community results unavailable ({e}); using the cached copy");
            cached_community()
        }
    }
}

pub fn cached_community() -> Community {
    if !store::exists(COMMUNITY_FILE) {
        return Community::default();
    }
    let c: Community = store::load(COMMUNITY_FILE);
    Community::parse(&serde_json::to_string(&c).unwrap_or_default()).unwrap_or_default()
}

/// The service's answer to a shared report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ShareReply {
    pub key: String,
    pub reports: u32,
    pub pick: Option<String>,
    pub agree: u32,
}

/// Post `payload` (from `Report::share_payload`) to the service. No account, no cookies; the
/// service stores the validated numbers and the day, never the address it came from.
pub fn send_report(payload: &str) -> anyhow::Result<ShareReply> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .post(&format!("{SERVICE_URL}/v1/reports"))
        .header("Content-Type", "application/json")
        .send(payload)?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    share_outcome(status, &text)
}

/// Turn the service's status and body into the reply or a sentence for the user.
fn share_outcome(status: u16, body: &str) -> anyhow::Result<ShareReply> {
    #[derive(Deserialize)]
    struct Problem {
        error: Option<String>,
    }
    match status {
        200 | 201 => Ok(serde_json::from_str(body)?),
        413 => anyhow::bail!("The report is too large to share."),
        429 => anyhow::bail!("Too many reports have been shared today. Try again tomorrow."),
        400..=499 => {
            let why = serde_json::from_str::<Problem>(body).ok().and_then(|p| p.error);
            anyhow::bail!("The service couldn't use this report. {}", why.unwrap_or_default())
        }
        _ => anyhow::bail!("The report service isn't available right now (HTTP {status}). Try again later, or share on GitHub."),
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
        assert_eq!(rep.suite, suite_id());
        assert_ne!(suite_id(), "librispeech-test-clean-9clips-v1");
        let mut v: serde_json::Value = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["suite"], suite_id());
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

    /// Review sol-F1 and second-F7 on #64: a report saved on the v1 set was still trusted, so the
    /// v2 benchmark never ran, the v1 pick stayed recommended and sharing offered a report the
    /// service rejects as stale.
    #[test]
    fn a_saved_report_from_an_older_set_counts_as_absent() {
        let _store = crate::store::TEST_DIR.lock();
        let dir = std::env::temp_dir().join(format!("pc-v1-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::store::set_data_dir(Some(dir.clone()));
        let current = Report::new(device().clone(), vec![r("parakeet-v3", 20.0, 0.03)]);
        // Saved before reports named their set, so it reads as v1.
        let mut v1 = serde_json::to_value(&current).unwrap();
        v1.as_object_mut().unwrap().remove("suite");
        std::fs::write(dir.join(REPORT_FILE), v1.to_string()).unwrap();
        let old = load_report();
        let old_best = best_for_device_from(old.as_ref());
        save_report(&current);
        let now = load_report();
        crate::store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert!(old.is_none(), "a v1 report was trusted: {old:?}");
        assert_eq!(old_best, None);
        assert_eq!(now.as_ref().map(|r| r.suite.as_str()), Some(suite_id()));
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
        assert!(!m.clips.is_empty() && !m.about.is_empty());
        assert_eq!(m.clips.len(), WAVS.len());
        for c in &m.clips {
            assert!(!c.license.is_empty() && !c.source.is_empty() && !c.category.is_empty());
            assert_eq!(c.file, format!("{}.wav", c.id));
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
        assert!((f64::from(res.audio_seconds) - suite().seconds).abs() < 0.1);
        assert_eq!(calls, texts.len() + 1);
        // Only the returned compute time counts: every clip at 100 ms, warm-up excluded.
        let want = 0.1 * texts.len() as f32;
        assert!((res.compute_seconds - want).abs() < 1e-3);
        assert!((res.speed - res.audio_seconds / want).abs() < 0.1);
        assert_eq!(res.clip_seconds.len(), texts.len());
        assert!((res.wait_seconds - 0.1).abs() < 1e-6);
    }

    #[test]
    fn wait_is_the_median_of_the_five_to_ten_second_clips() {
        let clips = load_clips().unwrap();
        let mut i = 0usize;
        // Compute time = 1/100 of the clip's length plus a step per clip, so each clip differs.
        let lengths: Vec<f32> = clips.iter().map(|c| c.samples.len() as f32 / 16_000.0).collect();
        let mut transcribe = |s: &[f32]| -> anyhow::Result<(String, Duration)> {
            let secs = s.len() as f32 / 16_000.0 / 100.0 + i as f32 * 0.001;
            i += 1;
            Ok((String::new(), Duration::from_secs_f32(secs)))
        };
        let res = run_suite("m", &mut transcribe, &|_, _| {}).unwrap();
        let waits: Vec<f32> = res
            .clip_seconds
            .iter()
            .zip(&lengths)
            .filter(|(_, l)| (5.0..=10.0).contains(*l))
            .map(|(t, _)| *t)
            .collect();
        assert!(waits.len() >= 5);
        assert_eq!(res.wait_seconds, median(&waits).unwrap());
        assert_eq!(res.wer, 1.0);
    }

    #[test]
    fn suite_json_agrees_with_the_bundled_set() {
        // registry/suite.json is what ingest.py and the service check reports against.
        let s = suite();
        let clips = load_clips().unwrap();
        assert_eq!(clips.len() as u32, s.clips);
        let total: f64 = clips.iter().map(|c| c.samples.len() as f64 / 16_000.0).sum();
        assert!((total - s.seconds).abs() < 0.05, "{total} vs {}", s.seconds);
        let manifest_total: f64 = manifest().unwrap().clips.iter().map(|c| f64::from(c.seconds)).sum();
        assert!((manifest_total - s.seconds).abs() < 0.05);
        assert!(s.legacy_ids.iter().all(|l| l != &s.id));
        assert_eq!(s.legacy_ids.first().map(String::as_str), Some("librispeech-test-clean-9clips-v1"));
        let size: usize = WAVS.iter().map(|(_, b)| b.len()).sum();
        assert!(size <= 3 << 20, "the bundled set is {size} bytes");
    }

    #[test]
    fn models_json_lists_the_catalog() {
        // registry/models.json is the list ingest.py and the service accept; it follows models.rs.
        #[derive(Deserialize)]
        struct Known {
            models: Vec<String>,
        }
        let known: Known = serde_json::from_str(include_str!("../../../registry/models.json")).unwrap();
        let mut known = known.models;
        known.sort();
        let mut catalog: Vec<String> = models::MODELS.iter().map(|m| m.id.to_string()).collect();
        catalog.sort();
        assert_eq!(known, catalog, "update registry/models.json to match models::MODELS");
    }

    #[test]
    fn a_clip_scores_on_its_better_reference() {
        let clip = |text: &str, alt: Option<&str>| Clip { text: text.into(), alt: alt.map(Into::into), seconds: 1.0, samples: vec![] };
        let code = clip("git commit -m \"fix login redirect\"", Some("git commit dash m quote fix login redirect quote"));
        assert_eq!(clip_errors(&code, "git commit -m \"fix login redirect\""), (0, 6));
        assert_eq!(clip_errors(&code, "git commit dash m quote fix login redirect quote"), (0, 9));
        assert_eq!(clip_errors(&code, "get commit dash m quote fix login redirect quote"), (1, 9));
        assert_eq!(clip_errors(&clip("the cat", None), "a cat"), (1, 2));
        // A tie keeps the main reference.
        assert_eq!(clip_errors(&clip("a b", Some("a b")), "a c"), (1, 2));
    }

    #[test]
    fn median_matches_python() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn shared_report_leaves_out_clip_timings() {
        let mut rep = Report::new(build_device("macos", "Apple M5", "x", 10, 32, None), vec![r("parakeet-v3", 20.0, 0.03)]);
        rep.results[0].clip_seconds = vec![0.1, 0.2];
        rep.results[0].wait_seconds = 0.4;
        let payload = rep.share_payload();
        assert!(!payload.contains("clip_seconds"));
        assert!(payload.contains("\"wait_seconds\": 0.4"));
        assert!(serde_json::to_string(&rep).unwrap().contains("clip_seconds"));
        let back: Report = serde_json::from_str(&payload).unwrap();
        assert_eq!(back.results[0].clip_seconds, Vec::<f32>::new());
        assert_eq!(back.device, rep.device);
    }

    fn community(pick: &str, agree: u32, models: &[(&str, f64, f64)]) -> Community {
        let mut devices = HashMap::new();
        devices.insert(
            "macos/apple-m5/10".to_string(),
            CommunityDevice {
                reports: agree.max(1),
                pick: Some(pick.into()),
                agree,
                models: models
                    .iter()
                    .map(|(m, speed, wer)| (m.to_string(), CommunityModel { reports: agree, speed: *speed, wer: *wer, wait: 0.4 }))
                    .collect(),
            },
        );
        Community { schema: 1, suite: suite_id().into(), devices }
    }

    #[test]
    fn community_pick_needs_three_agreeing_reports_and_both_bars() {
        let two = [("parakeet-v3", 20.0, 0.06), ("cohere-int4", 4.0, 0.05)];
        let key = "macos/apple-m5/10";
        assert_eq!(community("parakeet-v3", 3, &two).lookup(key).as_deref(), Some("parakeet-v3"));
        // Fewer than three agree: information only.
        assert_eq!(community("parakeet-v3", 2, &two).lookup(key), None);
        // The service's pick must be what the rule gives over the medians.
        assert_eq!(community("cohere-int4", 5, &two).lookup(key), None);
        // A pick that misses a bar never switches.
        assert_eq!(community("parakeet-v3", 5, &[("parakeet-v3", 9.0, 0.06)]).lookup(key), None);
        assert_eq!(community("parakeet-v3", 5, &[("parakeet-v3", 20.0, 0.09)]).lookup(key), None);
        // A model this build doesn't offer.
        assert_eq!(community("future-model", 5, &[("future-model", 30.0, 0.01)]).lookup(key), None);
        assert_eq!(community("parakeet-v3", 3, &two).lookup("linux/other/4"), None);
    }

    #[test]
    fn community_from_another_set_is_dropped() {
        let mut c = community("parakeet-v3", 3, &[("parakeet-v3", 20.0, 0.06)]);
        c.suite = "librispeech-test-clean-9clips-v1".into();
        let parsed = Community::parse(&serde_json::to_string(&c).unwrap()).unwrap();
        assert!(parsed.devices.is_empty());
        let json = r#"{"schema":1,"suite":"SUITE","min_agree":3,"devices":{"macos/apple-m5/10":{"reports":3,"pick":"parakeet-v3","agree":3,
            "models":{"parakeet-v3":{"reports":3,"speed":20.0,"wer":0.06,"wait":0.4}}}}}"#
            .replace("SUITE", suite_id());
        assert_eq!(Community::parse(&json).unwrap().lookup("macos/apple-m5/10").as_deref(), Some("parakeet-v3"));
        assert!(Community::parse(r#"{"schema":2,"suite":"x","devices":{}}"#).is_err());
    }

    #[test]
    fn share_outcomes_read_as_sentences() {
        let ok = share_outcome(201, r#"{"ok":true,"key":"macos/apple-m5/10","reports":2,"pick":"parakeet-v3","agree":2}"#).unwrap();
        assert_eq!((ok.key.as_str(), ok.reports, ok.agree), ("macos/apple-m5/10", 2, 2));
        let e = share_outcome(422, r#"{"error":"Unknown operating system.","stale":false}"#).unwrap_err().to_string();
        assert!(e.ends_with("Unknown operating system."), "{e}");
        assert!(share_outcome(429, "").unwrap_err().to_string().contains("tomorrow"));
        assert!(share_outcome(413, "").is_err());
        assert!(share_outcome(503, "").unwrap_err().to_string().contains("GitHub"));
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
