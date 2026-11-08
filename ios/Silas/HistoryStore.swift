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
