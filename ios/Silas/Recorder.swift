// Microphone capture: AVAudioEngine input, converted to 16 kHz mono floats for Parakeet.

import AVFoundation

enum RecorderError: LocalizedError {
    case noMicrophone
    case formatUnsupported

    var errorDescription: String? {
        switch self {
        case .noMicrophone: return "No microphone is available."
        case .formatUnsupported: return "The microphone's audio format isn't supported."
        }
    }
}

final class Recorder {
    static let sampleRate: Double = 16_000

    /// A fresh engine per recording, so a route change between recordings cannot leave a stale input format.
    private var engine = AVAudioEngine()
    private var observers: [NSObjectProtocol] = []
    private let lock = NSLock()
    private var samples: [Float] = []
    private var converter: AVAudioConverter?
    private let outFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: Recorder.sampleRate, channels: 1, interleaved: false)!
    private(set) var isRunning = false

    /// Called on the audio thread with a 0...1 loudness for the button's glow.
    var onLevel: ((Float) -> Void)?

    /// Called on the main queue when the capture was cut off (call, Siri, route change).
    /// The recorder is already halted; `stop()` still returns what was heard.
    var onInterrupted: (() -> Void)?

    func start() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.record, mode: .default, options: [.allowBluetoothHFP])
        try session.setActive(true)

        engine = AVAudioEngine()
        let input = engine.inputNode
        let inFormat = input.outputFormat(forBus: 0)
        guard inFormat.sampleRate > 0, inFormat.channelCount > 0 else { throw RecorderError.noMicrophone }
        guard let converter = AVAudioConverter(from: inFormat, to: outFormat) else {
            throw RecorderError.formatUnsupported
        }
        self.converter = converter
        lock.withLock { samples.removeAll(keepingCapacity: true) }

        // Deprecated in iOS 27 for a throwing form that the Swift SDK doesn't expose yet.
        input.installTap(onBus: 0, bufferSize: 4096, format: inFormat) { [weak self] buffer, _ in
            self?.append(buffer)
        }
        engine.prepare()
        do {
            try engine.start()
        } catch {
            input.removeTap(onBus: 0)
            throw error
        }
        isRunning = true
        observe()
    }

    private func observe() {
        let nc = NotificationCenter.default
        let cut: (Notification) -> Void = { [weak self] _ in self?.interrupted() }
        observers = [
            nc.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main, using: cut),
            nc.addObserver(
                forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
            ) { [weak self] note in
                let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt
                if raw == AVAudioSession.InterruptionType.began.rawValue { self?.interrupted() }
            },
        ]
    }

    private func interrupted() {
        guard isRunning else { return }
        halt()
        onInterrupted?()
    }

    /// Stop and return everything heard, as 16 kHz mono floats.
    func stop() -> [Float] {
        halt()
        return lock.withLock {
            let out = samples
            samples = []
            return out
        }
    }

    func cancel() {
        halt()
        lock.withLock { samples = [] }
    }

    private func halt() {
        guard isRunning else { return }
        for o in observers { NotificationCenter.default.removeObserver(o) }
        observers = []
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        isRunning = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func append(_ buffer: AVAudioPCMBuffer) {
        guard let converter else { return }
        let ratio = Recorder.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: capacity) else { return }
        var fed = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if fed {
                status.pointee = .noDataNow
                return nil
            }
            fed = true
            status.pointee = .haveData
            return buffer
        }
        guard error == nil, let data = out.floatChannelData?[0], out.frameLength > 0 else { return }
        let chunk = UnsafeBufferPointer(start: data, count: Int(out.frameLength))
        lock.withLock { samples.append(contentsOf: chunk) }

        var sum: Float = 0
        for s in chunk { sum += s * s }
        let rms = (sum / Float(chunk.count)).squareRoot()
        onLevel?(min(1, rms * 12))
    }
}
