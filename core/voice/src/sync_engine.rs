//! The sync engine (docs/sync.md sections 4 and 5): a TCP listener, DNS-SD advertise and browse,
//! peers by address, and a synced folder, all feeding [`crate::sync`]'s merge. It knows the
//! dictionary only through [`DictStore`], so the app, tests and the headless harness each hand it
//! their own.
//!
//! Every input from the network is bounded before it is trusted: at most [`MAX_INBOUND`]
//! connections at once, a 4-byte length checked against [`MAX_FRAME`] before any body is read,
//! one deadline for the whole frame, and a frame that doesn't open with the group's key is
//! dropped with nothing sent back.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::dictionary::Dictionary;
use crate::sync::{
    clip, now_ms, read_entries, sync_dictionary, Changes, FrameKey, SeenDevice, StateMessage, SyncKey,
    SyncState, MAX_FRAME, MIN_FRAME, PORT, SERVICE,
};
use crate::sync_key::KeyStore;

/// Inbound connections served at once; more are closed on accept.
pub const MAX_INBOUND: usize = 4;
/// A local change waits this long for more before it goes out.
pub const DEBOUNCE: Duration = Duration::from_secs(2);
/// Every known peer is synced this often.
pub const RESYNC: Duration = Duration::from_secs(10 * 60);
/// The synced folder is read this often even when the watcher is quiet.
pub const FOLDER_POLL: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const FOLDER_NAME: &str = "Silas Sync";
const MAX_DEVICES: usize = 64;

/// The live dictionary, however the host keeps it.
pub trait DictStore: Send + Sync + 'static {
    /// Run `f` on the dictionary while holding its lock; save it when `f` returns true.
    fn update(&self, f: &mut dyn FnMut(&mut Dictionary) -> bool);
}

/// A dictionary.json at a path, for tests and the headless harness.
pub struct FileDict {
    path: PathBuf,
    dict: Mutex<Dictionary>,
}

impl FileDict {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let dict = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, dict: Mutex::new(dict) }
    }

    pub fn get(&self) -> Dictionary {
        self.dict.lock().clone()
    }

    /// Change the dictionary and save it, as the app does after a user's edit.
    pub fn edit(&self, f: impl FnOnce(&mut Dictionary)) {
        let mut d = self.dict.lock();
        f(&mut d);
        self.save(&d);
    }

    fn save(&self, d: &Dictionary) {
        if let Ok(bytes) = serde_json::to_vec_pretty(d) {
            let tmp = self.path.with_extension("tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}

impl DictStore for FileDict {
    fn update(&self, f: &mut dyn FnMut(&mut Dictionary) -> bool) {
        let mut d = self.dict.lock();
        if f(&mut d) {
            self.save(&d);
        }
    }
}

pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct Options {
    /// Where sync.json lives (the app-data folder, next to dictionary.json).
    pub dir: PathBuf,
    pub keys: Arc<dyn KeyStore>,
    pub dict: Arc<dyn DictStore>,
    /// Events for the UI: `sync` with a [`SyncStatus`].
    pub emit: Emit,
    /// `None`: 47823, or any free port if it is taken. `Some(0)`: any free port.
    pub port: Option<u16>,
    pub bind: IpAddr,
    /// Advertise and browse over DNS-SD. Tests turn it off.
    pub discovery: bool,
    /// The whole of one frame must arrive within this.
    pub frame_timeout: Duration,
    /// Shown to other devices when the person has not named this one.
    pub default_name: String,
}

impl Options {
    pub fn new(dir: PathBuf, keys: Arc<dyn KeyStore>, dict: Arc<dyn DictStore>) -> Self {
        Self {
            dir,
            keys,
            dict,
            emit: Arc::new(|_, _| {}),
            port: None,
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            discovery: true,
            frame_timeout: Duration::from_secs(15),
            default_name: computer_name(),
        }
    }
}

/// The computer's name, without a trailing `.local`.
pub fn computer_name() -> String {
    let name = sysinfo::System::host_name().unwrap_or_default();
    let name = name.trim_end_matches(".local").trim();
    if name.is_empty() {
        "Silas".into()
    } else {
        clip(name, 63)
    }
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

// ---------------------------------------------------------------- what the UI sees

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub paired: bool,
    pub device_id: String,
    pub name: String,
    pub platform: &'static str,
    /// The port the listener got, while sync runs.
    pub port: Option<u16>,
    pub peers: Vec<String>,
    pub folder: Option<String>,
    pub devices: Vec<DeviceView>,
    pub last: Option<LastSync>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub last_synced: u64,
    pub via: String,
}

/// The last sync, for "Synced with Donovan's iPhone just now (3 new words)".
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    pub with: String,
    pub at: u64,
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub code: String,
    pub url: String,
}

// ---------------------------------------------------------------- framing

/// 4-byte big-endian length, then the frame.
pub fn write_frame(w: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    if frame.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large"));
    }
    w.write_all(&(frame.len() as u32).to_be_bytes())?;
    w.write_all(frame)?;
    w.flush()
}

fn read_full(s: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> io::Result<()> {
    let mut off = 0;
    while off < buf.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "peer too slow"));
        }
        s.set_read_timeout(Some(left))?;
        match s.read(&mut buf[off..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => off += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "peer too slow"))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One length-prefixed frame, all of it before `deadline`. A length over [`MAX_FRAME`] (or too
/// short to be a frame) fails before any of the body is read, and the buffer grows only as bytes
/// arrive, so a peer that claims 8 MiB and sends nothing costs nothing.
pub fn read_frame(s: &mut TcpStream, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    read_full(s, &mut len, deadline)?;
    let len = u32::from_be_bytes(len) as usize;
    if !(MIN_FRAME..=MAX_FRAME).contains(&len) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame length out of bounds"));
    }
    let mut buf = Vec::new();
    while buf.len() < len {
        let start = buf.len();
        buf.resize(start + (len - start).min(64 * 1024), 0);
        read_full(s, &mut buf[start..], deadline)?;
    }
    Ok(buf)
}

// ---------------------------------------------------------------- the engine

enum Cmd {
    /// The dictionary was saved by the person.
    Changed,
    /// A merge changed the entries: the folder copy is stale.
    Dirty,
    Peer { device: String, addrs: Vec<SocketAddr> },
    FolderChanged,
    Now,
    Stop,
}

struct Runtime {
    alive: Arc<AtomicBool>,
    tx: mpsc::Sender<Cmd>,
    addr: SocketAddr,
    /// The thread that owns the listener. Joined on stop, so the port is free again by the time
    /// a restart binds it.
    accept: std::thread::JoinHandle<()>,
    mdns: Option<mdns_sd::ServiceDaemon>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Runtime {
    fn stop(self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::Stop);
        if let Some(m) = self.mdns {
            let _ = m.shutdown();
        }
        let _ = self.accept.join();
    }
}

pub struct SyncEngine {
    o: Options,
    me: Weak<SyncEngine>,
    st: Mutex<SyncState>,
    key: Mutex<Option<SyncKey>>,
    rt: Mutex<Option<Runtime>>,
    last: Mutex<Option<LastSync>>,
    inbound: AtomicUsize,
    /// mtime and length of each folder file already merged.
    folder_seen: Mutex<HashMap<PathBuf, (SystemTime, u64)>>,
    /// Hash of what this device last wrote to the folder.
    folder_written: Mutex<Option<[u8; 32]>>,
    /// When a stranger was last logged, so a noisy one can't fill the log.
    stranger_logged: Mutex<Option<Instant>>,
}

impl SyncEngine {
    pub fn new(o: Options) -> Arc<Self> {
        let st = SyncState::load(&o.dir);
        // Only touch the keychain when sync was set up: reading it can mean a system prompt.
        let key = if st.paired { o.keys.load() } else { None };
        if st.paired && key.is_none() {
            log::warn!("sync: this device was paired but its sync key is gone; pair it again");
        }
        Arc::new_cyclic(|me| Self {
            o,
            me: me.clone(),
            st: Mutex::new(st),
            key: Mutex::new(key),
            rt: Mutex::new(None),
            last: Mutex::new(None),
            inbound: AtomicUsize::new(0),
            folder_seen: Mutex::new(HashMap::new()),
            folder_written: Mutex::new(None),
            stranger_logged: Mutex::new(None),
        })
    }

    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("the engine is alive while its methods run")
    }

    // ------------------------------------------------------------ status and control

    pub fn status(&self) -> SyncStatus {
        let st = self.st.lock();
        let mut devices: Vec<DeviceView> = st
            .devices
            .iter()
            .map(|(id, d)| DeviceView {
                id: id.clone(),
                name: d.name.clone(),
                platform: d.platform.clone(),
                last_synced: d.last_synced,
                via: d.via.clone(),
            })
            .collect();
        devices.sort_by_key(|d| std::cmp::Reverse(d.last_synced));
        SyncStatus {
            enabled: st.enabled,
            paired: self.key.lock().is_some(),
            device_id: st.device.clone(),
            name: self.name_of(&st),
            platform: platform(),
            port: self.rt.lock().as_ref().map(|r| r.addr.port()),
            peers: st.peers.clone(),
            folder: st.folder.clone(),
            devices,
            last: self.last.lock().clone(),
        }
    }

    fn name_of(&self, st: &SyncState) -> String {
        if st.name.trim().is_empty() {
            self.o.default_name.clone()
        } else {
            st.name.clone()
        }
    }

    fn changed(&self) {
        (self.o.emit)("sync", serde_json::to_value(self.status()).unwrap_or(Value::Null));
    }

    fn edit_state(&self, f: impl FnOnce(&mut SyncState)) {
        let mut st = self.st.lock();
        f(&mut st);
        st.save(&self.o.dir);
    }

    /// Start the network and folder side if sync is on and paired. Safe to call again.
    pub fn start(&self) {
        let on = self.st.lock().enabled && self.key.lock().is_some();
        if on && self.rt.lock().is_none() {
            if let Err(e) = self.start_runtime() {
                log::error!("sync: couldn't start: {e:#}");
            }
        }
        self.changed();
    }

    pub fn stop(&self) {
        let rt = self.rt.lock().take();
        if let Some(rt) = rt {
            rt.stop();
            log::info!("sync: stopped");
        }
    }

    fn restart(&self) {
        self.stop();
        self.start();
    }

    pub fn set_enabled(&self, on: bool) -> Result<SyncStatus> {
        if on && self.key.lock().is_none() {
            self.install_key(SyncKey::generate())?;
        }
        self.edit_state(|st| st.enabled = on);
        self.restart();
        Ok(self.status())
    }

    /// The code and pairing URL, shown only when the person asks. Makes a key, and turns sync
    /// on, the first time.
    pub fn pairing_code(&self) -> Result<PairingCode> {
        if self.key.lock().is_none() || !self.st.lock().enabled {
            self.set_enabled(true)?;
        }
        let key = self.key.lock().clone().ok_or_else(|| anyhow!("no sync key"))?;
        Ok(PairingCode { code: key.code(), url: key.url() })
    }

    /// Join a group with a code typed in or a `silas://pair?k=` URL.
    pub fn join(&self, code: &str) -> Result<SyncStatus> {
        let code = code.trim();
        let code = code.strip_prefix("silas://pair?k=").unwrap_or(code);
        let key = SyncKey::parse(code).ok_or_else(|| anyhow!("That code isn't right. Check it and try again."))?;
        let same = self.key.lock().as_ref() == Some(&key);
        if !same {
            self.install_key(key)?;
            self.edit_state(|st| st.devices.clear());
        }
        self.edit_state(|st| st.enabled = true);
        self.restart();
        self.sync_now();
        Ok(self.status())
    }

    /// A fresh key: every other device must pair again.
    pub fn new_key(&self) -> Result<SyncStatus> {
        self.install_key(SyncKey::generate())?;
        self.edit_state(|st| {
            st.devices.clear();
            st.enabled = true;
        });
        self.restart();
        Ok(self.status())
    }

    /// Delete the key and stop. The dictionary stays as it is.
    pub fn leave(&self) -> Result<SyncStatus> {
        self.stop();
        self.o.keys.delete().context("removing the sync key")?;
        *self.key.lock() = None;
        *self.last.lock() = None;
        self.edit_state(|st| {
            st.enabled = false;
            st.paired = false;
            st.devices.clear();
        });
        self.changed();
        Ok(self.status())
    }

    fn install_key(&self, key: SyncKey) -> Result<()> {
        self.o.keys.store(&key).context("saving the sync key")?;
        *self.key.lock() = Some(key);
        *self.folder_written.lock() = None;
        self.folder_seen.lock().clear();
        self.edit_state(|st| st.paired = true);
        Ok(())
    }

    pub fn set_name(&self, name: &str) -> Result<SyncStatus> {
        let name = clip(name.trim(), 63);
        self.edit_state(|st| st.name = name);
        self.restart();
        Ok(self.status())
    }

    pub fn add_peer(&self, addr: &str) -> Result<SyncStatus> {
        let addr = addr.trim();
        if addr.is_empty() || addr.len() > 255 || addr.chars().any(char::is_whitespace) {
            bail!("Type a name or address, like rectory1:{PORT}.");
        }
        if let Some((_, port)) = addr.rsplit_once(':').filter(|_| !addr.ends_with(']')) {
            if port.parse::<u16>().is_err() && !addr.contains('[') && addr.matches(':').count() == 1 {
                bail!("{port:?} isn't a port number.");
            }
        }
        self.edit_state(|st| {
            if !st.peers.iter().any(|p| p == addr) {
                st.peers.push(addr.to_string());
            }
        });
        self.sync_now();
        self.changed();
        Ok(self.status())
    }

    pub fn remove_peer(&self, addr: &str) -> Result<SyncStatus> {
        self.edit_state(|st| st.peers.retain(|p| p != addr));
        self.changed();
        Ok(self.status())
    }

    pub fn set_folder(&self, folder: Option<&str>) -> Result<SyncStatus> {
        let folder = match folder.map(str::trim).filter(|f| !f.is_empty()) {
            None => None,
            Some(f) => {
                let path = expand_home(f);
                if !path.is_dir() {
                    bail!("There's no folder at {}.", path.display());
                }
                Some(path.to_string_lossy().into_owned())
            }
        };
        self.edit_state(|st| st.folder = folder);
        *self.folder_written.lock() = None;
        self.folder_seen.lock().clear();
        self.restart();
        Ok(self.status())
    }

    /// Sync every known peer and the folder now.
    pub fn sync_now(&self) {
        if let Some(rt) = self.rt.lock().as_ref() {
            let _ = rt.tx.send(Cmd::Now);
        }
    }

    /// The person changed the dictionary. Cheap: the work happens on the sync thread.
    pub fn local_changed(&self) {
        if let Some(rt) = self.rt.lock().as_ref() {
            let _ = rt.tx.send(Cmd::Changed);
        }
    }

    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.rt.lock().as_ref().map(|r| r.addr)
    }

    // ------------------------------------------------------------ merging

    fn frame_key(&self) -> Result<FrameKey> {
        Ok(self.key.lock().as_ref().ok_or_else(|| anyhow!("sync has no key"))?.frame_key())
    }

    /// Stamp the person's edits since the last round. Returns how many keys changed.
    fn absorb_local(&self) -> usize {
        let mut g = self.st.lock();
        let st = &mut *g;
        let mut stamped = 0;
        self.o.dict.update(&mut |d| {
            stamped = sync_dictionary(d, &mut st.entries, None, &mut st.clock, &st.device, now_ms()).0.stamped;
            false
        });
        if stamped > 0 {
            st.save(&self.o.dir);
        }
        stamped
    }

    fn state_frame(&self) -> Result<Vec<u8>> {
        self.absorb_local();
        let fk = self.frame_key()?;
        let st = self.st.lock();
        Ok(StateMessage::new(&st.device, &self.name_of(&st), platform(), &st.entries).seal(&fk))
    }

    /// Merge a peer's state into ours and the dictionary.
    fn take_in(&self, msg: StateMessage, via: &str) -> Changes {
        let incoming = read_entries(&msg.entries);
        let (round, name) = {
            let mut g = self.st.lock();
            let st = &mut *g;
            let mut round = Default::default();
            self.o.dict.update(&mut |d| {
                let (r, changed) =
                    sync_dictionary(d, &mut st.entries, Some(&incoming), &mut st.clock, &st.device, now_ms());
                round = r;
                changed
            });
            let name = clip(&msg.name, 64);
            st.devices.insert(
                msg.device.clone(),
                SeenDevice { name: name.clone(), platform: clip(&msg.platform, 16), last_synced: now_ms(), via: via.into() },
            );
            while st.devices.len() > MAX_DEVICES {
                let oldest = st.devices.iter().min_by_key(|(_, d)| d.last_synced).map(|(k, _)| k.clone());
                st.devices.remove(&oldest.expect("not empty"));
            }
            st.save(&self.o.dir);
            (round, name)
        };
        let c = round.changes;
        log::info!(
            "sync: synced with {name} over the {via} ({} new, {} changed, {} removed)",
            c.added,
            c.changed,
            c.removed
        );
        *self.last.lock() =
            Some(LastSync { with: name, at: now_ms(), added: c.added, changed: c.changed, removed: c.removed });
        if round.merged > 0 {
            if let Some(rt) = self.rt.lock().as_ref() {
                let _ = rt.tx.send(Cmd::Dirty);
            }
        }
        self.changed();
        c
    }

    fn stranger(&self, from: &str) {
        let mut last = self.stranger_logged.lock();
        if last.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
            *last = Some(Instant::now());
            log::warn!("sync: a device with a different sync key ({from}); ignored");
        }
    }

    // ------------------------------------------------------------ transport A: TCP

    /// Connect to a peer, swap states, merge. The frame goes out on its own thread while the
    /// reply is read, so two large states can't wedge each other.
    pub fn sync_with(&self, addr: SocketAddr) -> Result<Changes> {
        let out = self.state_frame()?;
        let fk = self.frame_key()?;
        let mut s = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).with_context(|| format!("connecting to {addr}"))?;
        let _ = s.set_nodelay(true);
        s.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let mut w = s.try_clone()?;
        let deadline = Instant::now() + self.o.frame_timeout;
        let reply = std::thread::scope(|sc| {
            let writer = sc.spawn(move || write_frame(&mut w, &out));
            let reply = read_frame(&mut s, deadline);
            let wrote = writer.join().unwrap_or_else(|_| Err(io::ErrorKind::Other.into()));
            wrote.and(reply)
        });
        let reply = reply.with_context(|| format!("syncing with {addr}"))?;
        let Some(plain) = fk.open(&reply) else {
            self.stranger(&addr.to_string());
            bail!("{addr} has a different sync key");
        };
        let msg = StateMessage::parse(&plain).ok_or_else(|| anyhow!("{addr} sent something that isn't a state"))?;
        if msg.device == self.st.lock().device {
            bail!("{addr} is this device");
        }
        Ok(self.take_in(msg, "network"))
    }

    /// Serve one inbound connection: read, check, merge, answer. A stranger gets nothing back.
    fn serve(&self, mut s: TcpStream, peer: SocketAddr) -> Result<()> {
        s.set_nonblocking(false)?;
        let _ = s.set_nodelay(true);
        s.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let frame = read_frame(&mut s, Instant::now() + self.o.frame_timeout)?;
        let fk = self.frame_key()?;
        let Some(plain) = fk.open(&frame) else {
            self.stranger(&peer.ip().to_string());
            return Ok(());
        };
        let Some(msg) = StateMessage::parse(&plain) else { return Ok(()) };
        if msg.device == self.st.lock().device {
            return Ok(());
        }
        self.take_in(msg, "network");
        write_frame(&mut s, &self.state_frame()?)?;
        Ok(())
    }

    fn sync_any(&self, addrs: &[SocketAddr]) -> bool {
        for a in addrs {
            match self.sync_with(*a) {
                Ok(_) => return true,
                Err(e) => log::debug!("sync: {e:#}"),
            }
        }
        false
    }

    fn sync_peer(&self, peer: &str) {
        let with_port = if peer.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
            peer.to_string()
        } else {
            format!("{peer}:{PORT}")
        };
        match with_port.to_socket_addrs() {
            Ok(addrs) => {
                let mut addrs: Vec<SocketAddr> = addrs.collect();
                addrs.sort_by_key(|a| !a.is_ipv4());
                if !self.sync_any(&addrs) {
                    log::info!("sync: couldn't reach {peer}");
                }
            }
            Err(e) => log::info!("sync: couldn't look up {peer}: {e}"),
        }
    }

    fn start_runtime(&self) -> Result<()> {
        let key = self.key.lock().clone().ok_or_else(|| anyhow!("no key"))?;
        let listener = match self.o.port {
            Some(p) => TcpListener::bind((self.o.bind, p))?,
            None => TcpListener::bind((self.o.bind, PORT)).or_else(|_| TcpListener::bind((self.o.bind, 0)))?,
        };
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let alive = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();

        let me = self.me.clone();
        let live = alive.clone();
        let accept =
            std::thread::Builder::new().name("sync-listen".into()).spawn(move || accept_loop(listener, me, live))?;

        let (device, name, folder) = {
            let st = self.st.lock();
            (st.device.clone(), self.name_of(&st), st.folder.clone())
        };
        let mdns = if self.o.discovery {
            start_mdns(addr.port(), &key.group_id(), &device, &name, tx.clone(), alive.clone())
                .map_err(|e| log::warn!("sync: no local network discovery: {e:#}"))
                .ok()
        } else {
            None
        };
        let watcher = folder.and_then(|f| {
            let dir = Path::new(&f).join(FOLDER_NAME);
            let _ = std::fs::create_dir_all(&dir);
            watch_folder(&dir, &device, tx.clone()).map_err(|e| log::info!("sync: folder watching is off ({e}); polling")).ok()
        });

        let engine = self.arc();
        let live = alive.clone();
        if let Err(e) = std::thread::Builder::new().name("sync".into()).spawn(move || engine.worker(rx, live)) {
            alive.store(false, Ordering::SeqCst);
            let _ = accept.join();
            return Err(e.into());
        }
        log::info!("sync: listening on port {}", addr.port());
        *self.rt.lock() = Some(Runtime { alive, tx, addr, accept, mdns, _watcher: watcher });
        Ok(())
    }

    fn worker(self: Arc<Self>, rx: mpsc::Receiver<Cmd>, alive: Arc<AtomicBool>) {
        let own = self.st.lock().device.clone();
        let mut found: HashMap<String, Vec<SocketAddr>> = HashMap::new();
        let mut push_due: Option<Instant> = None;
        let mut folder_due: Option<Instant> = Some(Instant::now());
        let mut resync_due = Instant::now();
        let mut poll_due = Instant::now() + FOLDER_POLL;
        while alive.load(Ordering::SeqCst) {
            let next = [push_due, folder_due, Some(resync_due), Some(poll_due)].into_iter().flatten().min().expect("some");
            let wait = next.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(Cmd::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Cmd::Changed) => {
                    if self.absorb_local() > 0 {
                        push_due = Some(Instant::now() + DEBOUNCE);
                    }
                }
                Ok(Cmd::Dirty) => {
                    folder_due.get_or_insert(Instant::now() + DEBOUNCE);
                }
                Ok(Cmd::Peer { device, addrs }) => {
                    let new = found.get(&device) != Some(&addrs);
                    found.insert(device.clone(), addrs.clone());
                    // The smaller id connects, so two devices that see each other sync once.
                    if new && own < device {
                        self.sync_any(&addrs);
                    }
                }
                Ok(Cmd::FolderChanged) => {
                    let soon = Instant::now() + Duration::from_secs(1);
                    folder_due = Some(folder_due.map_or(soon, |d| d.min(soon)));
                }
                Ok(Cmd::Now) => {
                    self.round(&own, &found, true);
                    folder_due = None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if !alive.load(Ordering::SeqCst) {
                break;
            }
            let now = Instant::now();
            // After a local change only this side knows, so it reaches every peer it can.
            if push_due.is_some_and(|d| d <= now) {
                push_due = None;
                self.round(&own, &found, true);
                folder_due = None;
            }
            if resync_due <= now {
                resync_due = now + RESYNC;
                self.round(&own, &found, false);
                folder_due = None;
            }
            if folder_due.is_some_and(|d| d <= now) || poll_due <= now {
                folder_due = None;
                poll_due = now + FOLDER_POLL;
                self.folder_round();
            }
        }
    }

    /// Sync found peers (all of them, or only those this side connects to), peers by address
    /// and the folder.
    fn round(&self, own: &str, found: &HashMap<String, Vec<SocketAddr>>, all: bool) {
        for (device, addrs) in found {
            if all || own < device.as_str() {
                self.sync_any(addrs);
            }
        }
        let peers = self.st.lock().peers.clone();
        for p in peers {
            self.sync_peer(&p);
        }
        self.folder_round();
    }

    // ------------------------------------------------------------ transport B: a synced folder

    /// Read every other device's file in the folder, then write ours if it changed.
    pub fn sync_folder(&self) -> Result<()> {
        let Some(folder) = self.st.lock().folder.clone() else { return Ok(()) };
        let dir = Path::new(&folder).join(FOLDER_NAME);
        std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
        let own = self.st.lock().device.clone();
        let fk = self.frame_key()?;
        let own_file = dir.join(format!("{own}.silas"));
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path == own_file || path.extension().is_none_or(|e| e != "silas") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || meta.len() as usize > MAX_FRAME {
                continue;
            }
            let stamp = (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len());
            if self.folder_seen.lock().get(&path) == Some(&stamp) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            self.folder_seen.lock().insert(path.clone(), stamp);
            let Some(plain) = fk.open(&bytes) else {
                self.stranger(&path.file_name().unwrap_or_default().to_string_lossy());
                continue;
            };
            if let Some(msg) = StateMessage::parse(&plain).filter(|m| m.device != own) {
                self.take_in(msg, "folder");
            }
        }
        // Write ours when it changed since the last write, or someone removed it.
        let (digest, frame) = {
            self.absorb_local();
            let st = self.st.lock();
            let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(&(&st.entries, self.name_of(&st)))?).into();
            (digest, StateMessage::new(&st.device, &self.name_of(&st), platform(), &st.entries).seal(&fk))
        };
        if *self.folder_written.lock() != Some(digest) || !own_file.exists() {
            let tmp = dir.join(format!(".{own}.silas.tmp"));
            std::fs::write(&tmp, &frame).with_context(|| format!("writing {}", tmp.display()))?;
            std::fs::rename(&tmp, &own_file)?;
            *self.folder_written.lock() = Some(digest);
        }
        Ok(())
    }

    fn folder_round(&self) {
        if let Err(e) = self.sync_folder() {
            log::warn!("sync: the sync folder: {e:#}");
        }
    }
}

fn accept_loop(listener: TcpListener, me: Weak<SyncEngine>, alive: Arc<AtomicBool>) {
    while alive.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((s, peer)) => {
                let Some(engine) = me.upgrade() else { return };
                if engine.inbound.fetch_add(1, Ordering::SeqCst) >= MAX_INBOUND {
                    engine.inbound.fetch_sub(1, Ordering::SeqCst);
                    drop(s);
                    continue;
                }
                let spawned = std::thread::Builder::new().name("sync-serve".into()).spawn({
                    let engine = engine.clone();
                    move || {
                        if let Err(e) = engine.serve(s, peer) {
                            log::debug!("sync: from {peer}: {e:#}");
                        }
                        engine.inbound.fetch_sub(1, Ordering::SeqCst);
                    }
                });
                if spawned.is_err() {
                    engine.inbound.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                log::warn!("sync: accept failed: {e}");
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn start_mdns(
    port: u16,
    group: &str,
    device: &str,
    name: &str,
    tx: mpsc::Sender<Cmd>,
    alive: Arc<AtomicBool>,
) -> Result<mdns_sd::ServiceDaemon> {
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
    let mdns = ServiceDaemon::new()?;
    let host = format!("silas-{}.local.", &device[..12.min(device.len())]);
    let props = [("v", "1"), ("g", group), ("d", device)];
    let instance = if name.is_empty() { "Silas" } else { name };
    let info = ServiceInfo::new(SERVICE, instance, &host, "", port, &props[..])?.enable_addr_auto();
    mdns.register(info)?;
    let rx = mdns.browse(SERVICE)?;
    let (group, own) = (group.to_string(), device.to_string());
    std::thread::Builder::new().name("sync-browse".into()).spawn(move || {
        while alive.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    let g = info.get_property_val_str("g");
                    let d = info.get_property_val_str("d").unwrap_or_default();
                    let valid = d.len() == 32 && d.bytes().all(|b| b.is_ascii_hexdigit());
                    if g != Some(group.as_str()) || d == own || !valid {
                        continue;
                    }
                    let mut addrs: Vec<SocketAddr> = info
                        .get_addresses()
                        .iter()
                        .map(|ip| ip.to_ip_addr())
                        .filter(|ip| match ip {
                            IpAddr::V4(_) => true,
                            IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
                        })
                        .map(|ip| SocketAddr::new(ip, info.get_port()))
                        .collect();
                    addrs.sort_by_key(|a| (!a.is_ipv4(), *a));
                    if !addrs.is_empty() {
                        let _ = tx.send(Cmd::Peer { device: d.to_string(), addrs });
                    }
                }
                Err(mdns_sd::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }
        }
    })?;
    Ok(mdns)
}

fn watch_folder(dir: &Path, own: &str, tx: mpsc::Sender<Cmd>) -> notify::Result<notify::RecommendedWatcher> {
    use notify::Watcher;
    let own = format!("{own}.silas");
    let tx = Mutex::new(tx);
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        let theirs = ev.paths.iter().any(|p| {
            p.extension().is_some_and(|e| e == "silas") && p.file_name().is_some_and(|n| n.to_string_lossy() != own)
        });
        if theirs {
            let _ = tx.lock().send(Cmd::FolderChanged);
        }
    })?;
    w.watch(dir, notify::RecursiveMode::NonRecursive)?;
    Ok(w)
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/").or_else(|| (path == "~").then_some("")) {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncKey;
    use crate::sync_key::FileKeyStore;
    use std::net::TcpListener;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("silas-sync-{tag}-{}", crate::sync::new_device_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, _) = l.accept().unwrap();
        (c, s)
    }

    #[test]
    fn an_oversize_length_is_refused_before_any_body_is_read() {
        let (mut c, mut s) = pair();
        c.write_all(&((MAX_FRAME as u32) + 1).to_be_bytes()).unwrap();
        let t = Instant::now();
        let e = read_frame(&mut s, Instant::now() + Duration::from_secs(5)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        assert!(t.elapsed() < Duration::from_secs(1));
        // Too short to be a frame, too.
        let (mut c, mut s) = pair();
        c.write_all(&3u32.to_be_bytes()).unwrap();
        assert_eq!(read_frame(&mut s, Instant::now() + Duration::from_secs(5)).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_slow_peer_is_cut_off_at_the_deadline() {
        let (mut c, mut s) = pair();
        // Claims a 1 KiB frame, then trickles a byte every 100 ms: each read succeeds, the whole
        // frame never arrives in time.
        let writer = std::thread::spawn(move || {
            let _ = c.write_all(&1024u32.to_be_bytes());
            for _ in 0..30 {
                if c.write_all(&[0]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let t = Instant::now();
        let e = read_frame(&mut s, Instant::now() + Duration::from_millis(500)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_millis(1500), "{:?}", t.elapsed());
        drop(s);
        writer.join().unwrap();
    }

    #[test]
    fn frames_round_trip() {
        let (mut c, mut s) = pair();
        let frame = vec![7u8; 100_000];
        let f2 = frame.clone();
        let w = std::thread::spawn(move || write_frame(&mut c, &f2).unwrap());
        assert_eq!(read_frame(&mut s, Instant::now() + Duration::from_secs(5)).unwrap(), frame);
        w.join().unwrap();
    }

    fn engine(tag: &str, key: Option<&SyncKey>) -> (Arc<SyncEngine>, Arc<FileDict>, PathBuf) {
        engine_on(tag, key, 0)
    }

    fn engine_on(tag: &str, key: Option<&SyncKey>, port: u16) -> (Arc<SyncEngine>, Arc<FileDict>, PathBuf) {
        let dir = temp(tag);
        let keys = Arc::new(FileKeyStore::new(dir.join("sync-key")));
        let dict = Arc::new(FileDict::open(dir.join("dictionary.json")));
        let mut o = Options::new(dir.clone(), keys, dict.clone());
        o.port = Some(port);
        o.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
        o.discovery = false;
        o.frame_timeout = Duration::from_millis(800);
        o.default_name = tag.into();
        let e = SyncEngine::new(o);
        match key {
            Some(k) => {
                e.join(&k.code()).unwrap();
            }
            None => {
                e.set_enabled(true).unwrap();
            }
        }
        (e, dict, dir)
    }

    #[test]
    fn a_strangers_frame_changes_nothing_and_gets_no_answer() {
        let (e, dict, dir) = engine("host", None);
        let addr = e.listen_addr().unwrap();
        let stranger = SyncKey::generate();
        let mut entries = crate::sync::Entries::new();
        let mut d = Dictionary::default();
        d.add_term("Intruder", "user");
        sync_dictionary(&mut d, &mut entries, None, &mut Default::default(), "ffffffffffffffffffffffffffffffff", 1);
        let frame = StateMessage::new("ffffffffffffffffffffffffffffffff", "x", "linux", &entries).seal(&stranger.frame_key());
        let mut s = TcpStream::connect(addr).unwrap();
        write_frame(&mut s, &frame).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut buf = [0u8; 1];
        assert_eq!(s.read(&mut buf).unwrap_or(0), 0, "a stranger gets nothing back");
        assert!(!dict.get().terms.iter().any(|t| t.text == "Intruder"));
        assert!(e.status().devices.is_empty());
        e.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn connections_beyond_the_limit_are_closed_at_once() {
        let (e, _dict, dir) = engine("limit", None);
        let addr = e.listen_addr().unwrap();
        // Hold every slot with a peer that sends nothing.
        let held: Vec<TcpStream> = (0..MAX_INBOUND).map(|_| TcpStream::connect(addr).unwrap()).collect();
        std::thread::sleep(Duration::from_millis(300));
        let mut extra = TcpStream::connect(addr).unwrap();
        extra.set_read_timeout(Some(Duration::from_millis(600))).unwrap();
        let mut buf = [0u8; 1];
        let t = Instant::now();
        let got = extra.read(&mut buf);
        assert!(matches!(got, Ok(0)) || got.as_ref().is_err_and(|e| e.kind() == io::ErrorKind::ConnectionReset), "{got:?}");
        assert!(t.elapsed() < Duration::from_millis(550), "closed at once, not timed out");
        drop(held);
        // The idle ones are cut at the frame deadline and the slots free up.
        std::thread::sleep(Duration::from_millis(1200));
        assert_eq!(e.inbound.load(Ordering::SeqCst), 0);
        e.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn leaving_forgets_the_key_and_joining_needs_a_real_code() {
        let (e, _d, dir) = engine("leave", None);
        assert!(e.status().paired);
        assert!(e.join("not a code").is_err());
        let code = e.pairing_code().unwrap();
        assert_eq!(code.url, format!("silas://pair?k={}", code.code.replace('-', "")));
        e.leave().unwrap();
        let s = e.status();
        assert!(!s.paired && !s.enabled && s.port.is_none());
        // Comes back paired after a restart only if it was.
        let keys = Arc::new(FileKeyStore::new(dir.join("sync-key")));
        assert!(keys.load().is_none());
        // The status never carries the code.
        let (e2, _d2, dir2) = engine("leave2", None);
        let text = serde_json::to_string(&e2.status()).unwrap();
        let code2 = e2.pairing_code().unwrap().code;
        assert!(!text.contains(&code2) && !text.contains(&code2.replace('-', "")));
        e2.stop();
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn a_restart_gets_the_same_port_back() {
        // A fixed port, as 47823 is: peers that know it must still reach this device after a
        // rename, a new folder or a toggle, each of which restarts the listener.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (e, _d, dir) = engine_on("port", None, port);
        assert_eq!(e.listen_addr().map(|a| a.port()), Some(port));
        e.set_name("Rectory").unwrap();
        assert_eq!(e.listen_addr().map(|a| a.port()), Some(port), "after a rename");
        e.set_enabled(false).unwrap();
        e.set_enabled(true).unwrap();
        assert_eq!(e.listen_addr().map(|a| a.port()), Some(port), "after a toggle");
        e.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn peers_by_address_are_checked() {
        let (e, _d, dir) = engine("peers", None);
        assert!(e.add_peer("rectory1:47823").is_ok());
        assert!(e.add_peer("rectory1").is_ok());
        assert!(e.add_peer("rectory1:http").is_err());
        assert!(e.add_peer("two words").is_err());
        assert_eq!(e.status().peers, vec!["rectory1:47823", "rectory1"]);
        e.remove_peer("rectory1").unwrap();
        assert_eq!(e.status().peers, vec!["rectory1:47823"]);
        e.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
