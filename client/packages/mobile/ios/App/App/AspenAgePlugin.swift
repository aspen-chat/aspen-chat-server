import Capacitor
import DeclaredAgeRange
import UIKit

/// Whether the device says its user is under 18 where the law asks the app to check
/// (`AspenAge` in the app's `src/api/ageGate.ts`, which Android's plugin of the same name answers
/// too), through Declared Age Range. Nothing is kept: the app asks each time it starts.
@objc(AspenAgePlugin)
public class AspenAgePlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "AspenAgePlugin"
    public let jsName = "AspenAge"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "check", returnType: CAPPluginReturnPromise),
    ]

    /// The youngest age that may use Aspen.
    static let adult = 18

    /// Answers `{ underage }`.
    @objc func check(_ call: CAPPluginCall) {
        Task { @MainActor in
            let underage = await Self.underage(in: self.bridge?.viewController)
            call.resolve(["underage": underage])
        }
    }

    /// Whether the system says the user is under `adult`. It is asked only where the user's
    /// region requires age assurance (`isEligibleForAgeFeatures`), so nobody elsewhere is shown
    /// the system's sheet. A system too old to say, a user who declines to share, or any failure
    /// answers `false`, as a device that reports no age does.
    @MainActor
    static func underage(in viewController: UIViewController?) async -> Bool {
        guard #available(iOS 26.2, *), let viewController else {
            return false
        }
        do {
            guard try await AgeRangeService.shared.isEligibleForAgeFeatures else {
                return false
            }
            let response = try await AgeRangeService.shared.requestAgeRange(
                ageGates: adult,
                in: viewController
            )
            guard case let .sharing(range) = response, let upper = range.upperBound else {
                return false
            }
            return upper < adult
        } catch {
            return false
        }
    }
}
