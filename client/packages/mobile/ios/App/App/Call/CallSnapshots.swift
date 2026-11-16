import Foundation

/// Every snapshot the call engine publishes (its `CallObserver`), kept for whoever asks what the
/// call is now and handed to each listener on the main queue, where the interface reads it: as
/// the record, and as the JSON the engine writes beside it (`listenJSON`, `latestJSON`), which
/// `AspenCallPlugin` hands the web view untouched; and each of the call's sounds the engine cues
/// (`playSound`), handed to `CallSounds` to play. The engine calls both on its own thread and
/// waits for them to return, so nothing here blocks or reaches back into the engine.
final class CallSnapshots: CallObserver, @unchecked Sendable {
    typealias Listener = (CallState) -> Void
    typealias JSONListener = (String) -> Void

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

    private let lock = NSLock()
    private let sounds: CallSounds
    private var current: CallState?
    private var currentJSON: String?
    private var listeners: [UUID: Listener] = [:]
    private var jsonListeners: [UUID: JSONListener] = [:]

    init(sounds: CallSounds) {
        self.sounds = sounds
    }

    func playSound(cue: SoundCue) {
        sounds.play(cue)
    }

    /// The latest snapshot, or `nil` before the engine published one.
    var latest: CallState? {
        lock.lock()
        defer { lock.unlock() }
        return current
    }

    /// The latest snapshot's JSON, or `nil` before the engine published one.
    var latestJSON: String? {
        lock.lock()
        defer { lock.unlock() }
        return currentJSON
    }

    func stateChanged(state: CallState, json: String) {
        lock.lock()
        current = state
        currentJSON = json
        let told = Array(listeners.values)
        let toldJSON = Array(jsonListeners.values)
        lock.unlock()
        DispatchQueue.main.async {
            for listener in told {
                listener(state)
            }
            for listener in toldJSON {
                listener(json)
            }
        }
    }

    /// As `listen`, with each snapshot's JSON.
    func listenJSON(_ listener: @escaping JSONListener) -> Subscription {
        let id = UUID()
        lock.lock()
        jsonListeners[id] = listener
        let latest = currentJSON
        lock.unlock()
        if let latest {
            DispatchQueue.main.async {
                listener(latest)
            }
        }
        return Subscription { [weak self] in
            guard let self else {
                return
            }
            self.lock.lock()
            self.jsonListeners[id] = nil
            self.lock.unlock()
        }
    }

    /// Calls `listener` on the main queue with the latest snapshot, when there is one, and with
    /// every one after it, until the subscription is cancelled or released.
    func listen(_ listener: @escaping Listener) -> Subscription {
        let id = UUID()
        lock.lock()
        listeners[id] = listener
        let latest = current
        lock.unlock()
        if let latest {
            DispatchQueue.main.async {
                listener(latest)
            }
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
}
