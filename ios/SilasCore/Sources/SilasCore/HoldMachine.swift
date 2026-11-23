// The hold-to-talk button, matching the desktop key (`keys.rs`): hold to speak and
// release to write; a tap shorter than `tap` starts hands-free until the next press.

import Foundation

public struct HoldMachine: Equatable {
    public enum Out: Equatable {
        case nothing, start, finish
    }

    enum State: Equatable {
        case idle
        case held(since: Date)
        case toggled
        /// Pressed again to end hands-free mode; swallow the release.
        case ending
    }

    /// Holding shorter than this is a tap (same as the desktop).
    public static let tap: TimeInterval = 0.28

    private(set) var state: State = .idle

    public init() {}

    public var isActive: Bool {
        switch state {
        case .held, .toggled: return true
        case .idle, .ending: return false
        }
    }

    public var isHandsFree: Bool { state == .toggled }

    public mutating func press(at now: Date = Date()) -> Out {
        switch state {
        case .idle:
            state = .held(since: now)
            return .start
        case .toggled:
            state = .ending
            return .finish
        case .held, .ending:
            return .nothing
        }
    }

    public mutating func release(at now: Date = Date()) -> Out {
        switch state {
        case .held(let since):
            if now.timeIntervalSince(since) < Self.tap {
                state = .toggled
                return .nothing
            }
            state = .idle
            return .finish
        case .ending:
            state = .idle
            return .nothing
        case .idle, .toggled:
            return .nothing
        }
    }

    /// For VoiceOver and keyboard: behave like a tap.
    public mutating func toggle() -> Out {
        switch state {
        case .idle:
            state = .toggled
            return .start
        case .toggled, .held:
            state = .idle
            return .finish
        case .ending:
            return .nothing
        }
    }

    public mutating func reset() {
        state = .idle
    }
}
