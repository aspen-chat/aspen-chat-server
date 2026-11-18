import Capacitor
import Foundation
import Security

/// The app's saved sign-ins (`AspenSecrets` in the app's `src/api/sessions.ts`), as one keychain
/// item in the access group only the app is in (`UploadStore.accessGroup`), readable once the
/// phone has been unlocked after starting and on this device only, so it goes into no backup and
/// moves to no new phone.
@objc(AspenSecretsPlugin)
public class AspenSecretsPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "AspenSecretsPlugin"
    public let jsName = "AspenSecrets"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "read", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "write", returnType: CAPPluginReturnPromise),
    ]

    private static let accessible = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

    private func query() -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "com.aspen-chat.client.sessions",
            kSecAttrAccount as String: "sessions",
        ]
        if let group = UploadStore.accessGroup {
            query[kSecAttrAccessGroup as String] = group
        }
        return query
    }

    @objc func read(_ call: CAPPluginCall) {
        var query = query()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            if let data = item as? Data, let contents = String(data: data, encoding: .utf8) {
                call.resolve(["status": "ok", "contents": contents])
            } else {
                call.resolve(["status": "unreadable"])
            }
        case errSecItemNotFound:
            call.resolve(["status": "ok", "contents": NSNull()])
        case errSecMissingEntitlement, errSecNotAvailable:
            call.resolve(["status": "unavailable"])
        default:
            call.resolve(["status": "unreadable"])
        }
    }

    @objc func write(_ call: CAPPluginCall) {
        guard let contents = call.getString("contents") else {
            call.reject("contents is required")
            return
        }
        let attributes: [String: Any] = [
            kSecValueData as String: Data(contents.utf8),
            kSecAttrAccessible as String: Self.accessible,
        ]
        var status = SecItemUpdate(query() as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var insert = query()
            insert.merge(attributes) { _, new in new }
            status = SecItemAdd(insert as CFDictionary, nil)
        }
        call.resolve(["kept": status == errSecSuccess])
    }
}
