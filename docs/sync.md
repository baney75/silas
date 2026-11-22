# Dictionary sync (v1)

Every Silas a person uses (Mac, Windows, Linux, iPhone, iPad) shares one personal dictionary:
the words it must spell right, the corrections it applies, the snippets it expands and the
fillers it drops. Sync has no server and no account. Devices talk directly on the local network,
and, for devices that are never on the same network, through a folder the person already syncs
(iCloud Drive, Syncthing, Dropbox, Proton Drive). Everything that leaves a device is encrypted
with a key only the person's devices hold. Desktop (`core/voice/src/sync.rs`) and iOS
(`SilasCore/Sync.swift`) implement this page and must both pass `docs/sync-vectors.json`.

What syncs: terms, corrections, snippets, fillers. What never syncs: history, audio, settings,
pending (not yet learned) corrections, use counts.

## 1. The sync key and the group

- The first device makes a **sync key**: 32 random bytes from the OS CSPRNG.
- It is shown to the person as a **pairing code**: the key in RFC 4648 base32, no padding,
  upper case, in groups of 4 joined by `-` (52 characters, 13 groups). Parsing ignores case,
  spaces and dashes, and maps `0`→`O`, `1`→`L`, `8`→`B`. The desktop also shows it as a QR code
  of the URL `silas://pair?k=<code without dashes>`, which the iOS app scans.
- Entering the code on another device joins it to the group. Every device in a group holds the
  same key. "Leave sync" deletes the key; "New sync key" makes a fresh one (other devices must
  re-pair).
- **Group id**: the first 8 bytes of `SHA-256("silas-sync-group" || key)`, as 16 lower-case
  hex characters.
- **Frame key**: `HKDF-SHA256(ikm = key, salt = empty, info = "silas-sync v1 frame", L = 32)`.
- The key is stored only on the device: the macOS and iOS Keychain, the Windows Credential
  Manager, the Linux Secret Service (falling back to a `0600` file in the app-data folder when no
  Secret Service is running). Never in `settings.json`, logs or the UI after pairing (the
  pairing code can be shown again only on request).

## 2. Entries

The synced state is a map from **key** to **entry**. An entry is `{ "v": <value or null>, "h": <hlc> }`.
`v: null` is a tombstone (deleted). Tombstones are kept forever; they are tiny.

| Kind | Key | Value |
| --- | --- | --- |
| term | `t:` + norm(text) | `{ "text": "Parish1", "added": "<RFC 3339>", "source": "edit" }` |
| correction | `c:` + norm(from) | `{ "from": "parish one", "to": "Parish1", "added": "<RFC 3339>", "source": "user" }` |
| snippet | `s:` + match_key(NFC(trigger)) | `{ "trigger": "my address", "text": "12 Benson Rd…", "added": "<RFC 3339>" }` |
| filler | `f:` + norm(word) | `{ "word": "um" }` |

`norm(s)`: Unicode NFC, then trim, then collapse every run of whitespace to one ASCII space,
then lower-case with the full Unicode lower-case mapping (Rust `str::to_lowercase`, Swift
`lowercased()` with `Locale(identifier: "en_US_POSIX")`).

`match_key(s)` (snippets only) is `snippets::match_key` in `core/voice/src/snippets.rs`, the
form Silas matches spoken triggers in, so two triggers that match the same speech share one key.
The sync key applies Unicode NFC first, as `norm` does. Then, one Unicode scalar at a time
(Rust `char`, Swift `unicodeScalars`, never `Character`):

1. Drop every apostrophe: U+0027 `'` and U+2019 `’`.
2. Keep a scalar that is alphanumeric (Rust `char::is_alphanumeric`: the Unicode `Alphabetic`
   property, or general category Nd, Nl or No; Swift `properties.isAlphabetic` or
   `properties.generalCategory` in `.decimalNumber`, `.letterNumber`, `.otherNumber`). Replace
   every other scalar with one ASCII space.
3. Split on Unicode whitespace and join the pieces with one ASCII space (this also trims).
4. Lower-case with the same mapping as `norm`.

So "My Address." and "my address" are both `s:my address`, "e-mail" and "e mail" are both
`s:e mail`, and "Donovan's phone" is `s:donovans phone`. A trigger whose `match_key` is empty
never syncs. The value keeps the trigger as typed; on a merge the winning entry's trigger is the
one shown.

### Hybrid logical clock

An hlc is `{ "ms": <u64 unix ms>, "c": <u32>, "d": "<device id>" }`. Order: compare `ms`, then
`c`, then `d` as bytes. The device id is 16 random bytes as 32 lower-case hex characters, made
once and stored next to the sync state.

- Each device keeps `clock = (ms, c)`, the largest it has made or seen.
- **Local change**: `ms' = max(now_ms, clock.ms)`; `c' = clock.c + 1` if `ms' == clock.ms`,
  else `0`. Stamp the entry with `(ms', c', own id)` and set `clock = (ms', c')`.
- **Receive**: for every incoming hlc `h`, `clock = max(clock, (h.ms, h.c))`.

### Merge

For each key, keep the entry with the larger hlc. Merge is commutative, associative and
idempotent, so the order and number of syncs never matter, and replaying an old state can't undo
anything.

### Mapping to the dictionary

The app keeps working on its `Dictionary` as before. After every save it compares the dictionary
with the last synced projection and stamps each added, changed or removed key with a new hlc.
After every merge it projects the live entries back: add what's new, update `to`/`text`, remove
tombstoned items. Local-only fields (`uses`, `count`, pending) stay as they are for items that
survive. A projection that changes nothing writes nothing.

## 3. Frames

Every message, on the wire or in a file, is one **frame**:

```
frame = nonce (12 random bytes) || ChaCha20-Poly1305(frame key, nonce, plaintext, aad = "silas-sync v1")
```

The plaintext is UTF-8 JSON. A frame that fails to open is dropped and logged as "a device with a
different sync key"; nothing else about it is trusted.

### State message

```json
{ "type": "state", "v": 1, "device": "<id>", "name": "Donovan's Mac", "platform": "macos",
  "sent": <unix ms>, "entries": { "<key>": { "v": {...} | null, "h": {...} } } }
```

A device that sees `"v"` greater than 1 keeps the entries it understands (kinds it knows) and
ignores the rest.

## 4. Transport A: local network (peer to peer)

- Each device with sync on runs a TCP listener (port 47823, or any free port if taken) and
  advertises it over DNS-SD: service `_silas-sync._tcp.local.`, instance name the device name,
  TXT `v=1`, `g=<group id>`, `d=<device id>`.
- It browses the same service and ignores instances whose `g` differs or whose `d` is its own.
- **A sync**: the device with the smaller device id connects. Both send one length-prefixed frame
  (4-byte big-endian length, then the frame; at most 8 MiB) holding their state message, read the
  other's, merge, and close. Each side syncs a peer when it appears, after a local change (debounced
  2 s), and every 10 minutes.
- **Peers by address**: a list of `host:port` the person adds by hand (a tailnet name such as
  `rectory1:47823`), synced the same way, for networks where multicast doesn't reach.
- On iOS the listener and browser run only while the app is open (Network.framework,
  `NSLocalNetworkUsageDescription`, `NSBonjourServices = ["_silas-sync._tcp"]`).

## 5. Transport B: a synced folder

- The person picks a folder. Silas writes its own state to `<folder>/Silas Sync/<device id>.silas`
  (one frame, written to a temp name then renamed) after each local change (debounced 2 s), and
  reads and merges every other `*.silas` file in that folder at start, when it changes (file
  watcher; polled every 60 s where watching is unreliable) and before writing.
- On iOS the folder comes from the document picker and is kept as a security-scoped bookmark, so
  iCloud Drive works without any paid entitlement.

## 6. UI

Settings → Sync: on or off; "Pair a device" (shows the code and QR); "Join with a code"; the list
of devices seen (name, platform, last synced); peers by address; sync folder; "Leave sync".
Status text after each sync: "Synced with Donovan's iPhone just now (3 new words)".

## 7. Tests both sides must pass

`docs/sync-vectors.json` holds: pairing-code encode and parse, group id, frame key, `norm`,
hlc ordering, merge cases (concurrent edit, delete vs edit, delete then re-add, idempotence,
three-way associativity) and one frame that must open to a known plaintext. A frame made by one
side must open on the other.
