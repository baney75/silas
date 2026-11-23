import Foundation
import SilasCore

/// Past dictations, newest first, kept in one JSON file in Application Support.
/// The file logic lives in SilasCore (`HistoryFile`); this only picks the folder.
enum HistoryStore {
    static var directory: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Silas", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    static var fileURL: URL { directory.appendingPathComponent("history.json") }

    static func load() -> HistoryFile.Loaded { HistoryFile.load(from: fileURL) }

    /// Returns false (and logs) when the write fails, so the caller can retry later.
    @discardableResult
    static func save(_ items: [Dictation]) -> Bool {
        do {
            try HistoryFile.save(items, to: fileURL)
            return true
        } catch {
            NSLog("SILAS history save failed: %@", String(describing: error))
            return false
        }
    }
}

/// The personal dictionary, in the same JSON shape as the desktop's `dictionary.json`.
/// The file logic lives in SilasCore (`DictionaryFile`); the caller saves only after a load that
/// was not `.unreadable`.
enum DictionaryStore {
    static var fileURL: URL { HistoryStore.directory.appendingPathComponent("dictionary.json") }

    static func load() -> DictionaryFile.Loaded { DictionaryFile.load(from: fileURL) }

    static func save(_ dict: PersonalDictionary) {
        do {
            try DictionaryFile.save(dict, to: fileURL)
        } catch {
            NSLog("SILAS dictionary save failed: %@", String(describing: error))
        }
    }
}

/// The sync ledger (device id, clock, entries, peers seen, peers by address). The key is in the Keychain.
enum LedgerStore {
    static var fileURL: URL { HistoryStore.directory.appendingPathComponent("sync.json") }

    static func load() -> SyncLedger {
        guard let data = try? Data(contentsOf: fileURL),
            let ledger = try? JSONDecoder().decode(SyncLedger.self, from: data)
        else { return SyncLedger() }
        return ledger
    }

    static func save(_ ledger: SyncLedger) {
        do {
            try JSONEncoder().encode(ledger).write(to: fileURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            NSLog("SILAS sync ledger save failed: %@", String(describing: error))
        }
    }
}
