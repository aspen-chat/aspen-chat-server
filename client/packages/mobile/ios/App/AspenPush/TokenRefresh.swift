import Foundation

/// A new session token for an account whose kept one the API server refused, from its refresh
/// token (`POST /auth/token-refresh`, which refresh tokens outlive, so the web view and the
/// native code each renew without taking the other's away). The notification service extension
/// and the call engine's credentials both renew this way and write the result back with
/// `PushStateStore.setSessionToken`.
enum TokenRefresh {
    /// Asks `account`'s deployment for a new session token through `session`; `done` gets it, or
    /// `nil` when the deployment refused or could not be reached.
    static func refresh(account: PushState.Account, session: URLSession, done: @escaping (String?) -> Void) {
        guard let url = URL(string: "\(account.origin)/api/v1/auth/token-refresh") else {
            done(nil)
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["refreshToken": account.refreshToken])
        session.dataTask(with: request) { data, response, _ in
            guard (response as? HTTPURLResponse)?.statusCode == 200,
                  let data,
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let token = object["sessionToken"] as? String
            else {
                done(nil)
                return
            }
            done(token)
        }.resume()
    }
}
