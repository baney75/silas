import AVFoundation
#if DEBUG
import FluidAudio
#endif
import Observation
import SilasCore
import UIKit

enum Status: Equatable {
    case loading
    case downloading(progress: Double)
    case ready
    case listening
    case writing
    case error(String)

    var label: String {
        switch self {
        case .loading: return "Loading model…"
        case .downloading(let p): return "Downloading speech model · \(Int(p * 100))%"
        case .ready: return "Ready"
        case .listening: return "Listening"
        case .writing: return "Writing…"
        case .error: return "Something went wrong"
        }
    }
}

@MainActor
@Observable
final class DictationModel {
    private(set) var status: Status = .loading
    private(set) var history: [Dictation] = HistoryStore.load()
    private(set) var selectedID: UUID?
    /// A short note under the status line ("Didn't catch that", "Copied").
    private(set) var notice: String?
    private(set) var level: Float = 0
    var handsFree: Bool { machine.isHandsFree }

    /// The editor's text. Edits are saved to the selected dictation.
    var text: String = "" {
        didSet {
            guard text != oldValue, let id = selectedID,
                let i = history.firstIndex(where: { $0.id == id }), history[i].text != text
            else { return }
            history[i].text = text
            scheduleSave()
        }
    }

    var canRecord: Bool {
        switch status {
        case .ready, .listening: return true
        default: return false
        }
    }

    private let engine = SpeechEngine()
    private let recorder = Recorder()
    private var machine = HoldMachine()
    private var recordingStarted: Date?
    private var maxTimer: Task<Void, Never>?
    private var saveTask: Task<Void, Never>?
    private var noticeTask: Task<Void, Never>?

    /// Hands-free left on by mistake would grow without end; stop at 5 minutes like the desktop.
    private static let maxRecording: Duration = .seconds(300)
    private static let minSeconds = 0.35

    init() {
        if let first = history.first {
            selectedID = first.id
            text = first.text
        }
        recorder.onLevel = { [weak self] lvl in
            Task { @MainActor in self?.level = lvl }
        }
    }

    // MARK: Model

    func prepare() async {
        let needsDownload = !SpeechEngine.modelIsDownloaded
        status = needsDownload ? .downloading(progress: 0) : .loading
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = false }

        let monitor: Task<Void, Never>? = needsDownload ? Task { [weak self] in
            while !Task.isCancelled {
                let bytes = SpeechEngine.downloadedBytes()
                let p = min(0.99, Double(bytes) / Double(SpeechEngine.expectedDownloadBytes))
                await MainActor.run {
                    guard let self, case .downloading = self.status else { return }
                    self.status = SpeechEngine.modelIsDownloaded ? .loading : .downloading(progress: p)
                }
                try? await Task.sleep(for: .milliseconds(400))
            }
        } : nil

        do {
            try await engine.prepare()
            monitor?.cancel()
            status = .ready
            #if DEBUG
            await selfTest()
            #endif
        } catch {
            monitor?.cancel()
            status = .error(needsDownload
                ? "Couldn't download the speech model. Check Wi-Fi and try again."
                : "Couldn't load the speech model: \(error.localizedDescription)")
        }
    }

    #if DEBUG
    /// `-selftest <path.wav>` (debug builds only): transcribe a file at launch and log it,
    /// so the engine can be checked in the simulator without touching the screen.
    private func selfTest() async {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-selftest"), i + 1 < args.count else { return }
        do {
            let samples = try AudioConverter().resampleAudioFile(URL(fileURLWithPath: args[i + 1]))
            let start = Date()
            let raw = try await engine.transcribe(samples)
            NSLog("SILAS-SELFTEST raw=%@ clean=%@ secs=%.2f", raw, Cleanup.apply(raw), Date().timeIntervalSince(start))
        } catch {
            NSLog("SILAS-SELFTEST failed: %@", String(describing: error))
        }
    }
    #endif

    func retry() {
        Task { await prepare() }
    }

    // MARK: Button

    func press() {
        guard canRecord else { return }
        act(machine.press())
    }

    func release() {
        act(machine.release())
    }

    /// VoiceOver and the keyboard: a tap.
    func toggle() {
        guard canRecord else { return }
        act(machine.toggle())
    }

    func cancel() {
        guard recorder.isRunning else { return }
        machine.reset()
        stopTimers()
        recorder.cancel()
        status = .ready
    }

    /// The app is leaving the screen: keep what was said.
    func finishIfRecording() {
        guard recorder.isRunning else { return }
        machine.reset()
        finish()
    }

    private func act(_ out: HoldMachine.Out) {
        switch out {
        case .start: start()
        case .finish: finish()
        case .nothing: break
        }
    }

    private func start() {
        Task {
            guard await AVAudioApplication.requestRecordPermission() else {
                machine.reset()
                status = .error("Silas needs the microphone. Turn it on in Settings › Apps › Silas.")
                return
            }
            // The press may have ended while the permission prompt was up.
            guard machine.isActive, !recorder.isRunning else { return }
            do {
                try recorder.start()
            } catch {
                machine.reset()
                status = .error("Microphone: \(error.localizedDescription)")
                return
            }
            recordingStarted = Date()
            status = .listening
            UIApplication.shared.isIdleTimerDisabled = true
            maxTimer = Task {
                try? await Task.sleep(for: Self.maxRecording)
                guard !Task.isCancelled else { return }
                self.finishIfRecording()
            }
        }
    }

    private func finish() {
        guard recorder.isRunning else { return }
        stopTimers()
        let samples = recorder.stop()
        level = 0
        let seconds = Double(samples.count) / Recorder.sampleRate
        guard seconds >= Self.minSeconds else {
            status = .ready
            return
        }
        status = .writing
        Task {
            do {
                let raw = try await engine.transcribe(samples)
                let cleaned = Cleanup.apply(raw)
                status = .ready
                guard !cleaned.isEmpty else {
                    show("Didn't catch that. Try again a little closer.")
                    return
                }
                let item = Dictation(id: UUID(), date: Date(), raw: raw, text: cleaned, seconds: seconds)
                history.insert(item, at: 0)
                selectedID = item.id
                text = cleaned
                HistoryStore.save(history)
            } catch {
                status = .error("Couldn't transcribe: \(error.localizedDescription)")
            }
        }
    }

    private func stopTimers() {
        maxTimer?.cancel()
        maxTimer = nil
        UIApplication.shared.isIdleTimerDisabled = false
    }

    /// Clear an error and go back to Ready if the model is loaded.
    func dismissError() {
        Task {
            if await engine.isLoaded { status = .ready } else { await prepare() }
        }
    }

    // MARK: History

    var selected: Dictation? { history.first { $0.id == selectedID } }

    func select(_ d: Dictation) {
        selectedID = d.id
        text = d.text
    }

    func delete(_ d: Dictation) {
        history.removeAll { $0.id == d.id }
        if selectedID == d.id {
            selectedID = history.first?.id
            text = history.first?.text ?? ""
        }
        HistoryStore.save(history)
    }

    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled, let self else { return }
            HistoryStore.save(self.history)
        }
    }

    func saveNow() {
        saveTask?.cancel()
        HistoryStore.save(history)
    }

    // MARK: Actions

    func copy() {
        UIPasteboard.general.string = text
        show("Copied")
    }

    var obsidianURL: URL? {
        Obsidian.newNoteURL(text: text, date: selected?.date ?? Date())
    }

    func show(_ message: String) {
        notice = message
        noticeTask?.cancel()
        noticeTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            self?.notice = nil
        }
    }
}
