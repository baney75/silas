//! Dictionary sync, the pure part: pairing codes, the group id, frames, the hybrid logical
//! clock, merge, and the projection between synced entries and the [`Dictionary`]. No sockets,
//! no threads, no keychain: those live in `sync_engine` and `sync_key`. The contract is
//! `docs/sync.md`; `docs/sync-vectors.json` is checked by the tests below.
//!
//! The projection works on the dictionary's JSON, not its struct, so a kind the struct gains
//! later (snippets) syncs without touching this file, and a kind it lacks is left alone.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::dictionary::Dictionary;

/// DNS-SD service type (section 4).
pub const SERVICE: &str = "_silas-sync._tcp.local.";
/// The listener's port when it is free.
pub const PORT: u16 = 47823;
/// The largest frame either transport accepts.
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
/// The protocol version this build writes.
pub const VERSION: u32 = 1;

const AAD: &[u8] = b"silas-sync v1";
const FRAME_INFO: &[u8] = b"silas-sync v1 frame";
const GROUP_PREFIX: &[u8] = b"silas-sync-group";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// The smallest frame that could open: a nonce, a tag and at least `{}`.
pub const MIN_FRAME: usize = NONCE_LEN + TAG_LEN + 2;

// ---------------------------------------------------------------- randomness

/// Bytes from the OS CSPRNG.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the OS random source failed");
    b
}

/// A fresh device id: 16 random bytes as 32 lower-case hex characters.
pub fn new_device_id() -> String {
    hex(&random_bytes::<16>())
}

pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

// ---------------------------------------------------------------- the sync key

/// The 32-byte secret every device in a group holds. Its `Debug` never shows the bytes, and the
/// bytes are wiped when it drops.
#[derive(Clone)]
pub struct SyncKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncKey(hidden)")
    }
}

impl PartialEq for SyncKey {
    fn eq(&self, other: &Self) -> bool {
        // Both keys are this device's own (the one it holds and one it was given or held
        // before), so a timing difference tells no one anything.
        self.0[..] == other.0[..]
    }
}

impl SyncKey {
    pub fn generate() -> Self {
        Self(Zeroizing::new(random_bytes::<32>()))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The pairing code: RFC 4648 base32, no padding, upper case, groups of 4 joined by `-`.
    pub fn code(&self) -> String {
        let plain = Zeroizing::new(data_encoding::BASE32_NOPAD.encode(&self.0[..]));
        let groups: Vec<&str> = plain
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).expect("base32 is ASCII"))
            .collect();
        groups.join("-")
    }

    /// Ignores case, spaces and dashes; reads `0` as `O`, `1` as `L` and `8` as `B`.
    pub fn parse(code: &str) -> Option<Self> {
        let cleaned: Zeroizing<String> = Zeroizing::new(
            code.chars()
                .filter(|c| !c.is_whitespace() && *c != '-')
                .map(|c| match c.to_ascii_uppercase() {
                    '0' => 'O',
                    '1' => 'L',
                    '8' => 'B',
                    other => other,
                })
                .collect(),
        );
        if cleaned.len() != 52 {
            return None;
        }
        let bytes = Zeroizing::new(data_encoding::BASE32_NOPAD.decode(cleaned.as_bytes()).ok()?);
        let key: [u8; 32] = bytes.as_slice().try_into().ok()?;
        Some(Self::from_bytes(key))
    }

    /// The pairing URL the desktop shows as a QR code.
    pub fn url(&self) -> String {
        format!("silas://pair?k={}", self.code().replace('-', ""))
    }

    /// First 8 bytes of SHA-256("silas-sync-group" || key), as hex.
    pub fn group_id(&self) -> String {
        let mut h = Sha256::new();
        h.update(GROUP_PREFIX);
        h.update(&self.0[..]);
        hex(&h.finalize()[..8])
    }

    pub fn frame_key(&self) -> FrameKey {
        let hk = hkdf::Hkdf::<Sha256>::new(None, &self.0[..]);
        let mut okm = Zeroizing::new([0u8; 32]);
        hk.expand(FRAME_INFO, &mut okm[..]).expect("32 bytes is a valid HKDF length");
        FrameKey(okm)
    }
}

/// The ChaCha20-Poly1305 key frames are sealed with.
pub struct FrameKey(Zeroizing<[u8; 32]>);

impl FrameKey {
    #[cfg(test)]
    fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.0[..]))
    }

    /// `nonce || ChaCha20-Poly1305(key, nonce, plaintext, aad)` with a random nonce.
    pub fn seal(&self, plaintext: &[u8]) -> Vec<u8> {
        self.seal_with_nonce(plaintext, random_bytes::<NONCE_LEN>())
    }

    pub fn seal_with_nonce(&self, plaintext: &[u8], nonce: [u8; NONCE_LEN]) -> Vec<u8> {
        let sealed = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: AAD })
            .expect("ChaCha20-Poly1305 seals any length under 256 GiB");
        let mut out = Vec::with_capacity(NONCE_LEN + sealed.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        out
    }

    /// The plaintext, or `None` for a frame made with another key (or damaged on the way).
    pub fn open(&self, frame: &[u8]) -> Option<Vec<u8>> {
        if frame.len() < NONCE_LEN + TAG_LEN || frame.len() > MAX_FRAME {
            return None;
        }
        let (nonce, sealed) = frame.split_at(NONCE_LEN);
        self.cipher()
            .decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad: AAD })
            .ok()
    }
}

// ---------------------------------------------------------------- keys and norm

/// Unicode NFC, trim, collapse whitespace runs to one space, full lower-case mapping.
pub fn norm(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    nfc.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// What a synced entry is, by its key's prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Term,
    Correction,
    Snippet,
    Filler,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Term, Kind::Correction, Kind::Snippet, Kind::Filler];

    pub fn prefix(self) -> &'static str {
        match self {
            Kind::Term => "t:",
            Kind::Correction => "c:",
            Kind::Snippet => "s:",
            Kind::Filler => "f:",
        }
    }

    pub fn of_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| key.starts_with(k.prefix()))
    }

    /// The array in the dictionary's JSON.
    fn field(self) -> &'static str {
        match self {
            Kind::Term => "terms",
            Kind::Correction => "corrections",
            Kind::Snippet => "snippets",
            Kind::Filler => "fillers",
        }
    }

    /// The field the key is made from.
    fn name(self) -> &'static str {
        match self {
            Kind::Term => "text",
            Kind::Correction => "from",
            Kind::Snippet => "trigger",
            Kind::Filler => "word",
        }
    }

    /// The fields that sync. Everything else on an item (uses, count) stays on the device.
    fn synced(self) -> &'static [&'static str] {
        match self {
            Kind::Term => &["text", "added", "source"],
            Kind::Correction => &["from", "to", "added", "source"],
            Kind::Snippet => &["trigger", "text", "added"],
            Kind::Filler => &["word"],
        }
    }

    /// Fields a new item needs before the dictionary struct will read it.
    fn required(self) -> &'static [&'static str] {
        match self {
            Kind::Term => &["text", "added", "source"],
            Kind::Correction => &["from", "to", "added", "source"],
            Kind::Snippet => &["trigger", "text", "added"],
            Kind::Filler => &["word"],
        }
    }

    /// The key for an entry named `name`, or None when its normal form is empty. A snippet's
    /// key is the form Silas matches spoken triggers in (`snippets::match_key`), after NFC.
    pub fn key_for(self, name: &str) -> Option<String> {
        let n = match self {
            Kind::Snippet => crate::snippets::match_key(&name.nfc().collect::<String>()),
            _ => norm(name),
        };
        (!n.is_empty()).then(|| format!("{}{n}", self.prefix()))
    }
}

// ---------------------------------------------------------------- hybrid logical clock

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hlc {
    pub ms: u64,
    pub c: u32,
    pub d: String,
}

impl Ord for Hlc {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.ms, self.c, self.d.as_bytes()).cmp(&(o.ms, o.c, o.d.as_bytes()))
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The largest `(ms, c)` this device has made or seen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Clock {
    pub ms: u64,
    pub c: u32,
}

impl Clock {
    /// A local change at wall time `now_ms`: the stamp to use, which becomes the clock.
    pub fn tick(&mut self, now_ms: u64) -> Clock {
        let ms = now_ms.max(self.ms);
        let c = if ms == self.ms { self.c.saturating_add(1) } else { 0 };
        *self = Clock { ms, c };
        *self
    }

    pub fn observe(&mut self, h: &Hlc) {
        *self = (*self).max(Clock { ms: h.ms, c: h.c });
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------- entries and merge

/// `{ "v": <value or null>, "h": <hlc> }`. `v: None` is a tombstone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub v: Option<Value>,
    pub h: Hlc,
}

pub type Entries = BTreeMap<String, Entry>;

/// The entry that wins for one key: the larger hlc. Equal hlcs come from one device's one stamp,
/// so they carry the same value; the value's text breaks the tie anyway so merge stays commutative
/// even against a misbehaving peer.
pub fn wins<'a>(a: &'a Entry, b: &'a Entry) -> &'a Entry {
    match a.h.cmp(&b.h) {
        Ordering::Greater => a,
        Ordering::Less => b,
        Ordering::Equal => {
            let rank = |e: &Entry| e.v.as_ref().map(|v| v.to_string());
            if rank(a) >= rank(b) {
                a
            } else {
                b
            }
        }
    }
}

/// Merge `incoming` into `into`. Returns the keys whose entry changed.
pub fn merge(into: &mut Entries, incoming: &Entries) -> Vec<String> {
    let mut changed = Vec::new();
    for (k, theirs) in incoming {
        match into.get(k) {
            Some(ours) if wins(ours, theirs) == ours => {}
            _ => {
                into.insert(k.clone(), theirs.clone());
                changed.push(k.clone());
            }
        }
    }
    changed
}

/// Whether a value is one this build can put in the dictionary under `key`: an object whose
/// synced fields are strings and whose name field normalizes to the key.
fn usable(kind: Kind, key: &str, v: &Value) -> bool {
    let Some(obj) = v.as_object() else { return false };
    let fields_ok = kind
        .synced()
        .iter()
        .all(|f| obj.get(*f).is_none_or(Value::is_string));
    fields_ok
        && obj
            .get(kind.name())
            .and_then(Value::as_str)
            .and_then(|n| kind.key_for(n))
            .is_some_and(|k| k == key)
}

/// Read entries from a peer: kinds this build knows, well formed, keys of sane length. Anything
/// else is skipped one entry at a time, so one bad entry never costs the rest.
pub fn read_entries(raw: &Map<String, Value>) -> Entries {
    raw.iter()
        .filter(|(k, _)| k.len() <= 4096 && Kind::of_key(k).is_some())
        .filter_map(|(k, v)| {
            let e: Entry = serde_json::from_value(v.clone()).ok()?;
            let shape_ok = e.v.as_ref().is_none_or(Value::is_object) && e.h.d.len() <= 64;
            shape_ok.then(|| (k.clone(), e))
        })
        .collect()
}

// ---------------------------------------------------------------- projection

/// The kinds this build's dictionary has a field for. Snippets sync once the struct has them.
pub fn kinds_in(dict: &Value) -> Vec<Kind> {
    Kind::ALL
        .into_iter()
        .filter(|k| dict.get(k.field()).is_some_and(Value::is_array))
        .collect()
}

fn item_value(kind: Kind, item: &Value) -> Option<Value> {
    match kind {
        Kind::Filler => Some(serde_json::json!({ "word": item.as_str()? })),
        _ => {
            let obj = item.as_object()?;
            let mut v = Map::new();
            for f in kind.synced() {
                if let Some(s) = obj.get(*f).filter(|s| s.is_string()) {
                    v.insert((*f).to_string(), s.clone());
                }
            }
            Some(Value::Object(v))
        }
    }
}

/// The synced view of a dictionary's JSON: key → synced fields, for the kinds it has.
pub fn project_from(dict: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for kind in kinds_in(dict) {
        for item in dict[kind.field()].as_array().into_iter().flatten() {
            let Some(v) = item_value(kind, item) else { continue };
            let Some(key) = v.get(kind.name()).and_then(Value::as_str).and_then(|n| kind.key_for(n)) else {
                continue;
            };
            out.insert(key, v);
        }
    }
    out
}

fn same_synced(kind: Kind, a: &Value, b: &Value) -> bool {
    kind.synced().iter().all(|f| a.get(*f) == b.get(*f))
}

/// Stamp every difference between the dictionary (`current`, from [`project_from`]) and the live
/// entries: added or changed items get their value, removed ones a tombstone. Unknown fields on an
/// entry (from a newer build) are kept. Returns how many keys were stamped.
pub fn absorb(
    entries: &mut Entries,
    current: &BTreeMap<String, Value>,
    kinds: &[Kind],
    clock: &mut Clock,
    device: &str,
    now: u64,
) -> usize {
    let mut stamped = 0;
    let mut stamp = |entries: &mut Entries, key: &str, v: Option<Value>| {
        let t = clock.tick(now);
        entries.insert(key.to_string(), Entry { v, h: Hlc { ms: t.ms, c: t.c, d: device.to_string() } });
        stamped += 1;
    };
    for (key, cur) in current {
        let Some(kind) = Kind::of_key(key) else { continue };
        let live = entries.get(key).and_then(|e| e.v.as_ref());
        match live {
            Some(v) if same_synced(kind, v, cur) => {}
            _ => {
                let mut next = live.and_then(Value::as_object).cloned().unwrap_or_default();
                for (f, x) in cur.as_object().into_iter().flatten() {
                    next.insert(f.clone(), x.clone());
                }
                stamp(entries, key, Some(Value::Object(next)));
            }
        }
    }
    let gone: Vec<String> = entries
        .iter()
        .filter(|(k, e)| {
            let Some(kind) = Kind::of_key(k) else { return false };
            kinds.contains(&kind)
                && e.v.as_ref().is_some_and(|v| usable(kind, k, v))
                && !current.contains_key(*k)
        })
        .map(|(k, _)| k.clone())
        .collect();
    for key in gone {
        stamp(entries, &key, None);
    }
    stamped
}

/// What a projection changed in the dictionary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

impl Changes {
    pub fn any(&self) -> bool {
        self.added + self.changed + self.removed > 0
    }
}

/// Bring the dictionary's JSON in line with the live entries: add what's new, update synced
/// fields, remove tombstoned items. Local-only fields of surviving items are kept.
pub fn project_into(dict: &mut Value, entries: &Entries) -> Changes {
    let mut ch = Changes::default();
    let now_rfc = chrono::Utc::now().to_rfc3339();
    for kind in kinds_in(dict) {
        let desired: BTreeMap<&str, &Value> = entries
            .iter()
            .filter(|(k, _)| Kind::of_key(k) == Some(kind))
            .filter_map(|(k, e)| e.v.as_ref().filter(|v| usable(kind, k, v)).map(|v| (k.as_str(), v)))
            .collect();
        let items = dict[kind.field()].as_array_mut().expect("kinds_in checked the array");
        let mut seen = std::collections::BTreeSet::new();
        let mut kept = Vec::with_capacity(items.len());
        for mut item in std::mem::take(items) {
            let key = item_value(kind, &item)
                .and_then(|v| v.get(kind.name()).and_then(Value::as_str).and_then(|n| kind.key_for(n)));
            let Some(key) = key else {
                kept.push(item); // nothing sync can name: leave it be
                continue;
            };
            let Some(want) = desired.get(key.as_str()) else {
                ch.removed += 1;
                continue;
            };
            seen.insert(key);
            if update_item(kind, &mut item, want) {
                ch.changed += 1;
            }
            kept.push(item);
        }
        for (key, want) in &desired {
            if !seen.contains(*key) {
                kept.push(new_item(kind, want, &now_rfc));
                ch.added += 1;
            }
        }
        *items = kept;
    }
    ch
}

fn update_item(kind: Kind, item: &mut Value, want: &Value) -> bool {
    if kind == Kind::Filler {
        let w = want["word"].clone();
        return if *item != w {
            *item = w;
            true
        } else {
            false
        };
    }
    let Some(obj) = item.as_object_mut() else { return false };
    let mut changed = false;
    for f in kind.synced() {
        if let Some(x) = want.get(*f) {
            if obj.get(*f) != Some(x) {
                obj.insert((*f).to_string(), x.clone());
                changed = true;
            }
        }
    }
    changed
}

fn new_item(kind: Kind, want: &Value, now_rfc: &str) -> Value {
    if kind == Kind::Filler {
        return want["word"].clone();
    }
    let mut obj = Map::new();
    for f in kind.synced() {
        if let Some(x) = want.get(*f) {
            obj.insert((*f).to_string(), x.clone());
        }
    }
    for f in kind.required() {
        if !obj.contains_key(*f) {
            let fill = match *f {
                "added" => now_rfc.to_string(),
                "source" => "user".to_string(),
                "to" | "text" => String::new(),
                _ => continue,
            };
            obj.insert((*f).to_string(), Value::String(fill));
        }
    }
    Value::Object(obj)
}

/// What one round did to a dictionary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Round {
    /// Local edits stamped before merging.
    pub stamped: usize,
    /// Keys whose entry the incoming state replaced.
    pub merged: usize,
    pub changes: Changes,
}

/// The whole step on a live dictionary, as the engine runs it under the dictionary's lock: stamp
/// local edits, merge `incoming`, project back. Returns `None` when the dictionary is unchanged
/// (nothing to save), else the round's counts.
pub fn sync_dictionary(
    dict: &mut Dictionary,
    entries: &mut Entries,
    incoming: Option<&Entries>,
    clock: &mut Clock,
    device: &str,
    now: u64,
) -> (Round, bool) {
    let Ok(mut v) = serde_json::to_value(&*dict) else {
        return (Round::default(), false);
    };
    let kinds = kinds_in(&v);
    let stamped = absorb(entries, &project_from(&v), &kinds, clock, device, now);
    let mut round = Round { stamped, ..Round::default() };
    if let Some(inc) = incoming {
        for e in inc.values() {
            clock.observe(&e.h);
        }
        round.merged = merge(entries, inc).len();
    }
    if round.merged == 0 {
        return (round, false);
    }
    let changes = project_into(&mut v, entries);
    if !changes.any() {
        return (round, false);
    }
    match serde_json::from_value::<Dictionary>(v) {
        Ok(next) => {
            round.changes = changes;
            *dict = next;
            (round, true)
        }
        Err(e) => {
            log::warn!("sync: the merged dictionary didn't read back ({e}); keeping the old one");
            (round, false)
        }
    }
}

// ---------------------------------------------------------------- messages

/// The one message type of v1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub v: u32,
    pub device: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub sent: u64,
    #[serde(default)]
    pub entries: Map<String, Value>,
}

impl StateMessage {
    pub fn new(device: &str, name: &str, platform: &str, entries: &Entries) -> Self {
        let entries = entries
            .iter()
            .map(|(k, e)| (k.clone(), serde_json::to_value(e).expect("entries serialize")))
            .collect();
        Self {
            kind: "state".into(),
            v: VERSION,
            device: device.into(),
            name: name.into(),
            platform: platform.into(),
            sent: now_ms(),
            entries,
        }
    }

    /// Parse an opened frame. `None` for anything that is not a state message.
    pub fn parse(plaintext: &[u8]) -> Option<Self> {
        let m: StateMessage = serde_json::from_slice(plaintext).ok()?;
        (m.kind == "state" && m.v >= 1 && !m.device.is_empty() && m.device.len() <= 64).then_some(m)
    }

    pub fn seal(&self, key: &FrameKey) -> Vec<u8> {
        key.seal(&serde_json::to_vec(self).expect("a state message serializes"))
    }
}

/// Shorten to at most `max` bytes on a character boundary (names from peers end up in the UI).
pub fn clip(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].chars().filter(|c| !c.is_control()).collect()
}

// ---------------------------------------------------------------- state file

/// A device this one has synced with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SeenDevice {
    pub name: String,
    pub platform: String,
    /// Unix ms of the last sync with it.
    pub last_synced: u64,
    /// "network" or "folder".
    pub via: String,
}

/// Everything sync keeps on disk next to dictionary.json, except the key (that is in the
/// keychain). `entries` is the last synced projection of the dictionary, tombstones included.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncState {
    pub device: String,
    pub clock: Clock,
    pub entries: Entries,
    pub enabled: bool,
    /// A key was stored. Lets startup skip the keychain entirely when sync was never set up.
    pub paired: bool,
    /// This device's name as others see it. Empty: the computer's name.
    pub name: String,
    /// `host:port` peers for networks where multicast doesn't reach.
    pub peers: Vec<String>,
    pub folder: Option<String>,
    pub devices: BTreeMap<String, SeenDevice>,
}

pub const STATE_FILE: &str = "sync.json";

impl SyncState {
    /// Read the state from `dir`, making a device id the first time.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(STATE_FILE);
        let mut st: SyncState = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("sync: {STATE_FILE} is unreadable ({e}); keeping a copy and starting fresh");
                let _ = std::fs::copy(&path, path.with_extension("broken.json"));
                SyncState::default()
            }),
            Err(_) => SyncState::default(),
        };
        if st.device.len() != 32 || !st.device.bytes().all(|b| b.is_ascii_hexdigit()) {
            st.device = new_device_id();
            st.save(dir);
        }
        st
    }

    /// Temp file then rename, so a crash never leaves half a file.
    pub fn save(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join(STATE_FILE);
        let tmp = path.with_extension("tmp");
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            Err(e) => log::error!("sync: could not save {STATE_FILE}: {e}"),
        }
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../../../docs/sync-vectors.json")).expect("vectors parse")
    }

    fn unhex(s: &str) -> Vec<u8> {
        data_encoding::HEXLOWER.decode(s.as_bytes()).unwrap()
    }

    fn vector_key(v: &Value) -> SyncKey {
        SyncKey::from_bytes(unhex(v["pairing"]["key_hex"].as_str().unwrap()).try_into().unwrap())
    }

    #[test]
    fn pairing_codes_match_the_vectors() {
        let v = vectors();
        let key = vector_key(&v);
        let code = v["pairing"]["code"].as_str().unwrap();
        assert_eq!(key.code(), code);
        assert_eq!(code.len(), 64);
        assert_eq!(code.split('-').count(), 13);
        for ok in v["pairing"]["parse_ok"].as_array().unwrap() {
            assert_eq!(SyncKey::parse(ok.as_str().unwrap()), Some(key.clone()), "{ok}");
        }
        for bad in v["pairing"]["parse_fail"].as_array().unwrap() {
            assert!(SyncKey::parse(bad.as_str().unwrap()).is_none(), "{bad}");
        }
        assert_eq!(key.url(), format!("silas://pair?k={}", code.replace('-', "")));
        let fresh = SyncKey::generate();
        assert_eq!(SyncKey::parse(&fresh.code()), Some(fresh));
    }

    #[test]
    fn group_id_and_frame_key_match_the_vectors() {
        let v = vectors();
        let key = vector_key(&v);
        assert_eq!(key.group_id(), v["group_id"].as_str().unwrap());
        assert_eq!(hex(key.frame_key().bytes()), v["frame_key_hex"].as_str().unwrap());
    }

    #[test]
    fn the_vector_frame_opens_and_reseals_byte_for_byte() {
        let v = vectors();
        let fk = vector_key(&v).frame_key();
        let f = &v["frame"];
        assert_eq!(f["aad"].as_str().unwrap().as_bytes(), AAD);
        let frame = unhex(f["frame_hex"].as_str().unwrap());
        let plain = fk.open(&frame).expect("the vector frame opens");
        assert_eq!(plain, f["plaintext"].as_str().unwrap().as_bytes());
        let nonce: [u8; 12] = unhex(f["nonce_hex"].as_str().unwrap()).try_into().unwrap();
        assert_eq!(fk.seal_with_nonce(&plain, nonce), frame);
        let msg = StateMessage::parse(&plain).expect("a state message");
        let entries = read_entries(&msg.entries);
        assert_eq!(entries["t:parish1"].v.as_ref().unwrap()["text"], "Parish1");
    }

    #[test]
    fn a_frame_from_another_key_or_tampered_does_not_open() {
        let a = SyncKey::generate().frame_key();
        let b = SyncKey::generate().frame_key();
        let frame = a.seal(b"{\"type\":\"state\"}");
        assert!(a.open(&frame).is_some());
        assert!(b.open(&frame).is_none());
        let mut bent = frame.clone();
        *bent.last_mut().unwrap() ^= 1;
        assert!(a.open(&bent).is_none());
        assert!(a.open(&frame[..20]).is_none());
        assert!(a.open(&[]).is_none());
    }

    #[test]
    fn norm_matches_the_vectors() {
        for pair in vectors()["norm"].as_array().unwrap() {
            assert_eq!(norm(pair[0].as_str().unwrap()), pair[1].as_str().unwrap());
        }
        // Decomposed é (e + U+0301) is the same key as the composed one.
        assert_eq!(norm("Cafe\u{301}"), norm("Café"));
    }

    #[test]
    fn snippet_keys_match_the_vectors() {
        for pair in vectors()["snippet_key"].as_array().unwrap() {
            assert_eq!(Kind::Snippet.key_for(pair[0].as_str().unwrap()).as_deref(), pair[1].as_str(), "{pair}");
        }
    }

    fn hlc(v: &Value) -> Hlc {
        serde_json::from_value(v.clone()).unwrap()
    }

    #[test]
    fn hlc_order_and_clock_rules_match_the_vectors() {
        let v = vectors();
        for case in v["hlc_order"].as_array().unwrap() {
            let want = match case["cmp"].as_i64().unwrap() {
                -1 => Ordering::Less,
                0 => Ordering::Equal,
                _ => Ordering::Greater,
            };
            assert_eq!(hlc(&case["a"]).cmp(&hlc(&case["b"])), want, "{case}");
        }
        for case in v["clock"].as_array().unwrap() {
            let mut clock: Clock = serde_json::from_value(case["clock"].clone()).unwrap();
            let got = clock.tick(case["now"].as_u64().unwrap());
            let want: Clock = serde_json::from_value(case["local"].clone()).unwrap();
            assert_eq!(got, want, "{}", case["name"]);
            assert_eq!(clock, want);
        }
        let mut clock = Clock { ms: 5, c: 1 };
        clock.observe(&Hlc { ms: 5, c: 7, d: "x".into() });
        assert_eq!(clock, Clock { ms: 5, c: 7 });
        clock.observe(&Hlc { ms: 4, c: 99, d: "x".into() });
        assert_eq!(clock, Clock { ms: 5, c: 7 });
    }

    fn entries(v: &Value) -> Entries {
        serde_json::from_value(v.clone()).unwrap()
    }

    fn merged(states: &[Entries]) -> Entries {
        let mut out = Entries::new();
        for s in states {
            merge(&mut out, s);
        }
        out
    }

    #[test]
    fn merges_match_the_vectors_in_every_order() {
        for case in vectors()["merge"].as_array().unwrap() {
            let states: Vec<Entries> = case["states"].as_array().unwrap().iter().map(entries).collect();
            let want = entries(&case["merged"]);
            let name = case["name"].as_str().unwrap();
            // Every order (commutative), merged twice over (idempotent).
            let n = states.len();
            let mut idx: Vec<usize> = (0..n).collect();
            permute(&mut idx, 0, &mut |order| {
                let s: Vec<Entries> = order.iter().map(|&i| states[i].clone()).collect();
                assert_eq!(merged(&s), want, "{name} in order {order:?}");
                let twice: Vec<Entries> = s.iter().chain(s.iter()).cloned().collect();
                assert_eq!(merged(&twice), want, "{name} twice");
            });
            // Associative: (a ∪ b) ∪ c == a ∪ (b ∪ c).
            if n == 3 {
                let left = merged(&[merged(&states[..2]), states[2].clone()]);
                let right = merged(&[states[0].clone(), merged(&states[1..])]);
                assert_eq!(left, right, "{name} associativity");
            }
        }
    }

    fn permute(v: &mut Vec<usize>, k: usize, f: &mut dyn FnMut(&[usize])) {
        if k == v.len() {
            return f(v);
        }
        for i in k..v.len() {
            v.swap(k, i);
            permute(v, k + 1, f);
            v.swap(k, i);
        }
    }

    #[test]
    fn equal_stamps_with_different_values_still_merge_the_same_both_ways() {
        let h = Hlc { ms: 1, c: 0, d: "a".into() };
        let a = Entries::from([("t:x".into(), Entry { v: Some(json!({"text": "X"})), h: h.clone() })]);
        let b = Entries::from([("t:x".into(), Entry { v: None, h })]);
        assert_eq!(merged(&[a.clone(), b.clone()]), merged(&[b, a]));
    }

    fn dict() -> Dictionary {
        let mut d = Dictionary::default();
        d.add_term("Parish1", "user");
        d.add_correction("parish one", "Parish1", "user");
        d
    }

    #[test]
    fn projection_round_trips() {
        let d = dict();
        let mut entries = Entries::new();
        let mut clock = Clock::default();
        let (round, changed) = sync_dictionary(&mut d.clone(), &mut entries, None, &mut clock, "aa", 10);
        assert!(!changed);
        // One term, one correction, every default filler.
        assert_eq!(round.stamped, 2 + d.fillers.len());
        // Projecting the entries into an empty dictionary gives back the same synced fields.
        let mut empty = serde_json::to_value(Dictionary { fillers: vec![], ..Dictionary::default() }).unwrap();
        let ch = project_into(&mut empty, &entries);
        assert_eq!(ch.added, round.stamped);
        assert_eq!(project_from(&empty), project_from(&serde_json::to_value(&d).unwrap()));
        // A second pass stamps nothing: an unchanged dictionary is quiet.
        let (again, changed) = sync_dictionary(&mut d.clone(), &mut entries, None, &mut clock, "aa", 11);
        assert_eq!((again.stamped, changed), (0, false));
    }

    /// Two devices, each with its own dictionary and entries, syncing by exchanging entries.
    fn exchange(a: (&mut Dictionary, &mut Entries, &mut Clock), b: (&mut Dictionary, &mut Entries, &mut Clock), now: u64) {
        sync_dictionary(a.0, a.1, None, a.2, "aaaa", now);
        let from_a = a.1.clone();
        sync_dictionary(b.0, b.1, Some(&from_a), b.2, "bbbb", now);
        let from_b = b.1.clone();
        sync_dictionary(a.0, a.1, Some(&from_b), a.2, "aaaa", now);
    }

    #[test]
    fn an_edit_on_one_side_appears_on_the_other_and_a_delete_propagates() {
        let (mut da, mut ea, mut ca) = (dict(), Entries::new(), Clock::default());
        let (mut db, mut eb, mut cb) = (Dictionary::default(), Entries::new(), Clock::default());
        exchange((&mut da, &mut ea, &mut ca), (&mut db, &mut eb, &mut cb), 100);
        assert!(db.terms.iter().any(|t| t.text == "Parish1"));
        assert!(db.corrections.iter().any(|c| c.from == "parish one" && c.to == "Parish1"));
        assert_eq!(db.terms[0].uses, 0);

        // B changes the correction and adds a word; A gets both, and keeps its own use count.
        da.terms[0].uses = 7;
        db.corrections[0].to = "Parish 1".into();
        db.add_term("Chapel1", "user");
        exchange((&mut db, &mut eb, &mut cb), (&mut da, &mut ea, &mut ca), 200);
        assert_eq!(da.corrections[0].to, "Parish 1");
        assert!(da.terms.iter().any(|t| t.text == "Chapel1"));
        assert_eq!(da.terms.iter().find(|t| t.text == "Parish1").unwrap().uses, 7);

        // A deletes a word and a filler; B loses both.
        da.remove_term("Parish1");
        da.fillers.retain(|f| f != "um");
        exchange((&mut da, &mut ea, &mut ca), (&mut db, &mut eb, &mut cb), 300);
        assert!(!db.terms.iter().any(|t| t.text == "Parish1"));
        assert!(!db.fillers.contains(&"um".to_string()));
        assert_eq!(ea, eb);
        assert_eq!(ea["t:parish1"].v, None);

        // Re-added later, it comes back everywhere.
        db.add_term("Parish1", "user");
        exchange((&mut db, &mut eb, &mut cb), (&mut da, &mut ea, &mut ca), 400);
        assert!(da.terms.iter().any(|t| t.text == "Parish1"));
    }

    #[test]
    fn pending_and_counts_never_leave_the_device() {
        let mut d = dict();
        d.pending.push(crate::dictionary::Pending { from: "a".into(), to: "b".into(), count: 1, last_seen: "x".into() });
        d.corrections[0].count = 9;
        let mut e = Entries::new();
        sync_dictionary(&mut d, &mut e, None, &mut Clock::default(), "aa", 1);
        let text = serde_json::to_string(&e).unwrap();
        assert!(!text.contains("pending") && !text.contains("\"count\"") && !text.contains("\"uses\""), "{text}");
        assert!(e.keys().all(|k| Kind::of_key(k).is_some()));
    }

    #[test]
    fn snippets_wait_until_the_dictionary_has_them() {
        // This build's Dictionary may not have snippets yet: a snippet from a peer is kept in the
        // entries (so it relays) and never tombstoned for being absent here.
        let mut d = Dictionary::default();
        let has_snippets = kinds_in(&serde_json::to_value(&d).unwrap()).contains(&Kind::Snippet);
        let mut e = Entries::new();
        let mut clock = Clock::default();
        let incoming = Entries::from([(
            "s:my address".to_string(),
            Entry { v: Some(json!({"trigger": "my address", "text": "12 Benson Rd", "added": "2026-11-22T00:00:00Z"})), h: Hlc { ms: 5, c: 0, d: "bb".into() } },
        )]);
        sync_dictionary(&mut d, &mut e, Some(&incoming), &mut clock, "aa", 10);
        sync_dictionary(&mut d, &mut e, None, &mut clock, "aa", 20);
        assert!(e["s:my address"].v.is_some(), "the snippet must survive a round");
        let shown = serde_json::to_value(&d).unwrap();
        assert_eq!(shown.get("snippets").and_then(Value::as_array).map(Vec::len), has_snippets.then_some(1));

        // And on a dictionary JSON that has the field, it projects like any other kind.
        let mut with = json!({"terms": [], "corrections": [], "fillers": [], "snippets": []});
        let ch = project_into(&mut with, &e);
        assert_eq!(ch.added, 1 + d.fillers.len());
        assert_eq!(with["snippets"].as_array().unwrap().len(), 1);
        assert_eq!(with["snippets"][0]["text"], "12 Benson Rd");
        assert!(project_from(&with).contains_key("s:my address"));
    }

    #[test]
    fn an_entry_whose_value_does_not_match_its_key_is_left_alone() {
        let mut d = Dictionary::default();
        let mut e = Entries::new();
        let mut clock = Clock::default();
        let odd = Entries::from([(
            "t:x".to_string(),
            Entry { v: Some(json!({"text": "Y", "added": "", "source": "user"})), h: Hlc { ms: 5, c: 0, d: "bb".into() } },
        )]);
        sync_dictionary(&mut d, &mut e, Some(&odd), &mut clock, "aa", 10);
        assert!(!d.terms.iter().any(|t| t.text == "Y"));
        let (round, _) = sync_dictionary(&mut d, &mut e, None, &mut clock, "aa", 20);
        assert_eq!(round.stamped, 0, "no churn from an entry this build can't place");
    }

    #[test]
    fn unknown_kinds_and_bad_entries_are_skipped_one_at_a_time() {
        let raw = json!({
            "t:ok": {"v": {"text": "OK"}, "h": {"ms": 1, "c": 0, "d": "aa"}},
            "x:new kind": {"v": {"a": 1}, "h": {"ms": 1, "c": 0, "d": "aa"}},
            "t:bad": {"v": 3, "h": {"ms": 1, "c": 0, "d": "aa"}},
            "t:worse": {"h": "no"},
        });
        let e = read_entries(raw.as_object().unwrap());
        assert_eq!(e.keys().collect::<Vec<_>>(), vec!["t:ok"]);
    }

    #[test]
    fn a_state_message_round_trips_and_strangers_are_refused() {
        let key = SyncKey::generate();
        let mut e = Entries::new();
        sync_dictionary(&mut dict(), &mut e, None, &mut Clock::default(), "aa", 1);
        let m = StateMessage::new("aa", "Test", "linux", &e);
        let frame = m.seal(&key.frame_key());
        let back = StateMessage::parse(&key.frame_key().open(&frame).unwrap()).unwrap();
        assert_eq!(read_entries(&back.entries), e);
        assert!(SyncKey::generate().frame_key().open(&frame).is_none());
        assert!(StateMessage::parse(b"{\"type\":\"other\",\"v\":1,\"device\":\"a\"}").is_none());
    }

    #[test]
    fn the_key_never_shows_in_debug_output() {
        let key = SyncKey::generate();
        let shown = format!("{key:?}");
        assert!(!shown.contains(&key.code()[..4]) || shown == "SyncKey(hidden)");
        assert_eq!(shown, "SyncKey(hidden)");
    }

    #[test]
    fn the_state_file_keeps_its_device_id() {
        let dir = std::env::temp_dir().join(format!("silas-sync-state-{}", new_device_id()));
        let a = SyncState::load(&dir);
        assert_eq!(a.device.len(), 32);
        assert_eq!(SyncState::load(&dir).device, a.device);
        let _ = std::fs::remove_dir_all(dir);
    }
}
