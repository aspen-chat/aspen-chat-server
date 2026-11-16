import Foundation
import WebRTC

/// The media tracks on this device, by the ids the call engine passes around and never looks
/// inside: the microphone and camera the app opens, and the tracks each receiver gives, which
/// whatever draws or plays them looks up here. A track's id is WebRTC's own (`trackId`), unique
/// within the process. The first wrapper of an id is the one kept: `RTCRtpReceiver.track` makes
/// a new wrapper of the same native track on every read, and a video wrapper takes the renderers
/// added through it off the track when it is deallocated, so replacing the wrapper held would
/// stop the renderers drawing at the engine's next read of a receiver's track.
final class CallTracks: @unchecked Sendable {
    private let lock = NSLock()
    private var tracks: [TrackId: RTCMediaStreamTrack] = [:]

    /// Keeps `track`, unless one of its id is held already, and answers its id.
    @discardableResult
    func add(_ track: RTCMediaStreamTrack) -> TrackId {
        lock.lock()
        defer { lock.unlock() }
        if tracks[track.trackId] == nil {
            tracks[track.trackId] = track
        }
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
