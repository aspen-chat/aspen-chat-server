import AVFoundation
import Capacitor
import Foundation

/// The web view's way to the call engine (`AspenCall` in the app's `src/api/nativeCall.ts`,
/// which Android's plugin of the same name answers too): the engine of each deployment the page
/// names by its origin, from `CallEngines`, the process's one engine per deployment, which a DM's
/// call ringing the phone drives too, so a ring answered from the lock screen is the call the page
/// then shows; its snapshots are published to the page as `state` events and answered afresh by
/// `state`, each the JSON the engine writes of its state (`call_bridge_schema.json`), handed on
/// untouched beside the deployment's origin, the one set of routes the device's call plays
/// through, published as
/// `routes` events, and the call's video: the cameras (`CallCamera`), the holes the page reports
/// for the pictures (`setTiles`, laid out by `CallVideo`), each picture's shape back as a `shape`
/// event, and the system's picture in picture as `pip` events. Joining asks the system for the
/// microphone first: granted, the microphone is opened and given to the engine; refused, the
/// engine is given none and fails the join itself where the channel lets the user speak
/// (`errorKind` `microphone`), as the web's call does when the browser refuses it. `startCamera`
/// asks for the camera the same way and rejects with the code `denied`, `none`, or `failed`
/// (`CallCameraFailure`) when none opens. The join itself, leaving, and muting go through
/// `CallEngine` and so the system's call (`CallSystem`). Everything runs on the main queue, where
/// the engine's assembly, the routes, the camera, and the video live.
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
        CAPPluginMethod(name: "cameras", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "startCamera", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "switchCamera", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "stopCamera", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "clearCameraError", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setWatched", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setTiles", returnType: CAPPluginReturnPromise),
    ]

    /// The subscriptions that publish one deployment's engine's snapshots and follow them.
    private final class Followed {
        var snapshots: CallSnapshots.Subscription?
        var camera: CallSnapshots.Subscription?
    }

    /// The engines the page has named, by the deployment's origin without a trailing slash, each
    /// followed from its first naming.
    private var followed: [String: Followed] = [:]
    private var routes: CallRoutes.Subscription?
    private var shapes: CallVideo.Subscription?
    private var pictureInPicture: CallVideo.Subscription?

    override public func load() {
        DispatchQueue.main.async {
            self.routes = CallEngine.routes.listen { [weak self] snapshot in
                self?.notifyListeners("routes", data: Self.routesData(snapshot))
            }
            self.shapes = CallEngine.video.listenShapes { [weak self] shape in
                self?.notifyListeners("shape", data: Self.shapeData(shape))
            }
            self.pictureInPicture = CallEngine.video.listenPictureInPicture { [weak self] active in
                self?.notifyListeners("pip", data: ["active": active])
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

    /// The deployment's engine (`CallEngines.shared.forOrigin`), followed for the page from the
    /// first time the page names its origin: its snapshots go to the page as `state` events, and
    /// the camera follows its call. Call on the main queue. Internal so the bridge's proof can
    /// drive the engine the page's commands reach.
    func engine(at origin: String) -> CallEngine {
        let engine = CallEngines.shared.forOrigin(origin)
        if followed[origin] == nil {
            let following = Followed()
            following.snapshots = engine.snapshots.listenJSON { [weak self] json in
                self?.notifyListeners("state", data: Self.published(json, origin: origin))
            }
            following.camera = engine.snapshots.listen { state in
                Self.cameraFollows(state, origin: origin)
            }
            followed[origin] = following
        }
        return engine
    }

    /// The camera is on only while its engine sends it: the call ending, or the voice server
    /// refusing the camera, stops the capture, so the light goes off with the picture.
    private static func cameraFollows(_ state: CallState, origin: String) {
        let camera = CallEngine.camera
        guard camera.isOn, camera.origin == origin else {
            return
        }
        let ended = state.status == .idle || state.status == .failed
        let refused = state.sourceRefused?.source == .camera
        if ended || refused {
            camera.stop()
        }
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

    // MARK: Video

    /// The phone's cameras, each with its facing as the system says; the user is asked for the
    /// camera as one opens, not here.
    @objc func cameras(_ call: CAPPluginCall) {
        DispatchQueue.main.async {
            call.resolve(["cameras": CallEngine.camera.devices.map(Self.cameraData)])
        }
    }

    /// Opens the camera `device` names (the phone's default for `null`) and gives the engine its
    /// track to send, once the system has answered for the camera; resolves with the camera
    /// opened and its facing (`OpenedCamera` in `nativeCall.ts`), or rejects with the code
    /// `denied`, `none`, or `failed`.
    @objc func startCamera(_ call: CAPPluginCall) {
        guard let origin = origin(of: call) else {
            return
        }
        let device = call.getString("device")
        CallCamera.requestAccess { granted in
            DispatchQueue.main.async {
                guard granted else {
                    call.reject("the camera is not allowed", CallCameraFailure.denied.rawValue)
                    return
                }
                let engine = self.engine(at: origin)
                CallEngine.camera.start(device: device, origin: origin) { result in
                    switch result {
                    case .success(let opened):
                        engine.engine.startSource(source: .camera, track: opened.track)
                        call.resolve(Self.openedData(opened.device))
                    case .failure(let failure):
                        call.reject(Self.cameraFailureText(failure), failure.rawValue)
                    }
                }
            }
        }
    }

    /// Moves a camera that is on to the one `device` names, the track and the engine's sender
    /// unchanged; resolves with the camera opened and its facing.
    @objc func switchCamera(_ call: CAPPluginCall) {
        guard let origin = origin(of: call) else {
            return
        }
        let device = call.getString("device")
        DispatchQueue.main.async {
            guard CallEngine.camera.isOn else {
                call.reject("the camera is off", CallCameraFailure.failed.rawValue)
                return
            }
            CallEngine.camera.start(device: device, origin: origin) { result in
                switch result {
                case .success(let opened):
                    call.resolve(Self.openedData(opened.device))
                case .failure(let failure):
                    call.reject(Self.cameraFailureText(failure), failure.rawValue)
                }
            }
        }
    }

    /// Stops sending the camera and turns it off.
    @objc func stopCamera(_ call: CAPPluginCall) {
        withEngine(call) { engine in
            engine.engine.stopSource(source: .camera)
            CallEngine.camera.stop()
        }
    }

    /// Dismisses the voice server's refusal of a source.
    @objc func clearCameraError(_ call: CAPPluginCall) {
        withEngine(call) { engine in
            engine.engine.clearSourceRefusal()
        }
    }

    /// Asks the engine to watch `video`, each a user's screen or camera.
    @objc func setWatched(_ call: CAPPluginCall) {
        guard let listed = call.getArray("video") else {
            call.reject("video is required")
            return
        }
        var video: [WatchedVideo] = []
        for entry in listed {
            guard let entry = entry as? [String: Any], let user = entry["user"] as? String, !user.isEmpty,
                  let source = Self.videoSource(entry["source"] as? String) else {
                call.reject("each video names a user and a source, screen or camera")
                return
            }
            video.append(WatchedVideo(user: user, source: source))
        }
        withEngine(call) { engine in
            engine.engine.setWatched(video: video)
        }
    }

    /// The two pictures a participant sends, as the page names them.
    private static func videoSource(_ name: String?) -> MediaSource? {
        switch name {
        case "screen": return .screen
        case "camera": return .camera
        default: return nil
        }
    }

    /// Where the page's holes are (`TileReport` in `nativeCall.ts`): the pictures are laid out
    /// there, in the web view's points, which the page's CSS pixels are.
    @objc func setTiles(_ call: CAPPluginCall) {
        guard let listed = call.getArray("tiles") else {
            call.reject("tiles is required")
            return
        }
        var tiles: [CallVideoTile] = []
        for entry in listed {
            guard let entry = entry as? [String: Any], let tile = Self.tile(entry) else {
                call.reject("each tile names its tile, surface, rect, and how it is drawn")
                return
            }
            tiles.append(tile)
        }
        DispatchQueue.main.async {
            CallEngine.video.apply(tiles)
            call.resolve()
        }
    }

    static func tile(_ entry: [String: Any]) -> CallVideoTile? {
        guard let tile = entry["tile"] as? String, !tile.isEmpty,
              let surface = entry["surface"] as? String, !surface.isEmpty,
              let rect = entry["rect"] as? [String: Any],
              let x = (rect["x"] as? NSNumber)?.doubleValue, let y = (rect["y"] as? NSNumber)?.doubleValue,
              let width = (rect["width"] as? NSNumber)?.doubleValue, let height = (rect["height"] as? NSNumber)?.doubleValue,
              let visible = entry["visible"] as? Bool, let mirrored = entry["mirrored"] as? Bool,
              let cover = entry["cover"] as? Bool, let fullScreen = entry["fullScreen"] as? Bool,
              let pictureInPicture = entry["pictureInPicture"] as? Bool,
              x.isFinite, y.isFinite, width.isFinite, height.isFinite, width >= 0, height >= 0
        else {
            return nil
        }
        return CallVideoTile(
            tile: tile, surface: surface, rect: CGRect(x: x, y: y, width: width, height: height), visible: visible,
            mirrored: mirrored, cover: cover, fullScreen: fullScreen, pictureInPicture: pictureInPicture)
    }

    /// `CallCameraDevice` as the page reads it (`PhoneCamera` in `nativeCall.ts`).
    static func cameraData(_ device: CallCameraDevice) -> JSObject {
        ["id": device.id, "label": device.label ?? NSNull(), "facing": device.facing?.rawValue ?? NSNull()]
    }

    /// The camera opened as the page reads it (`OpenedCamera` in `nativeCall.ts`).
    static func openedData(_ device: CallCameraDevice) -> JSObject {
        ["device": device.id, "facing": device.facing?.rawValue ?? NSNull()]
    }

    static func cameraFailureText(_ failure: CallCameraFailure) -> String {
        switch failure {
        case .denied: return "the camera is not allowed"
        case .none: return "the phone has no such camera"
        case .failed: return "the camera would not start"
        }
    }

    /// `CallVideoShape` as the page reads it (`PictureShape` in `nativeCall.ts`).
    static func shapeData(_ shape: CallVideoShape) -> JSObject {
        ["surface": shape.surface, "width": shape.width, "height": shape.height, "rotation": shape.rotation]
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
