import Foundation

/// The call engines of this process, one per deployment: whatever acts as an account's engine
/// looks its deployment up here, so an incoming call answered (`CallSystem`) and the plugin the
/// web view commands drive the same engine, and the one call the engine holds is the one the
/// system knows. An engine is made the first time its deployment is asked for and kept for the
/// process's life. Use on the main queue.
final class CallEngines {
    static let shared = CallEngines()

    /// By the deployment's `public_url`, with no trailing slash.
    private var engines: [String: CallEngine] = [:]

    /// The engine of the deployment at `origin` (its `public_url`), made on first use. A debug
    /// build lets a development deployment's voice servers be reached over plain HTTP at this
    /// machine's own addresses; a release build never does.
    func forOrigin(_ origin: String) -> CallEngine {
        let key = Self.trimmed(origin)
        if let engine = engines[key] {
            return engine
        }
        #if DEBUG
            let loopbackPlainText = true
        #else
            let loopbackPlainText = false
        #endif
        let engine = CallEngine(origin: key, loopbackPlainText: loopbackPlainText)
        engines[key] = engine
        return engine
    }

    /// The engine already made for `origin`, if any.
    func existing(_ origin: String) -> CallEngine? {
        engines[Self.trimmed(origin)]
    }

    static func trimmed(_ origin: String) -> String {
        var trimmed = Substring(origin)
        while trimmed.hasSuffix("/") {
            trimmed.removeLast()
        }
        return String(trimmed)
    }
}
