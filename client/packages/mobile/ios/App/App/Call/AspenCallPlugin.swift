import AVFoundation
import Capacitor
import Foundation

/// The web view's way to the call engine (`AspenCall` in the app's `src/api/nativeCall.ts`,
/// which Android's plugin of the same name answers too): a `CallEngine` for each deployment the
/// page names by its origin, made on first use and kept for the process, whose snapshots are
/// published to the page as `state` events and answered afresh by `state`, each the JSON the
/// engine writes of its state (`call_bridge_schema.json`), handed on untouched beside the
/// deployment's origin, and the one set of routes the device's call plays through, published as
/// `routes` events. Joining asks the system
/// for the microphone first: granted, the microphone is opened and given to the engine; refused,
/// the engine is given none and fails the join itself where the channel lets the user speak
/// (`errorKind` `microphone`), as the web's call does when the browser refuses it. The join
/// itself, leaving, and muting go through `CallEngine` and so the system's call (`CallSystem`).
/// Everything runs on the main queue, where the engine's assembly and the routes live.
@objc(AspenCallPlugin)
public class AspenCallPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "AspenCallPlugin"
    public let jsName = "AspenCall"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "join", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "leave", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setMuted", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setDeafened", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setHeld", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setServerMuted", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setUserVolume", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "sessionEnded", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "acknowledgeEnd", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "clearStateRefusal", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "state", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "routes", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setRoute", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "clearRoute", returnType: CAPPluginReturnPromise),
    ]

    /// One deployment's engine and the subscription that publishes its snapshots.
    private final class Held {
        let engine: CallEngine
        var snapshots: CallSnapshots.Subscription?

        init(engine: CallEngine) {
            self.engine = engine
        }
    }

    /// Engines by the deployment's origin as `CallCredentials` keys accounts, without a trailing slash.
    private var engines: [String: Held] = [:]
    private var routes: CallRoutes.Subscription?

    override public func load() {
        DispatchQueue.main.async {
            self.routes = CallEngine.routes.listen { [weak self] snapshot in
                self?.notifyListeners("routes", data: Self.routesData(snapshot))
            }
        }
    }

    // MARK: Engines

    /// The deployment the call names, or a rejected call.
    private func origin(of call: CAPPluginCall) -> String? {
        guard let origin = call.getString("origin"), let url = URL(string: origin), url.host != nil else {
            call.reject("origin must be the deployment's address")
            return nil
        }
        var trimmed = Substring(origin)
        while trimmed.hasSuffix("/") {
            trimmed.removeLast()
        }
        return String(trimmed)
    }

    /// The deployment's engine, made on first use. Call on the main queue. A debuggable build
    /// lets the engine reach a development deployment's voice servers over plain HTTP at this
    /// machine's own addresses, as the web client's development build does; a release build
    /// never does.
    private func engine(at origin: String) -> CallEngine {
        if let held = engines[origin] {
            return held.engine
        }
        let made = CallEngine(origin: origin, loopbackPlainText: Self.debuggable)
        let held = Held(engine: made)
        held.snapshots = made.snapshots.listenJSON { [weak self] json in
            self?.notifyListeners("state", data: Self.published(json, origin: origin))
        }
        engines[origin] = held
        return made
    }

    private static var debuggable: Bool {
        #if DEBUG
            return true
        #else
            return false
        #endif
    }

    /// Runs `body` with the call's engine on the main queue, resolving the call after it.
    private func withEngine(_ call: CAPPluginCall, _ body: @escaping (CallEngine) -> Void) {
        guard let origin = origin(of: call) else {
            return
        }
        DispatchQueue.main.async {
            body(self.engine(at: origin))
            call.resolve()
        }
    }

    // MARK: Commands

    /// Joins `channel`'s call at `origin`, shown by the system as `name`, once the system has
    /// answered for the microphone.
    @objc func join(_ call: CAPPluginCall) {
        guard let origin = origin(of: call) else {
            return
        }
        guard let channel = call.getString("channel"), !channel.isEmpty else {
            call.reject("channel is required")
            return
        }
        let name = call.getString("name")
        Self.requestMicrophone { granted in
            DispatchQueue.main.async {
                let engine = self.engine(at: origin)
                if granted {
                    engine.engine.setMicrophone(track: engine.audio.openMicrophone())
                } else {
                    engine.audio.closeMicrophone()
                    engine.engine.setMicrophone(track: nil)
                }
                engine.join(channel: channel, name: name)
                call.resolve()
            }
        }
    }

    /// Asks the system for the microphone, which it does once per install; later asks answer at once.
    private static func requestMicrophone(_ answer: @escaping (Bool) -> Void) {
        if #available(iOS 17.0, *) {
            AVAudioApplication.requestRecordPermission(completionHandler: answer)
        } else {
            AVAudioSession.sharedInstance().requestRecordPermission(answer)
        }
    }

    @objc func leave(_ call: CAPPluginCall) {
        withEngine(call) { engine in
            engine.leave()
        }
    }

    @objc func setMuted(_ call: CAPPluginCall) {
        guard let muted = call.getBool("muted") else {
            call.reject("muted is required")
            return
        }
        withEngine(call) { engine in
            engine.setMuted(muted)
        }
    }

    @objc func setDeafened(_ call: CAPPluginCall) {
        guard let deafened = call.getBool("deafened") else {
            call.reject("deafened is required")
            return
        }
        withEngine(call) { engine in
            engine.engine.setDeafened(deafened: deafened)
        }
    }

    @objc func setHeld(_ call: CAPPluginCall) {
        guard let held = call.getBool("held") else {
            call.reject("held is required")
            return
        }
        withEngine(call) { engine in
            engine.engine.setHeld(held: held)
        }
    }

    @objc func setServerMuted(_ call: CAPPluginCall) {
        guard let serverMuted = call.getBool("serverMuted") else {
            call.reject("serverMuted is required")
            return
        }
        withEngine(call) { engine in
            engine.engine.setServerMuted(serverMuted: serverMuted)
        }
    }

    @objc func setUserVolume(_ call: CAPPluginCall) {
        guard let user = call.getString("user"), !user.isEmpty else {
            call.reject("user is required")
            return
        }
        guard let source = Self.audioSource(call.getString("source")) else {
            call.reject("source must be microphone or screenAudio")
            return
        }
        guard let gain = call.getDouble("gain"), gain.isFinite, gain >= 0 else {
            call.reject("gain must be a number from 0")
            return
        }
        withEngine(call) { engine in
            engine.engine.setUserVolume(user: user, source: source, gain: gain)
        }
    }

    /// The two sounds a participant sends whose volumes are set apart, as the page names them.
    private static func audioSource(_ name: String?) -> MediaSource? {
        switch name {
        case "microphone": return .microphone
        case "screenAudio": return .screenAudio
        default: return nil
        }
    }

    @objc func sessionEnded(_ call: CAPPluginCall) {
        guard let session = call.getString("session"), !session.isEmpty,
              let channel = call.getString("channel"), !channel.isEmpty else {
            call.reject("session and channel are required")
            return
        }
        guard let reason = Self.sessionEndReason(call.getString("reason")) else {
            call.reject("reason must be a voiceSessionEnded reason")
            return
        }
        withEngine(call) { engine in
            engine.engine.sessionEnded(session: session, channel: channel, reason: reason)
        }
    }

    /// `VoiceSessionEndReason` as the API server's event names it.
    private static func sessionEndReason(_ name: String?) -> SessionEndReason? {
        switch name {
        case "empty": return .empty
        case "serverLost": return .serverLost
        case "idle": return .idle
        case "serverRemoved": return .serverRemoved
        default: return nil
        }
    }

    @objc func acknowledgeEnd(_ call: CAPPluginCall) {
        withEngine(call) { engine in
            engine.engine.acknowledgeEnd()
        }
    }

    @objc func clearStateRefusal(_ call: CAPPluginCall) {
        withEngine(call) { engine in
            engine.engine.clearStateRefusal()
        }
    }

    // MARK: State

    /// The engine's latest snapshot, for a page that slept through the events.
    @objc func state(_ call: CAPPluginCall) {
        guard let origin = origin(of: call) else {
            return
        }
        DispatchQueue.main.async {
            let engine = self.engine(at: origin)
            call.resolve(Self.published(engine.snapshots.latestJSON ?? engine.engine.stateJson(), origin: origin))
        }
    }

    /// A snapshot as the page reads it (`PublishedState` in `nativeCall.ts`): the engine's JSON
    /// as written, and whose it is.
    static func published(_ json: String, origin: String) -> JSObject {
        ["origin": origin, "state": json]
    }

    // MARK: Routes

    /// What the call can play through now, as `CallRoutes` has it.
    @objc func routes(_ call: CAPPluginCall) {
        DispatchQueue.main.async {
            call.resolve(Self.routesData(CallEngine.routes.current))
        }
    }

    /// Plays the call through the first route of `kind`, holding the choice for the call.
    @objc func setRoute(_ call: CAPPluginCall) {
        guard let kind = call.getString("kind").flatMap(CallRouteKind.init(rawValue:)) else {
            call.reject("kind must be a route kind")
            return
        }
        DispatchQueue.main.async {
            guard CallEngine.routes.current.routes.contains(where: { $0.kind == kind }) else {
                call.reject("the device has no route of the kind \(kind.rawValue)")
                return
            }
            CallEngine.routes.select(kind: kind)
            call.resolve()
        }
    }

    /// Lets the choice go: the system's own route follows.
    @objc func clearRoute(_ call: CAPPluginCall) {
        DispatchQueue.main.async {
            CallEngine.routes.reset()
            call.resolve()
        }
    }

    /// `CallRoutes.Snapshot` as the page reads it (`PhoneRoutes` in `nativeCall.ts`); each kind
    /// is `CallRouteKind`'s name, which the page's `RouteKind` shares.
    static func routesData(_ snapshot: CallRoutes.Snapshot) -> JSObject {
        [
            "routes": snapshot.routes.map { route -> JSObject in
                ["kind": route.kind.rawValue, "name": route.name ?? NSNull(), "inUse": route.inUse]
            },
            "hasReceiver": snapshot.hasReceiver,
            "chosen": snapshot.chosen?.rawValue ?? NSNull(),
        ]
    }
}
