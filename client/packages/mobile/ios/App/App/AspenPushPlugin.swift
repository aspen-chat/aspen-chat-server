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

    /// Which platform, app, and relay this build is: APNs, the bundle id as its topic, the
    /// sandbox for a debug build and production otherwise, and the relay `AspenPushRelay` in
    /// `Info.plist` names, without which the build has no push.
    @objc func describe(_ call: CAPPluginCall) {
        guard let relay = Bundle.main.object(forInfoDictionaryKey: "AspenPushRelay") as? String,
              !relay.isEmpty
        else {
            call.reject("this build names no push relay")
            return
        }
        #if DEBUG
            let environment = "sandbox"
        #else
            let environment = "production"
        #endif
        call.resolve([
            "platform": "apns",
            "app": Bundle.main.bundleIdentifier ?? "",
            "environment": environment,
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
