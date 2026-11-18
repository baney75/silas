//! The speech models Silas offers, how to download them, and how to run them.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use transcribe_rs::onnx::cohere::{CohereModel, CohereParams};
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::onnx::Quantization;

use crate::benchmark::{self, ModelResult};
use crate::store::models_dir;

pub struct Spec {
    pub id: &'static str,
    pub name: &'static str,
    pub tier: &'static str,
    pub tagline: &'static str,
    pub size_mb: u32,
    pub languages: &'static str,
    pub recommended: bool,
    pub source: Source,
}

pub enum Source {
    /// One .tar.gz holding a single top-level folder.
    Archive {
        url: &'static str,
        folder: &'static str,
        /// SHA-256 of the archive, checked before it is unpacked. None: not checked.
        sha256: Option<&'static str>,
    },
    /// Loose files under one base URL.
    Files {
        base: &'static str,
        files: &'static [File],
    },
}

pub struct File {
    pub name: &'static str,
    /// SHA-256 of the file, checked before the model counts as installed. None: not checked.
    pub sha256: Option<&'static str>,
}

/// A download that gets no new bytes for this long fails instead of hanging.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The default model's one file. `Engine::load` finds the model by this name.
const PARAKEET_V3_GGUF: &str = "parakeet-tdt-0.6b-v3-Q4_K_M.gguf";

/// The ONNX build of Parakeet v3 that installs before 0.1.7 put under `parakeet-v3`.
pub const PARAKEET_V3_ONNX: &str = "parakeet-v3-onnx";

pub const MODELS: &[Spec] = &[
    Spec {
        id: "parakeet-v3",
        name: "Parakeet v3",
        tier: "balanced",
        tagline:
            "Fast and accurate, and the best of these with accents. Transcribes a minute of speech in about 5 seconds on a laptop CPU.",
        size_mb: 463,
        languages: "English + 24 European languages",
        recommended: true,
        // Q4_K_M GGUF on transcribe.cpp: the model study's pick (bench/models/accent-2026-11.md).
        // Pinned to one revision of the public repo; the hash is the Hub's Git LFS object id, and
        // bench/models/accent-2026-11.candidates.json records the same one.
        source: Source::Files {
            base: "https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf/resolve/d8d8c32cb2017c57e5307767343e97870f8a7f53",
            files: &[File {
                name: PARAKEET_V3_GGUF,
                sha256: Some("b68557be1e3c40207fd7c4bd9d63f1d3316b963f15325bfb0cc16a8bb0ffd181"),
            }],
        },
    },
    Spec {
        id: PARAKEET_V3_ONNX,
        name: "Parakeet v3 (ONNX)",
        tier: "balanced",
        tagline: "The same model on the older engine. Slower to load and less accurate with accents; for computers that can't run Parakeet v3.",
        size_mb: 478,
        languages: "English + 24 European languages",
        recommended: false,
        source: Source::Archive {
            url: "https://github.com/baney75/silas/releases/download/models-v1/parakeet-v3-int8.tar.gz",
            folder: "parakeet-tdt-0.6b-v3-int8",
            // Handy's archive (its model.rs before onnx was removed, cjpais/Handy 5197faf^), matched
            // against the file on 2026-11-10 and mirrored byte for byte to our own release on
            // 2026-11-17: Handy has moved to GGUF models, so its host may drop this file.
            sha256: Some("43d37191602727524a7d8c6da0eef11c4ba24320f5b4730f1a2497befc2efa77"),
        },
    },
    Spec {
        id: "cohere-int4",
        name: "Cohere Transcribe",
        tier: "best",
        tagline: "Cleanest punctuation, but 5x slower. Best for short notes on a fast machine.",
        size_mb: 2000,
        languages: "English + 13 languages",
        recommended: false,
        // Pinned to one revision, so the files can't change under their hashes. The hashes are
        // the Hub's own: Git LFS object ids for the four model files; tokens.txt isn't in LFS, so
        // its SHA-256 is of the copy whose git blob id matched the Hub's (35dd2895).
        source: Source::Files {
            base: "https://huggingface.co/cstr/cohere-transcribe-onnx-int4/resolve/2c870f89692c61348481884eb6e1222ec3ba25c9",
            files: &[
                File {
                    name: "cohere-decoder.int4.onnx",
                    sha256: Some("f176a71e8b743d32bb8e4b8ec19bf70f367c5c3c93b08857826d8f5b60b6ad1e"),
                },
                File {
                    name: "cohere-decoder.int4.onnx.data",
                    sha256: Some("ed9461dd2233c44cd86f56c0a4d17c089de317bdbdcf18dc92e44a8a0cf525b3"),
                },
                File {
                    name: "cohere-encoder.int4.onnx",
                    sha256: Some("1bd619df7113a27d75b136ddc9f35d6b65131baeec15a7680ddda0c18c0b20a5"),
                },
                File {
                    name: "cohere-encoder.int4.onnx.data",
                    sha256: Some("f5bffddbf657edff4b14e650926cdb39b62a8644feb9f6ee4e3287df38e7f900"),
                },
                File {
                    name: "tokens.txt",
                    sha256: Some("013ede043ae2480e3a9205cc34550d9686100cc682bacc90f702facdfbb93035"),
                },
            ],
        },
    },
];

pub fn spec(id: &str) -> Option<&'static Spec> {
    MODELS.iter().find(|m| m.id == id)
}

/// The GGUF build keeps `parakeet-v3` as its id but lives in `parakeet-v3-gguf`: a Silas before 0.1.7
/// reads `parakeet-v3/.complete` as its ONNX build, so after a downgrade it would load the GGUF file as
/// ONNX. With the folder apart, it sees no model and downloads its own.
pub fn model_path(id: &str) -> PathBuf {
    models_dir().join(if id == "parakeet-v3" { "parakeet-v3-gguf" } else { id })
}

/// Written into the ONNX build's folder when its files came from an older Silas, so only those users'
/// settings and benchmark report are retargeted; `retarget_legacy_parakeet` removes it when done.
const MIGRATED: &str = ".migrated";

/// Downloaded and checked. A model of loose files also needs each file in place, so the ONNX files an
/// older Silas left under `parakeet-v3` never count as the GGUF model.
pub fn installed(id: &str) -> bool {
    let dir = model_path(id);
    dir.join(".complete").exists()
        && match spec(id).map(|m| &m.source) {
            Some(Source::Files { files, .. }) => files.iter().all(|f| dir.join(f.name).exists()),
            _ => true,
        }
}

/// Whether this computer can run the model. The GGUF engine is built for a CPU floor
/// (`.cargo/config.toml`): on x86 that is AVX2, FMA and F16C (macOS x86: AVX and F16C). Loading it
/// anywhere below that would crash on an illegal instruction instead of failing.
pub fn runs_here(id: &str) -> bool {
    id != "parakeet-v3" || cpu_runs_gguf()
}

#[cfg(all(any(target_arch = "x86", target_arch = "x86_64"), target_os = "macos"))]
fn cpu_runs_gguf() -> bool {
    is_x86_feature_detected!("sse4.2") && is_x86_feature_detected!("avx") && is_x86_feature_detected!("f16c")
}

#[cfg(all(any(target_arch = "x86", target_arch = "x86_64"), not(target_os = "macos")))]
fn cpu_runs_gguf() -> bool {
    ["sse4.2", "avx", "avx2", "fma", "f16c", "bmi2"].iter().all(|f| match *f {
        "sse4.2" => is_x86_feature_detected!("sse4.2"),
        "avx" => is_x86_feature_detected!("avx"),
        "avx2" => is_x86_feature_detected!("avx2"),
        "fma" => is_x86_feature_detected!("fma"),
        "f16c" => is_x86_feature_detected!("f16c"),
        _ => is_x86_feature_detected!("bmi2"),
    })
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn cpu_runs_gguf() -> bool {
    true
}

/// Silas before 0.1.7 kept Parakeet v3's ONNX files under `parakeet-v3`, the id that now names the
/// GGUF model. Move them to `parakeet-v3-onnx` so they keep working as the fallback. A rename on the
/// same disk: nothing is downloaded or deleted. True when it moved them. Run before any model loads.
pub fn move_legacy_parakeet() -> anyhow::Result<bool> {
    let old = models_dir().join("parakeet-v3");
    let onnx = model_path(PARAKEET_V3_ONNX);
    if !old.join(".complete").exists() || old.join(PARAKEET_V3_GGUF).exists() {
        return Ok(false);
    }
    let moved = !onnx.join(".complete").exists();
    if moved {
        let _ = fs::remove_dir_all(&onnx);
        fs::rename(&old, &onnx)?;
    } else {
        // Both copies are here (a crash between steps, or an older build run again): keep the one in place.
        fs::remove_dir_all(&old)?;
    }
    fs::write(onnx.join(MIGRATED), b"")?;
    Ok(moved)
}

/// Run at every start, after `move_legacy_parakeet`. Where it marked moved files, settings and this
/// computer's benchmark report that say `parakeet-v3` meant those files, so they now say
/// `parakeet-v3-onnx`; both are saved, then the mark goes. A start that quits midway finishes on the
/// next one. Who chose the model stays as it was. The report's pick is dropped rather than moved: it
/// was made before the GGUF build existed, and kept it would mark the ONNX build "best for this
/// computer" over it. True when settings changed.
pub fn retarget_legacy_parakeet(settings: &mut crate::store::Settings) -> bool {
    let mark = model_path(PARAKEET_V3_ONNX).join(MIGRATED);
    if !mark.exists() {
        return false;
    }
    if crate::store::exists(benchmark::REPORT_FILE) {
        let mut r: benchmark::Report = crate::store::load(benchmark::REPORT_FILE);
        if r.results.iter().any(|x| x.model == "parakeet-v3") || r.pick.as_deref() == Some("parakeet-v3") {
            for x in r.results.iter_mut().filter(|x| x.model == "parakeet-v3") {
                x.model = PARAKEET_V3_ONNX.into();
            }
            r.pick = r.pick.filter(|p| p != "parakeet-v3");
            benchmark::save_report(&r);
        }
    }
    let legacy = settings.model == "parakeet-v3";
    if legacy {
        settings.model = PARAKEET_V3_ONNX.into();
        crate::store::save("settings.json", settings);
    }
    let _ = fs::remove_file(mark);
    legacy
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub tier: String,
    pub tagline: String,
    pub size_mb: u32,
    pub languages: String,
    pub installed: bool,
    pub active: bool,
    pub recommended: bool,
    /// This computer's measured speed and accuracy for the model, if it has been run.
    pub device_result: Option<ModelResult>,
    /// The registry's or this computer's own benchmark's pick for this device.
    pub best_for_device: bool,
}

pub fn list(active: Option<&str>) -> Vec<ModelEntry> {
    let report = benchmark::load_report();
    let best = benchmark::best_for_device_from(report.as_ref());
    MODELS
        .iter()
        .map(|m| ModelEntry {
            id: m.id.into(),
            name: m.name.into(),
            tier: m.tier.into(),
            tagline: m.tagline.into(),
            size_mb: m.size_mb,
            languages: m.languages.into(),
            installed: installed(m.id),
            active: active == Some(m.id) && installed(m.id),
            // Recommend what is known to suit this computer; the static flag otherwise.
            recommended: recommends(m, best.as_deref(), &runs_here),
            device_result: report.as_ref().and_then(|r| r.results.iter().find(|x| x.model == m.id).cloned()),
            best_for_device: best.as_deref().filter(|b| runs_here(b)) == Some(m.id),
        })
        .collect()
}

/// What is known to suit this computer, a benchmark's or the registry's pick that can run here; else
/// the static flag: Parakeet v3, or its ONNX build on a CPU the GGUF engine can't run on.
fn recommends(m: &Spec, best: Option<&str>, runs: &dyn Fn(&str) -> bool) -> bool {
    let fallback = if m.id == PARAKEET_V3_ONNX { !runs("parakeet-v3") } else { m.recommended };
    runs(m.id) && best.filter(|b| runs(b)).map_or(fallback, |b| b == m.id)
}

/// Download a model into place. `progress(downloaded, total, stage)` is called often;
/// `cancelled` is asked between reads, and a yes stops the download with nothing left behind.
/// Every file with a hash is checked before `.complete` is written, so a bad copy never loads.
pub fn download(
    id: &str,
    progress: &dyn Fn(u64, u64, &str),
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<()> {
    let spec = spec(id).ok_or_else(|| anyhow::anyhow!("unknown model {id}"))?;
    if id == "parakeet-v3" {
        // ONNX files an older Silas left in this folder would be deleted below; they move aside first.
        move_legacy_parakeet().map_err(|e| {
            anyhow::anyhow!("Couldn't move the older Parakeet v3 files aside, so the new build won't download over them: {e}")
        })?;
    }
    let dest = model_path(id);
    let staging = models_dir().join(format!(".{id}.staging"));
    let part = models_dir().join(format!(".{id}.tar.gz.part"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let total = spec.size_mb as u64 * 1_048_576;
    let agent = agent();
    let run = || -> anyhow::Result<()> {
        match &spec.source {
            Source::Archive { url, folder, sha256 } => {
                fetch(&agent, url, &part, 0, total, progress, cancelled, *sha256)?;
                progress(total, total, "unpacking");
                let file = fs::File::open(&part)?;
                tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(&staging)?;
                let _ = fs::remove_file(&part);
                let inner = staging.join(folder);
                let _ = fs::remove_dir_all(&dest);
                fs::rename(&inner, &dest)?;
                let _ = fs::remove_dir_all(&staging);
            }
            Source::Files { base, files } => {
                let mut done = 0;
                for f in files.iter() {
                    done += fetch(
                        &agent,
                        &format!("{base}/{}", f.name),
                        &staging.join(f.name),
                        done,
                        total,
                        progress,
                        cancelled,
                        f.sha256,
                    )?;
                }
                let _ = fs::remove_dir_all(&dest);
                fs::rename(&staging, &dest)?;
            }
        }
        Ok(())
    };
    if let Err(e) = run() {
        // A failed or cancelled download leaves nothing half there.
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    fs::write(dest.join(".complete"), b"ok")?;
    progress(total, total, "done");
    Ok(())
}

/// An agent whose reads give up after `READ_TIMEOUT` without a byte. ureq's own body timeout is
/// one budget for the whole body, far too short for 2 GB on a slow line or too long to notice a
/// stall, so each wait for input is capped instead (`StallGuard`).
fn agent() -> ureq::Agent {
    use ureq::unversioned::resolver::DefaultResolver;
    use ureq::unversioned::transport::{Connector, DefaultConnector};
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(READ_TIMEOUT))
        .timeout_recv_response(Some(READ_TIMEOUT))
        .build();
    ureq::Agent::with_parts(
        config,
        DefaultConnector::default().chain(StallGuard(READ_TIMEOUT)),
        DefaultResolver::default(),
    )
}

/// Wraps each connection so no single wait for input lasts longer than its limit.
#[derive(Debug)]
struct StallGuard(Duration);

#[derive(Debug)]
struct Guarded<T> {
    inner: T,
    limit: Duration,
}

/// The wait ureq asked for, or `limit` if that is longer (or never ends).
fn capped(asked: ureq::unversioned::transport::NextTimeout, limit: Duration) -> ureq::unversioned::transport::NextTimeout {
    use ureq::unversioned::transport::{time, NextTimeout};
    if asked.after.is_not_happening() || *asked.after > limit {
        NextTimeout { after: time::Duration::Exact(limit), reason: ureq::Timeout::RecvBody }
    } else {
        asked
    }
}

impl<In: ureq::unversioned::transport::Transport> ureq::unversioned::transport::Connector<In> for StallGuard {
    type Out = Guarded<In>;

    fn connect(
        &self,
        _: &ureq::unversioned::transport::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| Guarded { inner, limit: self.0 }))
    }
}

impl<T: ureq::unversioned::transport::Transport> ureq::unversioned::transport::Transport for Guarded<T> {
    fn buffers(&mut self) -> &mut dyn ureq::unversioned::transport::Buffers {
        self.inner.buffers()
    }

    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<(), ureq::Error> {
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: ureq::unversioned::transport::NextTimeout) -> Result<bool, ureq::Error> {
        self.inner.await_input(capped(timeout, self.limit))
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

/// Fails when a file's SHA-256 isn't the one published for it.
fn check_hash(name: &str, got: &str, want: Option<&str>) -> anyhow::Result<()> {
    match want {
        Some(want) if !got.eq_ignore_ascii_case(want) => anyhow::bail!(
            "{name} didn't match its published checksum, so it wasn't kept. Try the download again."
        ),
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn fetch(
    agent: &ureq::Agent,
    url: &str,
    to: &Path,
    offset: u64,
    total: u64,
    progress: &dyn Fn(u64, u64, &str),
    cancelled: &dyn Fn() -> bool,
    sha256: Option<&str>,
) -> anyhow::Result<u64> {
    anyhow::ensure!(!cancelled(), crate::core::CANCELLED);
    let resp = agent.get(url).call()?;
    let len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let total = if offset == 0 { len.unwrap_or(total) } else { total };
    let mut reader = resp.into_body().into_reader();
    let mut out = fs::File::create(to)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    let mut last = 0u64;
    loop {
        anyhow::ensure!(!cancelled(), crate::core::CANCELLED);
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
        got += n as u64;
        if got - last > 1 << 20 {
            last = got;
            progress(offset + got, total, "downloading");
        }
    }
    out.flush()?;
    if let Some(len) = len {
        anyhow::ensure!(
            got == len,
            "download of {url} stopped early ({got} of {len} bytes)"
        );
    }
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    check_hash(&name, &digest, sha256)?;
    Ok(got)
}

pub fn remove(id: &str) -> anyhow::Result<()> {
    let p = model_path(id);
    if p.exists() {
        fs::remove_dir_all(p)?;
    }
    Ok(())
}

pub enum Engine {
    Gguf(transcribe_cpp::Session),
    Parakeet(ParakeetModel),
    Cohere(CohereModel),
}

static GGUF_THREADS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// `silas-bench --threads`: this process's GGUF engine uses `n` threads; 0 goes back to `gguf_threads`'s count.
pub fn set_gguf_threads(n: i32) {
    GGUF_THREADS.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// CPU threads for the GGUF engine: one per physical core this process may run on, at most 8.
/// ggml's own default counts hyperthreads, and two threads on one core only slow it down; the
/// smaller of the two counts respects a CPU affinity (taskset, a container's cpuset).
pub fn gguf_threads() -> i32 {
    let set = GGUF_THREADS.load(std::sync::atomic::Ordering::Relaxed);
    if set > 0 {
        return set;
    }
    let allowed = std::thread::available_parallelism().map_or(4, |n| n.get());
    let cores = sysinfo::System::physical_core_count().unwrap_or(allowed);
    allowed.min(cores).clamp(1, 8) as i32
}

impl Engine {
    pub fn load(id: &str) -> anyhow::Result<Self> {
        let dir = model_path(id);
        anyhow::ensure!(installed(id), "model {id} is not installed");
        anyhow::ensure!(
            runs_here(id),
            "this computer's processor can't run it (it needs AVX2). Choose Parakeet v3 (ONNX) instead."
        );
        Ok(match id {
            "parakeet-v3" => {
                use transcribe_cpp::{Backend, Model, ModelOptions, SessionOptions};
                static LOGGING: std::sync::Once = std::sync::Once::new();
                LOGGING.call_once(transcribe_cpp::init_logging);
                let options = ModelOptions { backend: Backend::Cpu, ..Default::default() };
                let model = Model::load_with(dir.join(PARAKEET_V3_GGUF), &options)?;
                // The session keeps the model alive.
                Engine::Gguf(model.session_with(&SessionOptions { n_threads: gguf_threads(), ..Default::default() })?)
            }
            PARAKEET_V3_ONNX => Engine::Parakeet(ParakeetModel::load(&dir, &Quantization::Int8)?),
            "cohere-int4" => Engine::Cohere(CohereModel::load(&dir, &Quantization::Int4)?),
            other => anyhow::bail!("unknown model {other}"),
        })
    }

    pub fn transcribe(&mut self, samples: &[f32], language: &str) -> anyhow::Result<String> {
        let text = match self {
            // Parakeet v3 finds the language itself; transcribe.cpp takes none for it.
            Engine::Gguf(s) => s.run(samples, &transcribe_cpp::RunOptions::default())?.text,
            Engine::Parakeet(m) => m.transcribe_with(samples, &ParakeetParams::default())?.text,
            Engine::Cohere(m) => {
                let lang = if language.is_empty() || language == "auto" {
                    "en"
                } else {
                    language
                };
                let p = CohereParams {
                    language: Some(lang.into()),
                    ..Default::default()
                };
                m.transcribe_with(samples, &p)?.text
            }
        };
        Ok(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::unversioned::transport::{time, NextTimeout};

    #[test]
    fn every_model_file_has_a_published_hash() {
        for m in MODELS {
            match &m.source {
                Source::Archive { sha256, .. } => assert!(sha256.is_some_and(|h| h.len() == 64), "{}", m.id),
                Source::Files { files, .. } => {
                    for f in files.iter() {
                        assert!(f.sha256.is_some_and(|h| h.len() == 64), "{} {}", m.id, f.name)
                    }
                }
            }
        }
    }

    #[test]
    fn a_file_that_doesnt_match_its_hash_is_refused() {
        // SHA-256 of "abc".
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(check_hash("x", abc, Some(abc)).is_ok());
        assert!(check_hash("x", abc, Some(&abc.to_uppercase())).is_ok());
        assert!(check_hash("x", abc, None).is_ok());
        let e = check_hash("tokens.txt", abc, Some(&"0".repeat(64))).unwrap_err().to_string();
        assert!(e.contains("tokens.txt") && e.contains("checksum"), "{e}");
    }

    #[test]
    fn no_read_waits_longer_than_the_limit() {
        let limit = Duration::from_secs(30);
        let never = NextTimeout { after: time::Duration::NotHappening, reason: ureq::Timeout::Global };
        let long = NextTimeout { after: time::Duration::from_secs(600), reason: ureq::Timeout::RecvBody };
        let short = NextTimeout { after: time::Duration::from_secs(5), reason: ureq::Timeout::RecvResponse };
        assert_eq!(*capped(never, limit).after, limit, "ureq waits forever by default: the bug");
        assert_eq!(*capped(long, limit).after, limit);
        assert_eq!(capped(short, limit), short, "a shorter wait ureq asked for stands");
    }

    #[test]
    fn the_onnx_fallback_honors_tdt_durations() {
        // Needs the vendored transcribe-rs (vendor/transcribe-rs/VENDORED.md): the crates.io release
        // has no TdtCursor, and its decoder emits a word once per frame the model meant to skip.
        use transcribe_rs::onnx::parakeet::TdtCursor;
        let (vocab, blank) = (4, 3);
        // One word spanning 4 frames, then silence: token 1 with duration 4, then blanks.
        let joint = |t: usize| {
            let mut v = vec![0.0f32; vocab + 5];
            if t < 4 {
                v[1] = 1.0;
                v[vocab + 4] = 1.0;
            } else {
                v[blank as usize] = 1.0;
                v[vocab + 1] = 1.0;
            }
            v
        };
        let mut cursor = TdtCursor::default();
        let mut words = vec![];
        while cursor.t < 8 {
            let t = cursor.t;
            words.extend(cursor.step(&joint(t), vocab, blank).map(|w| (t, w)));
        }
        assert_eq!(words, vec![(0, 1)], "the stock decoder emits it 10 times on each of 4 frames");
    }

    #[test]
    fn the_default_model_is_the_build_the_study_scored() {
        let study: serde_json::Value = serde_json::from_str(include_str!("../../../bench/models/accent-2026-11.candidates.json")).unwrap();
        let pick = study["candidates"].as_array().unwrap().iter().find(|c| c["id"] == "parakeet-v3-q4km-gguf").unwrap();
        let m = spec("parakeet-v3").unwrap();
        assert!(m.recommended);
        let Source::Files { base, files } = &m.source else { panic!("parakeet-v3 is one GGUF file") };
        assert_eq!(files.len(), 1);
        assert_eq!(pick["model"].as_str().unwrap(), format!("handy-computer/parakeet-tdt-0.6b-v3-gguf {}", files[0].name));
        assert_eq!(files[0].sha256, pick["sha256"].as_str());
        let mb = m.size_mb as u64 * 1_048_576 / 1_000_000;
        assert!(include_str!("../../../README.md").contains(&format!("The speech model ({mb} MB)")), "README's First run");
        // Pinned: a commit, not a branch, so the file can't change under its hash.
        let rev = base.rsplit('/').next().unwrap();
        assert!(base.starts_with("https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf/resolve/"), "{base}");
        assert!(rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()), "{base}");
        // The ONNX fallback stays on Silas's own mirror.
        assert!(matches!(spec(PARAKEET_V3_ONNX).unwrap().source, Source::Archive { url, .. } if url.contains("baney75/silas/releases")));
    }

    fn legacy_onnx_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::store::set_data_dir(Some(dir.clone()));
        let old = models_dir().join("parakeet-v3");
        std::fs::create_dir_all(&old).unwrap();
        for f in ["encoder-model.int8.onnx", "decoder_joint-model.int8.onnx", "vocab.txt", ".complete"] {
            std::fs::write(old.join(f), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn old_onnx_files_move_to_the_fallback_and_keep_their_user() {
        let _store = crate::store::TEST_DIR.lock();
        let dir = legacy_onnx_dir("legacy");
        let moved = move_legacy_parakeet().unwrap();
        let (onnx, gguf) = (installed(PARAKEET_V3_ONNX), installed("parakeet-v3"));
        // That start quit before saving: the next one moves nothing and must still retarget.
        let again = move_legacy_parakeet().unwrap();
        let r = |m: &str| ModelResult { model: m.into(), speed: 20.0, wer: 0.01, audio_seconds: 1.0, compute_seconds: 0.05, clips: 9 };
        let mut report = benchmark::Report::new(benchmark::detect(), vec![r("parakeet-v3"), r("cohere-int4")]);
        report.pick = Some("parakeet-v3".into());
        benchmark::save_report(&report);
        let mut s = crate::store::Settings { model_source: "default".into(), ..Default::default() };
        let retargeted = retarget_legacy_parakeet(&mut s);
        let report: benchmark::Report = crate::store::load(benchmark::REPORT_FILE);
        let saved: crate::store::Settings = crate::store::load("settings.json");
        // Done once: a later choice of the GGUF build is left alone.
        let mut later = crate::store::Settings::default();
        let later_changed = retarget_legacy_parakeet(&mut later);
        crate::store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert!(moved && onnx && !gguf, "moved {moved}, onnx {onnx}, gguf {gguf}");
        assert!(!again && retargeted);
        assert_eq!((s.model.as_str(), s.model_source.as_str(), saved.model.as_str()), (PARAKEET_V3_ONNX, "default", PARAKEET_V3_ONNX));
        let ids: Vec<_> = report.results.iter().map(|x| x.model.as_str()).collect();
        assert_eq!(ids, [PARAKEET_V3_ONNX, "cohere-int4"]);
        assert_eq!(report.pick, None, "the ONNX build must not be marked best over the GGUF one");
        assert!(!later_changed && later.model == "parakeet-v3");
    }

    /// A new user with both builds, who benchmarked the GGUF one and removed it, never had the old files.
    /// Their results stay the GGUF build's, and an older Silas never reads the GGUF folder as its own.
    #[test]
    fn only_files_an_older_silas_left_are_retargeted() {
        let _store = crate::store::TEST_DIR.lock();
        let dir = std::env::temp_dir().join(format!("pc-both-builds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::store::set_data_dir(Some(dir.clone()));
        for id in ["parakeet-v3", PARAKEET_V3_ONNX] {
            std::fs::create_dir_all(model_path(id)).unwrap();
            std::fs::write(model_path(id).join(".complete"), b"ok").unwrap();
        }
        std::fs::write(model_path("parakeet-v3").join(PARAKEET_V3_GGUF), b"x").unwrap();
        let older_build_sees_it = models_dir().join("parakeet-v3/.complete").exists();
        let gguf = installed("parakeet-v3");
        let r = ModelResult { model: "parakeet-v3".into(), speed: 20.0, wer: 0.01, audio_seconds: 1.0, compute_seconds: 0.05, clips: 9 };
        benchmark::save_report(&benchmark::Report::new(benchmark::detect(), vec![r]));
        remove("parakeet-v3").unwrap();
        let moved = move_legacy_parakeet().unwrap();
        let mut s = crate::store::Settings::default();
        let changed = retarget_legacy_parakeet(&mut s);
        let report: benchmark::Report = crate::store::load(benchmark::REPORT_FILE);
        crate::store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert!(gguf && !older_build_sees_it, "a .complete-only check must not see the GGUF install");
        assert!(!moved && !changed && s.model == "parakeet-v3");
        assert_eq!(report.results[0].model, "parakeet-v3");
    }

    #[test]
    fn a_pick_this_cpu_cant_run_leaves_the_onnx_build_recommended() {
        let no_avx2 = |id: &str| id != "parakeet-v3";
        let onnx = spec(PARAKEET_V3_ONNX).unwrap();
        for best in [None, Some("parakeet-v3")] {
            assert!(recommends(onnx, best, &no_avx2), "{best:?}");
            assert!(!recommends(spec("parakeet-v3").unwrap(), best, &no_avx2));
        }
        assert!(recommends(spec("parakeet-v3").unwrap(), Some("parakeet-v3"), &|_| true));
        assert!(!recommends(onnx, Some("parakeet-v3"), &|_| true));
    }

    /// The browser preview's benchmark reads a score for every model it lists.
    #[test]
    fn the_preview_mock_scores_every_model() {
        let mock = include_str!("../../../src/mock.ts");
        for m in MODELS {
            assert!(mock.contains(&format!("\"{0}\": {{ model: \"{0}\"", m.id)), "src/mock.ts mockScores lacks {}", m.id);
        }
    }

    #[test]
    fn a_failed_move_never_lets_the_gguf_download_delete_the_onnx_files() {
        let _store = crate::store::TEST_DIR.lock();
        let dir = legacy_onnx_dir("legacy-stuck");
        // A file where the fallback's folder goes makes the rename fail.
        std::fs::create_dir_all(models_dir()).unwrap();
        std::fs::write(model_path(PARAKEET_V3_ONNX), b"in the way").unwrap();
        let err = download("parakeet-v3", &|_, _, _| {}, &|| true).unwrap_err().to_string();
        let kept = models_dir().join("parakeet-v3/encoder-model.int8.onnx").exists();
        crate::store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert!(err.contains("move the older Parakeet v3 files aside"), "{err}");
        assert!(kept, "the ONNX install was deleted");
    }

    #[test]
    fn gguf_threads_stay_within_the_cores() {
        let n = gguf_threads();
        assert!((1..=8).contains(&n), "{n}");
        assert!(n as usize <= std::thread::available_parallelism().map_or(8, |p| p.get()));
        set_gguf_threads(11);
        let set = gguf_threads();
        set_gguf_threads(0);
        assert_eq!((set, gguf_threads()), (11, n), "silas-bench --threads reaches the engine and its rows");
    }

    /// Transcribes the first clip of the bundled set with a real model. Models are big, so this runs
    /// only with SILAS_TEST_MODELS set to a Silas data folder (CI's bench job sets it); a missing model
    /// is downloaded there, hash-checked, the way the app does.
    fn engine_on_a_bundled_clip(id: &str) {
        let Some(data) = std::env::var_os("SILAS_TEST_MODELS") else {
            eprintln!("skipped: set SILAS_TEST_MODELS to a data folder to run {id}");
            return;
        };
        let _store = crate::store::TEST_DIR.lock();
        crate::store::set_data_dir(Some(data.into()));
        let run = || -> anyhow::Result<(String, String, f32)> {
            move_legacy_parakeet()?;
            if !installed(id) {
                download(id, &|_, _, _| {}, &|| false)?;
            }
            let mut engine = Engine::load(id)?;
            let clip = benchmark::load_clips()?.remove(0);
            let text = engine.transcribe(&clip.samples, "en")?;
            let wer = benchmark::wer(&clip.text, &text);
            Ok((clip.text, text, wer))
        };
        let got = run();
        crate::store::set_data_dir(None);
        let (want, text, wer) = got.unwrap();
        eprintln!("{id}: {text:?}");
        assert!(wer <= 0.1, "{id} heard {text:?} for {want:?} ({:.0}% word errors)", wer * 100.0);
    }

    #[test]
    fn parakeet_v3_gguf_transcribes_a_clip() {
        engine_on_a_bundled_clip("parakeet-v3");
    }

    #[test]
    fn parakeet_v3_onnx_transcribes_a_clip() {
        engine_on_a_bundled_clip(PARAKEET_V3_ONNX);
    }

    #[test]
    fn a_cancelled_download_stops_and_leaves_nothing() {
        let _store = crate::store::TEST_DIR.lock();
        let dir = std::env::temp_dir().join(format!("pc-cancel-{}", std::process::id()));
        crate::store::set_data_dir(Some(dir.clone()));
        // Cancelled before the first read: nothing reaches the network, nothing stays on disk.
        let e = download("parakeet-v3", &|_, _, _| {}, &|| true);
        crate::store::set_data_dir(None);
        let e = e.unwrap_err().to_string();
        assert_eq!(e, crate::core::CANCELLED);
        let left: Vec<_> = std::fs::read_dir(dir.join("models")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(left.is_empty(), "{left:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
