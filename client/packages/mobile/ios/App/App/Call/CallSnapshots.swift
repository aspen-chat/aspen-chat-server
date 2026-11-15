import Foundation

/// Every snapshot the call engine publishes (its `CallObserver`), kept for whoever asks what the
/// call is now and handed to each listener on the main queue, where the interface reads it. The
/// engine calls `stateChanged` on its own thread and waits for it to return, so nothing here
/// blocks or reaches back into the engine.
final class CallSnapshots: CallObserver, @unchecked Sendable {
    typealias Listener = (CallState) -> Void

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
    private var current: CallState?
    private var listeners: [UUID: Listener] = [:]

    /// The latest snapshot, or `nil` before the engine published one.
    var latest: CallState? {
        lock.lock()
        defer { lock.unlock() }
        return current
    }

    func stateChanged(state: CallState) {
        lock.lock()
        current = state
        let told = Array(listeners.values)
        lock.unlock()
        DispatchQueue.main.async {
            for listener in told {
                listener(state)
            }
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
