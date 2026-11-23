// The Silas Keyboard handoff. iOS keyboards cannot use the microphone, so the keyboard asks the
// Silas app to listen (opening it by URL the first time), the app records and transcribes, and
// the text comes back through the App Group container, where the keyboard inserts it with
// `textDocumentProxy`. The pasteboard is never used.
//
// Each file has one writer, so the two processes never race on the same bytes:
//   session.json  the app:      is a session on, is it listening or writing, a heartbeat
//   request.json  the keyboard: start or stop, with a fresh id per tap, and the dictation and
//                 field it belongs to
//   pending.json  the app writes the finished text, tagged with its dictation; the keyboard types
//                 it only into the field that dictation started in, then deletes it
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
        /// The dictation this tap belongs to: a start's own id, carried by the stop and cancel after it.
        public var dictation: UUID?
        /// `textDocumentProxy.documentIdentifier` of the field the dictation started in.
        public var document: UUID?

        public init(id: UUID = UUID(), command: Command, at: Date = Date(), dictation: UUID? = nil, document: UUID? = nil) {
            self.id = id
            self.command = command
            self.at = at
            self.dictation = dictation
            self.document = document
        }
    }

    public struct Text: Codable, Equatable {
        public var id: UUID
        public var text: String
        public var date: Date
        /// The keyboard dictation (`Request.dictation`) this text answers; nil for any other.
        public var dictation: UUID?

        public init(id: UUID = UUID(), text: String, date: Date = Date(), dictation: UUID? = nil) {
            self.id = id
            self.text = text
            self.date = date
            self.dictation = dictation
        }
    }

    /// Text older than this is never typed without being asked.
    public static let pendingMaxAge: TimeInterval = 600

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

    /// The finished text for the keyboard to insert, and the new "last" dictation. `dictation` is
    /// the keyboard dictation it answers; without one the keyboard types it only when asked.
    public func publish(_ text: String, id: UUID = UUID(), date: Date = Date(), dictation: UUID? = nil) {
        let t = Text(id: id, text: text, date: date, dictation: dictation)
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

    /// A mic key tap. A start begins a dictation in the field `document`; a stop or cancel belongs
    /// to the dictation already on. `notify: false` when the tap opens Silas instead.
    public func send(_ command: Request.Command, document: UUID? = nil, notify: Bool = true) {
        var r = Request(command: command)
        if command == .start {
            r.dictation = r.id
            r.document = document
        } else if let on = request {
            // A stop names the field it was pressed in: iOS may give the field a new id after the
            // trip to Silas and back. Without one, keep the start's field.
            r.dictation = on.dictation
            r.document = document ?? on.document
        }
        write(r, "request.json")
        if notify { DarwinNotify.post(Self.requested) }
    }

    /// Whether the keyboard, now in the field `document`, may type `text` without being asked:
    /// only the answer to its own latest dictation, in the field that dictation started in.
    public static func typesAutomatically(_ text: Text, request: Request?, document: UUID?, now: Date = Date()) -> Bool {
        guard let d = text.dictation, let r = request, r.dictation == d,
            let doc = document, r.document == doc
        else { return false }
        return now.timeIntervalSince(text.date) < pendingMaxAge
    }

    /// Take the text waiting to be inserted into the field `document`, at most once. Text for
    /// another field stays for "type last again"; text too old to type is dropped.
    public func takePending(for document: UUID?, now: Date = Date()) -> Text? {
        guard let t: Text = read("pending.json") else { return nil }
        if Self.typesAutomatically(t, request: request, document: document, now: now) {
            try? FileManager.default.removeItem(at: url("pending.json"))
            return t
        }
        if now.timeIntervalSince(t.date) >= Self.pendingMaxAge {
            try? FileManager.default.removeItem(at: url("pending.json"))
        }
        return nil
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
