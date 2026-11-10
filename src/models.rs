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

pub const MODELS: &[Spec] = &[
    Spec {
        id: "parakeet-v3",
        name: "Parakeet v3",
        tier: "balanced",
        tagline:
            "Fast and accurate. Transcribes a minute of speech in about 5 seconds on a laptop CPU.",
        size_mb: 478,
        languages: "English + 24 European languages",
        recommended: true,
        source: Source::Archive {
            url: "https://blob.handy.computer/parakeet-v3-int8.tar.gz",
            folder: "parakeet-tdt-0.6b-v3-int8",
            // Published by Handy, which hosts the archive (its model.rs before onnx was removed,
            // cjpais/Handy 5197faf^), and matched against the file on 2026-11-10.
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

pub fn model_path(id: &str) -> PathBuf {
    models_dir().join(id)
}

pub fn installed(id: &str) -> bool {
    model_path(id).join(".complete").exists()
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
            recommended: best.as_deref().map_or(m.recommended, |b| b == m.id),
            device_result: report.as_ref().and_then(|r| r.results.iter().find(|x| x.model == m.id).cloned()),
            best_for_device: best.as_deref() == Some(m.id),
        })
        .collect()
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
    Parakeet(ParakeetModel),
    Cohere(CohereModel),
}

impl Engine {
    pub fn load(id: &str) -> anyhow::Result<Self> {
        let dir = model_path(id);
        anyhow::ensure!(installed(id), "model {id} is not installed");
        Ok(match id {
            "parakeet-v3" => Engine::Parakeet(ParakeetModel::load(&dir, &Quantization::Int8)?),
            "cohere-int4" => Engine::Cohere(CohereModel::load(&dir, &Quantization::Int4)?),
            other => anyhow::bail!("unknown model {other}"),
        })
    }

    pub fn transcribe(&mut self, samples: &[f32], language: &str) -> anyhow::Result<String> {
        let text = match self {
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
