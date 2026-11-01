// FluidAudio 0.9.1 rejects clips under one second at 16 kHz:
// Sources/FluidAudio/ASR/AsrTranscription.swift:11
//   guard audioSamples.count >= 16_000 else { throw ASRError.invalidAudioData }
// A short "Yes." hold would otherwise fail, so pad it with trailing silence.

import Foundation

public enum AudioPadding {
    public static let minimumSamples = 16_000

    /// Returns `samples` with trailing zeros so it has at least `minimum` samples.
    public static func padded(_ samples: [Float], minimum: Int = minimumSamples) -> [Float] {
        guard samples.count < minimum else { return samples }
        return samples + [Float](repeating: 0, count: minimum - samples.count)
    }
}
