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
/// It lives in a `Pending` subfolder that is excluded from backup, so raw voice never leaves the iPad
/// through iCloud or a Finder backup.
public enum PendingAudio {
    public struct Clip {
        public let id: UUID
        public let samples: [Float]
        /// When it was recorded, not when it was recovered.
        public let date: Date
    }

    public static func folder(in dir: URL) -> URL {
        dir.appendingPathComponent("Pending", isDirectory: true)
    }

    private static func name(id: UUID, date: Date) -> String {
        "pending-\(id.uuidString)-\(Int(date.timeIntervalSince1970)).f32"
    }

    public static func write(_ samples: [Float], id: UUID, date: Date = Date(), in dir: URL) throws {
        var folder = folder(in: dir)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        // Set every time: a restore clears the flag.
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try folder.setResourceValues(values)
        let data = samples.withUnsafeBufferPointer { Data(buffer: $0) }
        var file = folder.appendingPathComponent(name(id: id, date: date))
        try data.write(to: file, options: [.atomic, HistoryFile.protection])
        try file.setResourceValues(values)
    }

    private static func parse(_ name: String) -> (id: UUID, date: Date?)? {
        guard name.hasPrefix("pending-"), name.hasSuffix(".f32") else { return nil }
        let body = String(name.dropFirst("pending-".count).dropLast(".f32".count))
        guard let id = UUID(uuidString: String(body.prefix(36))) else { return nil }
        let rest = body.dropFirst(37)
        return (id, Double(rest).map { Date(timeIntervalSince1970: $0) })
    }

    public static func list(in dir: URL) -> [Clip] {
        let folder = folder(in: dir)
        let names = (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []
        var out: [Clip] = []
        for name in names.sorted() {
            guard let (id, date) = parse(name),
                let data = try? Data(contentsOf: folder.appendingPathComponent(name))
            else { continue }
            let samples = data.withUnsafeBytes { Array($0.bindMemory(to: Float.self)) }
            out.append(Clip(id: id, samples: samples, date: date ?? Date()))
        }
        return out
    }

    public static func remove(id: UUID, in dir: URL) {
        let folder = folder(in: dir)
        for name in (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []
        where parse(name)?.id == id {
            try? FileManager.default.removeItem(at: folder.appendingPathComponent(name))
        }
    }
}
