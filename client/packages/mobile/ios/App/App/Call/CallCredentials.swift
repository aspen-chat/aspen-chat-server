import Foundation

/// The session the call engine acts as (`Credentials`): the account at one deployment among
/// those the app keeps for push (`PushStateStore`, the keychain item the web view writes on
/// sign-in and the notification service extension reads). The token kept there is sent; when the
/// API server refuses it, a new one is got from the account's refresh token and written back, so
/// the extension and later calls use it too. Nothing is cached here: the keychain is read for
/// each request, so a token the web view renewed meanwhile is the one sent.
final class CallCredentials: Credentials, @unchecked Sendable {
    /// The deployment's `public_url`, as `PushState.Account.origin` names it.
    private let origin: String
    private let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 12
        configuration.timeoutIntervalForResource = 12
        return URLSession(configuration: configuration)
    }()

    init(origin: String) {
        self.origin = Self.trimmed(origin)
    }

    private static func trimmed(_ origin: String) -> String {
        var trimmed = Substring(origin)
        while trimmed.hasSuffix("/") {
            trimmed.removeLast()
        }
        return String(trimmed)
    }

    private func account() -> PushState.Account? {
        PushStateStore.load()?.accounts.first { Self.trimmed($0.origin) == origin }
    }

    func sessionToken() async -> String? {
        account()?.sessionToken
    }

    func renewedSessionToken(rejected: String) async -> String? {
        guard let account = account() else {
            return nil
        }
        // Another reader of the state (the extension, after a push) may have renewed the token
        // already; the one kept is used before another is asked for.
        if account.sessionToken != rejected {
            return account.sessionToken
        }
        let token: String? = await withCheckedContinuation { continuation in
            TokenRefresh.refresh(account: account, session: session) { token in
                continuation.resume(returning: token)
            }
        }
        if let token {
            PushStateStore.setSessionToken(subscription: account.subscription, token: token)
        }
        return token
    }
}
