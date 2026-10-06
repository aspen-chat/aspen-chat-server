import Foundation
import Security

/// What the notification code needs while the app is closed (`spec/push.md`, The app): the
/// `PushState` the app keeps through `AspenPush.saveState`, read here by the app and by the
/// notification service extension alike. Only what the extension reads is typed; the rest of
/// the object is kept as it was written, so the app's own fields survive a round trip.
struct PushState: Codable {
    struct Keys: Codable {
        struct PrivateKey: Codable {
            let d: String
        }

        let privateKey: PrivateKey
        /// The subscription's public key, an uncompressed P-256 point in base64url.
        let publicKey: String
        /// The 16-byte authentication secret in base64url.
        let auth: String
    }

    struct Account: Codable {
        /// The relay's subscription id, a push's `s`.
        let subscription: String
        let origin: String
        let userId: String
        var refreshToken: String
        var sessionToken: String
        let keys: Keys
    }

    let version: Int
    var accounts: [Account]
}

/// The `PushState`, as JSON, in a keychain item of the access group the app and the extension
/// share (`AspenKeychainAccessGroup` in each one's `Info.plist`, `$(AppIdentifierPrefix)` and
/// the app's bundle id with `.push`), readable once the phone has been unlocked after starting
/// (a push reaches the extension only then), and on this device only: the item holds sessions
/// and the subscriptions' private keys, so it goes into no backup and moves to no new phone.
enum PushStateStore {
    private static let service = "org.aspenchat.client.push"
    private static let account = "state"
    private static let accessible = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

    private static var accessGroup: String? {
        Bundle.main.object(forInfoDictionaryKey: "AspenKeychainAccessGroup") as? String
    }

    private static func query() -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        if let group = accessGroup, !group.isEmpty {
            query[kSecAttrAccessGroup as String] = group
        }
        return query
    }

    /// Makes the kept item this device's only, should it have been kept with an accessibility
    /// that backups carry; one already so, or none at all, is left as it is. Every read does
    /// it, which costs one keychain call and holds no state between calls.
    private static func migrate() {
        let attributes: [String: Any] = [kSecAttrAccessible as String: accessible]
        _ = SecItemUpdate(query() as CFDictionary, attributes as CFDictionary)
    }

    /// The state as the app wrote it, or `nil` when it has written none.
    static func loadJSON() -> String? {
        migrate()
        var query = query()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data
        else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    /// Keeps `json` as the state, replacing what was kept.
    static func save(json: String) -> Bool {
        let data = Data(json.utf8)
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: accessible,
        ]
        let status = SecItemUpdate(query() as CFDictionary, attributes as CFDictionary)
        if status == errSecSuccess {
            return true
        }
        if status != errSecItemNotFound {
            return false
        }
        var insert = query()
        insert.merge(attributes) { _, new in new }
        return SecItemAdd(insert as CFDictionary, nil) == errSecSuccess
    }

    /// The typed state, when one is kept and reads as version 1.
    static func load() -> PushState? {
        guard let json = loadJSON(),
              let state = try? JSONDecoder().decode(PushState.self, from: Data(json.utf8)),
              state.version == 1
        else {
            return nil
        }
        return state
    }

    /// Writes a new session token for `subscription`'s account into the kept JSON, touching
    /// nothing else of it.
    static func setSessionToken(subscription: String, token: String) {
        guard let json = loadJSON(),
              var object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
              var accounts = object["accounts"] as? [[String: Any]]
        else {
            return
        }
        for index in accounts.indices where accounts[index]["subscription"] as? String == subscription {
            accounts[index]["sessionToken"] = token
        }
        object["accounts"] = accounts
        if let data = try? JSONSerialization.data(withJSONObject: object),
           let text = String(data: data, encoding: .utf8)
        {
            _ = save(json: text)
        }
    }
}
