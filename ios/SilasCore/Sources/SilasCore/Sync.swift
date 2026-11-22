// Dictionary sync v1, exactly as docs/sync.md describes it. This file holds the spec's core:
// the sync key and pairing code, the group id, the frame key, frames, the hybrid logical clock,
// merge, the state message and the projection to and from the personal dictionary. The two
// transports (local network and a synced folder) and the Keychain live in SyncTransport.swift.
// `SyncVectorTests` runs every case in docs/sync-vectors.json against this file.

import CryptoKit
import Foundation

public enum SyncError: Error, Equatable {
    case badPairingCode
    case badKey
    case frameTooLarge
    /// "A device with a different sync key": the frame did not open.
    case foreignFrame
    case badMessage
}

// MARK: - Key, pairing code, group id, frame key

public enum SyncKey {
    public static let byteCount = 32

    /// 32 random bytes from the system CSPRNG.
    public static func generate() -> Data {
        SymmetricKey(size: .bits256).withUnsafeBytes { Data($0) }
    }

    private static let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".utf8)

    /// RFC 4648 base32, no padding, upper case, in groups of 4 joined by "-".
    public static func pairingCode(_ key: Data) -> String {
        let plain = base32(key)
        var groups: [String] = []
        var i = plain.startIndex
        while i < plain.endIndex {
            let end = plain.index(i, offsetBy: 4, limitedBy: plain.endIndex) ?? plain.endIndex
            groups.append(String(plain[i..<end]))
            i = end
        }
        return groups.joined(separator: "-")
    }

    static func base32(_ data: Data) -> String {
        var out: [UInt8] = []
        var buffer: UInt32 = 0
        var bits = 0
        for byte in data {
            buffer = (buffer << 8) | UInt32(byte)
            bits += 8
            while bits >= 5 {
                out.append(alphabet[Int((buffer >> UInt32(bits - 5)) & 31)])
                bits -= 5
            }
        }
        if bits > 0 {
            out.append(alphabet[Int((buffer << UInt32(5 - bits)) & 31)])
        }
        return String(decoding: out, as: UTF8.self)
    }

    /// Parse a pairing code: case, spaces and dashes are ignored, and 0→O, 1→L, 8→B.
    /// Returns the 32-byte key, or nil.
    public static func parse(_ code: String) -> Data? {
        var symbols: [UInt8] = []
        for scalar in code.unicodeScalars {
            if scalar == " " || scalar == "-" || scalar == "\t" || scalar == "\n" { continue }
            guard scalar.isASCII else { return nil }
            var c = UInt8(scalar.value)
            if (97...122).contains(c) { c -= 32 }
            switch c {
            case UInt8(ascii: "0"): c = UInt8(ascii: "O")
            case UInt8(ascii: "1"): c = UInt8(ascii: "L")
            case UInt8(ascii: "8"): c = UInt8(ascii: "B")
            default: break
            }
            guard let v = alphabet.firstIndex(of: c) else { return nil }
            symbols.append(UInt8(v))
        }
        // 32 bytes is 256 bits: 52 symbols of 5 bits, the last one carrying 1 bit.
        guard symbols.count == 52 else { return nil }
        var out = Data()
        var buffer: UInt32 = 0
        var bits = 0
        for s in symbols {
            buffer = (buffer << 5) | UInt32(s)
            bits += 5
            if bits >= 8 {
                out.append(UInt8((buffer >> UInt32(bits - 8)) & 0xFF))
                bits -= 8
            }
        }
        return out.count == byteCount ? out : nil
    }

    /// The desktop's QR code: `silas://pair?k=<code without dashes>`.
    public static func pairURL(_ key: Data) -> URL {
        URL(string: "silas://pair?k=" + base32(key))!
    }

    /// The key in a `silas://pair?k=` URL, or nil.
    public static func key(fromPairURL url: URL) -> Data? {
        guard url.scheme?.lowercased() == "silas", url.host?.lowercased() == "pair",
            let k = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                .queryItems?.first(where: { $0.name == "k" })?.value
        else { return nil }
        return parse(k)
    }

    /// First 8 bytes of SHA-256("silas-sync-group" || key), as 16 lower-case hex characters.
    public static func groupID(_ key: Data) -> String {
        var h = SHA256()
        h.update(data: Data("silas-sync-group".utf8))
        h.update(data: key)
        return Data(h.finalize().prefix(8)).hex
    }

    /// HKDF-SHA256(ikm = key, salt = empty, info = "silas-sync v1 frame", L = 32).
    public static func frameKey(_ key: Data) -> SymmetricKey {
        HKDF<SHA256>.deriveKey(
            inputKeyMaterial: SymmetricKey(data: key), salt: Data(),
            info: Data("silas-sync v1 frame".utf8), outputByteCount: 32)
    }
}

// MARK: - Frames

public enum SyncFrame {
    public static let aad = Data("silas-sync v1".utf8)
    public static let maxLength = 8 * 1024 * 1024

    /// nonce (12 random bytes) || ChaCha20-Poly1305(frame key, nonce, plaintext, aad).
    public static func seal(_ plaintext: Data, key: Data, nonce: Data? = nil) throws -> Data {
        let n = try nonce.map { try ChaChaPoly.Nonce(data: $0) } ?? ChaChaPoly.Nonce()
        let box = try ChaChaPoly.seal(plaintext, using: SyncKey.frameKey(key), nonce: n, authenticating: aad)
        return box.combined
    }

    /// The plaintext, or `SyncError.foreignFrame` when the frame was sealed with another key.
    public static func open(_ frame: Data, key: Data) throws -> Data {
        guard frame.count >= 12 + 16,
            let box = try? ChaChaPoly.SealedBox(combined: frame),
            let plain = try? ChaChaPoly.open(box, using: SyncKey.frameKey(key), authenticating: aad)
        else { throw SyncError.foreignFrame }
        return plain
    }

    /// 4-byte big-endian length, then the frame (the local network transport).
    public static func lengthPrefixed(_ frame: Data) throws -> Data {
        guard frame.count <= maxLength else { throw SyncError.frameTooLarge }
        var n = UInt32(frame.count).bigEndian
        return Data(bytes: &n, count: 4) + frame
    }

    public static func length(fromPrefix prefix: Data) throws -> Int {
        guard prefix.count == 4 else { throw SyncError.badMessage }
        let n = prefix.reduce(0) { ($0 << 8) | Int($1) }
        guard n <= maxLength else { throw SyncError.frameTooLarge }
        return n
    }
}

// MARK: - norm

/// The key normalizer: NFC, trim, collapse whitespace runs to one ASCII space, full lower case.
public func syncNorm(_ s: String) -> String {
    let nfc = s.precomposedStringWithCanonicalMapping
    let words = nfc.split(whereSeparator: { $0.isWhitespace })
    return words.joined(separator: " ").lowercased(with: Locale(identifier: "en_US_POSIX"))
}

// MARK: - JSON values

/// Entry values are kept as JSON so fields this build doesn't know survive a round trip.
public enum JSONValue: Codable, Equatable, Hashable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let b = try? c.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? c.decode(Double.self) {
            self = .number(n)
        } else if let s = try? c.decode(String.self) {
            self = .string(s)
        } else if let a = try? c.decode([JSONValue].self) {
            self = .array(a)
        } else {
            self = .object(try c.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }

    public subscript(key: String) -> String? {
        if case .object(let o) = self, case .string(let s)? = o[key] { return s }
        return nil
    }
}

// MARK: - Hybrid logical clock

public struct HLC: Codable, Equatable, Hashable, Comparable {
    public var ms: UInt64
    public var c: UInt32
    /// Device id: 32 lower-case hex characters.
    public var d: String

    public init(ms: UInt64, c: UInt32, d: String) {
        self.ms = ms
        self.c = c
        self.d = d
    }

    /// Compare `ms`, then `c`, then `d` as bytes.
    public static func < (a: HLC, b: HLC) -> Bool {
        if a.ms != b.ms { return a.ms < b.ms }
        if a.c != b.c { return a.c < b.c }
        return Array(a.d.utf8).lexicographicallyPrecedes(Array(b.d.utf8))
    }
}

/// The largest (ms, c) this device has made or seen.
public struct SyncClock: Codable, Equatable {
    public var ms: UInt64
    public var c: UInt32

    public init(ms: UInt64 = 0, c: UInt32 = 0) {
        self.ms = ms
        self.c = c
    }

    /// A local change at wall time `now`: returns the new (ms, c) and keeps it.
    public mutating func tick(now: UInt64) -> (ms: UInt64, c: UInt32) {
        let m = max(now, ms)
        let n: UInt32 = m == ms ? c + 1 : 0
        ms = m
        c = n
        return (m, n)
    }

    /// An incoming hlc: clock = max(clock, (h.ms, h.c)).
    public mutating func see(_ h: HLC) {
        if h.ms > ms || (h.ms == ms && h.c > c) {
            ms = h.ms
            c = h.c
        }
    }

    public static func nowMS() -> UInt64 { UInt64(Date().timeIntervalSince1970 * 1000) }
}

// MARK: - Entries and merge

public struct SyncEntry: Codable, Equatable {
    /// nil is a tombstone.
    public var v: JSONValue?
    public var h: HLC

    public init(v: JSONValue?, h: HLC) {
        self.v = v
        self.h = h
    }

    enum CodingKeys: String, CodingKey { case v, h }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let raw = try c.decodeIfPresent(JSONValue.self, forKey: .v)
        v = raw == .null ? nil : raw
        h = try c.decode(HLC.self, forKey: .h)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        if let v { try c.encode(v, forKey: .v) } else { try c.encodeNil(forKey: .v) }
        try c.encode(h, forKey: .h)
    }
}

public typealias SyncState = [String: SyncEntry]

public enum SyncMerge {
    /// Kinds this build understands: term, correction, snippet, filler.
    public static let knownPrefixes = ["t:", "c:", "s:", "f:"]

    public static func isKnown(_ key: String) -> Bool { knownPrefixes.contains { key.hasPrefix($0) } }

    /// For each key keep the entry with the larger hlc. Commutative, associative, idempotent.
    public static func merge(_ a: SyncState, _ b: SyncState) -> SyncState {
        var out = a
        for (k, e) in b {
            if let mine = out[k], !(mine.h < e.h) { continue }
            out[k] = e
        }
        return out
    }
}

// MARK: - State message

public struct StateMessage: Codable, Equatable {
    public var type: String = "state"
    public var v: Int = 1
    public var device: String
    public var name: String
    public var platform: String
    public var sent: UInt64
    public var entries: SyncState

    public init(device: String, name: String, platform: String, sent: UInt64, entries: SyncState) {
        self.device = device
        self.name = name
        self.platform = platform
        self.sent = sent
        self.entries = entries
    }

    enum CodingKeys: String, CodingKey { case type, v, device, name, platform, sent, entries }

    /// Lenient on purpose: a newer version may add kinds this build skips; a single bad entry
    /// drops only itself.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        type = try c.decode(String.self, forKey: .type)
        v = try c.decode(Int.self, forKey: .v)
        device = try c.decode(String.self, forKey: .device)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        platform = try c.decodeIfPresent(String.self, forKey: .platform) ?? ""
        sent = try c.decodeIfPresent(UInt64.self, forKey: .sent) ?? 0
        let raw = try c.decodeIfPresent([String: Lossy<SyncEntry>].self, forKey: .entries) ?? [:]
        var entries: SyncState = [:]
        for (k, e) in raw {
            if let e = e.value, SyncMerge.isKnown(k) { entries[k] = e }
        }
        self.entries = entries
    }

    public func json() throws -> Data {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return try e.encode(self)
    }

    public static func decode(_ data: Data) throws -> StateMessage {
        guard let m = try? JSONDecoder().decode(StateMessage.self, from: data), m.type == "state", m.v >= 1 else {
            throw SyncError.badMessage
        }
        return m
    }

    public func sealed(key: Data) throws -> Data { try SyncFrame.seal(try json(), key: key) }

    public static func opened(_ frame: Data, key: Data) throws -> StateMessage {
        try decode(try SyncFrame.open(frame, key: key))
    }
}

private struct Lossy<T: Decodable>: Decodable {
    let value: T?
    init(from decoder: Decoder) throws { value = try? T(from: decoder) }
}

// MARK: - Projection: dictionary <-> entries

public enum SyncProjection {
    /// The live entries the dictionary stands for, by key.
    public static func values(of dict: PersonalDictionary) -> [String: JSONValue] {
        var out: [String: JSONValue] = [:]
        for t in dict.terms {
            out["t:" + syncNorm(t.text)] = .object([
                "text": .string(t.text), "added": .string(t.added), "source": .string(t.source),
            ])
        }
        for c in dict.corrections {
            out["c:" + syncNorm(c.from)] = .object([
                "from": .string(c.from), "to": .string(c.to), "added": .string(c.added),
                "source": .string(c.source),
            ])
        }
        for s in dict.snippets {
            out["s:" + syncNorm(s.trigger)] = .object([
                "trigger": .string(s.trigger), "text": .string(s.text), "added": .string(s.added),
            ])
        }
        for f in dict.fillers {
            out["f:" + syncNorm(f)] = .object(["word": .string(f)])
        }
        return out
    }

    /// Bring the dictionary in line with the live entries of `state`: add what's new, update
    /// `to`/`text`, remove tombstoned items. Local-only fields (uses, count, pending) stay for
    /// items that survive. Returns how many terms were added, or nil when nothing changed.
    public static func apply(_ state: SyncState, to dict: inout PersonalDictionary) -> Int? {
        var next = dict
        var newTerms = 0

        func live(_ prefix: String) -> [(String, JSONValue)] {
            state.filter { $0.key.hasPrefix(prefix) }
                .compactMap { k, e in e.v.map { (k, $0) } }
                .sorted { $0.0 < $1.0 }
        }
        func dead(_ key: String) -> Bool { state[key].map { $0.v == nil } ?? false }

        next.terms.removeAll { dead("t:" + syncNorm($0.text)) }
        for (k, v) in live("t:") {
            guard let text = v["text"] else { continue }
            let entry = Term(text: text, source: v["source"] ?? "sync", added: v["added"] ?? "")
            if let i = next.terms.firstIndex(where: { "t:" + syncNorm($0.text) == k }) {
                next.terms[i].text = entry.text
                next.terms[i].source = entry.source
                next.terms[i].added = entry.added
            } else {
                next.terms.append(entry)
                newTerms += 1
            }
        }

        next.corrections.removeAll { dead("c:" + syncNorm($0.from)) }
        for (k, v) in live("c:") {
            guard let from = v["from"], let to = v["to"] else { continue }
            if let i = next.corrections.firstIndex(where: { "c:" + syncNorm($0.from) == k }) {
                next.corrections[i].from = from
                next.corrections[i].to = to
                next.corrections[i].source = v["source"] ?? next.corrections[i].source
                next.corrections[i].added = v["added"] ?? next.corrections[i].added
            } else {
                next.corrections.append(
                    Correction(from: from, to: to, source: v["source"] ?? "sync", added: v["added"] ?? ""))
                next.pending.removeAll { syncNorm($0.from) == syncNorm(from) }
            }
        }

        next.snippets.removeAll { dead("s:" + syncNorm($0.trigger)) }
        for (k, v) in live("s:") {
            guard let trigger = v["trigger"], let text = v["text"] else { continue }
            let s = Snippet(trigger: trigger, text: text, added: v["added"] ?? "")
            if let i = next.snippets.firstIndex(where: { "s:" + syncNorm($0.trigger) == k }) {
                next.snippets[i] = s
            } else {
                next.snippets.append(s)
            }
        }

        next.fillers.removeAll { dead("f:" + syncNorm($0)) }
        for (k, v) in live("f:") {
            guard let word = v["word"] else { continue }
            if let i = next.fillers.firstIndex(where: { "f:" + syncNorm($0) == k }) {
                next.fillers[i] = word
            } else {
                next.fillers.append(word)
            }
        }

        if next == dict { return nil }
        dict = next
        return newTerms
    }
}

// MARK: - Per-device sync state

/// What one device keeps about sync, next to (never inside) settings: its id, clock, the synced
/// entries and the devices it has seen. The key itself lives in the Keychain.
public struct SyncLedger: Codable, Equatable {
    public struct Peer: Codable, Equatable, Identifiable {
        public var id: String
        public var name: String
        public var platform: String
        public var lastSynced: Date
    }

    public var deviceID: String
    public var clock = SyncClock()
    public var entries: SyncState = [:]
    public var peers: [Peer] = []
    /// `host:port` the person added by hand, for networks multicast doesn't reach.
    public var addresses: [String] = []

    public init(deviceID: String = SyncLedger.newDeviceID()) {
        self.deviceID = deviceID
    }

    public static func newDeviceID() -> String {
        SymmetricKey(size: .bits128).withUnsafeBytes { Data($0) }.hex
    }

    /// Compare the dictionary with the synced entries and stamp every added, changed or removed
    /// key with a new hlc. Returns true when something was stamped.
    @discardableResult
    public mutating func recordLocal(_ dict: PersonalDictionary, now: UInt64 = SyncClock.nowMS()) -> Bool {
        let values = SyncProjection.values(of: dict)
        var changed = false
        for (k, v) in values.sorted(by: { $0.key < $1.key }) where entries[k]?.v != v {
            let t = clock.tick(now: now)
            entries[k] = SyncEntry(v: v, h: HLC(ms: t.ms, c: t.c, d: deviceID))
            changed = true
        }
        for (k, e) in entries.sorted(by: { $0.key < $1.key }) where e.v != nil && values[k] == nil {
            let t = clock.tick(now: now)
            entries[k] = SyncEntry(v: nil, h: HLC(ms: t.ms, c: t.c, d: deviceID))
            changed = true
        }
        return changed
    }

    /// Merge what another device sent. Returns the keys whose entry changed here.
    @discardableResult
    public mutating func receive(_ incoming: SyncState) -> [String] {
        let known = incoming.filter { SyncMerge.isKnown($0.key) }
        for e in known.values { clock.see(e.h) }
        let merged = SyncMerge.merge(entries, known)
        let changed = merged.keys.filter { merged[$0] != entries[$0] }.sorted()
        entries = merged
        return changed
    }

    public mutating func saw(_ message: StateMessage, at date: Date = Date()) {
        guard message.device != deviceID else { return }
        let p = Peer(id: message.device, name: message.name, platform: message.platform, lastSynced: date)
        if let i = peers.firstIndex(where: { $0.id == message.device }) {
            peers[i] = p
        } else {
            peers.append(p)
        }
    }

    public func message(name: String, platform: String, now: UInt64 = SyncClock.nowMS()) -> StateMessage {
        StateMessage(device: deviceID, name: name, platform: platform, sent: now, entries: entries)
    }
}

extension Data {
    var hex: String { map { String(format: "%02x", $0) }.joined() }

    init?(hex: String) {
        let chars = Array(hex.utf8)
        guard chars.count % 2 == 0 else { return nil }
        var out = Data(capacity: chars.count / 2)
        var i = 0
        while i < chars.count {
            guard let b = UInt8(String(decoding: chars[i..<i + 2], as: UTF8.self), radix: 16) else { return nil }
            out.append(b)
            i += 2
        }
        self = out
    }
}
