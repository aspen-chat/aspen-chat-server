import Foundation

/// The call engine (`voice_client`, reached through its generated `VoiceClient` module) with
/// this device's platform under it, for one deployment: HTTP and WebSockets over URLSession,
/// peer connections over WebRTC, the session from the account kept for push, and the audio
/// session and microphone. The engine's methods are the call's commands and `snapshots` is what
/// it reports; the interface that commands it, and what plays and draws its tracks, are later
/// pieces, so nothing holds one of these yet.
final class CallEngine {
    /// Every track on the device, and one WebRTC factory for the process, however many
    /// deployments' engines run.
    static let tracks = CallTracks()
    static let peerConnections = CallPeerConnections(tracks: tracks)

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
        let snapshots = CallSnapshots()
        let credentials = CallCredentials(origin: origin)
        self.snapshots = snapshots
        self.credentials = credentials
        self.audio = CallAudio(factory: media.factory, tracks: Self.tracks)
        self.engine = Engine.start(
            platform: Platform(http: CallHttp(), sockets: sockets, peerConnections: peerConnections ?? media),
            options: Options(apiUrl: origin, credentials: credentials, loopbackPlainText: loopbackPlainText),
            observer: snapshots
        )
    }
}
