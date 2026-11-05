//! The speech models Silas offers, how to download them, and how to run them.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

use serde::Serialize;
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
    },
    /// Loose files under one base URL.
    Files {
        base: &'static str,
        files: &'static [&'static str],
    },
}

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
        source: Source::Files {
            base: "https://huggingface.co/cstr/cohere-transcribe-onnx-int4/resolve/main",
            files: &[
                "cohere-decoder.int4.onnx",
                "cohere-decoder.int4.onnx.data",
                "cohere-encoder.int4.onnx",
                "cohere-encoder.int4.onnx.data",
                "tokens.txt",
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

/// Download a model into place. `progress(downloaded, total, stage)` is called often.
pub fn download(id: &str, progress: &dyn Fn(u64, u64, &str)) -> anyhow::Result<()> {
    let spec = spec(id).ok_or_else(|| anyhow::anyhow!("unknown model {id}"))?;
    let dest = model_path(id);
    let staging = models_dir().join(format!(".{id}.staging"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let total = spec.size_mb as u64 * 1_048_576;
    match &spec.source {
        Source::Archive { url, folder } => {
            let part = models_dir().join(format!(".{id}.tar.gz.part"));
            fetch(url, &part, 0, total, progress)?;
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
                    &format!("{base}/{f}"),
                    &staging.join(f),
                    done,
                    total,
                    progress,
                )?;
            }
            let _ = fs::remove_dir_all(&dest);
            fs::rename(&staging, &dest)?;
        }
    }
    fs::write(dest.join(".complete"), b"ok")?;
    progress(total, total, "done");
    Ok(())
}

fn fetch(
    url: &str,
    to: &PathBuf,
    offset: u64,
    total: u64,
    progress: &dyn Fn(u64, u64, &str),
) -> anyhow::Result<u64> {
    let resp = ureq::get(url).call()?;
    let len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let total = if offset == 0 { len.unwrap_or(total) } else { total };
    let mut reader = resp.into_body().into_reader();
    let mut out = fs::File::create(to)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    let mut last = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
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
