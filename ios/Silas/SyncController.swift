// Dictionary sync for the app (docs/sync.md). SilasCore does the protocol; this decides when it
// runs: the local network only while the app is open, after each change (2 s debounce), every
// 10 minutes, and the synced folder at start, every 60 s and after each change.

import Foundation
import Network
import Observation
import SilasCore
import UIKit

@MainActor
@Observable
final class SyncController {
    private(set) var isOn: Bool
    private(set) var peers: [SyncLedger.Peer] = []
    private(set) var addresses: [String] = []
    /// The last sync: who with, when, how many new words.
    private(set) var lastWith: String?
    private(set) var lastAt: Date?
    private(set) var lastNewWords = 0
    /// A problem worth showing (a device with a different key, a folder that can't be read).
    private(set) var problem: String?
    private(set) var folderName: String?
    private(set) var busy = false

    var deviceName: String {
        didSet {
            guard deviceName != oldValue else { return }
            UserDefaults.standard.set(deviceName, forKey: "deviceName")
            Task {
                await peer.rename(deviceName)
                if isOn, active { await restartNetwork() }
            }
        }
    }

    let platform: String
    @ObservationIgnored let peer: SyncPeer
    @ObservationIgnored private let keys = KeychainKeyStore()
    @ObservationIgnored private let network = SyncNetwork()
    /// The dictionary to sync, or nil while it isn't loaded (its file can't be read yet): then no
    /// frame goes out and nothing merges, so a file that can't be read never syncs as deletions.
    @ObservationIgnored var currentDictionary: () -> PersonalDictionary? = { nil }
    @ObservationIgnored var applyMerged: (PersonalDictionary) -> Void = { _ in }
    @ObservationIgnored private var active = false
    @ObservationIgnored private var debounce: Task<Void, Never>?
    @ObservationIgnored private var networkTimer: Task<Void, Never>?
    @ObservationIgnored private var folderTimer: Task<Void, Never>?

    private static let bookmarkKey = "syncFolderBookmark"

    init() {
        let idiom = UIDevice.current.userInterfaceIdiom
        platform = idiom == .pad ? "ipados" : "ios"
        // Since iOS 16 the system device name is generic; the person names this device here.
        let name = UserDefaults.standard.string(forKey: "deviceName") ?? (idiom == .pad ? "iPad" : "iPhone")
        deviceName = name
        let ledger = LedgerStore.load()
        peer = SyncPeer(ledger: ledger, keys: keys, name: name, platform: platform)
        isOn = keys.load() != nil
        peers = ledger.peers
        addresses = ledger.addresses
        folderName = resolveFolder()?.lastPathComponent

        network.ownFrame = { [weak self] in
            guard let self else { return nil }
            return await self.ownFrame()
        }
        network.receivedFrame = { [weak self] frame, _ in await self?.received(frame) }
        network.peerAppeared = { [weak self] endpoint in
            Task { @MainActor in await self?.syncPeer(endpoint) }
        }
        network.onState = { message in NSLog("SILAS sync: %@", message) }
    }

    // MARK: Lifecycle

    /// The app came to the screen.
    func activate() async {
        active = true
        guard isOn else { return }
        await restartNetwork()
        await syncEverything()
        networkTimer?.cancel()
        networkTimer = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(600))
                guard !Task.isCancelled else { return }
                await self?.syncEverything()
            }
        }
        folderTimer?.cancel()
        folderTimer = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                guard !Task.isCancelled else { return }
                await self?.syncFolder()
            }
        }
    }

    /// The app left the screen: iOS allows the listener and browser only while it is open.
    func deactivate() {
        active = false
        network.stop()
        networkTimer?.cancel()
        folderTimer?.cancel()
        Task { await saveLedger() }
    }

    /// The dictionary changed on this device.
    func localChange() {
        guard isOn else { return }
        debounce?.cancel()
        debounce = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            await self?.syncEverything()
        }
    }

    // MARK: Pairing

    /// The code to show on "Pair a device", making the key on the first device.
    func pairingCode() async throws -> String {
        if let key = await peer.key { return SyncKey.pairingCode(key) }
        let key = try await peer.newKey()
        isOn = true
        await activate()
        return SyncKey.pairingCode(key)
    }

    var pairURL: URL? {
        keys.load().map(SyncKey.pairURL)
    }

    func join(code: String) async throws {
        try await peer.join(code: code)
        isOn = true
        problem = nil
        await activate()
    }

    func newKey() async throws -> String {
        let key = try await peer.newKey()
        isOn = true
        problem = nil
        if active { await restartNetwork() }
        return SyncKey.pairingCode(key)
    }

    func leave() async {
        await peer.leave()
        isOn = false
        network.stop()
        networkTimer?.cancel()
        folderTimer?.cancel()
        await saveLedger()
    }

    // MARK: Peers by address

    func addAddress(_ hostPort: String) async {
        guard parseHostPort(hostPort) != nil else {
            problem = "Use host:port, like rectory1:47823."
            return
        }
        await peer.addAddress(hostPort)
        await saveLedger()
        localChange()
    }

    func removeAddress(_ hostPort: String) async {
        await peer.removeAddress(hostPort)
        await saveLedger()
    }

    // MARK: Folder

    func setFolder(_ url: URL) async {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            let data = try url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
            UserDefaults.standard.set(data, forKey: Self.bookmarkKey)
            folderName = url.lastPathComponent
            problem = nil
        } catch {
            problem = "Couldn't keep access to that folder: \(error.localizedDescription)"
            return
        }
        await syncFolder()
    }

    func clearFolder() {
        UserDefaults.standard.removeObject(forKey: Self.bookmarkKey)
        folderName = nil
    }

    private func resolveFolder() -> URL? {
        guard let data = UserDefaults.standard.data(forKey: Self.bookmarkKey) else { return nil }
        var stale = false
        guard let url = try? URL(resolvingBookmarkData: data, options: [], relativeTo: nil, bookmarkDataIsStale: &stale)
        else { return nil }
        if stale, let fresh = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) {
            UserDefaults.standard.set(fresh, forKey: Self.bookmarkKey)
        }
        return url
    }

    // MARK: Running a sync

    func syncEverything() async {
        guard isOn, !busy else { return }
        busy = true
        defer { busy = false }
        // Peers by address and every peer the browser has kept, so a change reaches a peer found
        // earlier on the local network whichever side has the smaller device id.
        await network.syncAll(addresses: addresses)
        await syncFolder()
        await saveLedger()
    }

    private func syncPeer(_ endpoint: NWEndpoint) async {
        do {
            try await network.sync(with: endpoint)
        } catch {
            NSLog("SILAS sync with %@ failed: %@", "\(endpoint)", String(describing: error))
        }
    }

    func syncFolder() async {
        guard isOn, let url = resolveFolder() else { return }
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            guard let snapshot = currentDictionary() else { return }
            let outcomes = try await peer.syncFolder(SyncFolder(root: url), dict: snapshot)
            if let merged = outcomes.last(where: { $0.dictionary != nil })?.dictionary {
                applyMerged(SyncProjection.rebase(merged, from: snapshot, onto: currentDictionary() ?? snapshot))
            }
            for o in outcomes { report(o) }
            await saveLedger()
        } catch {
            problem = "Couldn't use the sync folder: \(error.localizedDescription)"
        }
    }

    private func restartNetwork() async {
        guard let group = await peer.groupID else { return }
        let id = await peer.ledger.deviceID
        do {
            try await network.start(identity: .init(name: deviceName, groupID: group, deviceID: id))
        } catch {
            problem = "Local network sync couldn't start: \(error.localizedDescription)"
        }
    }

    private func ownFrame() async -> Data? {
        guard let dict = currentDictionary() else { return nil }
        return try? await peer.frame(for: dict)
    }

    private func received(_ frame: Data) async {
        do {
            guard let snapshot = currentDictionary() else { return }
            let o = try await peer.receive(frame, into: snapshot)
            if let d = o.dictionary {
                applyMerged(SyncProjection.rebase(d, from: snapshot, onto: currentDictionary() ?? snapshot))
            }
            report(o)
            await saveLedger()
        } catch {
            problem = "A device with a different sync key tried to sync."
            NSLog("SILAS sync: a frame didn't open (a device with a different sync key)")
        }
    }

    private func report(_ o: SyncPeer.Outcome) {
        lastWith = o.from
        lastAt = Date()
        lastNewWords = o.newTerms
        problem = nil
    }

    private func saveLedger() async {
        let ledger = await peer.ledger
        LedgerStore.save(ledger)
        peers = ledger.peers.sorted { $0.lastSynced > $1.lastSynced }
        addresses = ledger.addresses
    }

    /// "Synced with Donovan's iPhone just now (3 new words)".
    func statusText(now: Date = Date()) -> String? {
        guard let with = lastWith, let at = lastAt else { return nil }
        let when: String
        if now.timeIntervalSince(at) < 60 {
            when = "just now"
        } else {
            let f = RelativeDateTimeFormatter()
            f.unitsStyle = .full
            when = f.localizedString(for: at, relativeTo: now)
        }
        let words = lastNewWords == 0 ? "" : lastNewWords == 1 ? " (1 new word)" : " (\(lastNewWords) new words)"
        return "Synced with \(with) \(when)\(words)"
    }
}
