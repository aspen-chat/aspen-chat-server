import AVFoundation
import Foundation
import UIKit

/// What a call can play through, by kind. The interface shows one list on every phone and
/// labels each entry by its kind; a port's name is carried for display beside it and decides
/// nothing.
enum CallRouteKind: String, CaseIterable {
    case receiver
    case loudspeaker
    case bluetooth
    case wiredHeadset
    case car
    case other
}

/// One thing the call can play through, now.
struct CallRoute: Equatable {
    let kind: CallRouteKind
    /// The system's name for the port (a headset's own name), for display alone; `nil` for the
    /// receiver and loudspeaker when no port on the route names them.
    let name: String?
    /// The input port's `uid`, for the routes chosen with `setPreferredInput` (a Bluetooth or
    /// wired headset, a car); `nil` for the receiver and loudspeaker, chosen by overriding the
    /// output, and for an output the system offers no input of.
    let uid: String?
    let inUse: Bool
}

/// The call's audio routes over `AVAudioSession`: what the session can play through, read from
/// the current route's outputs and the available inputs, which of them is in use, and the choice
/// the user made, held for the call. The receiver and loudspeaker are chosen by overriding the
/// output port, which lasts only until the next route change, so the choice is applied again on
/// every `routeChangeNotification` while it stands; a headset or car is chosen by preferring its
/// input, which the system keeps itself. A new device arriving (`newDeviceAvailable`: headphones
/// plugged in, a headset connected) takes the call, as the Phone app's does, and ends the choice;
/// a device going away leaves it, so a loudspeaker call stays on the loudspeaker when the headset
/// is unplugged. The proximity sensor is on while the route in use is the receiver, so the screen
/// turns off at the ear, and off otherwise. Everything runs on the main queue, where listeners
/// are called with each change and `UIDevice` is used.
final class CallRoutes {
    struct Snapshot: Equatable {
        let routes: [CallRoute]
        /// Whether the device has a receiver to hold to the ear: an iPhone has one, an iPad does
        /// not, whatever the current route says.
        let hasReceiver: Bool
        /// The kind the user chose for this call, until `reset` or a new device takes it.
        let chosen: CallRouteKind?
    }

    typealias Listener = (Snapshot) -> Void

    /// Stops the listener it was given for.
    final class Subscription {
        private let stop: () -> Void

        fileprivate init(stop: @escaping () -> Void) {
            self.stop = stop
        }

        func cancel() {
            stop()
        }

        deinit {
            stop()
        }
    }

    private enum Choice: Equatable {
        case receiver
        case loudspeaker
        /// The input port with this `uid`.
        case input(String)

        var kind: CallRouteKind? {
            switch self {
            case .receiver: return .receiver
            case .loudspeaker: return .loudspeaker
            case .input: return nil
            }
        }
    }

    private let session: AVAudioSession
    private let hasReceiver: Bool
    /// Turns the proximity sensor on or off; `UIDevice`'s, or a test's recorder.
    private let proximity: (Bool) -> Void
    private var choice: Choice?
    private var listeners: [UUID: Listener] = [:]
    private var observer: NSObjectProtocol?
    private(set) var current: Snapshot
    /// Whether the sensor was last asked on.
    private(set) var proximityOn = false

    init(
        session: AVAudioSession = .sharedInstance(),
        hasReceiver: Bool = UIDevice.current.userInterfaceIdiom == .phone,
        proximity: @escaping (Bool) -> Void = { UIDevice.current.isProximityMonitoringEnabled = $0 }
    ) {
        self.session = session
        self.hasReceiver = hasReceiver
        self.proximity = proximity
        self.current = Snapshot(routes: [], hasReceiver: hasReceiver, chosen: nil)
        current = read()
        observer = NotificationCenter.default.addObserver(
            forName: AVAudioSession.routeChangeNotification, object: session, queue: .main
        ) { [weak self] notification in
            self?.routeChanged(notification)
        }
    }

    deinit {
        if let observer {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    // MARK: Reading

    /// The kind a port is, by its type alone.
    static func kind(of port: AVAudioSession.Port) -> CallRouteKind {
        switch port {
        case .builtInReceiver: return .receiver
        case .builtInSpeaker: return .loudspeaker
        case .bluetoothHFP, .bluetoothA2DP, .bluetoothLE: return .bluetooth
        case .headphones, .headsetMic, .lineOut: return .wiredHeadset
        case .carAudio: return .car
        default: return .other
        }
    }

    /// Whether the sensor is wanted for `routes`: only while the receiver plays the call.
    static func proximityWanted(_ routes: [CallRoute]) -> Bool {
        routes.contains { $0.inUse && $0.kind == .receiver }
    }

    /// The routes as the session has them now: the receiver (on a phone) and the loudspeaker,
    /// then every input that is not the built-in microphone, then, when none of those is what
    /// plays, the output that does (Bluetooth headphones with no microphone, AirPlay). An input
    /// is in use when the playing output carries its `uid`, which a headset's two ports share,
    /// or, where they do not, when it is the one input of the playing output's kind.
    private func read() -> Snapshot {
        let outputs = session.currentRoute.outputs
        let playing = outputs.first
        let playingKind = playing.map { Self.kind(of: $0.portType) }
        var routes: [CallRoute] = []
        func builtIn(_ kind: CallRouteKind, _ port: AVAudioSession.Port) {
            let onRoute = outputs.first { $0.portType == port }
            routes.append(CallRoute(kind: kind, name: onRoute?.portName, uid: nil, inUse: playingKind == kind))
        }
        if hasReceiver {
            builtIn(.receiver, .builtInReceiver)
        }
        builtIn(.loudspeaker, .builtInSpeaker)
        let inputs = (session.availableInputs ?? []).filter { $0.portType != .builtInMic }
        for input in inputs {
            let kind = Self.kind(of: input.portType)
            let onlyOfKind = inputs.filter { Self.kind(of: $0.portType) == kind }.count == 1
            let inUse = playing?.uid == input.uid || (playingKind == kind && onlyOfKind)
            routes.append(CallRoute(kind: kind, name: input.portName, uid: input.uid, inUse: inUse))
        }
        if let playing, !routes.contains(where: \.inUse) {
            routes.append(CallRoute(kind: Self.kind(of: playing.portType), name: playing.portName, uid: nil, inUse: true))
        }
        return Snapshot(routes: routes, hasReceiver: hasReceiver, chosen: choice?.kind ?? chosenInput(routes))
    }

    private func chosenInput(_ routes: [CallRoute]) -> CallRouteKind? {
        guard case .input(let uid) = choice else {
            return nil
        }
        return routes.first { $0.uid == uid }?.kind
    }

    // MARK: Choosing

    /// Plays the call through `route`: the receiver or loudspeaker by overriding the output, a
    /// headset or car by preferring its input. The choice holds for the call.
    func select(_ route: CallRoute) {
        switch route.kind {
        case .receiver:
            choice = .receiver
        case .loudspeaker:
            choice = .loudspeaker
        case .bluetooth, .wiredHeadset, .car, .other:
            guard let uid = route.uid else {
                return
            }
            choice = .input(uid)
        }
        apply()
        changed()
    }

    /// Plays the call through the first route of `kind`, if there is one.
    func select(kind: CallRouteKind) {
        if let route = current.routes.first(where: { $0.kind == kind }) {
            select(route)
        }
    }

    /// Lets the choice go, as a call's end does; the system's own route follows.
    func reset() {
        choice = nil
        if session.category == .playAndRecord {
            try? session.overrideOutputAudioPort(.none)
        }
        changed()
    }

    /// Applies the standing choice to the session: on choosing, on every route change while it
    /// stands, and when the system activates the session for the call, since an override set
    /// before that is not kept. Outside the call's category there is nothing to apply to.
    func apply() {
        guard let choice, session.category == .playAndRecord else {
            return
        }
        let inputs = session.availableInputs ?? []
        switch choice {
        case .receiver, .loudspeaker:
            // Back from a headset the built-in microphone must be preferred too, or the
            // override alone leaves the headset's input and so the headset.
            if let builtIn = inputs.first(where: { $0.portType == .builtInMic }), session.preferredInput?.uid != builtIn.uid {
                try? session.setPreferredInput(builtIn)
            }
            let wanted: AVAudioSession.PortOverride = choice == .loudspeaker ? .speaker : .none
            let playing = session.currentRoute.outputs.first.map { Self.kind(of: $0.portType) }
            if playing != choice.kind {
                try? session.overrideOutputAudioPort(wanted)
            }
        case .input(let uid):
            guard let input = inputs.first(where: { $0.uid == uid }) else {
                return
            }
            try? session.overrideOutputAudioPort(.none)
            if session.preferredInput?.uid != uid {
                try? session.setPreferredInput(input)
            }
        }
    }

    private func routeChanged(_ notification: Notification) {
        let raw = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt
        let reason = raw.flatMap(AVAudioSession.RouteChangeReason.init(rawValue:))
        if reason == .newDeviceAvailable {
            choice = nil
        } else {
            apply()
        }
        changed()
    }

    /// Reads the routes again, follows the sensor, and tells the listeners when anything
    /// differs from what they last heard.
    private func changed() {
        let snapshot = read()
        let wanted = Self.proximityWanted(snapshot.routes)
        if wanted != proximityOn {
            proximityOn = wanted
            proximity(wanted)
        }
        guard snapshot != current else {
            return
        }
        current = snapshot
        for listener in listeners.values {
            listener(snapshot)
        }
    }

    /// Calls `listener` on the main queue with the routes now and with every change after,
    /// until the subscription is cancelled or released.
    func listen(_ listener: @escaping Listener) -> Subscription {
        let id = UUID()
        listeners[id] = listener
        let now = current
        DispatchQueue.main.async {
            listener(now)
        }
        return Subscription { [weak self] in
            self?.listeners[id] = nil
        }
    }
}
