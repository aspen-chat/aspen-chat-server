import Foundation
import WebRTC

/// The media tracks on this device, by the ids the call engine passes around and never looks
/// inside: the microphone the app opens, and the tracks each receiver gives, which whatever
/// draws or plays them looks up here. A track's id is WebRTC's own (`trackId`), unique within
/// the process.
final class CallTracks: @unchecked Sendable {
    private let lock = NSLock()
    private var tracks: [TrackId: RTCMediaStreamTrack] = [:]

    /// Keeps `track` and answers its id.
    @discardableResult
    func add(_ track: RTCMediaStreamTrack) -> TrackId {
        lock.lock()
        defer { lock.unlock() }
        tracks[track.trackId] = track
        return track.trackId
    }

    func track(_ id: TrackId) -> RTCMediaStreamTrack? {
        lock.lock()
        defer { lock.unlock() }
        return tracks[id]
    }

    func remove(_ id: TrackId) {
        lock.lock()
        defer { lock.unlock() }
        tracks[id] = nil
    }
}
