import AVFoundation
import CallKit
import Foundation
import WebRTC

/// The call as the system knows it, through CallKit: one provider for the process, and the one
/// call it holds at a time, whichever deployment's engine runs it. Joining a voice channel or a
/// DM call is an outgoing call started from the app (`CXStartCallAction`), reported connected
/// when the engine is, and ended with `CXEndCallAction` when the user leaves or reported ended,
/// with the reason, when the engine's state says the call is over; the system's own End, mute,
/// and hold reach the engine through the provider's actions, and an interruption by another
/// session holds the engine's call too. A DM's call ringing this phone is an incoming call,
/// reported from the VoIP push (`CallRings`, `reportIncoming`) and watched through the engine's
/// ring watch until it is answered here (`CXAnswerCallAction`, which joins through the
/// deployment's engine), declined here (`CXEndCallAction`, which ends the ring at the
/// deployment), or ended anywhere else (`ringEnded`). CallKit owns the audio session's
/// activation: the session is configured in the start action, activated by CallKit, and WebRTC's
/// audio runs between `didActivate` and `didDeactivate` (`CallAudio`), which CallKit also calls
/// around an interruption by another call. So the app never activates the session itself, and on
/// an interruption only stops WebRTC's audio and waits for the system to give the session back.
/// Everything runs on the main queue; `status` tells the interface what the system has done.
final class CallSystem: NSObject, CXProviderDelegate, @unchecked Sendable {
    static let shared = CallSystem()

    /// What the system has done with the call, for the interface; changes reach listeners on
    /// the main queue.
    struct Status: Equatable {
        /// The call's id at the system, while it holds one.
        var call: UUID?
        /// Whether the system has the audio session active for the call: between `didActivate`
        /// and `didDeactivate`. False while another call interrupts ours.
        var audioActive: Bool
        /// Whether the system holds the call (`CXSetHeldCallAction`). The engine's call is on
        /// hold while this or `interrupted` stands.
        var held: Bool
        /// Whether another audio session (a phone call, Siri, an alarm) has interrupted ours and
        /// the system has not yet given it back.
        var interrupted: Bool
        /// What the system last refused to do, in its words, for the interface to show; a start
        /// it refused never joined.
        var refusal: String?
        /// The incoming call ringing, by its id at the system, while one does.
        var incoming: UUID?

        static let idle = Status(call: nil, audioActive: false, held: false, interrupted: false, refusal: nil, incoming: nil)
    }

    typealias Listener = (Status) -> Void

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

    /// The call the system holds.
    private final class Current {
        let uuid: UUID
        let engine: CallEngine
        var channel: String
        /// What the system shows for the call.
        var name: String?
        /// Whether the system knows the call as outgoing (started from the app) rather than as
        /// an incoming call answered, which is connected by its answer action and never
        /// reported connecting or connected.
        let outgoing: Bool
        var subscription: CallSnapshots.Subscription?
        /// Whether a snapshot has shown the engine in the call, so that the idle state before
        /// the join is not taken for its end.
        var joined = false
        var connected = false
        /// Whether the end was asked of the system (`CXEndCallAction`), which then needs no
        /// report of the engine's idle state.
        var ending = false
        /// The hold last asked of the engine, so it is asked only when it changes.
        var heldAsked = false

        init(uuid: UUID, engine: CallEngine, channel: String, name: String?, outgoing: Bool = true) {
            self.uuid = uuid
            self.engine = engine
            self.channel = channel
            self.name = name
            self.outgoing = outgoing
        }
    }

    /// A DM's call ringing this phone, reported to the system and not yet answered, declined, or
    /// ended elsewhere.
    private final class Incoming {
        let uuid: UUID
        let ring: CallRings.IncomingRing
        /// The engine's watch on the ring, which hears of its end anywhere else.
        var watch: RingWatch?

        init(uuid: UUID, ring: CallRings.IncomingRing) {
            self.uuid = uuid
            self.ring = ring
        }
    }

    /// Hears a ring's end from the engine's watch, on the watch's thread, and hands it to the
    /// main queue; holds no reference to the watch, which is released only there.
    private final class RingEnds: RingObserver, @unchecked Sendable {
        let uuid: UUID

        init(uuid: UUID) {
            self.uuid = uuid
        }

        func ended(end: RingEnd) {
            let uuid = self.uuid
            DispatchQueue.main.async {
                CallSystem.shared.ringEnded(uuid, end: end)
            }
        }
    }

    private let provider: CXProvider
    private let controller = CXCallController()
    private let lock = NSLock()
    private var current: Current?
    private var incoming: Incoming?
    /// The audio of a call that has ended, until the system's `didDeactivate` for it.
    private var deactivating: CallAudio?
    private var listeners: [UUID: Listener] = [:]
    private var status = Status.idle
    private var observers: [NSObjectProtocol] = []

    private override init() {
        let configuration = CXProviderConfiguration()
        configuration.supportsVideo = true
        configuration.includesCallsInRecents = false
        configuration.maximumCallGroups = 1
        configuration.maximumCallsPerCallGroup = 1
        configuration.supportedHandleTypes = [.generic]
        provider = CXProvider(configuration: configuration)
        super.init()
        provider.setDelegate(self, queue: nil)
        observeInterruptions()
    }

    // MARK: Status

    var latest: Status {
        lock.lock()
        defer { lock.unlock() }
        return status
    }

    private func update(_ change: (inout Status) -> Void) {
        lock.lock()
        var changed = status
        change(&changed)
        guard changed != status else {
            lock.unlock()
            return
        }
        status = changed
        let told = Array(listeners.values)
        lock.unlock()
        for listener in told {
            listener(changed)
        }
    }

    /// Keeps the engine's call on hold exactly while the system holds it or another session
    /// has interrupted it: the voice server then pauses everything the user sends and sends
    /// them nothing, keeping their own mute and deafen for the hold's end, and the others see
    /// them on hold rather than muted. The peer connection stays up and the microphone track
    /// stays open through it; `didActivate` is what resumes the audio.
    private func applyHold(_ call: Current) {
        let held = latest.held || latest.interrupted
        guard call.heldAsked != held else {
            return
        }
        call.heldAsked = held
        call.engine.engine.setHeld(held: held)
    }

    /// Calls `listener` on the main queue with the status now and with every change after,
    /// until the subscription is cancelled or released.
    func listen(_ listener: @escaping Listener) -> Subscription {
        let id = UUID()
        lock.lock()
        listeners[id] = listener
        let now = status
        lock.unlock()
        DispatchQueue.main.async {
            listener(now)
        }
        return Subscription { [weak self] in
            guard let self else {
                return
            }
            self.lock.lock()
            self.listeners[id] = nil
            self.lock.unlock()
        }
    }

    // MARK: Commands from the app

    /// Joins `channel`'s call on `engine` as a call the system knows: a new outgoing call, or,
    /// while the system holds one of this engine's, a move the engine makes itself, with the
    /// call's handle updated. A call of another deployment's engine is left and reported ended
    /// first, since the system holds one call. `name` is what the system shows for the call,
    /// which has no name of its own there.
    func start(engine: CallEngine, channel: String, name: String?) {
        if let current, current.engine === engine {
            current.channel = channel
            current.name = name
            engine.engine.join(channel: channel)
            provider.reportCall(with: current.uuid, updated: Self.update(channel: channel, name: name))
            return
        }
        if let current {
            current.engine.engine.leave()
            ended(current, reason: .remoteEnded)
        }
        let made = Current(uuid: UUID(), engine: engine, channel: channel, name: name)
        let start = CXStartCallAction(call: made.uuid, handle: CXHandle(type: .generic, value: channel))
        start.isVideo = false
        current = made
        made.subscription = engine.snapshots.listen { [weak self, weak made] state in
            guard let self, let made else {
                return
            }
            self.stateChanged(state, of: made)
        }
        update { $0.refusal = nil }
        controller.request(CXTransaction(action: start)) { [weak self] error in
            guard let self, let error else {
                return
            }
            // Refused (another app's call holds the one group, or CallKit itself is not
            // available), the call is not joined: without the system there is no one to
            // activate the audio session.
            NSLog("Aspen's call was refused by the system: %@", String(describing: error))
            if self.current === made {
                made.subscription = nil
                self.current = nil
            }
            self.update {
                $0.call = nil
                $0.refusal = error.localizedDescription
            }
        }
    }

    /// Leaves the call: asks the system to end it, which reaches the engine through the
    /// provider's end action, or leaves directly when the system holds no call of this engine's.
    func end(engine: CallEngine) {
        guard let current, current.engine === engine else {
            engine.engine.leave()
            return
        }
        current.ending = true
        controller.request(CXTransaction(action: CXEndCallAction(call: current.uuid))) { [weak self] error in
            guard let self, let error else {
                return
            }
            NSLog("The system would not end Aspen's call: %@", String(describing: error))
            // Left all the same, and reported ended, so the system drops it.
            engine.engine.leave()
            self.ended(current, reason: .remoteEnded)
        }
    }

    /// Mutes or unmutes through the system, so its own controls show the same, when it holds a
    /// call of this engine's; directly otherwise.
    func setMuted(_ muted: Bool, engine: CallEngine) {
        guard let current, current.engine === engine else {
            engine.engine.setMuted(muted: muted)
            return
        }
        controller.request(CXTransaction(action: CXSetMutedCallAction(call: current.uuid, muted: muted))) { error in
            if error != nil {
                engine.engine.setMuted(muted: muted)
            }
        }
    }

    // MARK: Incoming calls

    /// Reports a DM's call ringing this phone, as its VoIP push describes it, synchronously, so
    /// that the push's handler has reported it before it returns, and starts the engine's watch
    /// on the ring. One incoming call rings at a time: a second is reported and ended at once,
    /// as is one the system refuses (another app's call, Do Not Disturb). Call on the main queue.
    func reportIncoming(_ ring: CallRings.IncomingRing) {
        let made = Incoming(uuid: UUID(), ring: ring)
        let update = Self.update(channel: ring.channel, name: ring.callerName)
        guard incoming == nil else {
            provider.reportNewIncomingCall(with: made.uuid, update: update) { [weak self] _ in
                self?.provider.reportCall(with: made.uuid, endedAt: nil, reason: .unanswered)
            }
            return
        }
        incoming = made
        provider.reportNewIncomingCall(with: made.uuid, update: update) { [weak self] error in
            guard let self, let error else {
                return
            }
            NSLog("The system would not show Aspen's incoming call: %@", String(describing: error))
            guard self.incoming === made else {
                return
            }
            made.watch?.stop()
            made.watch = nil
            self.incoming = nil
            self.update { $0.incoming = nil }
        }
        // The watch runs on a thread of its own and reports on it; `RingEnds` brings the end
        // here.
        made.watch = RingWatch.start(
            platform: RingPlatform(http: CallHttp(), sockets: CallSockets()),
            options: RingOptions(
                apiUrl: CallEngines.trimmed(ring.origin),
                credentials: CallCredentials(origin: ring.origin),
                loopbackPlainText: Self.loopbackPlainText
            ),
            ring: Ring(channel: ring.channel, session: ring.session, until: ring.until),
            observer: RingEnds(uuid: made.uuid)
        )
        update { $0.incoming = made.uuid }
    }

    /// Reports a call for a VoIP push the app could not read and ends it at once, as Apple
    /// requires of every VoIP push that must be reported.
    func reportUnreadable() {
        let uuid = UUID()
        let update = CXCallUpdate()
        update.remoteHandle = CXHandle(type: .generic, value: "aspen")
        update.hasVideo = false
        provider.reportNewIncomingCall(with: uuid, update: update) { [weak self] _ in
            self?.provider.reportCall(with: uuid, endedAt: nil, reason: .failed)
        }
    }

    /// The ring ended elsewhere, as the engine's watch says: the incoming call is reported ended
    /// with the reason, and nothing is asked of the deployment.
    func ringEnded(_ uuid: UUID, end: RingEnd) {
        guard let incoming, incoming.uuid == uuid else {
            return
        }
        self.incoming = nil
        incoming.watch = nil
        provider.reportCall(with: uuid, endedAt: nil, reason: Self.reason(end))
        update { $0.incoming = nil }
    }

    private static func reason(_ end: RingEnd) -> CXCallEndedReason {
        switch end {
        case .answeredElsewhere:
            return .answeredElsewhere
        case .declined:
            return .declinedElsewhere
        case .callEnded:
            return .remoteEnded
        case .timedOut:
            return .unanswered
        case .signedOut:
            return .failed
        }
    }

    private static var loopbackPlainText: Bool {
        #if DEBUG
            return true
        #else
            return false
        #endif
    }

    private static func update(channel: String, name: String?) -> CXCallUpdate {
        let update = CXCallUpdate()
        update.remoteHandle = CXHandle(type: .generic, value: channel)
        update.localizedCallerName = name
        update.hasVideo = false
        update.supportsHolding = true
        update.supportsGrouping = false
        update.supportsUngrouping = false
        update.supportsDTMF = false
        return update
    }

    // MARK: The engine's state

    private func stateChanged(_ state: CallState, of call: Current) {
        guard current === call else {
            return
        }
        switch state.status {
        case .joining, .rejoining:
            call.joined = true
        case .connected:
            call.joined = true
            if !call.connected {
                call.connected = true
                if call.outgoing {
                    provider.reportOutgoingCall(with: call.uuid, connectedAt: nil)
                }
            }
        case .idle, .failed:
            guard call.joined, !call.ending else {
                return
            }
            ended(call, reason: Self.reason(state))
        }
    }

    /// Why the call ended, for the system, from the engine's state at its end.
    private static func reason(_ state: CallState) -> CXCallEndedReason {
        if state.status == .failed {
            return .failed
        }
        switch state.endedReason {
        case .kicked, .accessLost, .idle, .none:
            return .remoteEnded
        }
    }

    /// The call is over without the system's end action: reported to it, with `reason`, or
    /// only let go of when `nil` (the system refused to end it).
    private func ended(_ call: Current, reason: CXCallEndedReason?) {
        guard current === call else {
            return
        }
        call.subscription = nil
        current = nil
        deactivating = call.engine.audio
        if let reason {
            provider.reportCall(with: call.uuid, endedAt: nil, reason: reason)
        }
        CallEngine.routes.reset()
        update {
            $0.call = nil
            $0.held = false
        }
    }

    // MARK: CXProviderDelegate

    func providerDidReset(_ provider: CXProvider) {
        // CallKit lost every call it knew of; so does the engine, and the session with it. A
        // ring it forgot is left to the deployment's clock.
        if let incoming {
            incoming.watch = nil
            self.incoming = nil
            update { $0.incoming = nil }
        }
        guard let current else {
            return
        }
        current.subscription = nil
        self.current = nil
        current.engine.engine.leave()
        current.engine.audio.deactivated(AVAudioSession.sharedInstance())
        CallEngine.routes.reset()
        update {
            $0 = Status(call: nil, audioActive: false, held: false, interrupted: false, refusal: $0.refusal)
        }
    }

    func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        guard let current, current.uuid == action.callUUID else {
            action.fail()
            return
        }
        // The session is configured before the call starts, so that the system activates it as
        // a call's, and never activated here.
        do {
            try current.engine.audio.configureForCall()
        } catch {
            NSLog("The audio session could not be configured for Aspen's call: %@", String(describing: error))
            current.subscription = nil
            self.current = nil
            update { $0.refusal = error.localizedDescription }
            action.fail()
            return
        }
        current.engine.engine.join(channel: current.channel)
        provider.reportOutgoingCall(with: current.uuid, startedConnectingAt: nil)
        provider.reportCall(with: current.uuid, updated: Self.update(channel: current.channel, name: current.name))
        action.fulfill()
        update { $0.call = current.uuid }
    }

    /// Answers the incoming call: the ring's deployment's engine joins the DM's call, which ends
    /// the ring at the deployment, as a call the system now holds. The audio session is
    /// configured before the action is fulfilled, as for a start, so the system activates it as a
    /// call's; a call of another engine the system held is left first.
    func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        guard let incoming, incoming.uuid == action.callUUID else {
            action.fail()
            return
        }
        self.incoming = nil
        incoming.watch?.stop()
        incoming.watch = nil
        if let current {
            current.engine.engine.leave()
            ended(current, reason: .remoteEnded)
        }
        let engine = CallEngines.shared.forOrigin(incoming.ring.origin)
        let made = Current(
            uuid: incoming.uuid,
            engine: engine,
            channel: incoming.ring.channel,
            name: incoming.ring.callerName,
            outgoing: false
        )
        do {
            try engine.audio.configureForCall()
        } catch {
            NSLog("The audio session could not be configured for Aspen's call: %@", String(describing: error))
            update {
                $0.incoming = nil
                $0.refusal = error.localizedDescription
            }
            action.fail()
            return
        }
        current = made
        made.subscription = engine.snapshots.listen { [weak self, weak made] state in
            guard let self, let made else {
                return
            }
            self.stateChanged(state, of: made)
        }
        engine.engine.join(channel: incoming.ring.channel)
        action.fulfill()
        update {
            $0.call = made.uuid
            $0.incoming = nil
            $0.refusal = nil
        }
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        if let incoming, incoming.uuid == action.callUUID {
            // Declined here: the ring ends at the deployment, so the person's other devices
            // stop ringing too.
            self.incoming = nil
            incoming.watch?.stop()
            incoming.watch = nil
            CallRings.shared.decline(origin: incoming.ring.origin, channel: incoming.ring.channel)
            action.fulfill()
            update { $0.incoming = nil }
            return
        }
        guard let current, current.uuid == action.callUUID else {
            action.fail()
            return
        }
        current.ending = true
        current.engine.engine.leave()
        ended(current, reason: nil)
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        guard let current, current.uuid == action.callUUID else {
            action.fail()
            return
        }
        // A mute asked while held is the user's own, kept beside the hold at the server for
        // when it ends.
        current.engine.engine.setMuted(muted: action.isMuted)
        action.fulfill()
    }

    /// The system's hold (its own controls, or a phone call answered on top of ours) is the
    /// engine's (`applyHold`).
    func provider(_ provider: CXProvider, perform action: CXSetHeldCallAction) {
        guard let current, current.uuid == action.callUUID else {
            action.fail()
            return
        }
        update { $0.held = action.isOnHold }
        applyHold(current)
        action.fulfill()
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        // Activated for no call (one ended as the system activated for it): the session goes
        // back with the `didDeactivate` that follows the ended report.
        guard let audio = current?.engine.audio else {
            return
        }
        audio.activated(audioSession)
        CallEngine.routes.apply()
        update {
            $0.audioActive = true
            $0.interrupted = false
        }
        if let current {
            applyHold(current)
        }
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        let audio = current?.engine.audio ?? deactivating
        deactivating = nil
        audio?.deactivated(audioSession)
        update { $0.audioActive = false }
    }

    // MARK: Interruptions

    /// Another session taking ours (a phone call, Siri, an alarm) and the system's word that it
    /// is over: `interruptionNotification` before iOS 27, which replaces it with
    /// `didBecomeInactiveNotification` (the system's deactivations alone count; the app's own,
    /// through CallKit, are `didDeactivate`) and `resumptionRecommendationNotification`.
    private func observeInterruptions() {
        let center = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()
        if #available(iOS 27, *) {
            observers.append(center.addObserver(forName: AVAudioSession.didBecomeInactiveNotification, object: session, queue: .main) {
                [weak self] notification in
                let context = notification.userInfo?[AVAudioSession.deactivationContextKey] as? AVAudioSession.DeactivationContext
                if context?.source == .system {
                    self?.interruptionBegan()
                }
            })
            observers.append(center.addObserver(forName: AVAudioSession.resumptionRecommendationNotification, object: session, queue: .main) {
                [weak self] notification in
                let context = notification.userInfo?[AVAudioSession.resumptionContextKey] as? AVAudioSession.ResumptionContext
                self?.interruptionEnded(shouldResume: context?.recommendation == .shouldResume)
            })
        } else {
            observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: session, queue: .main) {
                [weak self] notification in
                guard let raw = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                      let type = AVAudioSession.InterruptionType(rawValue: raw) else {
                    return
                }
                switch type {
                case .began:
                    self?.interruptionBegan()
                case .ended:
                    let options = notification.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
                    self?.interruptionEnded(shouldResume: AVAudioSession.InterruptionOptions(rawValue: options).contains(.shouldResume))
                @unknown default:
                    break
                }
            })
        }
    }

    /// WebRTC's audio stops; the session is the interrupter's now, and the engine's call goes
    /// on hold, since the user can neither hear nor be heard. For a call the system holds, its
    /// `didDeactivate` follows when the interrupter is another call.
    private func interruptionBegan() {
        guard let current else {
            return
        }
        current.engine.audio.interrupted()
        update { $0.interrupted = true }
        applyHold(current)
    }

    /// The interrupter is done. If the system still has our session active (it never called
    /// `didDeactivate`: a short interruption, Siri's) WebRTC's audio runs again on it and the
    /// hold ends; if it deactivated the session, `didActivate` is what resumes the call, and
    /// nothing is activated here, as CallKit requires.
    private func interruptionEnded(shouldResume: Bool) {
        guard let current else {
            return
        }
        let active = latest.audioActive
        if active, shouldResume {
            current.engine.audio.resumed()
        }
        update { $0.interrupted = false }
        applyHold(current)
    }
}
