import CallKit
import CryptoKit
import XCTest
@testable import App

/// A `call` push handled as PushKit would hand it over, on the simulator: encrypted to a key the
/// test writes into the kept push state as the web view would, read and reported to CallKit as an
/// incoming call before the handler returns, ended by the ring's clock when nothing can be
/// watched, and, when it cannot be read, reported and ended at once. The simulator's CallKit takes
/// incoming calls, so `CXCallObserver` shows what the system was told. A real VoIP push, the lock
/// screen's ring, and answering from it are device checks (`ios-app.md`, Rings).
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

    @MainActor
    func testACallPushIsReportedAsAnIncomingCallAndEndsWithTheRing() throws {
        // A deployment nothing answers at: the ring can only be watched by the clock.
        let origin = "http://127.0.0.1:1"
        let phone = Phone()
        try phone.keep(origin: origin)
        let until = ISO8601DateFormatter().string(from: Date().addingTimeInterval(4))
        let pointer: [String: Any] = [
            "v": 1, "kind": "call", "channel": UUID().uuidString.lowercased(),
            "session": UUID().uuidString.lowercased(), "caller": UUID().uuidString.lowercased(),
            "callerName": "Alice", "until": until,
        ]
        let ciphertext = try phone.encrypt(JSONSerialization.data(withJSONObject: pointer))
        let read = CallRings.read(payload: phone.payload(ciphertext: ciphertext))
        XCTAssertEqual(
            try read.get(),
            CallRings.IncomingRing(
                origin: origin, channel: pointer["channel"] as! String, session: pointer["session"] as! String,
                callerName: "Alice", until: until
            )
        )

        let calls = Calls()
        CallRings.shared.handle(payload: phone.payload(ciphertext: ciphertext), mustReport: true)
        let ringing = try XCTUnwrap(CallSystem.shared.latest.incoming, "the call is reported before the handler returns")
        calls.wait(self, seconds: 5) { uuid, outgoing, _, ended in uuid == ringing && !outgoing && !ended }
        // Nothing listens at the deployment, so the watch ends the ring by its clock.
        calls.wait(self, seconds: 10) { uuid, _, _, ended in uuid == ringing && ended }
        XCTAssertNil(CallSystem.shared.latest.incoming)
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
