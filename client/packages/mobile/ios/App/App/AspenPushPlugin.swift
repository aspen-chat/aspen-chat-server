import Capacitor
import Foundation

/// The native side of push (`AspenPush` in the app's `src/api/pushBridge.ts`, which Android's
/// plugin of the same name answers too): what this build is, and the `PushState` the
/// notification service extension reads while the app is closed, kept in the keychain access
/// group the two share (`PushStateStore`).
@objc(AspenPushPlugin)
public class AspenPushPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "AspenPushPlugin"
    public let jsName = "AspenPush"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "describe", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "loadState", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "saveState", returnType: CAPPluginReturnPromise),
    ]

    /// Which platform, app, and relay this build is: APNs, the bundle id as its topic, the APNs
    /// environment its signing gives its tokens (`ApsEnvironment`), and the relay
    /// `AspenPushRelay` in `Info.plist` names. A build naming no relay, or signed without push,
    /// has none.
    @objc func describe(_ call: CAPPluginCall) {
        guard let relay = Bundle.main.object(forInfoDictionaryKey: "AspenPushRelay") as? String,
              !relay.isEmpty
        else {
            call.reject("this build names no push relay")
            return
        }
        guard let environment = ApsEnvironment.ofThisBuild() else {
            call.reject("this build's provisioning profile grants no aps-environment")
            return
        }
        call.resolve([
            "platform": "apns",
            "app": Bundle.main.bundleIdentifier ?? "",
            "environment": environment.rawValue,
            "relay": relay,
        ])
    }

    @objc func loadState(_ call: CAPPluginCall) {
        if let state = PushStateStore.loadJSON() {
            call.resolve(["state": state])
        } else {
            call.resolve(["state": NSNull()])
        }
    }

    @objc func saveState(_ call: CAPPluginCall) {
        guard let state = call.getString("state") else {
            call.reject("state is required")
            return
        }
        if PushStateStore.save(json: state) {
            call.resolve()
        } else {
            call.reject("the push state could not be kept")
        }
    }
}

/// The APNs environment this build's tokens belong to, which the relay must send to: the
/// `aps-environment` entitlement its provisioning profile grants, not whether it was compiled for
/// debugging, since a Release build signed for development is given sandbox tokens. A build with
/// no profile in its bundle is the App Store's or TestFlight's, whose tokens are production; on
/// the simulator they are the sandbox's. `nil` for a profile that grants none, or that cannot be
/// read, whose build is given no token.
enum ApsEnvironment: String {
    case sandbox
    case production

    static func ofThisBuild() -> ApsEnvironment? {
        guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision")
        else {
            #if targetEnvironment(simulator)
                return .sandbox
            #else
                return .production
            #endif
        }
        guard let profile = try? Data(contentsOf: url) else {
            return nil
        }
        return of(profile: profile)
    }

    /// Reads a provisioning profile's entitlement: the profile is a CMS envelope holding a
    /// property list, read whole once cut out of it. `nil` for a profile that grants none.
    static func of(profile: Data) -> ApsEnvironment? {
        guard let start = profile.range(of: Data("<?xml".utf8)),
              let end = profile.range(of: Data("</plist>".utf8), in: start.lowerBound ..< profile.endIndex),
              let plist = try? PropertyListSerialization.propertyList(
                  from: profile.subdata(in: start.lowerBound ..< end.upperBound), format: nil
              ) as? [String: Any],
              let entitlements = plist["Entitlements"] as? [String: Any],
              let granted = entitlements["aps-environment"] as? String
        else {
            return nil
        }
        switch granted {
        case "development": return .sandbox
        case "production": return .production
        default: return nil
        }
    }
}
