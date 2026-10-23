//! Updates from GitHub Releases on two channels.
//!
//! stable  (macOS only): the latest full release, built and signed on Donovan's Mac.
//! nightly (all):        the rolling `nightly` pre-release that CI rebuilds every night.

use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::core::{Shared, Status};

const REPO: &str = "https://github.com/baney75/silas/releases";

/// Each channel has its own signing key. The stable key never leaves the release Mac,
/// so nothing CI signs can ever install on a stable user's machine.
const STABLE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI0OUZBOTBFNEEyQUQzNTgKUldSWTB5cEtEcW1mdE5FYXBNV1AyT3NGaXdzVXFvUHRPeGZlZHFiWE9mNXhZTUtEZTJWZGhicWMK";
const NIGHTLY_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ1QTkyOTlGNzM0MEJGMTIKUldRU3YwQnpueW1wMVUvOEtteWFXL0xyUys3L1JOSmhZbkFqNEtUVTh5RjdwWGZMdWV1VzA4cGcK";

/// Set at build time by a distro package (`SILAS_PACKAGED=pacman`). Its package manager owns
/// the files, so Silas must not replace itself.
pub const PACKAGED: Option<&str> = option_env!("SILAS_PACKAGED");

pub fn endpoint(channel: &str) -> String {
    match channel {
        "stable" => format!("{REPO}/latest/download/latest.json"),
        _ => format!("{REPO}/download/nightly/latest.json"),
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
    pub channel: String,
}

async fn find(app: &AppHandle) -> anyhow::Result<Option<(tauri_plugin_updater::Update, String)>> {
    if let Some(manager) = PACKAGED {
        anyhow::bail!("This copy of Silas came from {manager}; update it there.");
    }
    let channel = app
        .state::<Shared>()
        .settings
        .lock()
        .effective_channel()
        .to_string();
    let url = endpoint(&channel).parse()?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])?
        .pubkey(if channel == "stable" { STABLE_PUBKEY } else { NIGHTLY_PUBKEY })
        // Moving from nightly back to stable can mean a lower version number; allow it.
        .version_comparator(|current, remote| remote.version != current)
        .timeout(Duration::from_secs(30))
        .build()?;
    Ok(updater.check().await?.map(|u| (u, channel)))
}

pub async fn check(app: &AppHandle, announce: bool) -> anyhow::Result<Option<UpdateInfo>> {
    let found = find(app).await?;
    let info = found.map(|(u, channel)| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
        channel,
    });
    if let (Some(i), true) = (&info, announce) {
        let _ = app.emit("update-available", i);
    }
    Ok(info)
}

pub async fn install(app: &AppHandle) -> anyhow::Result<()> {
    let Some((update, _)) = find(app).await? else {
        return Ok(());
    };
    let a = app.clone();
    let mut got = 0u64;
    update
        .download_and_install(
            move |chunk, total| {
                got += chunk as u64;
                let _ = a.emit(
                    "update-progress",
                    json!({ "downloaded": got, "total": total }),
                );
            },
            || {},
        )
        .await?;
    app.restart();
}

/// Check at launch and every four hours. With auto-update on, install once Silas has
/// been idle for two minutes so a restart never lands mid-dictation.
pub fn schedule(app: AppHandle) {
    if PACKAGED.is_some() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio_sleep(Duration::from_secs(20)).await;
        loop {
            match check(&app, true).await {
                Ok(Some(_)) => {
                    let core = app.state::<Shared>().inner().clone();
                    if core.settings.lock().auto_update {
                        loop {
                            let idle =
                                core.last_activity.lock().elapsed() > Duration::from_secs(120);
                            if idle && core.status().0 == Status::Ready {
                                break;
                            }
                            tokio_sleep(Duration::from_secs(30)).await;
                        }
                        if let Err(e) = install(&app).await {
                            log::warn!("auto-update failed: {e}");
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => log::info!("update check failed: {e}"),
            }
            tokio_sleep(Duration::from_secs(4 * 3600)).await;
        }
    });
}

async fn tokio_sleep(d: Duration) {
    tokio::time::sleep(d).await;
}
