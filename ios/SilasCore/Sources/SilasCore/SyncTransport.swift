// The two sync transports from docs/sync.md (local network, synced folder) and the Keychain
// home of the sync key. Each transport moves sealed state messages; what to send and what to do
// with what arrives is the caller's (`SyncPeer`), so the merge rules stay in Sync.swift.

import Foundation
import Network
import Security

// MARK: - Keychain

public protocol SyncKeyStore: AnyObject {
    func load() -> Data?
    func save(_ key: Data) throws
    func delete()
}

/// The sync key in the Keychain, this device only, readable after the first unlock so a sync can
/// run while the app is in the background.
public final class KeychainKeyStore: SyncKeyStore {
    let service: String
    let account: String

    public init(service: String = "com.baney.silas.sync", account: String = "sync-key") {
        self.service = service
        self.account = account
    }

    private var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: account]
    }

    public func load() -> Data? {
        var q = query
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess,
            let data = out as? Data, data.count == SyncKey.byteCount
        else { return nil }
        return data
    }

    public func save(_ key: Data) throws {
        guard key.count == SyncKey.byteCount else { throw SyncError.badKey }
        SecItemDelete(query as CFDictionary)
        var q = query
        q[kSecValueData as String] = key
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(q as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
    }

    public func delete() {
        SecItemDelete(query as CFDictionary)
    }
}

/// For tests and previews.
public final class MemoryKeyStore: SyncKeyStore {
    private var key: Data?
    public init(_ key: Data? = nil) { self.key = key }
    public func load() -> Data? { key }
    public func save(_ key: Data) throws { self.key = key }
    public func delete() { key = nil }
}

// MARK: - Transport B: a synced folder

/// `<folder>/Silas Sync/<device id>.silas`, one frame each.
public struct SyncFolder {
    public let root: URL

    public init(root: URL) {
        self.root = root
    }

    public var directory: URL { root.appendingPathComponent("Silas Sync", isDirectory: true) }

    /// Write this device's frame to a temp name, then rename it into place.
    public func write(_ frame: Data, deviceID: String) throws {
        let fm = FileManager.default
        try fm.createDirectory(at: directory, withIntermediateDirectories: true)
        let final = directory.appendingPathComponent("\(deviceID).silas")
        let temp = directory.appendingPathComponent(".\(deviceID).\(UUID().uuidString).tmp")
        try frame.write(to: temp)
        if fm.fileExists(atPath: final.path) {
            _ = try fm.replaceItemAt(final, withItemAt: temp)
        } else {
            try fm.moveItem(at: temp, to: final)
        }
    }

    /// Every other device's frame. iCloud placeholders are asked to download and are read on a
    /// later pass.
    public func readOthers(excluding deviceID: String) -> [(device: String, frame: Data)] {
        let fm = FileManager.default
        let names = (try? fm.contentsOfDirectory(atPath: directory.path)) ?? []
        var out: [(String, Data)] = []
        for name in names.sorted() {
            if name.hasPrefix("."), name.hasSuffix(".silas.icloud") {
                let real = directory.appendingPathComponent(String(name.dropFirst().dropLast(".icloud".count)))
                try? fm.startDownloadingUbiquitousItem(at: real)
                continue
            }
            guard name.hasSuffix(".silas") else { continue }
            let device = String(name.dropLast(".silas".count))
            guard device != deviceID,
                let data = try? Data(contentsOf: directory.appendingPathComponent(name))
            else { continue }
            out.append((device, data))
        }
        return out
    }
}

// MARK: - Transport A: local network

/// Advertises and browses `_silas-sync._tcp`, listens on 47823 (or any free port) and runs one
/// sync per connection: both sides send one length-prefixed frame, read the other's, and close.
/// All callbacks run on `queue`; `onFrame` gets each frame that arrived and must return this
/// device's own frame to send back (or nil to drop the connection).
public final class SyncNetwork {
    public static let serviceType = "_silas-sync._tcp"
    public static let defaultPort: UInt16 = 47823

    public struct Identity: Equatable {
        public var name: String
        public var groupID: String
        public var deviceID: String

        public init(name: String, groupID: String, deviceID: String) {
            self.name = name
            self.groupID = groupID
            self.deviceID = deviceID
        }
    }

    public let queue = DispatchQueue(label: "silas.sync.network")
    private var listener: NWListener?
    private var browser: NWBrowser?
    private var identity: Identity?
    /// Builds this device's sealed frame (called once per sync).
    public var ownFrame: (() async -> Data?)?
    /// Handles a frame that arrived from `endpoint`.
    public var receivedFrame: ((Data, String) async -> Void)?
    /// A peer of the same group appeared on the network and this side should connect.
    public var peerAppeared: ((NWEndpoint) -> Void)?
    public private(set) var port: UInt16?
    public var onState: ((String) -> Void)?

    public init() {}

    /// Start listening and, with `advertise`, publishing and browsing. `port` 0 means any free port.
    public func start(identity: Identity, port: UInt16 = SyncNetwork.defaultPort, advertise: Bool = true) async throws {
        stop()
        self.identity = identity
        let l: NWListener
        do {
            l = try NWListener(using: .tcp, on: NWEndpoint.Port(rawValue: port) ?? .any)
        } catch {
            l = try NWListener(using: .tcp, on: .any)
        }
        if advertise {
            var txt = NWTXTRecord()
            txt["v"] = "1"
            txt["g"] = identity.groupID
            txt["d"] = identity.deviceID
            l.service = NWListener.Service(name: identity.name, type: Self.serviceType, txtRecord: txt)
        }
        l.newConnectionHandler = { [weak self] conn in self?.serve(conn) }
        listener = l
        let bound: UInt16? = await withCheckedContinuation { k in
            let once = Once()
            l.stateUpdateHandler = { [weak self, weak l] state in
                switch state {
                case .ready:
                    if once.claim() { k.resume(returning: l?.port?.rawValue) }
                case .failed(let e):
                    self?.onState?("listener failed: \(e)")
                    if once.claim() { k.resume(returning: nil) }
                case .cancelled:
                    if once.claim() { k.resume(returning: nil) }
                default: break
                }
            }
            l.start(queue: queue)
            queue.asyncAfter(deadline: .now() + 5) { if once.claim() { k.resume(returning: nil) } }
        }
        self.port = bound
        if advertise { browse(identity) }
    }

    public func stop() {
        listener?.cancel()
        listener = nil
        browser?.cancel()
        browser = nil
        port = nil
        seenLock.withLock { seen.removeAll() }
    }

    private func browse(_ me: Identity) {
        let b = NWBrowser(for: .bonjourWithTXTRecord(type: Self.serviceType, domain: nil), using: .tcp)
        b.browseResultsChangedHandler = { [weak self] _, changes in
            guard let self else { return }
            func txt(_ r: NWBrowser.Result) -> (g: String?, d: String?) {
                guard case .bonjour(let t) = r.metadata else { return (nil, nil) }
                return (t["g"], t["d"])
            }
            for change in changes {
                switch change {
                case .added(let r):
                    self.browsed(r.endpoint, group: txt(r).g, device: txt(r).d, me: me)
                case .changed(let old, let r, _):
                    if old.endpoint != r.endpoint { self.browsed(old.endpoint, group: nil, device: nil, me: me) }
                    self.browsed(r.endpoint, group: txt(r).g, device: txt(r).d, me: me)
                case .removed(let r):
                    self.browsed(r.endpoint, group: nil, device: nil, me: me)
                default:
                    break
                }
            }
        }
        browser = b
        b.start(queue: queue)
    }

    /// Peers of this group the browser has seen and not lost, with their device ids.
    private let seenLock = NSLock()
    private var seen: [NWEndpoint: String] = [:]

    /// The local network peers `syncAll` dials.
    public var discovered: [NWEndpoint] { seenLock.withLock { Array(seen.keys) } }

    /// One browse result: `group` and `device` from its TXT record, both nil when it went away.
    /// Other groups and this device are ignored. A peer seen for the first time is synced at once
    /// by the side with the smaller device id; after that either side dials it on its own change.
    func browsed(_ endpoint: NWEndpoint, group: String?, device: String?, me: Identity) {
        let first: String? = seenLock.withLock {
            guard group == me.groupID, let device, device != me.deviceID else {
                seen[endpoint] = nil
                return nil
            }
            defer { seen[endpoint] = device }
            return seen[endpoint] == nil ? device : nil
        }
        if let d = first, Array(me.deviceID.utf8).lexicographicallyPrecedes(Array(d.utf8)) {
            peerAppeared?(endpoint)
        }
    }

    /// One sync with every peer by address and every peer the browser has kept. Failures are
    /// reported through `onState` and do not stop the rest.
    public func syncAll(addresses: [String]) async {
        var targets: [(String, NWEndpoint)] = []
        for a in addresses {
            guard let hp = parseHostPort(a) else { continue }
            targets.append((a, .hostPort(host: NWEndpoint.Host(hp.host), port: NWEndpoint.Port(rawValue: hp.port) ?? .any)))
        }
        targets += discovered.map { ("\($0)", $0) }
        for (label, endpoint) in targets {
            do {
                try await sync(with: endpoint)
            } catch {
                onState?("sync with \(label) failed: \(error)")
            }
        }
    }

    /// Sync with a peer: as the connecting side, send first, then read.
    public func sync(with endpoint: NWEndpoint, timeout: TimeInterval = 15) async throws {
        let conn = NWConnection(to: endpoint, using: .tcp)
        defer { conn.cancel() }
        try await Self.ready(conn, queue: queue, timeout: timeout)
        guard let frame = await ownFrame?() else { return }
        try await Self.send(try SyncFrame.lengthPrefixed(frame), on: conn)
        let theirs = try await Self.readFrame(conn)
        await receivedFrame?(theirs, "\(endpoint)")
    }

    /// `host:port` from the person's list of peers by address.
    public func sync(host: String, port: UInt16) async throws {
        try await sync(with: .hostPort(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port) ?? .any))
    }

    private func serve(_ conn: NWConnection) {
        conn.start(queue: queue)
        Task {
            defer { conn.cancel() }
            do {
                let theirs = try await Self.readFrame(conn)
                guard let frame = await self.ownFrame?() else { return }
                try await Self.send(try SyncFrame.lengthPrefixed(frame), on: conn)
                await self.receivedFrame?(theirs, "\(conn.endpoint)")
            } catch {
                self.onState?("incoming sync failed: \(error)")
            }
        }
    }

    // MARK: Connection helpers

    private final class Once: @unchecked Sendable {
        private let lock = NSLock()
        private var done = false
        func claim() -> Bool { lock.withLock { if done { return false }; done = true; return true } }
    }

    static func ready(_ conn: NWConnection, queue: DispatchQueue, timeout: TimeInterval) async throws {
        try await withCheckedThrowingContinuation { (k: CheckedContinuation<Void, Error>) in
            let once = Once()
            conn.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    if once.claim() { k.resume() }
                case .failed(let e), .waiting(let e):
                    if once.claim() { k.resume(throwing: e) }
                case .cancelled:
                    if once.claim() { k.resume(throwing: CancellationError()) }
                default: break
                }
            }
            conn.start(queue: queue)
            queue.asyncAfter(deadline: .now() + timeout) {
                if once.claim() { k.resume(throwing: NWError.posix(.ETIMEDOUT)) }
            }
        }
    }

    static func send(_ data: Data, on conn: NWConnection) async throws {
        try await withCheckedThrowingContinuation { (k: CheckedContinuation<Void, Error>) in
            conn.send(content: data, completion: .contentProcessed { e in
                if let e { k.resume(throwing: e) } else { k.resume() }
            })
        }
    }

    static func receive(exactly n: Int, on conn: NWConnection) async throws -> Data {
        var out = Data()
        while out.count < n {
            let chunk: Data = try await withCheckedThrowingContinuation { k in
                conn.receive(minimumIncompleteLength: 1, maximumLength: n - out.count) { data, _, complete, e in
                    if let e {
                        k.resume(throwing: e)
                    } else if let data, !data.isEmpty {
                        k.resume(returning: data)
                    } else if complete {
                        k.resume(throwing: SyncError.badMessage)
                    } else {
                        k.resume(returning: Data())
                    }
                }
            }
            out.append(chunk)
        }
        return out
    }

    static func readFrame(_ conn: NWConnection) async throws -> Data {
        let n = try SyncFrame.length(fromPrefix: try await receive(exactly: 4, on: conn))
        return try await receive(exactly: n, on: conn)
    }
}

// MARK: - One device's sync

/// Ties the ledger, the key and the transports together for one device. The dictionary stays the
/// caller's: `sync` takes the current dictionary and hands back the merged one.
public actor SyncPeer {
    public struct Outcome: Equatable {
        /// The device the frame came from, by name.
        public var from: String
        /// Terms added to this device's dictionary.
        public var newTerms: Int
        /// The merged dictionary, when it changed.
        public var dictionary: PersonalDictionary?
    }

    public private(set) var ledger: SyncLedger
    private let keys: SyncKeyStore
    public var name: String
    public let platform: String

    public init(ledger: SyncLedger, keys: SyncKeyStore, name: String, platform: String) {
        self.ledger = ledger
        self.keys = keys
        self.name = name
        self.platform = platform
    }

    public var key: Data? { keys.load() }
    public var isOn: Bool { keys.load() != nil }
    public var groupID: String? { keys.load().map(SyncKey.groupID) }

    public func rename(_ name: String) { self.name = name }

    /// Make a fresh key ("Pair a device" on the first device, or "New sync key").
    public func newKey() throws -> Data {
        let k = SyncKey.generate()
        try keys.save(k)
        return k
    }

    /// Join with a code someone typed or scanned.
    public func join(code: String) throws {
        guard let k = SyncKey.parse(code) else { throw SyncError.badPairingCode }
        try keys.save(k)
    }

    public func join(key: Data) throws {
        try keys.save(key)
    }

    /// "Leave sync": delete the key. The ledger stays, so rejoining does not resurrect deletes.
    public func leave() {
        keys.delete()
    }

    /// Stamp local changes, then seal this device's state.
    public func frame(for dict: PersonalDictionary) throws -> Data? {
        guard let key = keys.load() else { return nil }
        ledger.recordLocal(dict)
        return try ledger.message(name: name, platform: platform).sealed(key: key)
    }

    /// Open a frame from another device, merge it, and project the result onto `dict`.
    public func receive(_ frame: Data, into dict: PersonalDictionary) throws -> Outcome {
        guard let key = keys.load() else { throw SyncError.badKey }
        let msg = try StateMessage.opened(frame, key: key)
        guard msg.device != ledger.deviceID else { return Outcome(from: name, newTerms: 0, dictionary: nil) }
        ledger.recordLocal(dict)
        ledger.receive(msg.entries)
        ledger.saw(msg)
        var next = dict
        let added = SyncProjection.apply(ledger.entries, to: &next)
        return Outcome(from: msg.name, newTerms: added ?? 0, dictionary: added == nil ? nil : next)
    }

    /// Folder transport: read and merge every other device's file, then write this device's own.
    public func syncFolder(_ folder: SyncFolder, dict: PersonalDictionary) throws -> [Outcome] {
        var current = dict
        var outcomes: [Outcome] = []
        for (_, frame) in folder.readOthers(excluding: ledger.deviceID) {
            guard let o = try? receive(frame, into: current) else { continue }
            if let d = o.dictionary { current = d }
            outcomes.append(o)
        }
        if let frame = try frame(for: current) {
            try folder.write(frame, deviceID: ledger.deviceID)
        }
        return outcomes
    }

    public func addAddress(_ hostPort: String) {
        let t = hostPort.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty, !ledger.addresses.contains(t) else { return }
        ledger.addresses.append(t)
    }

    public func removeAddress(_ hostPort: String) {
        ledger.addresses.removeAll { $0 == hostPort }
    }
}

/// Split "rectory1:47823" (or "[::1]:47823"); a missing port means 47823.
public func parseHostPort(_ s: String) -> (host: String, port: UInt16)? {
    let t = s.trimmingCharacters(in: .whitespaces)
    guard !t.isEmpty else { return nil }
    if t.hasPrefix("["), let close = t.firstIndex(of: "]") {
        let host = String(t[t.index(after: t.startIndex)..<close])
        let rest = t[t.index(after: close)...]
        if rest.hasPrefix(":"), let p = UInt16(rest.dropFirst()) { return (host, p) }
        return (host, SyncNetwork.defaultPort)
    }
    let parts = t.split(separator: ":", omittingEmptySubsequences: false)
    if parts.count == 2, let p = UInt16(parts[1]) { return (String(parts[0]), p) }
    if parts.count == 1 { return (t, SyncNetwork.defaultPort) }
    return nil
}
