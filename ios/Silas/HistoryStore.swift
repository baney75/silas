import Foundation

struct Dictation: Codable, Identifiable, Equatable {
    let id: UUID
    let date: Date
    /// What Parakeet heard, before cleanup.
    var raw: String
    /// The text the user sees and edits.
    var text: String
    var seconds: Double
}

/// Past dictations, newest first, kept in one JSON file in Application Support.
enum HistoryStore {
    static var fileURL: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Silas", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("history.json")
    }

    static func load() -> [Dictation] {
        guard let data = try? Data(contentsOf: fileURL) else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return (try? decoder.decode([Dictation].self, from: data)) ?? []
    }

    static func save(_ items: [Dictation]) {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        guard let data = try? encoder.encode(items) else { return }
        try? data.write(to: fileURL, options: [.atomic, .completeFileProtection])
    }
}
