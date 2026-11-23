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
    /// Where the model picker lists it when it isn't the one recommended for this computer.
    pub group: Group,
    /// The lab that trained it, for the picker and the README's credits.
    pub lab: &'static str,
    /// The weights' license as its model card names it, and the license text.
    pub license: &'static str,
    pub license_url: &'static str,
    /// The original model on Hugging Face (`org/name`); its card is what the picker links to.
    pub base_model: &'static str,
    pub source: Source,
}

/// The picker's sections, by what a model is best at (bench/models/catalog-2026-11.md).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    English,
    ManyLanguages,
    SmallAndFast,
    MostAccurate,
}

impl Spec {
    /// The base model's card on Hugging Face.
    pub fn card_url(&self) -> String {
        format!("https://huggingface.co/{}", self.base_model)
    }
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
        group: Group::ManyLanguages,
        lab: "NVIDIA",
        license: "CC-BY-4.0",
        license_url: "https://creativecommons.org/licenses/by/4.0/",
        base_model: "nvidia/parakeet-tdt-0.6b-v3",
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
        group: Group::ManyLanguages,
        lab: "NVIDIA",
        license: "CC-BY-4.0",
        license_url: "https://creativecommons.org/licenses/by/4.0/",
        base_model: "nvidia/parakeet-tdt-0.6b-v3",
        source: Source::Archive {
            url: "https://github.com/baney75/silas/releases/download/models-v1/parakeet-v3-int8.tar.gz",
            folder: "parakeet-tdt-0.6b-v3-int8",
            // Handy's archive (its model.rs before onnx was removed, cjpais/Handy 5197faf^), matched
            // against the file on 2026-11-10 and mirrored byte for byte to our own release on
            // 2026-11-17: Handy has moved to GGUF models, so its host may drop this file.
            sha256: Some("43d37191602727524a7d8c6da0eef11c4ba24320f5b4730f1a2497befc2efa77"),
        },
    },
    // The study's other picks (bench/models/catalog-2026-11.md), each pinned like the default: one
    // revision of the public GGUF repo and the Hub's Git LFS object id for the file, both recorded in
    // bench/models/catalog-2026-11.candidates.json.
    Spec {
        id: "parakeet-v2",
        name: "Parakeet v2",
        tier: "balanced",
        tagline: "English only. As accurate as Parakeet v3 overall and the best of the fast models with code and technical words, but weaker with accents.",
        size_mb: 475,
        languages: "English",
        recommended: false,
        group: Group::English,
        lab: "NVIDIA",
        license: "CC-BY-4.0",
        license_url: "https://creativecommons.org/licenses/by/4.0/",
        base_model: "nvidia/parakeet-tdt-0.6b-v2",
        source: Source::Files {
            base: "https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v2-gguf/resolve/58239cf6d0ecb0af921e7936c1e5e0a8dba91dbb",
            files: &[File {
                name: "parakeet-tdt-0.6b-v2-Q4_K_M.gguf",
                sha256: Some("4853f9653f641d376e6f7de65d73c7a34a73677704a606727bf51acc83f999f3"),
            }],
        },
    },
    Spec {
        id: "canary-180m",
        name: "Canary 180M Flash",
        tier: "fast",
        tagline: "Less than half the download of Parakeet with a shorter wait, for older laptops. Nearly as accurate with clear speech, weaker with accents. It can't detect a language, so choose yours in Settings.",
        size_mb: 218,
        languages: "English, German, French, Spanish",
        recommended: false,
        group: Group::SmallAndFast,
        lab: "NVIDIA",
        license: "CC-BY-4.0",
        license_url: "https://creativecommons.org/licenses/by/4.0/",
        base_model: "nvidia/canary-180m-flash",
        source: Source::Files {
            base: "https://huggingface.co/handy-computer/canary-180m-flash-gguf/resolve/8b01fc77ac169a0a05c5c0332bbfe54acf65c26f",
            files: &[File {
                name: "canary-180m-flash-Q8_0.gguf",
                sha256: Some("f942ab746040b531a29ab78ca3afc8dc9564efd8e4c942b5541e53644be9c408"),
            }],
        },
    },
    Spec {
        id: "moonshine-tiny",
        name: "Moonshine Tiny",
        tier: "fast",
        tagline: "The smallest and lightest: a 35 MB download that runs on almost any computer. Makes more than twice as many mistakes as Parakeet.",
        size_mb: 35,
        languages: "English",
        recommended: false,
        group: Group::SmallAndFast,
        lab: "Moonshine AI",
        license: "MIT",
        license_url: "https://opensource.org/license/mit",
        base_model: "UsefulSensors/moonshine-tiny",
        source: Source::Files {
            base: "https://huggingface.co/handy-computer/moonshine-tiny-gguf/resolve/c71a93b8f1cb2b7f7336f31bb54bc96960c16f41",
            files: &[File {
                name: "moonshine-tiny-Q8_0.gguf",
                sha256: Some("2fd348d7b38f97d309cc3ec6848f3f57f537b80244950f07d2637e463f95a3a1"),
            }],
        },
    },
    Spec {
        id: "qwen3-asr-0.6b",
        name: "Qwen3-ASR 0.6B",
        tier: "balanced",
        tagline: "For languages Parakeet lacks, like Chinese, Japanese, Korean, Arabic and Hindi. Nearly as accurate in English, but text takes about 2 seconds to appear on a laptop.",
        size_mb: 590,
        languages: "30 languages + 22 Chinese dialects",
        recommended: false,
        group: Group::ManyLanguages,
        lab: "Qwen (Alibaba)",
        license: "Apache-2.0",
        license_url: "https://www.apache.org/licenses/LICENSE-2.0",
        base_model: "Qwen/Qwen3-ASR-0.6B",
        source: Source::Files {
            base: "https://huggingface.co/handy-computer/Qwen3-ASR-0.6B-gguf/resolve/7e7c69a577fa2725dfedbe34b5e999336e30a19b",
            files: &[File {
                name: "Qwen3-ASR-0.6B-Q4_K_M.gguf",
                sha256: Some("5b58f32a58ffa2c8783e0b0963485623e286e6272d953dfc9e28bc3447dee0c0"),
            }],
        },
    },
    Spec {
        id: "whisper-turbo",
        name: "Whisper Large v3 Turbo",
        tier: "best",
        tagline: "The most languages by far. Slow on a laptop, where text can take 10 seconds or more to appear; best on a fast computer or for transcribing files.",
        size_mb: 536,
        languages: "99 languages",
        recommended: false,
        group: Group::ManyLanguages,
        lab: "OpenAI",
        license: "MIT",
        license_url: "https://opensource.org/license/mit",
        base_model: "openai/whisper-large-v3-turbo",
        source: Source::Files {
            base: "https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf/resolve/18ada752005f5fddef499256f9a458d0b37a203a",
            files: &[File {
                name: "whisper-large-v3-turbo-Q4_K_M.gguf",
                sha256: Some("c6e172d0074f0fa9440b88247854b50643cd4bb58b8515c797bbed16d858c7df"),
            }],
        },
    },
    Spec {
        id: "canary-qwen",
        name: "Canary-Qwen 2.5B",
        tier: "best",
        tagline: "English only. The fewest mistakes with code and dictation of any model here. Needs a fast computer: on a laptop your text takes about 4 seconds to appear.",
        size_mb: 1738,
        languages: "English",
        recommended: false,
        group: Group::MostAccurate,
        lab: "NVIDIA",
        license: "CC-BY-4.0",
        license_url: "https://creativecommons.org/licenses/by/4.0/",
        base_model: "nvidia/canary-qwen-2.5b",
        source: Source::Files {
            base: "https://huggingface.co/handy-computer/canary-qwen-2.5b-gguf/resolve/a4480da236b7d99d7a12186868de021ef4ff5ced",
            files: &[File {
                name: "canary-qwen-2.5b-Q4_K_M.gguf",
                sha256: Some("db5162229d6fa22597d06a613bd9b543eddb3ee02e6afc5e759120fde02bebf7"),
            }],
        },
    },
    Spec {
        id: "cohere-int4",
        name: "Cohere Transcribe",
        tier: "best",
        tagline: "Few mistakes with code, in 14 languages, but a 2 GB download that waits about 4.5 seconds on a laptop. It can't detect a language, so choose yours in Settings.",
        size_mb: 2000,
        languages: "English + 13 languages",
        recommended: false,
        group: Group::MostAccurate,
        lab: "Cohere",
        license: "Apache-2.0",
        license_url: "https://www.apache.org/licenses/LICENSE-2.0",
        base_model: "CohereLabs/cohere-transcribe-03-2026",
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

/// The one GGUF file of a model that runs on transcribe.cpp; None for the ONNX models.
pub fn gguf_file(id: &str) -> Option<&'static str> {
    match &spec(id)?.source {
        Source::Files { files: [f], .. } if f.name.ends_with(".gguf") => Some(f.name),
        _ => None,
    }
}

/// Whether this computer can run the model. The GGUF engine is built for a CPU floor
/// (`.cargo/config.toml`): on x86 that is AVX2, FMA and F16C (macOS x86: AVX and F16C). Loading it
/// anywhere below that would crash on an illegal instruction instead of failing.
pub fn runs_here(id: &str) -> bool {
    gguf_file(id).is_none() || cpu_runs_gguf()
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
    pub group: Group,
    pub lab: String,
    pub license: String,
    pub license_url: String,
    pub base_model: String,
    pub card_url: String,
    pub installed: bool,
    pub active: bool,
    pub recommended: bool,
    /// This computer's measured speed and accuracy for the model, if it has been run.
    pub device_result: Option<ModelResult>,
    /// This computer's own benchmark's pick for this device; else the registry's, else a
    /// settled community pick. Shown as information; only `recommended` chooses a download.
    pub best_for_device: bool,
}

pub fn list(active: Option<&str>) -> Vec<ModelEntry> {
    let report = benchmark::load_report();
    let best = benchmark::best_for_device_from(report.as_ref());
    // First-run setup downloads the recommended model, so only a trusted pick may set it.
    let trusted = benchmark::trusted_best_for_device_from(report.as_ref());
    MODELS
        .iter()
        .map(|m| ModelEntry {
            id: m.id.into(),
            name: m.name.into(),
            tier: m.tier.into(),
            tagline: m.tagline.into(),
            size_mb: m.size_mb,
            languages: m.languages.into(),
            group: m.group,
            lab: m.lab.into(),
            license: m.license.into(),
            license_url: m.license_url.into(),
            base_model: m.base_model.into(),
            card_url: m.card_url(),
            installed: installed(m.id),
            active: active == Some(m.id) && installed(m.id),
            // Recommend what is known to suit this computer; the static flag otherwise.
            recommended: recommends(m, trusted.as_deref(), &runs_here),
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
    Gguf(GgufEngine),
    Parakeet(ParakeetModel),
    Cohere(CohereModel),
}

/// A model on transcribe.cpp, with what it says about languages.
pub struct GgufEngine {
    session: transcribe_cpp::Session,
    languages: Vec<String>,
    detects: bool,
}

/// The language to ask a transcribe.cpp model for. A model of one language (or none listed) is left to
/// its default. A multilingual one gets the user's language when it has it; on "auto" it detects the
/// language itself if it can, else it is asked for English, which every catalog model has.
fn gguf_language(languages: &[String], detects: bool, wanted: &str) -> Option<String> {
    if languages.len() <= 1 {
        return None;
    }
    let has = |l: &str| languages.iter().any(|x| x.eq_ignore_ascii_case(l));
    if !wanted.is_empty() && wanted != "auto" && has(wanted) {
        return Some(wanted.to_ascii_lowercase());
    }
    if detects {
        return None;
    }
    has("en").then(|| "en".to_string())
}

/// Canary 180M on transcribe.cpp writes German, French and Spanish with a space before each mark
/// ("nur , wenn du mitkommst ."); English comes out right. Outside English, a `,` or `.` (and `?`, `!`,
/// `;`, `:` except in French, which spaces them) that follows a word and ends one gets its space back.
fn attach_punctuation(text: &str, language: &str) -> String {
    let marks: &[char] = if language == "fr" { &[',', '.'] } else { &[',', '.', '?', '!', ';', ':'] };
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &ch) in c.iter().enumerate() {
        let drop = ch == ' '
            && i > 0
            && c[i - 1].is_alphanumeric()
            && c.get(i + 1).is_some_and(|m| marks.contains(m))
            && c.get(i + 2).is_none_or(|n| n.is_whitespace());
        if !drop {
            out.push(ch);
        }
    }
    out
}

/// The 14 languages on Cohere Transcribe's model card.
const COHERE_LANGUAGES: &[&str] = &["en", "fr", "de", "it", "es", "pt", "el", "nl", "pl", "zh", "ja", "ko", "vi", "ar"];

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
        if let Some(file) = gguf_file(id) {
            use transcribe_cpp::{Backend, Model, ModelOptions, SessionOptions};
            static LOGGING: std::sync::Once = std::sync::Once::new();
            LOGGING.call_once(transcribe_cpp::init_logging);
            let options = ModelOptions { backend: Backend::Cpu, ..Default::default() };
            let model = Model::load_with(dir.join(file), &options)?;
            let caps = model.capabilities()?;
            // The session keeps the model alive.
            let session = model.session_with(&SessionOptions { n_threads: gguf_threads(), ..Default::default() })?;
            return Ok(Engine::Gguf(GgufEngine { session, languages: caps.languages, detects: caps.supports_language_detect }));
        }
        Ok(match id {
            PARAKEET_V3_ONNX => Engine::Parakeet(ParakeetModel::load(&dir, &Quantization::Int8)?),
            "cohere-int4" => Engine::Cohere(CohereModel::load(&dir, &Quantization::Int4)?),
            other => anyhow::bail!("unknown model {other}"),
        })
    }

    pub fn transcribe(&mut self, samples: &[f32], language: &str) -> anyhow::Result<String> {
        let text = match self {
            Engine::Gguf(g) => {
                let language = gguf_language(&g.languages, g.detects, language);
                let text = g.session.run(samples, &transcribe_cpp::RunOptions { language: language.clone(), ..Default::default() })?.text;
                match language.as_deref() {
                    Some(l) if l != "en" => attach_punctuation(&text, l),
                    _ => text,
                }
            }
            Engine::Parakeet(m) => m.transcribe_with(samples, &ParakeetParams::default())?.text,
            Engine::Cohere(m) => {
                // Cohere can't detect a language, and one it lacks isn't asked of it.
                let lang = if COHERE_LANGUAGES.contains(&language) { language } else { "en" };
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

    /// Every download is pinned: a Hugging Face file to a full commit (a branch could move under its hash)
    /// and every file to its SHA-256; an archive to Silas's own release and its SHA-256.
    #[test]
    fn every_catalog_entry_is_pinned_to_a_revision_and_hashes() {
        let hex = |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        for m in MODELS {
            match &m.source {
                Source::Archive { url, sha256, .. } => {
                    assert!(url.starts_with("https://github.com/baney75/silas/releases/download/"), "{}: {url}", m.id);
                    assert!(sha256.is_some_and(|h| hex(h, 64)), "{}", m.id);
                }
                Source::Files { base, files } => {
                    let (repo, rev) = base.split_once("/resolve/").unwrap_or_else(|| panic!("{}: {base}", m.id));
                    assert!(repo.starts_with("https://huggingface.co/") && repo.matches('/').count() == 4, "{}: {base}", m.id);
                    assert!(hex(rev, 40), "{}: {base} is not pinned to a full commit", m.id);
                    assert!(!files.is_empty(), "{}", m.id);
                    for f in files.iter() {
                        assert!(f.sha256.is_some_and(|h| hex(h, 64)), "{} {}", m.id, f.name);
                    }
                }
            }
        }
    }

    /// The trust rules for the catalog (bench/models/catalog-2026-11.md): weights from a known lab, a license
    /// that allows commercial use, GGUF data files only for transcribe.cpp models, and ids that are unique.
    #[test]
    fn every_catalog_entry_meets_the_trust_rules() {
        let labs = [
            ("NVIDIA", "nvidia/"),
            ("OpenAI", "openai/"),
            ("Mistral AI", "mistralai/"),
            ("IBM", "ibm-granite/"),
            ("Cohere", "CohereLabs/"),
            ("Qwen (Alibaba)", "Qwen/"),
            ("Moonshine AI", "UsefulSensors/"),
        ];
        let licenses = [
            ("CC-BY-4.0", "https://creativecommons.org/licenses/by/4.0/"),
            ("MIT", "https://opensource.org/license/mit"),
            ("Apache-2.0", "https://www.apache.org/licenses/LICENSE-2.0"),
            ("NVIDIA Open Model License", "https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/"),
            ("OpenMDW-1.1", "https://openmdw.ai/license/1-1/"),
        ];
        let mut ids = std::collections::HashSet::new();
        for m in MODELS {
            assert!(ids.insert(m.id), "{} twice", m.id);
            let (_, org) = labs.iter().find(|(l, _)| *l == m.lab).unwrap_or_else(|| panic!("{}: unknown lab {}", m.id, m.lab));
            assert!(m.base_model.starts_with(org), "{}: {} is not from {}", m.id, m.base_model, m.lab);
            assert!(licenses.contains(&(m.license, m.license_url)), "{}: {} {}", m.id, m.license, m.license_url);
            if let Source::Files { files, .. } = &m.source {
                if files.iter().any(|f| f.name.ends_with(".gguf")) {
                    assert_eq!(files.len(), 1, "{}: a transcribe.cpp model is one GGUF file", m.id);
                    assert_eq!(gguf_file(m.id), Some(files[0].name));
                }
                for f in files.iter() {
                    let ext = f.name.rsplit('.').next().unwrap();
                    assert!(["gguf", "onnx", "data", "txt"].contains(&ext), "{} {}: no pickled or code files", m.id, f.name);
                }
            }
            assert!(m.card_url().starts_with("https://huggingface.co/"));
        }
        assert_eq!(MODELS.iter().filter(|m| m.recommended).count(), 1, "one static default");
    }

    /// The device registry takes at most 10 results per report (registry/ingest.py), one per model.
    #[test]
    fn the_catalog_fits_a_device_report() {
        assert!(MODELS.len() <= 10, "{} models", MODELS.len());
    }

    /// Each transcribe.cpp entry is the exact build the catalog study scored: same repo, file, revision, hash.
    #[test]
    fn every_gguf_model_is_the_build_the_study_scored() {
        let study: serde_json::Value = serde_json::from_str(include_str!("../../../bench/models/catalog-2026-11.candidates.json")).unwrap();
        let scored = study["candidates"].as_array().unwrap();
        for m in MODELS {
            let Some(file) = gguf_file(m.id) else { continue };
            let Source::Files { base, files } = &m.source else { unreachable!() };
            let (repo, rev) = base.trim_start_matches("https://huggingface.co/").split_once("/resolve/").unwrap();
            let c = scored
                .iter()
                .find(|c| c["model"].as_str() == Some(&format!("{repo} {file}")))
                .unwrap_or_else(|| panic!("{}: {repo} {file} was not in the study", m.id));
            assert_eq!(c["revision"].as_str(), Some(rev), "{}", m.id);
            assert_eq!(c["sha256"].as_str(), files[0].sha256, "{}", m.id);
            assert_eq!((c["license"].as_str(), c["base_model"].as_str(), c["lab"].as_str()), (Some(m.license), Some(m.base_model), Some(m.lab)), "{}", m.id);
        }
    }

    #[test]
    fn punctuation_spaced_off_its_word_is_attached_outside_english() {
        assert_eq!(attach_punctuation("Ich gehe nur , wenn du mitkommst .", "de"), "Ich gehe nur, wenn du mitkommst.");
        assert_eq!(attach_punctuation("¿ Vienes ? Sí , ya voy !", "es"), "¿ Vienes? Sí, ya voy!");
        assert_eq!(attach_punctuation("Vraiment ? Oui , bien sûr .", "fr"), "Vraiment ? Oui, bien sûr.");
        assert_eq!(attach_punctuation("cd .. und dann ~/ . und x , y", "de"), "cd .. und dann ~/ . und x, y", "a mark after a symbol or before a word stays");
    }

    #[test]
    fn multilingual_models_get_the_users_language_and_english_only_ones_none() {
        let many: Vec<String> = ["en", "de", "fr"].iter().map(|s| s.to_string()).collect();
        assert_eq!(gguf_language(&many, false, "de"), Some("de".into()));
        assert_eq!(gguf_language(&many, true, "DE"), Some("de".into()));
        assert_eq!(gguf_language(&many, true, "auto"), None, "it detects the language itself");
        assert_eq!(gguf_language(&many, false, "auto"), Some("en".into()));
        assert_eq!(gguf_language(&many, false, "ja"), Some("en".into()), "a language it lacks falls back to English");
        assert_eq!(gguf_language(&["en".into()], false, "de"), None);
        assert_eq!(gguf_language(&[], true, "en"), None);
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
        let r = |m: &str| ModelResult { model: m.into(), speed: 20.0, wer: 0.01, audio_seconds: 1.0, compute_seconds: 0.05, clips: 9, wait_seconds: 0.0, clip_seconds: vec![] };
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
        let r = ModelResult { model: "parakeet-v3".into(), speed: 20.0, wer: 0.01, audio_seconds: 1.0, compute_seconds: 0.05, clips: 9, wait_seconds: 0.0, clip_seconds: vec![] };
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
        // Something where the fallback's folder goes makes the rename fail: a file, or on Windows, which
        // deletes that file, a folder holding a file opened without delete sharing.
        std::fs::create_dir_all(models_dir()).unwrap();
        #[cfg(not(windows))]
        std::fs::write(model_path(PARAKEET_V3_ONNX), b"in the way").unwrap();
        #[cfg(windows)]
        let _held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::create_dir_all(model_path(PARAKEET_V3_ONNX)).unwrap();
            std::fs::OpenOptions::new().write(true).create(true).share_mode(0).open(model_path(PARAKEET_V3_ONNX).join("held")).unwrap()
        };
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

    /// Review first-F1 on #64: first-run setup downloads the recommended model, and a community
    /// pick, which anyone can post, could make it the recommended one.
    #[test]
    fn a_community_pick_is_never_the_recommended_download() {
        let _store = crate::store::TEST_DIR.lock();
        let dir = std::env::temp_dir().join(format!("pc-community-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::store::set_data_dir(Some(dir.clone()));
        let key = benchmark::device().key.clone();
        let device = serde_json::json!({
            "reports": 3, "pick": "cohere-int4", "agree": 3,
            "models": { "cohere-int4": { "reports": 3, "speed": 12.0, "wer": 0.03, "wait": 0.5 } }
        });
        let mut devices = serde_json::Map::new();
        devices.insert(key.clone(), device);
        let community = serde_json::json!({ "schema": 1, "suite": benchmark::suite_id(), "devices": devices });
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(benchmark::COMMUNITY_FILE), community.to_string()).unwrap();
        let settled = benchmark::cached_community().lookup(&key);
        let entries = list(None);
        crate::store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(settled.as_deref(), Some("cohere-int4"), "the community does settle this device");
        let rec: Vec<_> = entries.iter().filter(|m| m.recommended).map(|m| m.id.as_str()).collect();
        assert!(!rec.is_empty() && !rec.contains(&"cohere-int4"), "{rec:?}");
    }

    /// Every other catalog model, through the same download, load and transcribe path (SILAS_TEST_MODELS).
    #[test]
    fn every_catalog_model_transcribes_a_clip() {
        for m in MODELS.iter().filter(|m| m.id != "parakeet-v3" && m.id != PARAKEET_V3_ONNX) {
            engine_on_a_bundled_clip(m.id);
        }
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
