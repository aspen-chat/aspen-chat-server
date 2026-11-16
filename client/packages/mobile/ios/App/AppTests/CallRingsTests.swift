import CallKit
import CryptoKit
import Network
import XCTest
@testable import App

/// A `call` push handled as PushKit would hand it over, on the simulator: encrypted to a key the
/// test writes into the kept push state as the web view would, read and reported to CallKit as an
/// incoming call before the handler returns, ended by the ring's clock when nothing can be
/// watched, and, when it cannot be read, reported and ended at once; and the answer action
/// performed on the provider's delegate as CallKit would deliver it, which joins the ring's
/// channel on the deployment's engine with the microphone. The simulator's CallKit takes incoming
/// calls, so `CXCallObserver` shows what the system was told. A real VoIP push, the lock screen's
/// ring, and answering from it are device checks (`ios-app.md`, Rings).
final class CallRingsTests: XCTestCase {
    private var keptPushState: String?

    override func setUp() {
        super.setUp()
        keptPushState = PushStateStore.loadJSON()
    }

    override func tearDown() {
        if let keptPushState {
            _ = PushStateStore.save(json: keptPushState)
        } else {
            PushStateStore.remove()
        }
        super.tearDown()
    }

    /// The subscription's keys, as the web view makes them, and the push state holding them.
    private struct Phone {
        let key = P256.KeyAgreement.PrivateKey()
        let auth = Data((0..<16).map { _ in UInt8.random(in: 0...255) })
        let subscription = "ring-test-\(UUID().uuidString.lowercased())"

        var publicKey: Data {
            key.publicKey.x963Representation
        }

        func keep(origin: String) throws {
            let account: [String: Any] = [
                "subscription": subscription,
                "origin": origin,
                "userId": UUID().uuidString.lowercased(),
                "refreshToken": "refresh",
                "sessionToken": "session",
                "keys": [
                    "privateKey": ["d": key.rawRepresentation.base64url],
                    "publicKey": publicKey.base64url,
                    "auth": auth.base64url,
                ],
            ]
            let state = try JSONSerialization.data(withJSONObject: [
                "version": 1,
                "device": ["id": "device-1", "secret": "secret-1", "token": "t"],
                "accounts": [account],
            ])
            XCTAssertTrue(PushStateStore.save(json: String(decoding: state, as: UTF8.self)))
        }

        /// RFC 8291 (`aes128gcm`, one record), as the deployment encrypts a pointer.
        func encrypt(_ plaintext: Data) throws -> Data {
            let ephemeral = P256.KeyAgreement.PrivateKey()
            let secret = try ephemeral.sharedSecretFromKeyAgreement(with: key.publicKey)
            var info = Data("WebPush: info\0".utf8)
            info.append(publicKey)
            info.append(ephemeral.publicKey.x963Representation)
            let ikm = secret.hkdfDerivedSymmetricKey(using: SHA256.self, salt: auth, sharedInfo: info, outputByteCount: 32)
            let salt = Data((0..<16).map { _ in UInt8.random(in: 0...255) })
            let cek = HKDF<SHA256>.deriveKey(inputKeyMaterial: ikm, salt: salt, info: Data("Content-Encoding: aes128gcm\0".utf8), outputByteCount: 16)
            let nonce = HKDF<SHA256>.deriveKey(inputKeyMaterial: ikm, salt: salt, info: Data("Content-Encoding: nonce\0".utf8), outputByteCount: 12)
            let sealed = try AES.GCM.seal(
                plaintext + Data([2]),
                using: cek,
                nonce: AES.GCM.Nonce(data: nonce.withUnsafeBytes { Data($0) })
            )
            var message = salt
            message.append(contentsOf: [0, 0, 16, 0])
            message.append(65)
            message.append(ephemeral.publicKey.x963Representation)
            message.append(sealed.ciphertext)
            message.append(sealed.tag)
            return message
        }

        func payload(ciphertext: Data) -> [AnyHashable: Any] {
            ["aspen": ["s": subscription, "c": ciphertext.base64url]]
        }
    }

    /// Watches the system's calls, keeping each change.
    private final class Calls: NSObject, CXCallObserverDelegate {
        let observer = CXCallObserver()
        var changes: [(UUID, Bool, Bool, Bool)] = []
        var onChange: (() -> Void)?

        override init() {
            super.init()
            observer.setDelegate(self, queue: .main)
        }

        func callObserver(_ callObserver: CXCallObserver, callChanged call: CXCall) {
            changes.append((call.uuid, call.isOutgoing, call.hasConnected, call.hasEnded))
            onChange?()
        }

        /// Waits for a change `matching` within `seconds`.
        func wait(_ test: XCTestCase, seconds: TimeInterval, _ matching: @escaping (UUID, Bool, Bool, Bool) -> Bool) {
            if changes.contains(where: matching) {
                return
            }
            let seen = test.expectation(description: "a call changed as expected")
            onChange = { [self] in
                if changes.contains(where: matching) {
                    seen.fulfill()
                    onChange = nil
                }
            }
            test.wait(for: [seen], timeout: seconds)
        }
    }

    /// Stands in for the deployment at a loopback port, serving nothing: it keeps each request's
    /// head (the request line and headers) and answers `404`, so the watch's stream is refused and
    /// tried again, and a decline is answered as a deployment that forgot the ring would.
    private final class Deployment {
        private(set) var port: UInt16 = 0
        private let listener: NWListener
        private let queue = DispatchQueue(label: "deployment")
        private let lock = NSLock()
        private var heads: [String] = []
        private var onRequest: (() -> Void)?

        var requests: [String] {
            lock.lock()
            defer { lock.unlock() }
            return heads
        }

        init() throws {
            listener = try NWListener(using: .tcp, on: .any)
            let listening = DispatchSemaphore(value: 0)
            var port: UInt16 = 0
            listener.stateUpdateHandler = { [listener] state in
                if case .ready = state {
                    port = listener.port?.rawValue ?? 0
                    listening.signal()
                }
            }
            listener.newConnectionHandler = { [weak self] connection in
                self?.serve(connection)
            }
            listener.start(queue: queue)
            if listening.wait(timeout: .now() + 5) == .timedOut || port == 0 {
                throw XCTSkip("the stand-in deployment could not listen")
            }
            self.port = port
        }

        deinit {
            listener.cancel()
        }

        private func serve(_ connection: NWConnection) {
            connection.start(queue: queue)
            receive(connection, head: Data())
        }

        private func receive(_ connection: NWConnection, head: Data) {
            connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, done, _ in
                guard let self else {
                    connection.cancel()
                    return
                }
                var head = head
                if let data {
                    head.append(data)
                }
                if let end = head.range(of: Data("\r\n\r\n".utf8)) {
                    let text = String(decoding: head[..<end.lowerBound], as: UTF8.self)
                    let onRequest: (() -> Void)?
                    self.lock.lock()
                    self.heads.append(text)
                    onRequest = self.onRequest
                    self.lock.unlock()
                    onRequest?()
                    let answer = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    connection.send(content: Data(answer.utf8), completion: .contentProcessed { _ in
                        connection.cancel()
                    })
                } else if done {
                    connection.cancel()
                } else {
                    self.receive(connection, head: head)
                }
            }
        }

        /// Waits for a request whose head `matching` accepts, within `seconds`.
        func wait(_ test: XCTestCase, seconds: TimeInterval, _ matching: @escaping (String) -> Bool) {
            lock.lock()
            if heads.contains(where: matching) {
                lock.unlock()
                return
            }
            let seen = test.expectation(description: "the deployment was asked as expected")
            onRequest = { [self] in
                lock.lock()
                let found = heads.contains(where: matching)
                if found {
                    onRequest = nil
                }
                lock.unlock()
                if found {
                    seen.fulfill()
                }
            }
            lock.unlock()
            test.wait(for: [seen], timeout: seconds)
        }
    }

    @MainActor
    func testACallPushIsReportedAsAnIncomingCallWatchedAndDeclinedAtTheDeployment() throws {
        // A stand-in deployment that serves nothing: the watch asks it for the event stream every
        // half second, and whichever ends the call, the system (the simulator's CallKit ends an
        // incoming call on its own, which is a decline here) or the ring's clock twelve seconds
        // on (past the ten the device's reading is given at least before `ready`), the requests
        // the deployment saw say what the app did.
        let deployment = try Deployment()
        let origin = "http://127.0.0.1:\(deployment.port)"
        let phone = Phone()
        try phone.keep(origin: origin)
        let rung = Date()
        let until = ISO8601DateFormatter().string(from: rung.addingTimeInterval(12))
        let channel = UUID().uuidString.lowercased()
        let pointer: [String: Any] = [
            "v": 1, "kind": "call", "channel": channel,
            "session": UUID().uuidString.lowercased(), "caller": UUID().uuidString.lowercased(),
            "callerName": "Alice", "until": until,
        ]
        let ciphertext = try phone.encrypt(JSONSerialization.data(withJSONObject: pointer))
        let read = CallRings.read(payload: phone.payload(ciphertext: ciphertext))
        XCTAssertEqual(
            try read.get(),
            CallRings.IncomingRing(
                origin: origin, channel: channel, session: pointer["session"] as! String,
                callerName: "Alice", until: until
            )
        )

        let calls = Calls()
        CallRings.shared.handle(payload: phone.payload(ciphertext: ciphertext), mustReport: true)
        let ringing = try XCTUnwrap(CallSystem.shared.latest.incoming, "the call is reported before the handler returns")
        XCTAssertNil(CallSystem.shared.latest.ringEnd)
        calls.wait(self, seconds: 5) { uuid, outgoing, _, ended in uuid == ringing && !outgoing && !ended }
        deployment.wait(self, seconds: 5) { $0.hasPrefix("GET /api/v1/events") && $0.contains("Upgrade: websocket") }
        calls.wait(self, seconds: 16) { uuid, _, _, ended in uuid == ringing && ended }
        print("call-rings-test: the system's calls changed \(calls.changes); status \(CallSystem.shared.latest)")
        XCTAssertNil(CallSystem.shared.latest.incoming)
        switch CallSystem.shared.latest.ringEnd {
        case .timedOut:
            // Nothing answered, so the watch ended the ring by the clock, and the deployment was
            // asked for the stream throughout.
            XCTAssertGreaterThanOrEqual(Date().timeIntervalSince(rung), 11.5, "at the ring's end, not before")
            XCTAssertGreaterThan(deployment.requests.filter { $0.hasPrefix("GET /api/v1/events") }.count, 10)
        case nil:
            // The system ended the call: declined here, so the deployment is told as the account.
            let decline = "DELETE /api/v1/channels/\(channel)/voice/rings/@me"
            deployment.wait(self, seconds: 5) { $0.hasPrefix(decline) && $0.contains("Authorization: Bearer session") }
            XCTAssertTrue(deployment.requests.contains { $0.hasPrefix(decline) }, "the decline reached the deployment")
        case .answeredElsewhere, .declined, .callEnded, .signedOut:
            XCTFail("the ring ended for a reason a deployment that serves nothing cannot give: \(String(describing: CallSystem.shared.latest.ringEnd))")
        }
    }

    /// Keeps every snapshot of an engine, on the main queue.
    private final class Snapshots {
        var seen: [CallState] = []
        var onChange: (() -> Void)?

        func add(_ state: CallState) {
            seen.append(state)
            onChange?()
        }

        /// Waits for a snapshot `matching` within `seconds`, and answers the first that does.
        func wait(_ test: XCTestCase, seconds: TimeInterval, _ matching: @escaping (CallState) -> Bool) throws -> CallState {
            if let found = seen.first(where: matching) {
                return found
            }
            let found = test.expectation(description: "the engine published a snapshot as expected")
            onChange = { [self] in
                if seen.contains(where: matching) {
                    found.fulfill()
                    onChange = nil
                }
            }
            test.wait(for: [found], timeout: seconds)
            return try XCTUnwrap(seen.first(where: matching))
        }
    }

    @MainActor
    func testAnsweringJoinsTheRingsChannelWithTheMicrophone() throws {
        // The simulator's CallKit ends an incoming call on its own within a second or two, so
        // the answer is performed on the provider's delegate as CallKit delivers one, rather than
        // requested of the system, which may have ended the call by then. The stand-in
        // deployment answers the join's first request with 404, so the join fails and the
        // system's call ends with it; the joining snapshot before that shows what the answer
        // gave the engine.
        let deployment = try Deployment()
        let origin = "http://127.0.0.1:\(deployment.port)"
        let phone = Phone()
        try phone.keep(origin: origin)
        // The engine the answer makes runs on an audio device in software, as every engine made
        // on the simulator must (`CallEngineTests`).
        CallEngine.peerConnections = CallPeerConnections(tracks: CallEngine.tracks, audioDevice: ToneAudioDevice())
        let channel = UUID().uuidString.lowercased()
        let pointer: [String: Any] = [
            "v": 1, "kind": "call", "channel": channel,
            "session": UUID().uuidString.lowercased(), "caller": UUID().uuidString.lowercased(),
            "callerName": "Alice", "until": ISO8601DateFormatter().string(from: Date().addingTimeInterval(30)),
        ]
        let calls = Calls()
        CallRings.shared.handle(payload: phone.payload(ciphertext: try phone.encrypt(JSONSerialization.data(withJSONObject: pointer))), mustReport: true)
        let ringing = try XCTUnwrap(CallSystem.shared.latest.incoming, "the call is reported before the handler returns")
        XCTAssertNil(CallEngines.shared.existing(origin), "no engine runs for a ring that is only ringing")

        let snapshots = Snapshots()
        let engine = CallEngines.shared.forOrigin(origin)
        let listening = engine.snapshots.listen { snapshots.add($0) }
        defer { listening.cancel() }
        XCTAssertNil(engine.engine.state().microphone)
        let system = CallSystem.shared
        system.provider(system.provider, perform: CXAnswerCallAction(call: ringing))
        XCTAssertNil(system.latest.incoming, "answered, the call no longer rings")
        XCTAssertEqual(system.latest.call, ringing, "the system's call is the answered one")
        XCTAssertNil(system.latest.refusal)

        let joining = try snapshots.wait(self, seconds: 10) { $0.status == .joining }
        let microphone = try XCTUnwrap(joining.microphone, "the answer gave the engine the microphone before the join")
        XCTAssertEqual(microphone, engine.audio.openMicrophone(), "the microphone is the engine's own, kept open")
        XCTAssertEqual(joining.channel, channel, "the ring's channel is joined")
        let join = "POST /api/v1/channels/\(channel)/voice/join"
        deployment.wait(self, seconds: 5) { $0.hasPrefix(join) && $0.contains("Authorization: Bearer session") }
        let failed = try snapshots.wait(self, seconds: 10) { $0.status == .failed }
        calls.wait(self, seconds: 10) { uuid, _, _, ended in uuid == ringing && ended }
        print("call-rings-test: answered; the engine joined \(channel) with microphone \(microphone) as its joining snapshot (revision \(joining.revision)) shows, asked the deployment to join, and failed with \(String(describing: failed.errorKind)) at its 404, on which the system's call ended; status \(system.latest)")
        XCTAssertNil(system.latest.call, "the system's call ended with the engine's failure")
        XCTAssertNotEqual(failed.errorKind, .microphone, "the join did not fail for want of a microphone")
    }

    @MainActor
    func testAPushThatCannotBeReadIsReportedAndEndedAtOnce() throws {
        let phone = Phone()
        try phone.keep(origin: "http://127.0.0.1:1")
        let calls = Calls()
        var garbage = try phone.encrypt(Data("not for this key".utf8))
        garbage[garbage.count - 1] ^= 0xff
        XCTAssertEqual(CallRings.read(payload: phone.payload(ciphertext: garbage)).failure, .doesNotDecrypt)
        CallRings.shared.handle(payload: phone.payload(ciphertext: garbage), mustReport: true)
        XCTAssertNil(CallSystem.shared.latest.incoming, "an unreadable push rings nothing")
        calls.wait(self, seconds: 5) { _, outgoing, _, ended in !outgoing && ended }
        XCTAssertTrue(calls.changes.contains { _, outgoing, _, ended in !outgoing && ended }, "the system was told of a call, and of its end")

        XCTAssertEqual(CallRings.read(payload: ["aspen": ["s": "unknown", "c": "AAAA"]]).failure, .noAccount)
        XCTAssertEqual(CallRings.read(payload: [:]).failure, .noSubscription)
        let message = try phone.encrypt(JSONSerialization.data(withJSONObject: ["v": 1, "kind": "message", "channel": "c", "message": "m"]))
        XCTAssertEqual(CallRings.read(payload: phone.payload(ciphertext: message)).failure, .notACall)
    }

    @MainActor
    func testAPushTheSystemDoesNotRequireReportedShowsNothing() throws {
        let phone = Phone()
        try phone.keep(origin: "http://127.0.0.1:1")
        let pointer: [String: Any] = [
            "v": 1, "kind": "call", "channel": UUID().uuidString.lowercased(),
            "session": UUID().uuidString.lowercased(), "caller": UUID().uuidString.lowercased(),
            "callerName": "Alice", "until": ISO8601DateFormatter().string(from: Date().addingTimeInterval(30)),
        ]
        let calls = Calls()
        CallRings.shared.handle(
            payload: phone.payload(ciphertext: try phone.encrypt(JSONSerialization.data(withJSONObject: pointer))),
            mustReport: false
        )
        XCTAssertNil(CallSystem.shared.latest.incoming)
        let quiet = expectation(description: "no call is reported")
        quiet.isInverted = true
        calls.onChange = { quiet.fulfill() }
        wait(for: [quiet], timeout: 1)
    }
}

private extension Data {
    var base64url: String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

private extension Result {
    var failure: Failure? {
        if case .failure(let failure) = self {
            return failure
        }
        return nil
    }
}
