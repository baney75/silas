//! Where the sync key lives (docs/sync.md section 1): the macOS Keychain, the Windows Credential
//! Manager or the Linux Secret Service, and on Linux without a Secret Service a `0600` file in the
//! app-data folder. Never settings.json, never a log line.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use zeroize::Zeroizing;

use crate::sync::SyncKey;

pub trait KeyStore: Send + Sync + 'static {
    fn load(&self) -> Option<SyncKey>;
    fn store(&self, key: &SyncKey) -> Result<()>;
    fn delete(&self) -> Result<()>;
}

/// The key as 64 hex characters in a file only this user can read.
pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn exists(&self) -> bool {
        self.path.exists()
    }
}

fn decode(text: &str) -> Option<SyncKey> {
    let bytes = Zeroizing::new(data_encoding::HEXLOWER_PERMISSIVE.decode(text.trim().as_bytes()).ok()?);
    Some(SyncKey::from_bytes(bytes.as_slice().try_into().ok()?))
}

fn encode(key: &SyncKey) -> Zeroizing<String> {
    Zeroizing::new(data_encoding::HEXLOWER.encode(key.bytes()))
}

impl KeyStore for FileKeyStore {
    fn load(&self) -> Option<SyncKey> {
        let text = Zeroizing::new(std::fs::read_to_string(&self.path).ok()?);
        decode(&text)
    }

    fn store(&self, key: &SyncKey) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        write_private(&tmp, encode(key).as_bytes()).context("writing the sync key file")?;
        std::fs::rename(&tmp, &self.path).context("moving the sync key file into place")?;
        Ok(())
    }

    fn delete(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// Create the file readable and writable by this user only, before any byte is written.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// The OS keychain. On Linux, a `0600` file stands in when no Secret Service answers.
pub struct OsKeyStore {
    service: String,
    account: String,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fallback: FileKeyStore,
}

impl OsKeyStore {
    /// One entry per data folder, so two copies of Silas with their own folders keep their own keys.
    pub fn new(data_dir: &Path) -> Self {
        use sha2::Digest;
        let tag = crate::sync::hex(&sha2::Sha256::digest(data_dir.to_string_lossy().as_bytes())[..4]);
        Self {
            service: "Silas Sync".into(),
            account: format!("sync-key-{tag}"),
            fallback: FileKeyStore::new(data_dir.join("sync-key")),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.account).context("opening the keychain")
    }
}

impl KeyStore for OsKeyStore {
    fn load(&self) -> Option<SyncKey> {
        #[cfg(target_os = "linux")]
        if self.fallback.exists() {
            return self.fallback.load();
        }
        match self.entry().and_then(|e| e.get_password().map_err(Into::into)) {
            Ok(text) => decode(&Zeroizing::new(text)),
            Err(e) => {
                if !matches!(e.downcast_ref::<keyring::Error>(), Some(keyring::Error::NoEntry)) {
                    log::warn!("sync: couldn't read the sync key from the keychain: {e}");
                }
                None
            }
        }
    }

    fn store(&self, key: &SyncKey) -> Result<()> {
        let saved = self.entry().and_then(|e| e.set_password(&encode(key)).map_err(Into::into));
        match saved {
            Ok(()) => {
                // A key in the keychain makes an older fallback file stale.
                let _ = self.fallback.delete();
                Ok(())
            }
            #[cfg(target_os = "linux")]
            Err(e) => {
                log::info!("sync: no Secret Service ({e}); keeping the sync key in a private file");
                self.fallback.store(key)
            }
            #[cfg(not(target_os = "linux"))]
            Err(e) => Err(e),
        }
    }

    fn delete(&self) -> Result<()> {
        let _ = self.fallback.delete();
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => {
                log::warn!("sync: couldn't remove the sync key from the keychain: {e}");
                // Without a Secret Service there is nothing in it to remove.
                if cfg!(target_os = "linux") {
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_store_keeps_the_key_private_and_forgets_it() {
        let dir = std::env::temp_dir().join(format!("silas-sync-key-{}", crate::sync::new_device_id()));
        let store = FileKeyStore::new(dir.join("sync-key"));
        assert!(store.load().is_none());
        let key = SyncKey::generate();
        store.store(&key).unwrap();
        assert_eq!(store.load(), Some(key.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("sync-key")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // The file never holds the pairing code, only hex.
        let text = std::fs::read_to_string(dir.join("sync-key")).unwrap();
        assert!(!text.contains(&key.code()[..9]));
        store.delete().unwrap();
        assert!(store.load().is_none());
        store.delete().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
