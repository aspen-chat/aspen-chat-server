import Foundation

/// The session the call engine acts as (`Credentials`): its account among those the app keeps
/// for push (`PushStateStore`, the keychain item the web view writes on
/// sign-in and the notification service extension reads). The token kept there is sent; when the
/// API server refuses it, a new one is got from the account's refresh token and written back, so
/// the extension and later calls use it too. Nothing is cached here: the keychain is read for
/// each request, so a token the web view renewed meanwhile is the one sent.
final class CallCredentials: Credentials, @unchecked Sendable {
    private let account: CallAccount
    private let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 12
        configuration.timeoutIntervalForResource = 12
        return URLSession(configuration: configuration)
    }()

    init(account: CallAccount) {
        self.account = account
    }

    private func kept() -> PushState.Account? {
        PushStateStore.load()?.accounts.first { account.names($0) }
    }

    func sessionToken() async -> String? {
        kept()?.sessionToken
    }

    func renewedSessionToken(rejected: String) async -> String? {
        guard let account = kept() else {
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
