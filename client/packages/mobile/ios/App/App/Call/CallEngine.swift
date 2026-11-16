import Foundation

/// The call engine (`voice_client`, reached through its generated `VoiceClient` module) with
/// this device's platform under it, for one deployment: HTTP and WebSockets over URLSession,
/// peer connections over WebRTC, the session from the account kept for push, and the audio
/// session, the microphone, and the call's sounds (`CallAudio`). Joining, leaving, and muting go
/// through the system (`CallSystem`, CallKit), which holds one call for the process and
/// activates the call's audio; the engine's
/// other methods are called directly and `snapshots` is what it reports. `AspenCallPlugin`
/// commands it for the web view, and `CallVideo` draws its video tracks.
final class CallEngine: @unchecked Sendable {
    /// Every track on the device, one WebRTC factory for the process, the one audio session's
    /// routes, the camera, and where video is drawn, however many deployments' engines run.
    static let tracks = CallTracks()
    /// Replaced by a test on the simulator, whose own audio unit aborts the process when
    /// initialized, with a factory on an audio device in software, before the first engine is
    /// made; every engine made with no `media` of its own takes it, and so does the camera.
    static var peerConnections = CallPeerConnections(tracks: tracks)
    static let routes = CallRoutes()
    static let video = CallVideo(tracks: tracks)
    static let camera = CallCamera(factory: peerConnections.factory, tracks: tracks)

    let engine: Engine
    let snapshots: CallSnapshots
    let audio: CallAudio
    let credentials: CallCredentials

    /// Starts an engine for the deployment at `origin` (its `public_url`), which the device's
    /// account there, kept by the web view on sign-in, is the session of. `loopbackPlainText`
    /// lets a development deployment's voice servers be reached over plain HTTP at this
    /// machine's own addresses; a release build never allows it. A test stands between the
    /// engine and the platform by giving `media` of its own (a `CallPeerConnections` on another
    /// audio device, whose factory the microphone then comes from too), `sockets` wrapping
    /// `CallSockets`, or `peerConnections` wrapping `media`.
    init(
        origin: String,
        loopbackPlainText: Bool = false,
        media: CallPeerConnections = CallEngine.peerConnections,
        sockets: SocketFactory = CallSockets(),
        peerConnections: PeerConnectionFactory? = nil
    ) {
        let sounds = CallSounds()
        let snapshots = CallSnapshots(sounds: sounds)
        let credentials = CallCredentials(origin: origin)
        self.snapshots = snapshots
        self.credentials = credentials
        self.audio = CallAudio(factory: media.factory, tracks: Self.tracks, sounds: sounds)
        self.engine = Engine.start(
            platform: Platform(http: CallHttp(), sockets: sockets, peerConnections: peerConnections ?? media),
            options: Options(apiUrl: origin, credentials: credentials, loopbackPlainText: loopbackPlainText),
            observer: snapshots
        )
    }

    /// Joins `channel`'s call as a call the system knows of, shown there as `name`: the system
    /// starts it, the audio session is configured for it, and the engine joins in the start
    /// action. Call on the main queue.
    func join(channel: String, name: String? = nil) {
        CallSystem.shared.start(engine: self, channel: channel, name: name)
    }

    /// Leaves the call through the system's end action. Call on the main queue.
    func leave() {
        CallSystem.shared.end(engine: self)
    }

    /// Mutes or unmutes through the system, so its own controls agree. Call on the main queue.
    func setMuted(_ muted: Bool) {
        CallSystem.shared.setMuted(muted, engine: self)
    }
}
