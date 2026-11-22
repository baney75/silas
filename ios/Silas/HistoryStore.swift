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
enum DictionaryStore {
    static var fileURL: URL { HistoryStore.directory.appendingPathComponent("dictionary.json") }

    static func load() -> PersonalDictionary {
        guard let data = try? Data(contentsOf: fileURL) else { return PersonalDictionary() }
        do {
            return try JSONDecoder().decode(PersonalDictionary.self, from: data)
        } catch {
            // Keep the unreadable file aside rather than overwrite it on the next save.
            let aside = fileURL.deletingPathExtension().appendingPathExtension("unreadable.json")
            try? FileManager.default.removeItem(at: aside)
            try? FileManager.default.moveItem(at: fileURL, to: aside)
            NSLog("SILAS dictionary unreadable, kept aside: %@", String(describing: error))
            return PersonalDictionary()
        }
    }

    static func save(_ dict: PersonalDictionary) {
        do {
            let e = JSONEncoder()
            e.outputFormatting = [.prettyPrinted, .sortedKeys]
            try e.encode(dict).write(to: fileURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
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
