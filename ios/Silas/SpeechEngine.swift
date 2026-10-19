// Parakeet TDT v3 on the iPad through FluidAudio (CoreML). The model is downloaded once
// from Hugging Face, like the desktop app; audio never leaves the device.

import FluidAudio
import Foundation
import SilasCore

enum EngineError: LocalizedError {
    case notReady

    var errorDescription: String? { "The speech model isn't loaded yet." }
}

actor SpeechEngine {
    /// Bytes FluidAudio fetches for Parakeet v3 (the four .mlmodelc folders plus vocab and
    /// config), measured from the Hugging Face repo on 2026-10-17. Only used for the
    /// progress bar, which is clamped below 100% until the download really ends.
    static let expectedDownloadBytes: Int64 = 483_289_190

    static var modelDirectory: URL { AsrModels.defaultCacheDirectory(for: .v3) }

    static var modelIsDownloaded: Bool {
        AsrModels.modelsExist(at: modelDirectory, version: .v3)
    }

    private var manager: AsrManager?

    var isLoaded: Bool { manager != nil }

    /// Download (first launch only) and load the model. Silas keeps it loaded: reload
    /// latency is the bug Silas exists to avoid.
    func prepare() async throws {
        if manager != nil { return }
        let models = try await AsrModels.downloadAndLoad(version: .v3)
        Self.excludeModelFromBackup()
        let m = AsrManager(config: .default)
        try await m.initialize(models: models)
        manager = m
    }

    /// `samples` are 16 kHz mono floats.
    func transcribe(_ samples: [Float]) async throws -> String {
        guard let manager else { throw EngineError.notReady }
        return try await manager.transcribe(AudioPadding.padded(samples), source: .microphone).text
    }

    /// The model is 480 MB and can be downloaded again; keep it out of iCloud backups.
    private static func excludeModelFromBackup() {
        var dir = modelDirectory.deletingLastPathComponent()
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? dir.setResourceValues(values)
    }

    /// Bytes on disk so far: finished files in the model folder plus URLSession's
    /// in-progress temp files. FluidAudio has no progress callback, so we measure.
    static func downloadedBytes() -> Int64 {
        var total = folderSize(modelDirectory)
        let tmp = FileManager.default.temporaryDirectory
        if let e = FileManager.default.enumerator(
            at: tmp, includingPropertiesForKeys: [.fileSizeKey, .isRegularFileKey])
        {
            for case let url as URL in e where url.lastPathComponent.hasPrefix("CFNetworkDownload") {
                total += Int64((try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0)
            }
        }
        return total
    }

    private static func folderSize(_ dir: URL) -> Int64 {
        guard
            let e = FileManager.default.enumerator(
                at: dir, includingPropertiesForKeys: [.fileSizeKey, .isRegularFileKey])
        else { return 0 }
        var total: Int64 = 0
        for case let url as URL in e {
            total += Int64((try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0)
        }
        return total
    }
}
