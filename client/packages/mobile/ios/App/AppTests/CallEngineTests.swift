import AVFoundation
import CallKit
import XCTest
@testable import App

/// The call engine with this device's whole platform under it, against a real deployment: it
/// joins a voice channel's call through the system (CallKit) as far as the simulator's CallKit
/// takes an outgoing call, then, driven directly, joins, is heard by and hears a second
/// participant in a browser, mutes and unmutes, holds the call and releases it, takes its
/// socket back after the socket is cut, and leaves.
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
        private(set) var opened: [Socket] = []

        func connect(url: String, bearer: String?) async throws -> ConnectedSocket {
            let connected = try await real.connect(url: url, bearer: bearer)
            lock.held { opened.append(connected.socket) }
            return connected
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
            return (opened.last as? CallSocket)?.negotiatedExtensions ?? "none"
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
        call.engine.setMicrophone(track: microphone)
        call.engine.join(channel: channel)
        defer { call.engine.leave() }
        let connected = try await until("the call connects", seconds: 40, call) { $0.status == .connected }
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

        // The socket cut: the engine takes its place back with `resume` on a new socket, media
        // flowing meanwhile on the same peer connection.
        let session = connected.session
        seen.clear()
        sockets.cut()
        let resumed = try await until("the call resumes", seconds: 20, call) { state in
            state.status == .connected && seen.statuses.contains(.rejoining)
        }
        XCTAssertEqual(resumed.session, session, "the same session")
        XCTAssertEqual(sockets.connections, socketsBefore + 2, "one socket more")
        XCTAssertEqual(peerConnections.created.count, connectionsBefore + 1, "the same peer connection")
        XCTAssertTrue(resumed.slots.contains { $0.user == bob }, "the slots stand")
        let afterResume = try await inboundAudio(connection)
        try await Task.sleep(nanoseconds: 2_000_000_000)
        let later = try await inboundAudio(connection)
        XCTAssertGreaterThan(later.packets, afterResume.packets, "media flows on: \(afterResume) then \(later)")

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
        print("call-engine-test: \(viaSystem); then, driven directly, joined and heard \(after.packets - before.packets) packets of the browser's tone in 4 s (energy \(before.energy) → \(after.energy), level \(after.level)); this side's microphone source stood at energy \(source.energy), level \(source.level); the browser received \(browserAfter.packets - browserBefore.packets) packets and \(browserAfter.samples - browserBefore.samples) samples in 4 s, peaking at \(browserPeak); muted and unmuted; \(hold); resumed on socket 2 with session \(session ?? "?"); the signalling socket negotiated \(sockets.latestExtensions); left")
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
