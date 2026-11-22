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

struct FailedClip {
    let samples: [Float]
    let seconds: Double
    let id: UUID
    let date: Date
    let toKeyboard: Bool
    /// The keyboard dictation it answers (`Handoff.Request.dictation`).
    let dictation: UUID?
}

enum Sheet: String, Identifiable {
    case history, dictionary, settings
    var id: String { rawValue }
}

@MainActor
@Observable
final class DictationModel {
    /// One model per process, so App Intents and URLs reach the same one the screen shows.
    static let shared = DictationModel()

    private(set) var status: Status = .loading
    private(set) var history: [Dictation] = []
    /// False while history.json exists but could not be read (device locked). Nothing saves until it loads.
    private var historyLoaded = false
    /// The same for dictionary.json; while false, sync doesn't run either.
    private var dictionaryLoaded = false
    @ObservationIgnored private var protectedDataObserver: NSObjectProtocol?
    private var unsavedChanges = false
    private(set) var selectedID: UUID?
    /// A short note under the status line ("Didn't catch that", "Copied").
    private(set) var notice: String?
    private(set) var level: Float = 0
    /// Live text while listening.
    private(set) var preview = ""
    var handsFree: Bool { machine.isHandsFree }

    /// The personal dictionary: terms, corrections, fillers. Same rules and file shape as the desktop.
    private(set) var dictionary: PersonalDictionary

    /// The sheet on a phone (history, dictionary, settings) or on an iPad (dictionary, settings).
    var sheet: Sheet?
    /// A pairing code from `silas://pair?k=`, waiting for the person to confirm.
    var joinCode: String?

    let sync = SyncController()

    /// The App Group handoff to the Silas Keyboard; nil when the build was signed without the App Group.
    let handoff = Handoff()
    var keyboardAvailable: Bool { handoff != nil }
    /// A keyboard session is on: the microphone stays armed while the person is in another app.
    var keyboardSession = false

    /// The editor's text. Edits are saved to the selected dictation.
    var text: String = "" {
        didSet {
            guard text != oldValue else { return }
            if selectedID == nil {
                // Typing with nothing selected (fresh install, or history emptied): keep it as a dictation.
                guard !text.isEmpty else { return }
                let item = Dictation(raw: "", text: text, seconds: 0)
                history.insert(item, at: 0)
                selectedID = item.id
                scheduleSave()
                return
            }
            guard let id = selectedID,
                let i = history.firstIndex(where: { $0.id == id }), history[i].text != text
            else { return }
            if editBase?.id != id { editBase = (id, history[i].text) }
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

    let engine = SpeechEngine()
    let recorder = Recorder()
    var machine = HoldMachine()
    private var recordingStarted: Date?
    private var maxTimer: Task<Void, Never>?
    private var saveTask: Task<Void, Never>?
    private var dictionarySaveTask: Task<Void, Never>?
    private var noticeTask: Task<Void, Never>?
    private var previewTask: Task<Void, Never>?
    /// Bumped whenever a pending start (waiting on the mic permission) must not go ahead.
    private var startToken = 0
    private var backgroundTaskID = UIBackgroundTaskIdentifier.invalid
    private var inFlight: Set<UUID> = []
    /// The text of the dictation being edited, as it was before the edit began, to learn from.
    @ObservationIgnored private var editBase: (id: UUID, text: String)?
    /// Every recording whose transcription failed this launch, oldest first, so "Try again" and
    /// "Discard" reach all of them. Recovery skips these ids so a bad clip is not retried in a loop.
    @ObservationIgnored private var failed: [FailedClip] = []
    private var preparing = false
    /// How many clips could not be transcribed. Recording stays available; the UI offers Try again / Discard.
    private(set) var failedClipCount = 0
    var hasFailedClip: Bool { failedClipCount > 0 }
    /// The capture in progress was started from the keyboard, so its text goes back to the keyboard.
    var captureToKeyboard = false
    #if DEBUG
    /// UI tests (`-handoff <text>`): every keyboard dictation comes back as this text.
    @ObservationIgnored var debugDictation: String?
    #endif
    /// A hands-free start asked for before the model was ready (App Intent, keyboard URL).
    var pendingStart: PendingStart?
    enum PendingStart { case app, keyboard }

    // Keyboard session state (DictationModel+Keyboard.swift).
    @ObservationIgnored var requestObserver: DarwinNotify.Observer?
    @ObservationIgnored var heartbeat: Task<Void, Never>?
    @ObservationIgnored var lastRequestID: UUID?
    @ObservationIgnored var sessionIdleSince = Date()

    #if DEBUG
    /// The bundled self-test clip's text, for the UI test.
    private(set) var selfTestResult: String?
    #endif

    /// UI tests launch with `-uitest`: no up-front microphone prompt over the screen under test.
    static let isUITest: Bool = {
        #if DEBUG
        return ProcessInfo.processInfo.arguments.contains("-uitest")
        #else
        return false
        #endif
    }()

    /// Hands-free left on by mistake would grow without end; stop at 5 minutes like the desktop.
    private static let maxRecording: Duration = .seconds(300)
    private static let minSeconds = 0.35

    private init() {
        dictionary = PersonalDictionary()
        loadDictionary()
        protectedDataObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, !self.dictionaryLoaded else { return }
                self.loadDictionary()
            }
        }
        loadHistory()
        recorder.onLevel = { [weak self] lvl in
            Task { @MainActor in self?.level = lvl }
        }
        recorder.onInterrupted = { [weak self] in
            Task { @MainActor in self?.interrupted() }
        }
        // Nil while dictionary.json can't be read: a sync then neither sends nor merges anything.
        sync.currentDictionary = { [weak self] in
            guard let self, self.dictionaryLoaded else { return nil }
            return self.dictionary
        }
        sync.applyMerged = { [weak self] merged in
            guard let self else { return }
            self.dictionary = merged
            self.saveDictionary()
        }
        if let last = history.first { handoff?.setLast(last.text, date: last.date) }
    }

    // MARK: Dictionary file

    /// Read dictionary.json. A file that exists but can't be read (protected data before the first
    /// unlock) leaves the dictionary unloaded: nothing saves over it and sync doesn't run from it
    /// until a later load (protected data available, the app active) succeeds.
    private func loadDictionary() {
        switch DictionaryStore.load() {
        case .missing:
            dictionaryLoaded = true
        case .ok(let d):
            dictionary = d
            dictionaryLoaded = true
        case .corrupt:
            dictionary = PersonalDictionary()
            dictionaryLoaded = true
            show("Your dictionary couldn't be read. The old file was kept aside.")
        case .unreadable:
            dictionaryLoaded = false
        }
    }

    private func saveDictionary() {
        guard dictionaryLoaded else { return }
        DictionaryStore.save(dictionary)
    }

    // MARK: History file

    /// Read history.json. A file that exists but cannot be read leaves the store unloaded, so a
    /// later save cannot replace it with a near-empty list.
    private func loadHistory() {
        switch HistoryStore.load() {
        case .missing:
            historyLoaded = true
        case .ok(let items):
            historyLoaded = true
            let known = Set(history.map(\.id))
            history = (history + items.filter { !known.contains($0.id) }).sorted { $0.date > $1.date }
        case .corrupt:
            historyLoaded = true
            show("History couldn't be read. The old file was kept aside.")
        case .unreadable:
            historyLoaded = false
        }
        if selectedID == nil, text.isEmpty, let first = history.first {
            selectedID = first.id
            text = first.text
        }
    }

    private func persist() {
        guard historyLoaded else {
            unsavedChanges = true
            return
        }
        unsavedChanges = !HistoryStore.save(history)
    }

    func leavingForeground() {
        commitEdit()
        // A keyboard session keeps listening in the background; that is its whole point.
        if !keyboardSession { finishIfRecording() }
        saveNow()
        sync.deactivate()
    }

    func becameActive() {
        if !historyLoaded { loadHistory() }
        if !dictionaryLoaded { loadDictionary() }
        if unsavedChanges { persist() }
        Task { await recoverPending() }
        Task { await sync.activate() }
    }

    // MARK: Model

    func prepare() async {
        // One run at a time, and never once the model is loaded: a rerun would flip a live
        // recording or transcription back to Ready and re-enable the idle timer.
        guard !preparing else { return }
        preparing = true
        defer { preparing = false }
        if await engine.isLoaded { return }
        let needsDownload = !SpeechEngine.modelIsDownloaded
        status = needsDownload ? .downloading(progress: 0) : .loading
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = status == .listening }

        let monitor: Task<Void, Never>? = needsDownload ? Task.detached { [weak self] in
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
            // Ask now, so the system alert never lands under a finger mid-hold.
            if AVAudioApplication.shared.recordPermission == .undetermined, pendingStart == nil, !Self.isUITest {
                _ = await AVAudioApplication.requestRecordPermission()
            }
            if let p = pendingStart {
                pendingStart = nil
                startHandsFree(toKeyboard: p == .keyboard)
            }
            await recoverPending()
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
    /// Debug builds only. `-selftest <path.wav>` transcribes a file at launch and logs it;
    /// `-selftest-bundled` does the same with the bundled clip and shows the text for the UI test.
    private func selfTest() async {
        let args = ProcessInfo.processInfo.arguments
        let url: URL
        if let i = args.firstIndex(of: "-selftest"), i + 1 < args.count {
            url = URL(fileURLWithPath: args[i + 1])
        } else if args.contains("-selftest-bundled"), let bundled = Bundle.main.url(forResource: "selftest", withExtension: "wav") {
            url = bundled
        } else {
            return
        }
        do {
            let samples = try AudioConverter().resampleAudioFile(url)
            let start = Date()
            let raw = try await engine.transcribe(samples)
            var d = dictionary
            let clean = d.apply(raw)
            selfTestResult = clean
            NSLog("SILAS-SELFTEST raw=%@ clean=%@ secs=%.2f", raw, clean, Date().timeIntervalSince(start))
        } catch {
            selfTestResult = "failed: \(error.localizedDescription)"
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
        if AVAudioApplication.shared.recordPermission == .undetermined {
            // Never start a hold under the permission alert: the system cancels the touch.
            Task { _ = await AVAudioApplication.requestRecordPermission() }
            return
        }
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

    /// Start hands-free (App Intent, the keyboard's mic, `silas://dictate`). Waits for the model.
    func startHandsFree(toKeyboard: Bool) {
        switch status {
        case .ready:
            guard !recorder.isCapturing else { return }
            machine.reset()
            _ = machine.toggle()
            start(toKeyboard: toKeyboard)
        case .listening:
            if toKeyboard { captureToKeyboard = true }
        case .loading, .downloading, .error:
            pendingStart = toKeyboard ? .keyboard : .app
        case .writing:
            break
        }
    }

    func cancel() {
        guard recorder.isCapturing else { return }
        machine.reset()
        stopTimers()
        if keyboardSession {
            _ = recorder.endCapture()
        } else {
            recorder.cancel()
        }
        preview = ""
        status = .ready
        pushSession()
    }

    /// The app is leaving the screen: keep what was said.
    func finishIfRecording() {
        // Also drops a start that is still waiting on the permission prompt.
        startToken += 1
        machine.reset()
        guard recorder.isCapturing else { return }
        finish()
    }

    /// The microphone was cut off (call, Siri, route change). Keep what was heard.
    private func interrupted() {
        startToken += 1
        machine.reset()
        finish(force: true)
        if keyboardSession { endKeyboardSession() }
        show("Recording stopped: the microphone was interrupted.")
    }

    private func act(_ out: HoldMachine.Out) {
        switch out {
        case .start: start(toKeyboard: false)
        case .finish: finish()
        case .nothing: break
        }
    }

    private func start(toKeyboard: Bool) {
        startToken += 1
        let token = startToken
        commitEdit()
        Task {
            let granted = await AVAudioApplication.requestRecordPermission()
            // A newer press or a stop owns the gesture now; leave its state alone.
            guard token == startToken else { return }
            guard granted else {
                machine.reset()
                status = .error("Silas needs the microphone. Turn it on in Settings › Apps › Silas.")
                return
            }
            // The press may have ended while the permission prompt was up.
            guard machine.isActive, !recorder.isCapturing, status == .ready else {
                machine.reset()
                return
            }
            do {
                if recorder.isRunning {
                    // Armed by a keyboard session: start keeping samples at once.
                    recorder.beginCapture()
                } else {
                    try recorder.start(mode: keyboardSession ? .session : .app)
                }
            } catch {
                machine.reset()
                status = .error("Microphone: \(error.localizedDescription)")
                return
            }
            captureToKeyboard = toKeyboard
            recordingStarted = Date()
            preview = ""
            status = .listening
            pushSession()
            UIApplication.shared.isIdleTimerDisabled = true
            maxTimer = Task {
                try? await Task.sleep(for: Self.maxRecording)
                guard !Task.isCancelled else { return }
                self.finishIfRecording()
            }
            startPreview()
        }
    }

    func finish(force: Bool = false) {
        guard recorder.isCapturing || force else { return }
        stopTimers()
        machine.reset()
        let toKeyboard = captureToKeyboard
        captureToKeyboard = false
        // The keyboard's start (or its stop, which carries it) names the dictation and its field.
        let dictation = toKeyboard ? handoff?.request?.dictation : nil
        // In a keyboard session the engine stays armed for the next tap.
        let samples = keyboardSession && recorder.isRunning ? recorder.endCapture() : recorder.stop()
        level = 0
        let seconds = Double(samples.count) / Recorder.sampleRate
        guard seconds >= Self.minSeconds else {
            preview = ""
            status = .ready
            pushSession()
            return
        }
        status = .writing
        pushSession()
        // Keep the audio on disk until the text is saved, and ask iOS for time to finish.
        let id = UUID()
        let date = Date()
        do {
            try PendingAudio.write(samples, id: id, date: date, in: HistoryStore.directory)
        } catch {
            NSLog("SILAS pending audio write failed: %@", String(describing: error))
        }
        beginBackgroundWork()
        Task {
            await transcribe(samples, seconds: seconds, id: id, date: date, toKeyboard: toKeyboard, dictation: dictation)
            endBackgroundWork()
        }
    }

    // MARK: Live text

    /// While listening, transcribe everything heard so far every second or so with the loaded
    /// model and show it. The interval grows with the clip so a long hands-free take stays light.
    private func startPreview() {
        previewTask?.cancel()
        previewTask = Task { [weak self] in
            var wait = 1.0
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(wait))
                guard let self, !Task.isCancelled, self.status == .listening else { return }
                let samples = self.recorder.snapshot()
                guard Double(samples.count) / Recorder.sampleRate >= 0.6 else { continue }
                let t0 = Date()
                guard let raw = try? await self.engine.transcribe(samples),
                    !Task.isCancelled, self.status == .listening
                else { continue }
                var d = self.dictionary
                self.preview = d.apply(raw)
                self.pushSession()
                wait = max(1.0, Date().timeIntervalSince(t0) * 1.5)
            }
        }
    }

    private func beginBackgroundWork() {
        guard backgroundTaskID == .invalid else { return }
        backgroundTaskID = UIApplication.shared.beginBackgroundTask(withName: "silas-transcribe") { [weak self] in
            // iOS calls this on the main thread and kills the app if the task is still open when it returns.
            MainActor.assumeIsolated { self?.endBackgroundWork() }
        }
    }

    private func endBackgroundWork() {
        guard backgroundTaskID != .invalid else { return }
        UIApplication.shared.endBackgroundTask(backgroundTaskID)
        backgroundTaskID = .invalid
    }

    private func transcribe(_ samples: [Float], seconds: Double, id: UUID, date: Date, toKeyboard: Bool, dictation: UUID? = nil) async {
        guard inFlight.insert(id).inserted else { return }
        defer { inFlight.remove(id) }
        do {
            let raw = try await engine.transcribe(samples)
            let cleaned = dictionary.apply(raw)
            scheduleDictionarySave()
            failed.removeAll { $0.id == id }
            failedClipCount = failed.count
            status = .ready
            preview = ""
            guard !cleaned.isEmpty else {
                PendingAudio.remove(id: id, in: HistoryStore.directory)
                show("Didn't catch that. Try again a little closer.")
                pushSession()
                return
            }
            let item = Dictation(id: id, date: date, raw: raw, text: cleaned, seconds: seconds)
            history.insert(item, at: 0)
            editBase = nil
            selectedID = item.id
            text = cleaned
            persist()
            if toKeyboard {
                handoff?.publish(cleaned, id: id, date: date, dictation: dictation)
            } else {
                handoff?.setLast(cleaned, date: date)
            }
            pushSession()
            // The audio is only safe to drop once the text is on disk.
            if historyLoaded && !unsavedChanges {
                PendingAudio.remove(id: id, in: HistoryStore.directory)
            }
        } catch {
            // Never block recording over one bad clip: stay Ready and offer Try again / Discard.
            failed.removeAll { $0.id == id }
            failed.append(FailedClip(samples: samples, seconds: seconds, id: id, date: date, toKeyboard: toKeyboard, dictation: dictation))
            failedClipCount = failed.count
            status = .ready
            preview = ""
            pushSession()
            show("Couldn't transcribe that recording: \(error.localizedDescription)")
        }
    }

    /// Finish recordings the app was suspended or killed before it could write.
    private func recoverPending() async {
        guard status == .ready else { return }
        let known = Set(history.map(\.id))
        for clip in PendingAudio.list(in: HistoryStore.directory) {
            let id = clip.id
            if known.contains(id) {
                if historyLoaded && !unsavedChanges { PendingAudio.remove(id: id, in: HistoryStore.directory) }
                continue
            }
            guard status == .ready, !inFlight.contains(id), !failed.contains(where: { $0.id == id }) else { continue }
            status = .writing
            await transcribe(clip.samples, seconds: Double(clip.samples.count) / Recorder.sampleRate, id: id, date: clip.date, toKeyboard: false)
        }
    }

    private func stopTimers() {
        maxTimer?.cancel()
        maxTimer = nil
        previewTask?.cancel()
        previewTask = nil
        UIApplication.shared.isIdleTimerDisabled = false
    }

    /// Clear a model error and go back to Ready if the model is loaded.
    func dismissError() {
        Task {
            if await engine.isLoaded {
                status = .ready
            } else {
                await prepare()
            }
        }
    }

    /// Retranscribe every clip that failed.
    func retryFailedClip() {
        guard !failed.isEmpty, status == .ready else { return }
        let clips = failed
        status = .writing
        Task {
            for (n, f) in clips.enumerated() {
                if n > 0 {
                    // Stop if the user started recording between clips.
                    guard status == .ready else { return }
                    status = .writing
                }
                await transcribe(f.samples, seconds: f.seconds, id: f.id, date: f.date, toKeyboard: f.toKeyboard, dictation: f.dictation)
            }
        }
    }

    /// Give up on every clip that failed: delete their audio and clear the banner.
    func discardFailedClip() {
        for f in failed {
            PendingAudio.remove(id: f.id, in: HistoryStore.directory)
        }
        failed = []
        failedClipCount = 0
    }

    // MARK: History

    var selected: Dictation? { history.first { $0.id == selectedID } }

    func select(_ d: Dictation) {
        commitEdit()
        selectedID = d.id
        text = d.text
    }

    func delete(_ d: Dictation) {
        if editBase?.id == d.id { editBase = nil }
        history.removeAll { $0.id == d.id }
        if selectedID == d.id {
            selectedID = history.first?.id
            text = history.first?.text ?? ""
        }
        PendingAudio.remove(id: d.id, in: HistoryStore.directory)
        persist()
    }

    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled, let self else { return }
            self.persist()
        }
    }

    func saveNow() {
        saveTask?.cancel()
        persist()
        dictionarySaveTask?.cancel()
        saveDictionary()
    }

    // MARK: Learning

    /// An edit to a dictation is over (the editor closed, another one was picked, the app left the
    /// screen): learn from it the way the desktop learns from an edit in its history, as explicit.
    func commitEdit() {
        guard let base = editBase else { return }
        editBase = nil
        guard let item = history.first(where: { $0.id == base.id }), item.text != base.text else { return }
        let result = dictionary.learn(typed: base.text, fixed: item.text, explicit: true)
        guard !result.isEmpty else { return }
        dictionaryChanged()
        if let c = result.learned.first {
            show("Learned: \(c.from) → \(c.to)")
        } else if let t = result.terms.first {
            show("Added \(t) to your words")
        } else if let p = result.pending.first {
            show("Noted \(p.from) → \(p.to). Accept it in Dictionary.")
        }
    }

    // MARK: Dictionary

    /// The person changed the dictionary: save it and tell the other devices.
    func dictionaryChanged() {
        dictionarySaveTask?.cancel()
        saveDictionary()
        sync.localChange()
    }

    /// Use counts change on every dictation; save them without syncing (they are local only).
    private func scheduleDictionarySave() {
        dictionarySaveTask?.cancel()
        dictionarySaveTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled, let self else { return }
            self.saveDictionary()
        }
    }

    @discardableResult
    func addTerm(_ text: String) -> Bool {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, dictionary.addTerm(t, source: "user") else { return false }
        dictionaryChanged()
        return true
    }

    func removeTerm(_ text: String) {
        dictionary.removeTerm(text)
        dictionaryChanged()
    }

    @discardableResult
    func addCorrection(_ from: String, _ to: String) -> Bool {
        let f = from.trimmingCharacters(in: .whitespacesAndNewlines)
        let t = to.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !f.isEmpty, !t.isEmpty, dictionary.addCorrection(f, t, source: "user") != nil else { return false }
        dictionaryChanged()
        return true
    }

    func removeCorrection(_ from: String) {
        dictionary.removeCorrection(from)
        dictionaryChanged()
    }

    func acceptPending(_ from: String) {
        dictionary.acceptPending(from)
        dictionaryChanged()
    }

    func dismissPending(_ from: String) {
        dictionary.dismissPending(from)
        saveDictionary()
    }

    func addFiller(_ word: String) {
        let w = word.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !w.isEmpty, !dictionary.fillers.contains(w) else { return }
        dictionary.fillers.append(w)
        dictionaryChanged()
    }

    func removeFiller(_ word: String) {
        dictionary.fillers.removeAll { $0 == word }
        dictionaryChanged()
    }

    // MARK: URLs

    /// `silas://dictate` (hands-free in the app), `silas://dictate?from=keyboard` (a keyboard
    /// session), `silas://pair?k=<code>` (join sync after the person confirms).
    func open(_ url: URL) {
        guard url.scheme == "silas" else { return }
        switch url.host() {
        case "dictate":
            let from = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                .queryItems?.first { $0.name == "from" }?.value
            if from == "keyboard" {
                startKeyboardSession()
            } else {
                startHandsFree(toKeyboard: false)
            }
        case "pair":
            if let key = SyncKey.key(fromPairURL: url) {
                joinCode = SyncKey.pairingCode(key)
                sheet = .settings
            } else {
                show("That pairing code isn't valid.")
            }
        default:
            break
        }
    }

    // MARK: Actions

    func copy() {
        commitEdit()
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

    #if DEBUG
    /// UI tests: a keyboard session with no microphone, in which every dictation the keyboard
    /// starts comes back as `text` a few seconds later (DictationModel+Keyboard.swift).
    func debugHandoff(_ text: String) {
        debugDictation = text
        openKeyboardSession()
    }
    #endif
}
