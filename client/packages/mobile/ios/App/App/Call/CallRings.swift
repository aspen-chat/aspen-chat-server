import Foundation
import PushKit
import os

/// A DM's call ringing this phone, from a `call` push to its end (`spec/push.md`, Calls and Rings):
/// PushKit's VoIP pushes, the VoIP token they need registered with the relay beside the alert
/// token the web view registers, and the handler that turns a push into an incoming call. A VoIP
/// push wakes the app itself, not the notification service extension, and the call must be
/// reported to CallKit before the handler returns (`CallSystem.reportIncoming`); one the app
/// cannot read is reported and ended at once, since an app that reports nothing for a VoIP push
/// is terminated. From iOS 26.4 PushKit says whether a push must be reported at all
/// (`PKVoIPPushMetadata.mustReport`): not when the app is in the foreground, already in a call,
/// or the push was delayed too long, and then nothing is shown, the web view's own event stream
/// telling of the ring where it matters. Nothing is ever pushed to end a ring; `CallSystem` learns
/// of that through the engine's ring watch.
final class CallRings: NSObject, PKPushRegistryDelegate {
    static let shared = CallRings()

    /// What a `call` pointer says, read for `CallSystem`.
    struct IncomingRing: Equatable {
        /// The deployment's `public_url`, from the account the push's subscription names.
        let origin: String
        let channel: String
        let session: String
        let callerName: String
        /// RFC 3339, on the deployment's clock.
        let until: String
    }

    private static let log = Logger(subsystem: "com.aspen-chat.client", category: "rings")
    /// Where the VoIP token, and which device it was last registered for, are kept between
    /// launches, so the relay is told once per token and device.
    private static let tokenKey = "AspenVoipToken"
    private static let registeredKey = "AspenVoipTokenRegistered"

    private var registry: PKPushRegistry?
    private let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 15
        return URLSession(configuration: configuration)
    }()

    /// Asks PushKit for VoIP pushes. Call once, at launch, on the main queue: a VoIP push that
    /// starts the app is delivered only after the registry exists.
    func start() {
        guard registry == nil else {
            return
        }
        let registry = PKPushRegistry(queue: .main)
        registry.delegate = self
        registry.desiredPushTypes = [.voIP]
        self.registry = registry
    }

    // MARK: The VoIP token

    func pushRegistry(_ registry: PKPushRegistry, didUpdate pushCredentials: PKPushCredentials, for type: PKPushType) {
        guard type == .voIP else {
            return
        }
        let token = pushCredentials.token.map { String(format: "%02x", $0) }.joined()
        UserDefaults.standard.set(token, forKey: Self.tokenKey)
        registerWithRelay()
    }

    func pushRegistry(_ registry: PKPushRegistry, didInvalidatePushTokenFor type: PKPushType) {
        guard type == .voIP else {
            return
        }
        UserDefaults.standard.removeObject(forKey: Self.tokenKey)
        UserDefaults.standard.removeObject(forKey: Self.registeredKey)
    }

    /// The web view kept a new `PushState` (`AspenPushPlugin.saveState`): its device at the relay
    /// may be a new one, which must be told the VoIP token too.
    func pushStateSaved() {
        registerWithRelay()
    }

    /// Tells the relay the VoIP token for the device the web view registered
    /// (`PATCH /v1/devices/{device}` with `voipToken`), once per token and device; nothing is
    /// sent until both exist. The relay is the one this build names (`AspenPushRelay`).
    private func registerWithRelay() {
        guard let token = UserDefaults.standard.string(forKey: Self.tokenKey),
              let device = PushStateStore.load()?.device,
              let relay = Bundle.main.object(forInfoDictionaryKey: "AspenPushRelay") as? String,
              !relay.isEmpty,
              let url = URL(string: "\(CallEngines.trimmed(relay))/v1/devices/\(device.id)")
        else {
            return
        }
        let registered = "\(device.id) \(token)"
        if UserDefaults.standard.string(forKey: Self.registeredKey) == registered {
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "PATCH"
        request.setValue("Bearer \(device.secret)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["voipToken": token])
        session.dataTask(with: request) { _, response, error in
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            if status == 204 {
                UserDefaults.standard.set(registered, forKey: Self.registeredKey)
            } else {
                Self.log.error("registering the VoIP token with the relay answered \(status, privacy: .public): \(String(describing: error), privacy: .public)")
            }
        }.resume()
    }

    // MARK: Pushes

    /// Before iOS 26.4: every VoIP push must be reported.
    func pushRegistry(
        _ registry: PKPushRegistry,
        didReceiveIncomingPushWith payload: PKPushPayload,
        for type: PKPushType,
        completion: @escaping () -> Void
    ) {
        if type == .voIP {
            handle(payload: payload.dictionaryPayload, mustReport: true)
        }
        completion()
    }

    @available(iOS 26.4, *)
    func pushRegistry(
        _ registry: PKPushRegistry,
        didReceiveIncomingVoIPPushWith payload: PKPushPayload,
        metadata: PKVoIPPushMetadata,
        withCompletionHandler completion: @escaping @Sendable () -> Void
    ) {
        handle(payload: payload.dictionaryPayload, mustReport: metadata.mustReport)
        completion()
    }

    /// Reads a VoIP push's payload and reports what it says to CallKit, synchronously, before
    /// returning: the call it rings, or, when it cannot be read and must be reported, a call
    /// ended at once. On the main queue.
    func handle(payload: [AnyHashable: Any], mustReport: Bool) {
        guard mustReport else {
            Self.log.info("a VoIP push the system does not require reported; nothing shown")
            return
        }
        switch Self.read(payload: payload) {
        case .success(let ring):
            CallSystem.shared.reportIncoming(ring)
        case .failure(let failure):
            Self.log.error("a VoIP push could not be read: \(failure.rawValue, privacy: .public)")
            CallSystem.shared.reportUnreadable()
        }
    }

    enum Unreadable: String, Error {
        case noSubscription
        case noAccount
        case badKeys
        case doesNotDecrypt
        case notAPointer
        case unknownVersion
        case notACall
    }

    /// The ring a VoIP push describes: its `aspen` carries the subscription and the ciphertext as
    /// an alert push's does, and the pointer decrypts with the subscription's account's keys.
    static func read(payload: [AnyHashable: Any]) -> Result<IncomingRing, Unreadable> {
        guard let aspen = payload["aspen"] as? [String: Any],
              let subscription = aspen["s"] as? String,
              let ciphertext = aspen["c"] as? String,
              let message = Data(base64url: ciphertext)
        else {
            return .failure(.noSubscription)
        }
        guard let account = PushStateStore.load()?.accounts.first(where: { $0.subscription == subscription }) else {
            return .failure(.noAccount)
        }
        guard let privateKey = Data(base64url: account.keys.privateKey.d),
              let publicKey = Data(base64url: account.keys.publicKey),
              let auth = Data(base64url: account.keys.auth)
        else {
            return .failure(.badKeys)
        }
        guard let plaintext = try? WebPush.decrypt(message: message, privateKey: privateKey, publicKey: publicKey, auth: auth) else {
            return .failure(.doesNotDecrypt)
        }
        guard let pointer = try? JSONSerialization.jsonObject(with: plaintext) as? [String: Any] else {
            return .failure(.notAPointer)
        }
        guard pointer["v"] as? Int == 1 else {
            return .failure(.unknownVersion)
        }
        guard pointer["kind"] as? String == "call",
              let channel = pointer["channel"] as? String,
              let session = pointer["session"] as? String,
              let callerName = pointer["callerName"] as? String,
              let until = pointer["until"] as? String
        else {
            return .failure(.notACall)
        }
        return .success(IncomingRing(
            origin: account.origin,
            channel: channel,
            session: session,
            callerName: callerName,
            until: until
        ))
    }

    // MARK: Declining

    /// Ends the ring at the deployment (`DELETE /channels/{channel}/voice/rings/@me`), as the
    /// account, so the person's other devices stop ringing too; the session token is renewed
    /// once when the deployment refuses it. Fire and forget: the system's end action does not
    /// wait on the network.
    func decline(origin: String, channel: String) {
        guard let account = PushStateStore.load()?.accounts.first(where: {
            CallEngines.trimmed($0.origin) == CallEngines.trimmed(origin)
        }) else {
            return
        }
        let url = "\(CallEngines.trimmed(origin))/api/v1/channels/\(channel)/voice/rings/@me"
        send(delete: url, token: account.sessionToken) { [self] status in
            guard status == 401 else {
                if status != 204 {
                    Self.log.error("declining a ring answered \(status, privacy: .public)")
                }
                return
            }
            TokenRefresh.refresh(account: account, session: session) { [self] token in
                guard let token else {
                    return
                }
                PushStateStore.setSessionToken(subscription: account.subscription, token: token)
                send(delete: url, token: token) { status in
                    if status != 204 {
                        Self.log.error("declining a ring with a renewed session answered \(status, privacy: .public)")
                    }
                }
            }
        }
    }

    private func send(delete url: String, token: String, done: @escaping (Int) -> Void) {
        guard let url = URL(string: url) else {
            done(0)
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "DELETE"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        session.dataTask(with: request) { _, response, _ in
            done((response as? HTTPURLResponse)?.statusCode ?? 0)
        }.resume()
    }
}
