import XCTest
@testable import App

/// The call engine with this device's whole platform under it, against a real deployment: it
/// joins a voice channel's call, is heard by and hears a second participant in a browser, takes
/// its socket back after the socket is cut, and leaves. `scripts/check_mobile_call.py` at the
/// repository's root runs the deployment, the browser, and this test, and gives it what it reads
/// from the environment (passed on by `xcodebuild test` with a `TEST_RUNNER_` prefix); without
/// them it skips.
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
        try call.audio.configureForCall()
        try call.audio.activate()
        defer { call.audio.deactivate() }
        let microphone = call.audio.openMicrophone()
        call.engine.setMicrophone(track: microphone)
        call.engine.join(channel: channel)

        // Joined: the microphone is sent and the renewed session token is kept.
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
        XCTAssertEqual(sockets.connections, 1)
        XCTAssertEqual(peerConnections.created.count, 1)
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

        // The socket cut: the engine takes its place back with `resume` on a new socket, media
        // flowing meanwhile on the same peer connection.
        let session = connected.session
        seen.clear()
        sockets.cut()
        let resumed = try await until("the call resumes", seconds: 20, call) { state in
            state.status == .connected && seen.statuses.contains(.rejoining)
        }
        XCTAssertEqual(resumed.session, session, "the same session")
        XCTAssertEqual(sockets.connections, 2, "one socket more")
        XCTAssertEqual(peerConnections.created.count, 1, "the same peer connection")
        XCTAssertTrue(resumed.slots.contains { $0.user == bob }, "the slots stand")
        let afterResume = try await inboundAudio(connection)
        try await Task.sleep(nanoseconds: 2_000_000_000)
        let later = try await inboundAudio(connection)
        XCTAssertGreaterThan(later.packets, afterResume.packets, "media flows on: \(afterResume) then \(later)")

        // Leaving ends everything: the state is idle and the connection closed.
        call.engine.leave()
        let left = try await until("the call is left", seconds: 10, call) { $0.status == .idle }
        XCTAssertNil(left.session)
        XCTAssertTrue(left.slots.isEmpty)
        XCTAssertEqual(connection.connectionState(), .closed)
        XCTAssertEqual(call.snapshots.latest, call.engine.state(), "the observer saw the latest snapshot")
        let revisions = seen.revisions
        XCTAssertTrue(zip(revisions, revisions.dropFirst()).allSatisfy { $1 == $0 + 1 }, "revisions count up by one: \(revisions)")
        print("call-engine-test: joined, heard \(after.packets - before.packets) packets of the browser's tone in 4 s (energy \(before.energy) → \(after.energy), level \(after.level)); this side's microphone source stood at energy \(source.energy), level \(source.level); the browser received \(browserAfter.packets - browserBefore.packets) packets and \(browserAfter.samples - browserBefore.samples) samples in 4 s, peaking at \(browserPeak); resumed on socket 2 with session \(session ?? "?"); left")
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
