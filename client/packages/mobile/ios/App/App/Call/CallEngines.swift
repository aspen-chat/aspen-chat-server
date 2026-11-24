import Foundation

/// An account the call engines act as: the deployment at `origin` (its `public_url`, with no
/// trailing slash) and the user signed in there, as `PushState.Account` names them. The phone may
/// hold several accounts on one deployment, so an engine, its session, and whatever follows its
/// call are known by both, never by the deployment alone.
struct CallAccount: Hashable {
    let origin: String
    let userId: String

    init(origin: String, userId: String) {
        self.origin = CallEngines.trimmed(origin)
        self.userId = userId
    }

    /// Whether `account`, kept for push, is this one.
    func names(_ account: PushState.Account) -> Bool {
        CallEngines.trimmed(account.origin) == origin && account.userId == userId
    }
}

/// The call engines of this process, one per account: whatever acts as an account's engine looks
/// it up here, so an incoming call answered (`CallSystem`) and the plugin the web view commands
/// drive the same engine, and the one call the engine holds is the one the system knows. An
/// engine is made the first time its account is asked for and kept for the process's life. Use
/// on the main queue.
final class CallEngines {
    static let shared = CallEngines()

    private var engines: [CallAccount: CallEngine] = [:]

    /// The engine of `account`, made on first use. A debug build lets a development deployment's
    /// voice servers be reached over plain HTTP at this machine's own addresses; a release build
    /// never does.
    func forAccount(_ account: CallAccount) -> CallEngine {
        if let engine = engines[account] {
            return engine
        }
        #if DEBUG
            let loopbackPlainText = true
        #else
            let loopbackPlainText = false
        #endif
        let engine = CallEngine(account: account, loopbackPlainText: loopbackPlainText)
        engines[account] = engine
        return engine
    }

    /// The engine already made for `account`, if any.
    func existing(_ account: CallAccount) -> CallEngine? {
        engines[account]
    }

    static func trimmed(_ origin: String) -> String {
        var trimmed = Substring(origin)
        while trimmed.hasSuffix("/") {
            trimmed.removeLast()
        }
        return String(trimmed)
    }
}
