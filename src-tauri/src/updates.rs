//! Updates from GitHub Releases on two channels.
//!
//! stable  (macOS only): the latest full release, built and signed on Donovan's Mac.
//! nightly (all):        the rolling `nightly` pre-release that CI rebuilds every night.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use parking_lot::Mutex;
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
        .pubkey(if channel == "stable" {
            STABLE_PUBKEY
        } else {
            NIGHTLY_PUBKEY
        })
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
    let bytes = update
        .download(
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
    // Never swap in a build macOS would treat as a different app: Accessibility would stay on
    // in Settings for the old signature while the new build is not trusted. Never call tccutil.
    let bytes = if cfg!(target_os = "macos") {
        let checked =
            tauri::async_runtime::spawn_blocking(move || check_candidate(&bytes).map(|()| bytes))
                .await?;
        if checked.is_err() {
            *REFUSED.lock() = Some(update.version.clone());
        }
        checked?
    } else {
        bytes
    };
    update.install(bytes)?;
    app.restart();
}

/// Check at launch and every four hours. With auto-update on, install once Silas has
/// been idle for two minutes so a restart never lands mid-dictation.
pub fn schedule(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio_sleep(Duration::from_secs(20)).await;
        loop {
            match check(&app, true).await {
                Ok(Some(info)) => {
                    let core = app.state::<Shared>().inner().clone();
                    // A version the signature guard refused stays refused; don't fetch it again.
                    let refused = REFUSED.lock().as_deref() == Some(info.version.as_str());
                    if refused {
                        log::info!(
                            "auto-update skips {}: signed differently from this install",
                            info.version
                        );
                    }
                    if core.settings.lock().auto_update && !refused {
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

/// What the user sees when an update would change how macOS identifies Silas.
pub const SIGNED_DIFFERENTLY: &str = "This update is signed differently from your install. Installing it would break Accessibility. Not installed.";

/// What the user sees when the update's signature names the right identity but doesn't verify.
pub const BAD_SIGNATURE: &str =
    "This update's signature doesn't check out, so it may be damaged. Not installed.";

/// The last version the guard refused, so auto-update doesn't download it again every check.
static REFUSED: Mutex<Option<String>> = Mutex::new(None);

/// BaneyMacUpdater's rule: the designated requirements must be the same string. An ad-hoc
/// requirement is the build's own `cdhash`, so two different ad-hoc builds never match; only
/// the identical build does. An empty (unreadable) requirement never matches.
pub fn same_designated_requirement(installed: &str, candidate: &str) -> bool {
    let (a, b) = (installed.trim(), candidate.trim());
    !a.is_empty() && a == b
}

/// macOS keeps Accessibility only while the designated requirement stays the same. A build
/// that changes it leaves the switch on in Settings while the new process is not trusted,
/// which is the stale grant this guard exists to prevent. An unreadable signature is a change.
/// `codesign -d` only prints the requirement, and a damaged signature can still print the right
/// one, so a match must also verify: the candidate's signature is valid and it satisfies the
/// installed build's requirement.
fn guard_requirement(
    installed: &Path,
    candidate: &Path,
    read: impl Fn(&Path) -> Option<String>,
    verify: impl Fn(&Path, &str) -> bool,
) -> anyhow::Result<()> {
    let old = read(installed).unwrap_or_default();
    let new = read(candidate).unwrap_or_default();
    if !same_designated_requirement(&old, &new) {
        log::warn!(
            "update refused: designated requirement differs (installed {}: {old:?}, update: {new:?})",
            installed.display()
        );
        anyhow::bail!(SIGNED_DIFFERENTLY)
    }
    if !verify(candidate, old.trim()) {
        log::warn!("update refused: signature doesn't verify against {old:?}");
        anyhow::bail!(BAD_SIGNATURE)
    }
    Ok(())
}

/// `codesign` arguments that check the whole bundle's signature, nested code included, and
/// test it against `requirement` (`-R=` takes the requirement as text, not a file).
fn verify_args(requirement: &str) -> Vec<String> {
    vec![
        "--verify".into(),
        "--deep".into(),
        "--strict".into(),
        format!("-R={requirement}"),
    ]
}

fn verify_signature(app: &Path, requirement: &str) -> bool {
    Command::new("codesign")
        .args(verify_args(requirement))
        .arg(app)
        .output()
        .is_ok_and(|out| out.status.success())
}

/// The requirement as `codesign -d -r-` prints it after "designated => ", the same line
/// scripts/check-requirement.sh reads.
fn parse_requirement(codesign: &str) -> Option<String> {
    codesign
        .lines()
        .find_map(|l| l.strip_prefix("designated => "))
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
}

fn read_requirement(app: &Path) -> Option<String> {
    let out = Command::new("codesign")
        .args(["-d", "-r-"])
        .arg(app)
        .output()
        .ok()?;
    parse_requirement(&format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// `Silas.app/Contents/MacOS/silas` -> `Silas.app`; None outside a bundle (a dev build).
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .nth(3)
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// Unpack the downloaded `Silas.app.tar.gz` into a scratch folder and compare its signature
/// with the running bundle's before anything is replaced.
fn check_candidate(archive: &[u8]) -> anyhow::Result<()> {
    let installed = bundle_of(&std::env::current_exe()?).ok_or_else(|| {
        anyhow::anyhow!("Silas isn't running from an app bundle, so the update was not installed.")
    })?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("silas-update-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let result = unpack(archive, &dir)
        .and_then(|app| guard_requirement(&installed, &app, read_requirement, verify_signature));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The updater installs every entry at its path minus the first component, whatever that
/// component is named. So the bundle checked here is the bundle installed only when every
/// entry sits under one plain `.app` root: a decoy `Silas.app` beside a `payload/` would pass
/// the check while the payload overwrote it on install.
fn unpack(archive: &[u8], dir: &Path) -> anyhow::Result<PathBuf> {
    let mut root: Option<OsString> = None;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut parts = path.components();
        let first = match parts.next() {
            Some(Component::Normal(name))
                if Path::new(name).extension().is_some_and(|e| e == "app") =>
            {
                name
            }
            _ => anyhow::bail!(
                "the update holds something outside its app bundle: {}",
                path.display()
            ),
        };
        anyhow::ensure!(
            parts.all(|c| matches!(c, Component::Normal(_))),
            "the update holds an unsafe path: {}",
            path.display()
        );
        let r = root.get_or_insert_with(|| first.to_owned());
        anyhow::ensure!(
            r.as_os_str() == first,
            "the update holds more than one app bundle: {} and {}",
            Path::new(r).display(),
            Path::new(first).display()
        );
        anyhow::ensure!(entry.unpack_in(dir)?, "couldn't unpack {}", path.display());
    }
    root.map(|r| dir.join(r))
        .ok_or_else(|| anyhow::anyhow!("the update holds no app bundle"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STABLE: &str = r#"identifier "com.baney.silas" and anchor apple generic and certificate leaf[subject.CN] = "Apple Development: Donovan Baney (ABCDE12345)" and certificate 1[field.1.2.840.113635.100.6.2.1] /* exists */"#;
    // The two nightlies from 2026-11-02 on rectory1.
    const ADHOC_OLD: &str = r#"cdhash H"9dd2baebd941b5cfe914bac60b1e3b7a0575ae51""#;
    const ADHOC_NEW: &str = r#"cdhash H"25781cbae1c8e035c5aa3d43e86d3a650470b3e3""#;

    fn reader<'a>(
        installed: Option<&'a str>,
        candidate: Option<&'a str>,
    ) -> impl Fn(&Path) -> Option<String> + 'a {
        move |p: &Path| {
            let found = if p == Path::new("/Applications/Silas.app") {
                installed
            } else {
                candidate
            };
            found.map(str::to_string)
        }
    }

    fn guard(installed: Option<&str>, candidate: Option<&str>) -> anyhow::Result<()> {
        guard_requirement(
            Path::new("/Applications/Silas.app"),
            Path::new("/tmp/new/Silas.app"),
            reader(installed, candidate),
            |_, _| true,
        )
    }

    #[test]
    fn a_matching_requirement_with_a_broken_signature_is_refused() {
        let asked = std::cell::RefCell::new(Vec::new());
        let e = guard_requirement(
            Path::new("/Applications/Silas.app"),
            Path::new("/tmp/new/Silas.app"),
            reader(Some(STABLE), Some(STABLE)),
            |p, req| {
                asked.borrow_mut().push((p.to_path_buf(), req.to_string()));
                false
            },
        )
        .unwrap_err();
        assert_eq!(e.to_string(), BAD_SIGNATURE);
        // The candidate is tested against the installed build's requirement.
        assert_eq!(
            asked.into_inner(),
            vec![(PathBuf::from("/tmp/new/Silas.app"), STABLE.to_string())]
        );
    }

    #[test]
    fn a_changed_requirement_is_refused_before_verifying() {
        let e = guard_requirement(
            Path::new("/Applications/Silas.app"),
            Path::new("/tmp/new/Silas.app"),
            reader(Some(STABLE), Some(ADHOC_NEW)),
            |_, _| panic!("verify ran on a mismatched requirement"),
        )
        .unwrap_err();
        assert_eq!(e.to_string(), SIGNED_DIFFERENTLY);
    }

    #[test]
    fn verification_is_strict_deep_and_tests_the_requirement() {
        assert_eq!(
            verify_args(STABLE),
            vec![
                "--verify".to_string(),
                "--deep".into(),
                "--strict".into(),
                format!("-R={STABLE}"),
            ]
        );
    }

    /// A `.tar.gz` with names written as given, so tests can build layouts `tar::Builder`
    /// would normalize away. A name ending in `/` is a folder.
    fn targz(entries: &[&str]) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for name in entries {
            let mut h = tar::Header::new_old();
            h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
            let dir = name.ends_with('/');
            let body: &[u8] = if dir { b"" } else { name.as_bytes() };
            h.set_entry_type(if dir {
                tar::EntryType::Directory
            } else {
                tar::EntryType::Regular
            });
            h.set_mode(0o755);
            h.set_size(body.len() as u64);
            h.set_cksum();
            b.append(&h, body).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn unpack_layout(entries: &[&str]) -> anyhow::Result<PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("silas-unpack-test-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let result = unpack(&targz(entries), &dir).map(|app| {
            assert!(app.join("Contents/MacOS/silas").is_file());
            app.strip_prefix(&dir).unwrap().to_path_buf()
        });
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn a_single_app_bundle_unpacks() {
        let got = unpack_layout(&[
            "Silas.app/",
            "Silas.app/Contents/",
            "Silas.app/Contents/MacOS/silas",
            "Silas.app/Contents/Info.plist",
        ]);
        assert_eq!(got.unwrap(), PathBuf::from("Silas.app"));
    }

    #[test]
    fn a_decoy_bundle_beside_a_payload_is_refused() {
        // The updater strips the first component of every entry, so `payload/Contents/...`
        // would overwrite the checked `Silas.app/Contents/...` on install.
        assert!(unpack_layout(&[
            "Silas.app/Contents/MacOS/silas",
            "payload/Contents/MacOS/silas",
        ])
        .is_err());
        assert!(unpack_layout(&[
            "Silas.app/Contents/MacOS/silas",
            "Other.app/Contents/MacOS/silas",
        ])
        .is_err());
    }

    #[test]
    fn roots_that_install_elsewhere_are_refused() {
        for bad in [
            "./Silas.app/Contents/MacOS/silas",
            "/Silas.app/Contents/MacOS/silas",
            "Silas.app/../Contents/MacOS/silas",
            "payload/Contents/MacOS/silas",
        ] {
            assert!(unpack_layout(&[bad]).is_err(), "{bad} unpacked");
        }
        assert!(unpack_layout(&[]).is_err());
    }

    #[test]
    fn the_same_identity_installs() {
        assert!(same_designated_requirement(STABLE, STABLE));
        assert!(guard(Some(STABLE), Some(STABLE)).is_ok());
    }

    #[test]
    fn the_same_adhoc_build_installs() {
        assert!(same_designated_requirement(ADHOC_NEW, ADHOC_NEW));
        assert!(guard(Some(ADHOC_NEW), Some(ADHOC_NEW)).is_ok());
    }

    #[test]
    fn a_different_adhoc_build_is_refused() {
        assert!(!same_designated_requirement(ADHOC_OLD, ADHOC_NEW));
        let e = guard(Some(ADHOC_OLD), Some(ADHOC_NEW)).unwrap_err();
        assert_eq!(e.to_string(), SIGNED_DIFFERENTLY);
    }

    #[test]
    fn an_adhoc_update_over_a_signed_install_is_refused() {
        assert!(!same_designated_requirement(STABLE, ADHOC_NEW));
        assert!(guard(Some(STABLE), Some(ADHOC_NEW)).is_err());
        assert!(guard(Some(ADHOC_NEW), Some(STABLE)).is_err());
    }

    #[test]
    fn a_changed_identity_is_refused() {
        let other = STABLE.replace("ABCDE12345", "ZZZZZ99999");
        assert!(guard(Some(STABLE), Some(&other)).is_err());
    }

    #[test]
    fn an_unreadable_signature_is_refused() {
        assert!(!same_designated_requirement("", ""));
        assert!(guard(None, Some(STABLE)).is_err());
        assert!(guard(Some(STABLE), None).is_err());
        assert!(guard(None, None).is_err());
    }

    #[test]
    fn requirement_is_read_from_codesign_output() {
        let out = format!(
            "Executable=/Applications/Silas.app/Contents/MacOS/silas\ndesignated => {ADHOC_NEW}\n"
        );
        assert_eq!(parse_requirement(&out).as_deref(), Some(ADHOC_NEW));
        assert_eq!(
            parse_requirement("code object is not signed at all\n"),
            None
        );
        assert_eq!(parse_requirement("designated => \n"), None);
    }

    #[test]
    fn the_bundle_is_found_from_the_executable() {
        assert_eq!(
            bundle_of(Path::new("/Applications/Silas.app/Contents/MacOS/silas")),
            Some(PathBuf::from("/Applications/Silas.app"))
        );
        assert_eq!(
            bundle_of(Path::new("/home/me/silas/target/debug/silas")),
            None
        );
    }

    #[test]
    fn the_refusal_reads_plainly() {
        assert!(!SIGNED_DIFFERENTLY.contains(['\u{2014}', '\u{2013}']));
        assert!(!BAD_SIGNATURE.contains(['\u{2014}', '\u{2013}']));
    }
}
