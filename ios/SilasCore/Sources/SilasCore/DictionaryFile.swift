// The personal dictionary on disk. Pure file logic, like HistoryFile, so a file that can't be read
// right now (data protection before the first unlock) never loads as empty and gets saved over,
// or synced to the other devices as a list of deletions.

import Foundation

public enum DictionaryFile {
    public enum Loaded: Equatable {
        /// No dictionary file yet: a fresh install. Safe to write.
        case missing
        case ok(PersonalDictionary)
        /// The file exists but could not be read. Never write over it, never sync from it; load
        /// again when protected data becomes available.
        case unreadable
        /// The file did not decode and was moved to `movedTo`, so starting fresh destroys nothing.
        case corrupt(movedTo: URL)
    }

    public static func load(from url: URL, now: Date = Date()) -> Loaded {
        let fm = FileManager.default
        guard fm.fileExists(atPath: url.path) else { return .missing }
        guard let data = try? Data(contentsOf: url) else { return .unreadable }
        if let dict = try? JSONDecoder().decode(PersonalDictionary.self, from: data) { return .ok(dict) }
        let stamp = Int(now.timeIntervalSince1970)
        let aside = url.deletingLastPathComponent().appendingPathComponent("dictionary.corrupt-\(stamp).json")
        do {
            try fm.moveItem(at: url, to: aside)
            return .corrupt(movedTo: aside)
        } catch {
            // Could not move it aside: treat it as unreadable so nothing overwrites it.
            return .unreadable
        }
    }

    public static func save(_ dict: PersonalDictionary, to url: URL) throws {
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        try e.encode(dict).write(to: url, options: [.atomic, HistoryFile.protection])
    }
}
