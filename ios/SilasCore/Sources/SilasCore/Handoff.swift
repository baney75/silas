// The Silas Keyboard handoff. iOS keyboards cannot use the microphone, so the keyboard asks the
// Silas app to listen (opening it by URL the first time), the app records and transcribes, and
// the text comes back through the App Group container, where the keyboard inserts it with
// `textDocumentProxy`. The pasteboard is never used.
//
// Each file has one writer, so the two processes never race on the same bytes:
//   session.json  the app:      is a session on, is it listening or writing, a heartbeat
//   request.json  the keyboard: start or stop, with a fresh id per tap
//   pending.json  the app writes the finished text; the keyboard deletes it once inserted
//   last.json     the app:      the last dictation, for "type last again"
// A Darwin notification tells the other side to look.

import Foundation

public struct Handoff {
    public static let appGroup = "group.com.baney.silas"
    public static let changed = "com.baney.silas.handoff.changed"
    public static let requested = "com.baney.silas.handoff.requested"
    /// The URL the keyboard opens to start a session.
    public static let dictateURL = URL(string: "silas://dictate?from=keyboard")!

    public enum Phase: String, Codable, Equatable {
        /// A session is on and the microphone is armed; nothing is being kept.
        case armed
        case listening
        case writing
        case off
    }

    public struct Session: Codable, Equatable {
        public var phase: Phase
        /// Updated every few seconds while the app holds the session; a stale heartbeat means off.
        public var heartbeat: Date
        /// Live text while listening, when the app has it.
        public var preview: String?

        public init(phase: Phase, heartbeat: Date = Date(), preview: String? = nil) {
            self.phase = phase
            self.heartbeat = heartbeat
            self.preview = preview
        }
    }

    public struct Request: Codable, Equatable {
        public enum Command: String, Codable { case start, stop, cancel }
        public var id: UUID
        public var command: Command
        public var at: Date

        public init(id: UUID = UUID(), command: Command, at: Date = Date()) {
            self.id = id
            self.command = command
            self.at = at
        }
    }

    public struct Text: Codable, Equatable {
        public var id: UUID
        public var text: String
        public var date: Date

        public init(id: UUID = UUID(), text: String, date: Date = Date()) {
            self.id = id
            self.text = text
            self.date = date
        }
    }

    /// A heartbeat older than this means the app is gone (killed, or the session ended badly).
    public static let staleAfter: TimeInterval = 8

    public let directory: URL

    public init(directory: URL) {
        self.directory = directory
    }

    /// The shared container, or nil when the App Group entitlement is missing (a free Personal
    /// Team build): the keyboard handoff is then off.
    public init?(groupID: String = Handoff.appGroup) {
        guard let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: groupID) else {
            return nil
        }
        self.init(directory: url.appendingPathComponent("Handoff", isDirectory: true))
    }

    private func url(_ name: String) -> URL { directory.appendingPathComponent(name) }

    private func read<T: Decodable>(_ name: String) -> T? {
        guard let data = try? Data(contentsOf: url(name)) else { return nil }
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return try? d.decode(T.self, from: data)
    }

    private func write<T: Encodable>(_ value: T, _ name: String) {
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        guard let data = try? e.encode(value) else { return }
        try? data.write(to: url(name), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    // MARK: App side

    public func setSession(_ session: Session) {
        write(session, "session.json")
        DarwinNotify.post(Self.changed)
    }

    public func endSession() {
        setSession(Session(phase: .off))
    }

    /// The finished text for the keyboard to insert, and the new "last" dictation.
    public func publish(_ text: String, id: UUID = UUID(), date: Date = Date()) {
        let t = Text(id: id, text: text, date: date)
        write(t, "pending.json")
        write(t, "last.json")
        DarwinNotify.post(Self.changed)
    }

    /// Keep "type last again" current without inserting anything.
    public func setLast(_ text: String, date: Date = Date()) {
        write(Text(text: text, date: date), "last.json")
    }

    public var request: Request? { read("request.json") }

    // MARK: Keyboard side

    /// The session as the keyboard should show it: `.off` when the heartbeat is stale.
    public func session(now: Date = Date()) -> Session {
        guard let s: Session = read("session.json") else { return Session(phase: .off, heartbeat: .distantPast) }
        if s.phase != .off && now.timeIntervalSince(s.heartbeat) > Self.staleAfter {
            return Session(phase: .off, heartbeat: s.heartbeat)
        }
        return s
    }

    public func send(_ command: Request.Command) {
        write(Request(command: command), "request.json")
        DarwinNotify.post(Self.requested)
    }

    /// Take the text waiting to be inserted, at most once.
    public func takePending() -> Text? {
        guard let t: Text = read("pending.json") else { return nil }
        try? FileManager.default.removeItem(at: url("pending.json"))
        return t
    }

    public var last: Text? { read("last.json") }
}

/// Darwin notifications cross the app/extension boundary without carrying data.
public enum DarwinNotify {
    public static func post(_ name: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(name as CFString), nil, nil, true)
    }

    /// Calls `handler` on the main queue each time `name` is posted. Keep the returned token;
    /// dropping it stops the observation.
    public final class Observer {
        let name: String
        let handler: () -> Void

        public init(_ name: String, handler: @escaping () -> Void) {
            self.name = name
            self.handler = handler
            let center = CFNotificationCenterGetDarwinNotifyCenter()
            CFNotificationCenterAddObserver(
                center, Unmanaged.passUnretained(self).toOpaque(),
                { _, observer, _, _, _ in
                    guard let observer else { return }
                    let me = Unmanaged<Observer>.fromOpaque(observer).takeUnretainedValue()
                    DispatchQueue.main.async { me.handler() }
                },
                name as CFString, nil, .deliverImmediately)
        }

        deinit {
            CFNotificationCenterRemoveObserver(
                CFNotificationCenterGetDarwinNotifyCenter(), Unmanaged.passUnretained(self).toOpaque(),
                CFNotificationName(name as CFString), nil)
        }
    }
}
