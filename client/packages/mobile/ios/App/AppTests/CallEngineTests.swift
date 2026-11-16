import AVFoundation
import CallKit
import UIKit
import XCTest
@testable import App

/// The call engine with this device's whole platform under it, against a real deployment: it
/// joins a voice channel's call through the system (CallKit) as far as the simulator's CallKit
/// takes an outgoing call, then, driven directly, joins, is heard by and hears a second
/// participant in a browser, mutes and unmutes, holds the call and releases it, watches the
/// browser's camera and draws it (`CallVideo`, a renderer counting frames and reporting the
/// picture's shape where the page would put its hole, and the picture in picture started on
/// it), sends a camera of its own (in software, `SyntheticVideoCapturer`) that the browser
/// decodes and sees a picture in, moves it to the other camera, takes its socket back after the
/// socket is cut with video flowing on, and leaves.
/// `scripts/check_mobile_call.py` at the repository's root runs the deployment, the browser, and
/// this test, and gives it what it reads from the environment (passed on by `xcodebuild test`
/// with a `TEST_RUNNER_` prefix); without them it skips.
///
/// | Variable                   | What                                                        |
/// | -------------------------- | ----------------------------------------------------------- |
/// | `ASPEN_CALL_API`           | The deployment's `public_url`, as the simulator reaches it  |
/// | `ASPEN_CALL_CHANNEL`       | The voice channel to join                                   |
/// | `ASPEN_CALL_USER_ID`       | The user whose call this is                                 |
/// | `ASPEN_CALL_REFRESH_TOKEN` | Their refresh token; the session token kept is a stale one, |
/// |                            | so the first request is refused and renewed from this       |
/// | `ASPEN_CALL_HELPER`        | The script's helper, which brings the browser in            |
final class CallEngineTests: XCTestCase {
    /// `CallSockets` with every socket opened kept, so the test can cut the live one.
    final class CuttableSockets: SocketFactory, @unchecked Sendable {
        private let real = CallSockets()
        private let lock = NSLock()
        private(set) var opened: [WatchedSocket] = []

        func connect(url: String, bearer: String?) async throws -> ConnectedSocket {
            let connected = try await real.connect(url: url, bearer: bearer)
            let watched = WatchedSocket(real: connected.socket)
            lock.held { opened.append(watched) }
            return ConnectedSocket(socket: watched)
        }

        var connections: Int {
            lock.lock()
            defer { lock.unlock() }
            return opened.count
        }

        /// What the socket opened last negotiated (`Sec-WebSocket-Extensions`), or "none".
        var latestExtensions: String {
            lock.lock()
            defer { lock.unlock() }
            return (opened.last?.real as? CallSocket)?.negotiatedExtensions ?? "none"
        }

        /// The kinds of the frames sent on the socket opened last, in order.
        var latestSent: [String] {
            lock.lock()
            defer { lock.unlock() }
            return opened.last?.sent ?? []
        }

        /// The kinds of the frames received on the socket opened last, in order.
        var latestReceived: [String] {
            lock.lock()
            defer { lock.unlock() }
            return opened.last?.received ?? []
        }

        /// Closes the socket opened last from this side, with no `leave` sent on it, as a
        /// network that drops it would.
        func cut() {
            lock.lock()
            let latest = opened.last
            lock.unlock()
            latest?.closeSocket()
        }
    }

    /// A socket of `real`'s with the kind (`type`) of each frame sent and received on it kept,
    /// as the engine's own proof reads its frames.
    final class WatchedSocket: Socket, @unchecked Sendable {
        let real: Socket
        private let lock = NSLock()
        private var sentKinds: [String] = []
        private var receivedKinds: [String] = []

        init(real: Socket) {
            self.real = real
        }

        func sendText(text: String) async throws {
            lock.held { sentKinds.append(Self.kind(of: text)) }
            try await real.sendText(text: text)
        }

        func receive() async -> Received {
            let received = await real.receive()
            if case .text(let text) = received {
                lock.held { receivedKinds.append(Self.kind(of: text)) }
            }
            return received
        }

        func closeSocket() {
            real.closeSocket()
        }

        var sent: [String] {
            lock.held { sentKinds }
        }

        var received: [String] {
            lock.held { receivedKinds }
        }

        private static func kind(of text: String) -> String {
            let object = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any]
            return object?["type"] as? String ?? "?"
        }
    }

    /// Peer connections from `real`, with every one made kept for its statistics.
    final class RecordingPeerConnections: PeerConnectionFactory, @unchecked Sendable {
        private let real: CallPeerConnections
        private let lock = NSLock()
        private(set) var created: [PeerConnection] = []

        init(real: CallPeerConnections) {
            self.real = real
        }

        func create(configuration: Configuration, observer: PeerConnectionObserver) async throws -> CreatedPeerConnection {
            let made = try await real.create(configuration: configuration, observer: observer)
            lock.held { created.append(made.connection) }
            return made
        }

        var latest: PeerConnection? {
            lock.lock()
            defer { lock.unlock() }
            return created.last
        }
    }

    /// What an `inbound-rtp` audio entry of a statistics report measured.
    struct InboundAudio {
        let packets: Int
        let energy: Double
        let level: Double
    }

    private var keptPushState: String?
    private var helper: URL!

    private func environment(_ key: String) throws -> String {
        guard let value = ProcessInfo.processInfo.environment[key], !value.isEmpty else {
            throw XCTSkip("\(key) is not set")
        }
        return value
    }

    override func setUp() {
        super.setUp()
        // The simulator's audio server starts no output for a test process (each `AVAudioPlayer`
        // start blocks the main queue for the HAL's timeouts and fails), so the call's sounds
        // play on players in software, which read the recording and count as playing for its
        // length.
        CallSounds.makePlayer = { try SoftwareSoundPlayer(url: $0) }
    }

    override func tearDown() {
        if let keptPushState {
            _ = PushStateStore.save(json: keptPushState)
        } else {
            PushStateStore.remove()
        }
        if let helper {
            var request = URLRequest(url: helper.appendingPathComponent("second-participant/leave"))
            request.httpMethod = "POST"
            let done = expectation(description: "the browser is told to leave")
            URLSession.shared.dataTask(with: request) { _, _, _ in done.fulfill() }.resume()
            wait(for: [done], timeout: 10)
        }
        super.tearDown()
    }

    func testJoinsHearsASecondParticipantResumesAndLeaves() async throws {
        let api = try environment("ASPEN_CALL_API")
        let channel = try environment("ASPEN_CALL_CHANNEL")
        let user = try environment("ASPEN_CALL_USER_ID")
        let refreshToken = try environment("ASPEN_CALL_REFRESH_TOKEN")
        helper = URL(string: try environment("ASPEN_CALL_HELPER"))
        continueAfterFailure = false

        // The account as the web view keeps it on sign-in, with a session token the deployment
        // refuses, so the first request goes through the renewal.
        keptPushState = PushStateStore.loadJSON()
        let account: [String: Any] = [
            "subscription": "call-engine-test",
            "origin": api,
            "userId": user,
            "refreshToken": refreshToken,
            "sessionToken": "stale-session-token",
            "keys": ["privateKey": ["d": "AA"], "publicKey": "AA", "auth": "AA"],
        ]
        let state = try JSONSerialization.data(withJSONObject: ["version": 1, "accounts": [account]])
        XCTAssertTrue(PushStateStore.save(json: String(decoding: state, as: UTF8.self)))

        // WebRTC on a microphone and speaker in software (`ToneAudioDevice`): the simulator's
        // own audio unit aborts the process when initialized from a test.
        let media = CallPeerConnections(tracks: CallEngine.tracks, audioDevice: ToneAudioDevice())
        let sockets = CuttableSockets()
        let peerConnections = RecordingPeerConnections(real: media)
        let call = CallEngine(
            origin: api, loopbackPlainText: true, media: media, sockets: sockets, peerConnections: peerConnections
        )
        let seen = Seen()
        let listening = call.snapshots.listen { seen.add($0) }
        defer { listening.cancel() }
        let microphone = call.audio.openMicrophone()
        call.engine.setMicrophone(track: microphone)

        // Joined through the system, as far as the simulator takes it: the start action is
        // performed (the audio session configured, the engine told to join) and the call shows
        // in the system's list as outgoing. Then the simulator's CallKit ends it: it opens its
        // in-call UI (`facetime://?showInCallUI=1`) for every outgoing call, the simulator has
        // no such app, and the daemon ends a call whose UI it could not open, with an end action
        // the provider performs, which leaves the engine. A device, which has the in-call app,
        // keeps the call; that is the device checklist's.
        let observer = CXCallObserver()
        let calls = CallsSeen()
        observer.setDelegate(calls, queue: nil)
        XCTAssertTrue(observer.calls.isEmpty, "no call stands before the join: \(observer.calls)")
        await MainActor.run { call.join(channel: channel, name: "Call engine test") }
        // Both read off listeners' records, since the daemon's end follows within the polling
        // interval, and a call that has ended leaves the observer's list.
        try await untilTrue("the start action joins the engine", seconds: 10) { seen.statuses.contains { $0 != .idle } }
        let started = try XCTUnwrap(seen.statuses.first { $0 != .idle })
        try await untilTrue("the system lists the call as outgoing", seconds: 10) { calls.changes.contains(where: \.outgoing) }
        let systemCall = try XCTUnwrap(calls.changes.first(where: \.outgoing)).uuid
        XCTAssertNil(CallSystem.shared.latest.refusal)
        let systemEnded = await waiting(seconds: 15) { calls.changes.contains { $0.uuid == systemCall && $0.ended } }
        if systemEnded {
            // The simulator's daemon ended the call: the provider's end action left the engine.
            _ = try await until("the system's end action leaves the engine", seconds: 10, call) { $0.status == .idle }
            XCTAssertNil(CallSystem.shared.latest.call, "the status lets the call go")
        } else {
            // A CallKit that keeps the call (a device's, or a simulator that has learnt to):
            // left here, since what follows drives the engine directly.
            XCTAssertEqual(CallSystem.shared.latest.call, systemCall)
            await MainActor.run { call.leave() }
            _ = try await until("the call is left", seconds: 10, call) { $0.status == .idle }
            try await untilTrue("the system's call ends", seconds: 10) { calls.changes.contains { $0.uuid == systemCall && $0.ended } }
        }
        let viaSystem = "the system performed the start action (engine \(started)), listed the call \(systemCall) as outgoing, and \(systemEnded ? "ended it itself, as the simulator's CallKit does, the end action leaving the engine" : "kept it until leave")"

        // The audio proof drives the engine directly, the test standing in for CallKit's
        // `didActivate`, since the simulator's CallKit holds no call to activate the session for.
        try call.audio.configureForCall()
        call.audio.activated(AVAudioSession.sharedInstance())
        defer { call.audio.deactivated(AVAudioSession.sharedInstance()) }
        // The system's join opened a socket and a peer connection of its own, as far as it got.
        let socketsBefore = sockets.connections
        let connectionsBefore = peerConnections.created.count
        // The call's sounds, as the engine cues them and the layer plays them on the session the
        // test activated as CallKit would, into players in software (`setUp`): each is recorded
        // as it starts, with the player's state and the session's category and mode then.
        let sounds = SoundsSeen()
        let soundsListening = await MainActor.run { call.audio.sounds.listen { sounds.add($0) } }
        defer { soundsListening.cancel() }
        call.engine.setMicrophone(track: microphone)
        call.engine.join(channel: channel)
        defer { call.engine.leave() }
        let connected = try await until("the call connects", seconds: 40, call) { $0.status == .connected }
        try await untilTrue("the joining is played", seconds: 10) { sounds.played.contains { $0.cue == .callJoined } }
        let joined = try XCTUnwrap(sounds.played.first { $0.cue == .callJoined })
        XCTAssertTrue(joined.playing, "the player plays as the cue is given")
        XCTAssertEqual(joined.category, .playAndRecord, "on the call's session")
        XCTAssertEqual(joined.mode, .voiceChat)
        XCTAssertEqual(sounds.played.map(\.cue), [.callJoined], "nothing before the connection, and the joining once")
        XCTAssertEqual(connected.channel, channel)
        XCTAssertEqual(connected.user, user)
        XCTAssertNotNil(connected.session)
        XCTAssertTrue(connected.grants.speak)
        XCTAssertEqual(connected.microphone, microphone)
        let kept = await call.credentials.sessionToken()
        XCTAssertNotEqual(kept, "stale-session-token", "the renewed token is kept")
        let sending = try await until("the microphone is live at the server", seconds: 15, call) {
            $0.sources.contains { $0.source == .microphone && $0.live }
        }
        XCTAssertEqual(sending.sources.map(\.track), [microphone])
        XCTAssertEqual(sockets.connections, socketsBefore + 1)
        XCTAssertEqual(peerConnections.created.count, connectionsBefore + 1)
        let connection = try XCTUnwrap(peerConnections.latest)
        XCTAssertEqual(connection.connectionState(), .connected)

        // A second participant from a browser, playing a tone: they take an audio slot, whose
        // section the connection has a track for, and their sound arrives on it.
        let second = try await post(helper.appendingPathComponent("second-participant"))
        let bob = try XCTUnwrap(second["user"] as? String)
        let heard = try await until("the second participant takes a slot", seconds: 30, call) {
            $0.participants.contains { $0.user == bob } && $0.slots.contains { $0.user == bob && $0.track != nil }
        }
        XCTAssertGreaterThan(heard.audioSlots, 0)
        XCTAssertEqual(heard.slots.filter { $0.track != nil }.count, heard.slots.count, "every slot has a track")
        // Someone else joining is the joining sound again, played as the first was.
        try await untilTrue("the browser's joining is played", seconds: 10) { sounds.played.filter { $0.cue == .callJoined }.count == 2 }
        XCTAssertTrue(sounds.played.allSatisfy(\.playing))
        let before = try await inboundAudio(connection)
        try await Task.sleep(nanoseconds: 4_000_000_000)
        let after = try await inboundAudio(connection)
        XCTAssertGreaterThan(after.packets, before.packets + 100, "packets arrive: \(before) then \(after)")
        XCTAssertGreaterThan(after.energy, before.energy, "the tone's energy rises: \(before) then \(after)")
        XCTAssertGreaterThan(after.level, 0, "the tone has a level: \(after)")

        // And the browser hears the simulator's microphone, the tone the audio device records:
        // packets arrive and decode, and the sound off its track is well above silence.
        let browserBefore = try await browserInbound()
        try await Task.sleep(nanoseconds: 4_000_000_000)
        let browserAfter = try await browserInbound()
        XCTAssertGreaterThan(browserAfter.packets, browserBefore.packets, "the browser receives audio: \(browserBefore) then \(browserAfter)")
        XCTAssertGreaterThan(browserAfter.samples, browserBefore.samples, "and decodes it: \(browserBefore) then \(browserAfter)")
        let browserPeak = try await browserLoudness()
        XCTAssertGreaterThan(browserPeak, 0.01, "and hears sound in it")
        let source = try await sourceAudio(connection)

        // The app's mute, which reaches the engine directly while the system holds no call of
        // this engine's (and through a `CXSetMutedCallAction` while it does, which the simulator
        // cannot show).
        await MainActor.run { call.setMuted(true) }
        _ = try await until("the mute mutes", seconds: 10, call) { $0.muted }
        await MainActor.run { call.setMuted(false) }
        _ = try await until("the unmute unmutes", seconds: 15, call) { !$0.muted }

        // On hold, as `CallSystem` puts the call for CallKit's hold action and for an
        // interruption (the simulator's CallKit holds no call to do it through): shown at once,
        // confirmed by the server, which tells the browser, and the server sends neither side
        // the other's sound meanwhile. The release shows only on the server's word, and the
        // sound comes back with it.
        call.engine.setHeld(held: true)
        let held = try await until("the hold shows", seconds: 10, call) { $0.held }
        XCTAssertFalse(held.muted, "the hold is beside the mute, not a mute")
        try await untilReported("the browser sees the phone on hold", seconds: 15) { try await self.browserOnHold() == [user] }
        try await Task.sleep(nanoseconds: 1_000_000_000)
        let heldPhone = try await inboundAudio(connection)
        let heldBrowser = try await browserInbound()
        try await Task.sleep(nanoseconds: 2_000_000_000)
        let heldPhoneLater = try await inboundAudio(connection)
        let heldBrowserLater = try await browserInbound()
        XCTAssertLessThan(heldPhoneLater.packets - heldPhone.packets, 20, "held, the phone is sent nothing: \(heldPhone) then \(heldPhoneLater)")
        XCTAssertLessThan(heldBrowserLater.packets - heldBrowser.packets, 20, "held, the browser is sent nothing: \(heldBrowser) then \(heldBrowserLater)")
        call.engine.setHeld(held: false)
        _ = try await until("the server lifts the hold", seconds: 15, call) { !$0.held }
        try await untilReported("the browser sees the phone off hold", seconds: 15) { try await self.browserOnHold().isEmpty }
        let released = try await inboundAudio(connection)
        try await Task.sleep(nanoseconds: 2_000_000_000)
        let releasedLater = try await inboundAudio(connection)
        XCTAssertGreaterThan(releasedLater.packets, released.packets + 50, "released, the sound comes back: \(released) then \(releasedLater)")
        let hold = "held (the browser listed \(user) on hold; \(heldPhoneLater.packets - heldPhone.packets) packets reached the phone and \(heldBrowserLater.packets - heldBrowser.packets) the browser in 2 s) and released (\(releasedLater.packets - released.packets) packets in 2 s)"

        // Video both ways. The browser shows video and turns its camera on (Chromium's fake camera,
        // a moving pattern); the engine is asked to watch it, gets a section for it, and its track
        // is drawn by a renderer where the page would put the hole (`CallVideo.apply`), which
        // counts frames and reports the picture's shape; the renderer's layer lies where the hole
        // is. The frames arrive upright: the sender turns them, no orientation extension being
        // negotiated.
        _ = try await post(helper.appendingPathComponent("second-participant/show-video"))
        _ = try await post(helper.appendingPathComponent("second-participant/camera/start"))
        call.engine.setWatched(video: [WatchedVideo(user: bob, source: .camera)])
        let watching = try await until("the browser's camera is received", seconds: 30, call) {
            $0.videos.contains { $0.user == bob && $0.source == .camera }
        }
        let remoteSurface = try XCTUnwrap(watching.videos.first { $0.user == bob && $0.source == .camera }).track
        let remoteBox = CGRect(x: 10, y: 20, width: 160, height: 120)
        let remoteTile = CallVideoTile(
            tile: "remote", surface: remoteSurface, rect: remoteBox, visible: true, mirrored: false, cover: true,
            fullScreen: false, pictureInPicture: true)
        await MainActor.run { CallEngine.video.apply([remoteTile]) }
        let remoteRendererMade = await MainActor.run { CallEngine.video.renderers["remote"] }
        let remoteRenderer = try XCTUnwrap(remoteRendererMade)
        try await untilTrue("the browser's camera is drawn", seconds: 20) { remoteRenderer.framesRendered >= 5 }
        let remoteShape = try XCTUnwrap(remoteRenderer.lastShape)
        XCTAssertGreaterThan(remoteShape.width, 0)
        XCTAssertGreaterThan(remoteShape.height, 0)
        XCTAssertEqual(remoteShape.rotation, 0, "received frames arrive upright")
        let remoteDrawnAt = await MainActor.run { CallEngine.video.frame(ofTile: "remote") }
        XCTAssertEqual(remoteDrawnAt, remoteBox, "the picture lies where the page put its hole")

        // The system's picture in picture, on the renderer the page marked for it: possible once
        // the layer draws, started, and the engine told as it starts and stops. The simulator's
        // AVKit supports none, so there this is the device checklist's.
        let pip = CallEngine.video.pictureInPicture
        let pipSeen = Flags()
        let pipListening = await MainActor.run { CallEngine.video.listenPictureInPicture { pipSeen.add($0) } }
        defer { pipListening.cancel() }
        let pictureInPicture: String
        if CallPictureInPicture.isSupported {
            try await untilMain("the picture in picture is possible", seconds: 10) { pip.isPossible }
            await MainActor.run { pip.start() }
            try await untilMain("the system shows the picture in picture", seconds: 15) { pipSeen.values.last == true && pip.isActive }
            await MainActor.run { pip.stop() }
            try await untilMain("the picture in picture ends", seconds: 15) { pipSeen.values.last == false && !pip.isActive }
            pictureInPicture = "the picture in picture started on the browser's camera and stopped (\(pipSeen.values))"
        } else {
            pictureInPicture = "the system here supports no picture in picture (AVPictureInPictureController.isPictureInPictureSupported is false on the simulator), so starting one is the device's to show"
        }

        // The phone's camera, in software on the simulator (`SyntheticVideoCapturer`, 320 by 240
        // frames carried with a phone's upright rotation of 90 degrees), opened as the engine's
        // camera source: the preview is drawn upright (240 by 320; the track's source turns the
        // frames itself once the encoder wants them turned, so the preview sees them turned or
        // turns them), and the browser decodes the frames, upright, and sees a picture in them
        // rather than black.
        let camera = CallCamera(factory: media.factory, tracks: CallEngine.tracks)
        camera.makeCapturer = { SyntheticVideoCapturer() }
        let opened = try await cameraStarted(camera, device: nil, origin: api)
        XCTAssertEqual(opened.device, SyntheticVideoCapturer.front)
        call.engine.startSource(source: .camera, track: opened.track)
        _ = try await until("the camera is live at the server", seconds: 20, call) {
            $0.sources.contains { $0.source == .camera && $0.live }
        }
        let selfBox = CGRect(x: 200, y: 300, width: 90, height: 120)
        let selfTile = CallVideoTile(
            tile: "self", surface: opened.track, rect: selfBox, visible: true, mirrored: true, cover: true,
            fullScreen: false, pictureInPicture: false)
        await MainActor.run { CallEngine.video.apply([remoteTile, selfTile]) }
        let selfRendererMade = await MainActor.run { CallEngine.video.renderers["self"] }
        let selfRenderer = try XCTUnwrap(selfRendererMade)
        try await untilTrue("the preview is drawn", seconds: 15) { selfRenderer.framesRendered >= 5 }
        let selfShape = try XCTUnwrap(selfRenderer.lastShape)
        XCTAssertEqual(selfShape.surface, opened.track)
        XCTAssertEqual([selfShape.width, selfShape.height], [240, 320], "upright: \(selfShape)")
        let selfDrawnAt = await MainActor.run { CallEngine.video.frame(ofTile: "self") }
        XCTAssertEqual(selfDrawnAt, selfBox, "the turned, mirrored preview still lies in its hole")
        try await untilReported("the browser decodes the camera", seconds: 20) { try await self.browserVideoFrames() > 10 }
        let sample = try await untilSample("the browser sees a picture in it", seconds: 20, of: user)
        XCTAssertGreaterThan(sample.brightness, 40, "not black: \(sample)")
        XCTAssertLessThan(sample.width, sample.height, "upright, as the phone holds it: \(sample)")
        // Moved to the other camera: the same track, so the engine's sender needs nothing.
        let moved = try await cameraStarted(camera, device: SyntheticVideoCapturer.back.id, origin: api)
        XCTAssertEqual(moved.device, SyntheticVideoCapturer.back)
        XCTAssertEqual(moved.track, opened.track)
        let video = "watched the browser's camera (\(remoteRenderer.framesRendered) frames drawn, \(remoteShape.width)×\(remoteShape.height)) in its hole \(remoteBox); sent a camera the browser decoded (\(try await browserVideoFrames()) frames, a frame of brightness \(Int(sample.brightness)) at \(sample.width)×\(sample.height)) with its preview \(selfShape.width)×\(selfShape.height) (rotation \(selfShape.rotation)°) in \(selfBox); moved it to \(moved.device.id)"

        // The socket cut: the engine takes its place back with `resume` on a new socket, media
        // flowing meanwhile on the same peer connection, and the call connected throughout, as
        // the web client's stays through its own resume; the new socket's frames say the resume
        // was presented and answered `ready`.
        let session = connected.session
        seen.clear()
        sockets.cut()
        try await untilTrue("the engine presents the resume on a new socket", seconds: 20) {
            sockets.connections == socketsBefore + 2 && sockets.latestSent.first == "resume" && sockets.latestReceived.contains("ready")
        }
        let resumed = try await until("the call is connected once taken back", seconds: 10, call) { $0.status == .connected }
        XCTAssertFalse(seen.statuses.contains(.rejoining), "a resume within the grace keeps the call connected: \(seen.statuses)")
        XCTAssertEqual(resumed.session, session, "the same session")
        XCTAssertEqual(sockets.connections, socketsBefore + 2, "one socket more")
        XCTAssertEqual(peerConnections.created.count, connectionsBefore + 1, "the same peer connection")
        XCTAssertTrue(resumed.slots.contains { $0.user == bob }, "the slots stand")
        let afterResume = try await inboundAudio(connection)
        let framesAfterResume = remoteRenderer.framesRendered
        try await Task.sleep(nanoseconds: 2_000_000_000)
        let later = try await inboundAudio(connection)
        XCTAssertGreaterThan(later.packets, afterResume.packets, "media flows on: \(afterResume) then \(later)")
        XCTAssertGreaterThan(remoteRenderer.framesRendered, framesAfterResume, "video flows on")
        XCTAssertTrue(resumed.videos.contains { $0.track == remoteSurface }, "the video watched stands")

        // The camera off: its source leaves the state and the capture stops.
        call.engine.stopSource(source: .camera)
        await MainActor.run { camera.stop() }
        _ = try await until("the camera is gone from the sources", seconds: 10, call) {
            !$0.sources.contains { $0.source == .camera }
        }
        XCTAssertFalse(camera.isOn)
        await MainActor.run { CallEngine.video.apply([]) }

        // Leaving ends everything: the state is idle and the connection closed. `leave` goes
        // through the system when it holds the call; here it holds none, so the engine's own.
        await MainActor.run { call.leave() }
        let left = try await until("the call is left", seconds: 10, call) { $0.status == .idle }
        XCTAssertNil(left.session)
        XCTAssertTrue(left.slots.isEmpty)
        XCTAssertEqual(connection.connectionState(), .closed)
        XCTAssertNil(CallSystem.shared.latest.call)
        XCTAssertEqual(call.snapshots.latest, call.engine.state(), "the observer saw the latest snapshot")
        let revisions = seen.revisions
        XCTAssertTrue(zip(revisions, revisions.dropFirst()).allSatisfy { $1 == $0 + 1 }, "revisions count up by one: \(revisions)")
        // Leaving is the leaving sound, on the session the test still holds active, and the
        // resume within the grace was no loss: the joining as the engine connected and as the
        // browser joined, then the leaving.
        try await untilTrue("the leaving is played", seconds: 10) { sounds.played.contains { $0.cue == .callLeft } }
        let played = sounds.played
        XCTAssertTrue(played.allSatisfy { $0.playing && $0.category == .playAndRecord && $0.mode == .voiceChat }, "every sound played on the call's session: \(played)")
        XCTAssertEqual(played.map(\.cue), [.callJoined, .callJoined, .callLeft], "a resume within the grace is no loss")
        print("call-engine-test: \(viaSystem); then, driven directly, joined and heard \(after.packets - before.packets) packets of the browser's tone in 4 s (energy \(before.energy) → \(after.energy), level \(after.level)); this side's microphone source stood at energy \(source.energy), level \(source.level); the browser received \(browserAfter.packets - browserBefore.packets) packets and \(browserAfter.samples - browserBefore.samples) samples in 4 s, peaking at \(browserPeak); muted and unmuted; \(hold); \(video); \(pictureInPicture); resumed on socket 2 with session \(session ?? "?"), connected throughout, with video flowing on; camera off; the signalling socket negotiated \(sockets.latestExtensions); left; the sounds \(played.map { String(describing: $0.cue) }) each played into a player in software on the active session (\(joined.category.rawValue), \(joined.mode.rawValue)), the simulator's audio server starting no output for a test process")
    }

    // MARK: Helpers

    /// Snapshots as the listener saw them, on the main queue.
    final class Seen: @unchecked Sendable {
        private let lock = NSLock()
        private var snapshots: [CallState] = []

        func add(_ state: CallState) {
            lock.lock()
            snapshots.append(state)
            lock.unlock()
        }

        func clear() {
            lock.lock()
            snapshots.removeAll()
            lock.unlock()
        }

        var statuses: [CallStatus] {
            lock.lock()
            defer { lock.unlock() }
            return snapshots.map(\.status)
        }

        var revisions: [UInt64] {
            lock.lock()
            defer { lock.unlock() }
            return snapshots.map(\.revision)
        }
    }

    /// Booleans a listener reported, in order.
    final class Flags: @unchecked Sendable {
        private let lock = NSLock()
        private var recorded: [Bool] = []

        func add(_ value: Bool) {
            lock.held { recorded.append(value) }
        }

        var values: [Bool] {
            lock.lock()
            defer { lock.unlock() }
            return recorded
        }
    }

    /// Each of the call's sounds as the layer started playing it, on the main queue: the cue,
    /// whether its player was playing as it was given, and the session's category and mode then.
    final class SoundsSeen: @unchecked Sendable {
        struct Played {
            let cue: SoundCue
            let playing: Bool
            let category: AVAudioSession.Category
            let mode: AVAudioSession.Mode
        }

        private let lock = NSLock()
        private var recorded: [Played] = []

        func add(_ played: CallSounds.Played) {
            lock.held {
                recorded.append(Played(cue: played.cue, playing: played.player.isPlaying, category: played.category, mode: played.mode))
            }
        }

        var played: [Played] {
            lock.lock()
            defer { lock.unlock() }
            return recorded
        }
    }

    /// Every change the system's call observer reported, on the main queue.
    final class CallsSeen: NSObject, CXCallObserverDelegate, @unchecked Sendable {
        struct Change {
            let uuid: UUID
            let outgoing: Bool
            let connected: Bool
            let ended: Bool
        }

        private let lock = NSLock()
        private var recorded: [Change] = []

        func callObserver(_ callObserver: CXCallObserver, callChanged call: CXCall) {
            lock.held {
                recorded.append(Change(uuid: call.uuid, outgoing: call.isOutgoing, connected: call.hasConnected, ended: call.hasEnded))
            }
        }

        var changes: [Change] {
            lock.lock()
            defer { lock.unlock() }
            return recorded
        }
    }

    /// Ends the test after a failure that leaves nothing more to check.
    struct Stopped: Error {}

    /// The engine's state once `holds` of it, or a failure after `seconds`, naming the last state.
    private func until(
        _ what: String, seconds: Double, _ call: CallEngine, holds: @escaping (CallState) -> Bool
    ) async throws -> CallState {
        let deadline = Date().addingTimeInterval(seconds)
        var state = call.engine.state()
        while !holds(state) {
            if Date() > deadline {
                XCTFail("\(what) did not happen within \(seconds) s: \(state)")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 100_000_000)
            state = call.engine.state()
        }
        return state
    }

    /// Waits until `holds`, or fails after `seconds`.
    private func untilTrue(_ what: String, seconds: Double, holds: @escaping () -> Bool) async throws {
        guard await waiting(seconds: seconds, for: holds) else {
            XCTFail("\(what) did not happen within \(seconds) s")
            throw Stopped()
        }
    }

    /// Waits until `holds`, read on the main queue, or fails after `seconds`.
    private func untilMain(_ what: String, seconds: Double, holds: @escaping @MainActor () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(seconds)
        while await !MainActor.run(body: holds) {
            if Date() > deadline {
                XCTFail("\(what) did not happen within \(seconds) s")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    /// Waits until `holds`, read from the helper each time, or fails after `seconds`.
    private func untilReported(_ what: String, seconds: Double, holds: @escaping () async throws -> Bool) async throws {
        let deadline = Date().addingTimeInterval(seconds)
        while try await !holds() {
            if Date() > deadline {
                XCTFail("\(what) did not happen within \(seconds) s")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 500_000_000)
        }
    }

    /// Whether `holds` came true within `seconds`.
    private func waiting(seconds: Double, for holds: @escaping () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while !holds() {
            if Date() > deadline {
                return false
            }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        return true
    }

    private func post(_ url: URL) async throws -> [String: Any] {
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 60
        let (data, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200, String(decoding: data, as: UTF8.self))
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func get(_ url: URL) async throws -> [String: Any] {
        let (data, response) = try await URLSession.shared.data(from: url)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200, String(decoding: data, as: UTF8.self))
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// The connection's inbound audio, summed over its `inbound-rtp` audio entries.
    private func inboundAudio(_ connection: PeerConnection) async throws -> InboundAudio {
        let json = try await connection.stats(of: .connection)
        let entries = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[String: Any]])
        var summed = InboundAudio(packets: 0, energy: 0, level: 0)
        for entry in entries where entry["type"] as? String == "inbound-rtp" && entry["kind"] as? String == "audio" {
            summed = InboundAudio(
                packets: summed.packets + ((entry["packetsReceived"] as? NSNumber)?.intValue ?? 0),
                energy: summed.energy + ((entry["totalAudioEnergy"] as? NSNumber)?.doubleValue ?? 0),
                level: max(summed.level, (entry["audioLevel"] as? NSNumber)?.doubleValue ?? 0)
            )
        }
        return summed
    }

    /// What the microphone's source has captured (`media-source`), after WebRTC's processing.
    private func sourceAudio(_ connection: PeerConnection) async throws -> (energy: Double, level: Double) {
        let json = try await connection.stats(of: .connection)
        let entries = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[String: Any]])
        let source = entries.first { $0["type"] as? String == "media-source" && $0["kind"] as? String == "audio" }
        return (
            (source?["totalAudioEnergy"] as? NSNumber)?.doubleValue ?? 0,
            (source?["audioLevel"] as? NSNumber)?.doubleValue ?? 0
        )
    }

    /// What the browser's connection received of audio, from the helper's report of it.
    private func browserInbound() async throws -> (packets: Int, samples: Int) {
        let report = try await get(helper.appendingPathComponent("second-participant/report"))
        let streams = try XCTUnwrap(report["streams"] as? [[String: Any]])
        var packets = 0
        var samples = 0
        for stream in streams where stream["direction"] as? String == "inbound" && stream["kind"] as? String == "audio" {
            packets += (stream["packets"] as? NSNumber)?.intValue ?? 0
            samples += (stream["samples"] as? NSNumber)?.intValue ?? 0
        }
        return (packets, samples)
    }

    /// Whom the browser's call shows on hold, from the helper's report of it.
    private func browserOnHold() async throws -> [String] {
        let report = try await get(helper.appendingPathComponent("second-participant/report"))
        return try XCTUnwrap(report["onHold"] as? [String])
    }

    /// Opens `camera` on `device` for `origin`'s engine, on the main queue, and answers the track
    /// and the camera opened.
    private func cameraStarted(
        _ camera: CallCamera, device: String?, origin: String
    ) async throws -> (track: TrackId, device: CallCameraDevice) {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.main.async {
                camera.start(device: device, origin: origin) { result in
                    switch result {
                    case .success(let opened): continuation.resume(returning: opened)
                    case .failure(let failure): continuation.resume(throwing: failure)
                    }
                }
            }
        }
    }

    /// How many video frames the browser's connection has decoded, summed over its inbound video.
    private func browserVideoFrames() async throws -> Int {
        let report = try await get(helper.appendingPathComponent("second-participant/report"))
        let streams = try XCTUnwrap(report["streams"] as? [[String: Any]])
        return streams
            .filter { $0["direction"] as? String == "inbound" && $0["kind"] as? String == "video" }
            .reduce(0) { $0 + (($1["framesDecoded"] as? NSNumber)?.intValue ?? 0) }
    }

    /// A frame of `user`'s camera as the browser plays it (the page's `cameraSample`).
    struct CameraSample {
        let width: Int
        let height: Int
        let brightness: Double
    }

    /// A sample of `user`'s camera from the browser, waiting up to `seconds` for a frame.
    private func untilSample(_ what: String, seconds: Double, of user: String) async throws -> CameraSample {
        let deadline = Date().addingTimeInterval(seconds)
        while true {
            var request = URLRequest(url: helper.appendingPathComponent("second-participant/camera-sample"))
            request.url = URL(string: "\(request.url!.absoluteString)?user=\(user)")
            let (data, _) = try await URLSession.shared.data(for: request)
            if let sample = try JSONSerialization.jsonObject(with: data) as? [String: Any],
               let width = (sample["width"] as? NSNumber)?.intValue, let height = (sample["height"] as? NSNumber)?.intValue,
               let brightness = (sample["brightness"] as? NSNumber)?.doubleValue, width > 0 {
                return CameraSample(width: width, height: height, brightness: brightness)
            }
            if Date() > deadline {
                XCTFail("\(what) did not happen within \(seconds) s")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 500_000_000)
        }
    }

    /// The loudest sample the browser's audio slots carried over a second, off their tracks (the
    /// page's `loudness`): its own level statistics read zero behind the muted element it plays
    /// each track in.
    private func browserLoudness() async throws -> Double {
        let answer = try await get(helper.appendingPathComponent("second-participant/loudness"))
        return (answer["peak"] as? NSNumber)?.doubleValue ?? 0
    }
}

private extension NSLock {
    /// Runs `body` holding the lock, from an async method too, where `lock()` and `unlock()` are
    /// not for use.
    func held<T>(_ body: () -> T) -> T {
        lock()
        defer { unlock() }
        return body()
    }
}
