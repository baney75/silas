//! Two sync engines in one process, each with its own data folder, as two devices would be.
//! Over TCP on 127.0.0.1 (no mDNS) and through a shared folder (docs/sync.md sections 4 and 5).

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use polycarp_voice::dictionary::Dictionary;
use polycarp_voice::sync::{new_device_id, now_ms, SyncKey};
use polycarp_voice::sync_engine::{FileDict, Options, SyncEngine};
use polycarp_voice::sync_key::FileKeyStore;

struct Device {
    engine: Arc<SyncEngine>,
    dict: Arc<FileDict>,
    dir: PathBuf,
}

impl Drop for Device {
    fn drop(&mut self) {
        self.engine.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn device(name: &str, key: &SyncKey, folder: Option<&PathBuf>) -> Device {
    let dir = std::env::temp_dir().join(format!("silas-sync-it-{name}-{}", new_device_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dict = Arc::new(FileDict::open(dir.join("dictionary.json")));
    let mut o = Options::new(dir.clone(), Arc::new(FileKeyStore::new(dir.join("sync-key"))), dict.clone());
    o.port = Some(0);
    o.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    o.discovery = false;
    o.default_name = name.into();
    let engine = SyncEngine::new(o);
    if let Some(f) = folder {
        engine.set_folder(Some(f.to_str().unwrap())).unwrap();
    }
    engine.join(&key.code()).unwrap();
    Device { engine, dict, dir }
}

fn has_term(d: &Dictionary, text: &str) -> bool {
    d.terms.iter().any(|t| t.text == text)
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(15);
    while !ok() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn two_engines_sync_both_ways_over_tcp() {
    let key = SyncKey::generate();
    let a = device("Alpha", &key, None);
    let b = device("Beta", &key, None);
    let b_addr = b.engine.listen_addr().expect("B listens");

    // A word added on A reaches B, and B's word reaches A, in one exchange.
    a.dict.edit(|d| {
        d.add_term("Parish1", "user");
    });
    b.dict.edit(|d| {
        d.add_correction("chapel one", "Chapel1", "user");
    });
    a.engine.sync_with(b_addr).unwrap();
    assert!(has_term(&b.dict.get(), "Parish1"));
    assert!(a.dict.get().corrections.iter().any(|c| c.from == "chapel one" && c.to == "Chapel1"));
    // B's state on disk is the merged one: a restart keeps it.
    let on_disk: Dictionary = serde_json::from_str(&std::fs::read_to_string(b.dir.join("dictionary.json")).unwrap()).unwrap();
    assert!(has_term(&on_disk, "Parish1"));
    let names: Vec<String> = a.engine.status().devices.iter().map(|d| d.name.clone()).collect();
    assert_eq!(names, vec!["Beta"]);
    let last = b.engine.status().last.unwrap();
    assert_eq!((last.with.as_str(), last.added), ("Alpha", 1));

    // The app's path: B lists A as a peer by address. Adding it syncs at once; let that finish.
    let a_addr = a.engine.listen_addr().unwrap();
    let t0 = now_ms();
    b.engine.add_peer(&a_addr.to_string()).unwrap();
    wait_for("B's first sync with its new peer", || b.engine.status().last.is_some_and(|l| l.at >= t0));

    // A delete on B goes out on its own after the 2 s debounce.
    b.dict.edit(|d| d.remove_term("Parish1"));
    let edited = Instant::now();
    b.engine.local_changed();
    wait_for("the delete to reach A", || !has_term(&a.dict.get(), "Parish1"));
    assert!(edited.elapsed() >= Duration::from_millis(1900), "debounced: {:?}", edited.elapsed());

    // "Sync now" pulls A's newer word without waiting.
    a.dict.edit(|d| {
        d.add_term("Rectory1", "user");
    });
    b.engine.sync_now();
    wait_for("A's new word to reach B", || has_term(&b.dict.get(), "Rectory1"));
}

#[test]
fn engines_with_different_keys_never_merge() {
    let a = device("Alpha", &SyncKey::generate(), None);
    let b = device("Beta", &SyncKey::generate(), None);
    a.dict.edit(|d| {
        d.add_term("Secret", "user");
    });
    let err = a.engine.sync_with(b.engine.listen_addr().unwrap()).unwrap_err();
    assert!(!format!("{err:#}").is_empty());
    assert!(!has_term(&b.dict.get(), "Secret"));
    assert!(b.engine.status().devices.is_empty());
}

#[test]
fn two_engines_sync_through_a_folder() {
    let key = SyncKey::generate();
    let folder = std::env::temp_dir().join(format!("silas-sync-folder-{}", new_device_id()));
    std::fs::create_dir_all(&folder).unwrap();
    let a = device("Alpha", &key, Some(&folder));
    let b = device("Beta", &key, Some(&folder));

    a.dict.edit(|d| {
        d.add_term("Parish1", "user");
    });
    a.engine.sync_folder().unwrap();
    b.engine.sync_folder().unwrap();
    assert!(has_term(&b.dict.get(), "Parish1"));
    let files: Vec<String> = std::fs::read_dir(folder.join("Silas Sync"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 2, "one frame per device and no temp files: {files:?}");
    assert!(files.iter().all(|f| f.ends_with(".silas") && !f.starts_with('.')));
    // The files are frames, not plain JSON.
    for f in &files {
        let bytes = std::fs::read(folder.join("Silas Sync").join(f)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("Parish1"));
    }

    // The other way, through the debounce and A's watcher: no explicit calls. First let the
    // writes the merge above scheduled land, so they can't carry the edit early.
    std::thread::sleep(Duration::from_millis(2500));
    let edited = Instant::now();
    b.dict.edit(|d| {
        d.add_term("Chapel1", "user");
    });
    b.engine.local_changed();
    wait_for("B's word to reach A through the folder", || has_term(&a.dict.get(), "Chapel1"));
    assert!(edited.elapsed() >= Duration::from_millis(1900), "debounced: {:?}", edited.elapsed());
    assert_eq!(a.engine.status().devices[0].via, "folder");

    // A stranger's file in the folder is ignored.
    std::fs::write(folder.join("Silas Sync").join("ffffffffffffffffffffffffffffffff.silas"), SyncKey::generate().frame_key().seal(b"{}")).unwrap();
    a.engine.sync_folder().unwrap();
    assert_eq!(a.engine.status().devices.len(), 1);
    let _ = std::fs::remove_dir_all(folder);
}
