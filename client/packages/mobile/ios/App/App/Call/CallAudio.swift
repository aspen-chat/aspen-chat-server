import AVFoundation
import Foundation
import WebRTC

/// The call's sound on this device: the shared audio session set up for a call, the microphone,
/// opened through WebRTC's audio device module as a track the engine sends, and the call's own
/// sounds (`CallSounds`), played on that session.
///
/// The system owns the session's activation (`CallSystem`, CallKit): the app configures the
/// session in the start action and never activates it; CallKit activates it and calls the
/// provider's `didActivate`, which is `activated(_:)` here, where WebRTC's audio starts and the
/// sounds may play, and `didDeactivate` is `deactivated(_:)`. WebRTC is told so with
/// `useManualAudio`, and its audio units run only between the two (`isAudioEnabled`), and not
/// while another session interrupts ours (`interrupted`, `resumed`). The route within the
/// session's category is `CallRoutes`'; the camera is `CallCamera`'s.
final class CallAudio {
    private let factory: RTCPeerConnectionFactory
    private let tracks: CallTracks
    let sounds: CallSounds
    private var microphone: RTCAudioTrack?

    init(factory: RTCPeerConnectionFactory, tracks: CallTracks, sounds: CallSounds) {
        self.factory = factory
        self.tracks = tracks
        self.sounds = sounds
        let session = RTCAudioSession.sharedInstance()
        session.useManualAudio = true
        session.isAudioEnabled = false
    }

    /// Sets the shared session up for a call: playing and recording, in the mode that applies
    /// the system's voice processing and allows Bluetooth headsets (HFP), with Bluetooth
    /// listening devices (A2DP) allowed too. Called before the call is reported to the system,
    /// which then activates the session as configured.
    func configureForCall() throws {
        let configuration = RTCAudioSessionConfiguration.webRTC()
        configuration.category = AVAudioSession.Category.playAndRecord.rawValue
        configuration.categoryOptions = [.allowBluetoothHFP, .allowBluetoothA2DP]
        configuration.mode = AVAudioSession.Mode.voiceChat.rawValue
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        try session.setConfiguration(configuration)
    }

    /// The system activated the session (`didActivate`, at the call's start and again after an
    /// interruption it resolved): WebRTC's audio units start on it, and the call's sounds play.
    func activated(_ session: AVAudioSession) {
        let shared = RTCAudioSession.sharedInstance()
        shared.audioSessionDidActivate(session)
        shared.isAudioEnabled = true
        sounds.sessionActivated()
    }

    /// The system deactivated the session (`didDeactivate`, at the call's end or for an
    /// interruption): WebRTC's audio units stop, and so do the sounds. Nothing here deactivates
    /// the session itself.
    func deactivated(_ session: AVAudioSession) {
        let shared = RTCAudioSession.sharedInstance()
        shared.isAudioEnabled = false
        sounds.sessionDeactivated()
        shared.audioSessionDidDeactivate(session)
    }

    /// Another session (a phone call, Siri, an alarm) took ours: WebRTC's audio stops and no
    /// sound plays, since the session is the interrupter's now.
    func interrupted() {
        RTCAudioSession.sharedInstance().isAudioEnabled = false
        sounds.sessionDeactivated()
    }

    /// The interrupter is done and the system still has our session active (it never called
    /// `didDeactivate`): WebRTC's audio runs again on it, and the sounds may play.
    func resumed() {
        RTCAudioSession.sharedInstance().isAudioEnabled = true
        sounds.sessionActivated()
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
