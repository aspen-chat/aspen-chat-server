import AVFoundation
import Foundation
import WebRTC

/// The call's sound on this device: the shared audio session set up for a call, and the
/// microphone, opened through WebRTC's audio device module as a track the engine sends.
///
/// WebRTC is told to leave the session's activation to the app (`useManualAudio`), as it must be
/// once CallKit owns the call: then the app configures the session and CallKit activates it,
/// and `activated(_:)` is called from the provider's `didActivate`. Until CallKit is in,
/// `activate()` does both. The route (receiver, loudspeaker, a headset) is chosen within the
/// category set here, which is a later piece's; so is the camera.
final class CallAudio {
    private let factory: RTCPeerConnectionFactory
    private let tracks: CallTracks
    private var microphone: RTCAudioTrack?

    init(factory: RTCPeerConnectionFactory, tracks: CallTracks) {
        self.factory = factory
        self.tracks = tracks
        let session = RTCAudioSession.sharedInstance()
        session.useManualAudio = true
        session.isAudioEnabled = false
    }

    /// Sets the shared session up for a call: playing and recording, in the mode that applies
    /// the system's voice processing, with Bluetooth devices allowed.
    func configureForCall() throws {
        let configuration = RTCAudioSessionConfiguration.webRTC()
        configuration.category = AVAudioSession.Category.playAndRecord.rawValue
        configuration.categoryOptions = [.allowBluetooth, .allowBluetoothA2DP]
        configuration.mode = AVAudioSession.Mode.voiceChat.rawValue
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        try session.setConfiguration(configuration)
    }

    /// Activates the session itself and starts WebRTC's audio on it. With CallKit, the provider
    /// activates the session and `activated(_:)` is what its `didActivate` calls.
    func activate() throws {
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        do {
            try session.setActive(true)
        } catch {
            session.unlockForConfiguration()
            throw error
        }
        session.unlockForConfiguration()
        activated(AVAudioSession.sharedInstance())
    }

    /// The session was activated (by the app, or by CallKit): WebRTC's audio units start.
    func activated(_ session: AVAudioSession) {
        let shared = RTCAudioSession.sharedInstance()
        shared.audioSessionDidActivate(session)
        shared.isAudioEnabled = true
    }

    /// Stops WebRTC's audio and gives the session back.
    func deactivate() {
        let shared = RTCAudioSession.sharedInstance()
        shared.isAudioEnabled = false
        shared.audioSessionDidDeactivate(AVAudioSession.sharedInstance())
        shared.lockForConfiguration()
        try? shared.setActive(false)
        shared.unlockForConfiguration()
    }

    /// The microphone as a track for the engine (`Engine.setMicrophone`), opened on the first
    /// call and kept until `closeMicrophone`. Capture starts when it is sent on an active
    /// session.
    func openMicrophone() -> TrackId {
        if let microphone {
            return microphone.trackId
        }
        let source = factory.audioSource(with: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil))
        let track = factory.audioTrack(with: source, trackId: "microphone-\(UUID().uuidString.lowercased())")
        microphone = track
        return tracks.add(track)
    }

    func closeMicrophone() {
        guard let microphone else {
            return
        }
        tracks.remove(microphone.trackId)
        self.microphone = nil
    }
}
