// History on disk. Pure file logic (no UIKit) so a failed read can never turn into a wipe.

import Foundation

public struct Dictation: Codable, Identifiable, Equatable {
    public let id: UUID
    public let date: Date
    /// What Parakeet heard, before cleanup.
    public var raw: String
    /// The text the user sees and edits.
    public var text: String
    public var seconds: Double

    public init(id: UUID = UUID(), date: Date = Date(), raw: String, text: String, seconds: Double) {
        self.id = id
        self.date = date
        self.raw = raw
        self.text = text
        self.seconds = seconds
    }
}

public enum HistoryFile {
    public enum Loaded: Equatable {
        /// No history file yet: a fresh install. Safe to write.
        case missing
        case ok([Dictation])
        /// The file exists but could not be read (data protection while locked). Never write over it.
        case unreadable
        /// The file did not decode. It was moved to `movedTo`, so a later save cannot destroy it.
        case corrupt(movedTo: URL?)
    }

    static let protection: Data.WritingOptions = .completeFileProtectionUntilFirstUserAuthentication

    public static func load(from url: URL, now: Date = Date()) -> Loaded {
        let fm = FileManager.default
        guard fm.fileExists(atPath: url.path) else { return .missing }
        guard let data = try? Data(contentsOf: url) else { return .unreadable }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        if let items = try? decoder.decode([Dictation].self, from: data) { return .ok(items) }
        let stamp = Int(now.timeIntervalSince1970)
        let aside = url.deletingLastPathComponent().appendingPathComponent("history.corrupt-\(stamp).json")
        do {
            try fm.moveItem(at: url, to: aside)
            return .corrupt(movedTo: aside)
        } catch {
            // Could not move it aside; treat as unreadable so nothing overwrites it.
            return .unreadable
        }
    }

    public static func save(_ items: [Dictation], to url: URL) throws {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(items)
        try data.write(to: url, options: [.atomic, protection])
    }
}

/// Audio captured but not yet transcribed, kept on disk so a suspended or killed app can finish later.
public enum PendingAudio {
    public static func url(id: UUID, in dir: URL) -> URL {
        dir.appendingPathComponent("pending-\(id.uuidString).f32")
    }

    public static func write(_ samples: [Float], id: UUID, in dir: URL) throws {
        let data = samples.withUnsafeBufferPointer { Data(buffer: $0) }
        try data.write(to: url(id: id, in: dir), options: [.atomic, HistoryFile.protection])
    }

    public static func list(in dir: URL) -> [(id: UUID, samples: [Float])] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        var out: [(UUID, [Float])] = []
        for name in names.sorted() where name.hasPrefix("pending-") && name.hasSuffix(".f32") {
            let idString = String(name.dropFirst("pending-".count).dropLast(".f32".count))
            guard let id = UUID(uuidString: idString),
                let data = try? Data(contentsOf: dir.appendingPathComponent(name))
            else { continue }
            let samples = data.withUnsafeBytes { Array($0.bindMemory(to: Float.self)) }
            out.append((id, samples))
        }
        return out
    }

    public static func remove(id: UUID, in dir: URL) {
        try? FileManager.default.removeItem(at: url(id: id, in: dir))
    }
}
