// The app's side of the Silas Keyboard. The keyboard opens `silas://dictate?from=keyboard`; the
// app starts listening and keeps the microphone armed for the session (background audio mode),
// so the person can go back to where they were typing. While the session is on, the keyboard's
// mic key sends start and stop through the App Group and a Darwin notification, and the
// finished text comes back the same way (SilasCore/Handoff.swift).

import Foundation
import SilasCore

extension DictationModel {
    /// The session ends after this long without a dictation, and the microphone closes.
    static let sessionIdle: TimeInterval = 5 * 60
    private static let beat: Duration = .seconds(2)

    func startKeyboardSession() {
        guard openKeyboardSession() else { return }
        startHandsFree(toKeyboard: true)
        pushSession()
    }

    /// Hold the session (heartbeat, the keyboard's requests) without starting a dictation.
    @discardableResult
    func openKeyboardSession() -> Bool {
        guard let handoff else {
            show("This build has no App Group, so the Silas Keyboard can't reach it. Reinstall with ios/scripts/install.sh.")
            return false
        }
        sessionIdleSince = Date()
        if !keyboardSession {
            keyboardSession = true
            requestObserver = DarwinNotify.Observer(Handoff.requested) { [weak self] in
                MainActor.assumeIsolated { self?.handleKeyboardRequest() }
            }
            // A request made before this launch (the tap that opened the app) is already handled.
            lastRequestID = handoff.request?.id
            heartbeat = Task { [weak self] in
                while !Task.isCancelled {
                    guard let self else { return }
                    self.pushSession()
                    self.endSessionIfIdle()
                    try? await Task.sleep(for: Self.beat)
                }
            }
        }
        pushSession()
        return true
    }

    func endKeyboardSession() {
        guard keyboardSession else { return }
        keyboardSession = false
        heartbeat?.cancel()
        heartbeat = nil
        requestObserver = nil
        if recorder.isCapturing {
            finish()
        }
        if recorder.isRunning {
            _ = recorder.stop()
        }
        handoff?.endSession()
    }

    /// The keyboard's mic key: start, stop or cancel, each with a fresh id so a tap counts once.
    func handleKeyboardRequest() {
        guard keyboardSession, let request = handoff?.request, request.id != lastRequestID else { return }
        lastRequestID = request.id
        // A stale request (the app was suspended when it was made) is ignored.
        guard Date().timeIntervalSince(request.at) < 30 else { return }
        sessionIdleSince = Date()
        switch request.command {
        case .start:
            #if DEBUG
            if let text = debugDictation {
                let dictation = request.dictation
                Task { [weak self] in
                    try? await Task.sleep(for: .seconds(4))
                    self?.handoff?.publish(text, dictation: dictation)
                }
                return
            }
            #endif
            startHandsFree(toKeyboard: true)
        case .stop:
            captureToKeyboard = true
            finishIfRecording()
        case .cancel:
            cancel()
        }
    }

    /// Tell the keyboard what the app is doing. A no-op outside a keyboard session.
    func pushSession() {
        guard keyboardSession, let handoff else { return }
        let phase: Handoff.Phase
        switch status {
        case .listening: phase = .listening
        case .writing: phase = .writing
        default: phase = .armed
        }
        handoff.setSession(Handoff.Session(phase: phase, preview: phase == .listening && !preview.isEmpty ? preview : nil))
    }

    private func endSessionIfIdle() {
        guard keyboardSession, status == .ready || status == .loading || isDownloading,
            Date().timeIntervalSince(sessionIdleSince) > Self.sessionIdle
        else {
            if status == .listening || status == .writing { sessionIdleSince = Date() }
            return
        }
        endKeyboardSession()
    }

    private var isDownloading: Bool {
        if case .downloading = status { return true }
        return false
    }
}
