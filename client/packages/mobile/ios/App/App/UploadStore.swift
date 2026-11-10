import Foundation
import Security

/// An upload as `UploadCenter` keeps it from `send` until `discard`: where its bytes go, whose
/// they are and the session to confirm them with, and how far it has got. Its bytes are not
/// here but in the spool file of its name.
struct UploadRecord: Codable {
    enum State: String, Codable {
        case sending, done, failed
    }

    enum Cause: String, Codable {
        case refused, unreachable
    }

    /// The attachment's id as the page wrote it, which is how the page is told of it.
    let id: String
    var origin: String
    var userId: String
    /// Empty once the upload is `done` or `failed`: nothing more is asked as the user then, and
    /// a `send` that takes it up again brings tokens of its own.
    var sessionToken: String
    var refreshToken: String
    /// The presigned URL the bytes are put to; empty when the tokens are.
    var uploadUrl: String
    var contentType: String
    var name: String
    var mimeType: String
    var byteSize: Int64
    /// The languages the deployment is asked to answer in, as an `Accept-Language` header;
    /// `nil` or empty leaves it to the system's.
    var acceptLanguage: String?
    var failedTitle: String
    var failedBody: String
    /// When `send` last took it up, which a record's age is counted from.
    var sentAt: Date
    var state: State = .sending
    /// The background session's task sending its bytes. A task of another number that names
    /// this upload is one since replaced, and how it ends says nothing of the upload.
    var task: Int?
    /// Whether storage has the bytes, so that only confirming them is left.
    var arrived = false
    /// How many other URLs have been asked for since `send`.
    var renewals = 0
    /// Once `done`: the attachment's record as the deployment answered it, JSON text.
    var attachment: String?
    var cause: Cause?
    /// For a refusal, the deployment's own words.
    var reason: String?
}

/// Every `UploadRecord`, as JSON, in a keychain item of the app's own, since each holds a
/// session and the refresh token behind it. The item is readable once the phone has been
/// unlocked after starting, because an upload can end, and be confirmed, while the phone is
/// locked, and on this device only, so it goes into no backup and moves to no new phone.
enum UploadStore {
    private static let service = "com.aspen-chat.client.uploads"
    private static let account = "records"
    private static let accessible = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

    /// The access group only the app is in, its application identifier
    /// (`AspenAppIdentifierPrefix` in `Info.plist`, then the bundle id), or `nil` for a build
    /// signed for no team. It is named because an item given no group goes into the first of
    /// the app's `keychain-access-groups`, which is the one it shares with the notification
    /// service extension (`PushStateStore`), and the extension has no use for these.
    private static var accessGroup: String? {
        guard let prefix = Bundle.main.object(forInfoDictionaryKey: "AspenAppIdentifierPrefix") as? String,
              !prefix.isEmpty,
              let bundle = Bundle.main.bundleIdentifier
        else {
            return nil
        }
        return prefix + bundle
    }

    private struct Kept: Codable {
        let version: Int
        let uploads: [UploadRecord]
    }

    private static func query() -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        if let group = accessGroup {
            query[kSecAttrAccessGroup as String] = group
        }
        return query
    }

    /// The records kept, none when nothing readable as version 1 is kept, or `nil` when the
    /// keychain cannot say (the phone has not been unlocked since it started). The caller must
    /// tell the last two apart: taking `nil` for none would have the next `save` write over
    /// records that are still there.
    static func load() -> [UploadRecord]? {
        var query = query()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound {
            return []
        }
        guard status == errSecSuccess, let data = item as? Data else {
            return nil
        }
        guard let kept = try? JSONDecoder().decode(Kept.self, from: data), kept.version == 1 else {
            return []
        }
        return kept.uploads
    }

    /// Keeps `records` in place of what was kept.
    static func save(_ records: [UploadRecord]) -> Bool {
        guard let data = try? JSONEncoder().encode(Kept(version: 1, uploads: records)) else {
            return false
        }
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
}
