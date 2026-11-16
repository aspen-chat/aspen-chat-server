import AVFoundation
import Foundation
@testable import App

/// A sound player in software for the simulator (`CallSoundPlayer`), whose audio server starts
/// no output for a test process: each `AVAudioPlayer` start would block the main queue for the
/// HAL's timeouts and fail. It reads the recording (so a cue's file must be bundled and
/// decodable) and counts as playing for the recording's length from `play`, as a player that
/// rendered it would, so the layer's rules (which cue plays which file, only on an active
/// session, at most two at once) are proved on what the simulator can do.
final class SoftwareSoundPlayer: CallSoundPlayer {
    let url: URL
    /// The recording's length, read from its file.
    let duration: TimeInterval
    private var startedAt: Date?

    init(url: URL) throws {
        self.url = url
        let file = try AVAudioFile(forReading: url)
        duration = Double(file.length) / file.processingFormat.sampleRate
    }

    var currentTime: TimeInterval = 0

    var isPlaying: Bool {
        guard let startedAt else {
            return false
        }
        return Date().timeIntervalSince(startedAt) < duration - currentTime
    }

    func prepareToPlay() -> Bool {
        true
    }

    func play() -> Bool {
        startedAt = Date()
        return true
    }

    func stop() {
        startedAt = nil
    }
}
