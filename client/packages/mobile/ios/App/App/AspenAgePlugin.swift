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

    /// How long the system may take to say whether the user's region requires age assurance
    /// before the app goes on as where it does not. The question has been seen never to be
    /// answered (and so the app never to start); the sheet a person answers is not timed.
    static let eligibilityWaitNanoseconds: UInt64 = 5_000_000_000

    /// The launch argument a simulator answers from (`-AspenAgeCheck underage`, or `adult`), in
    /// place of Declared Age Range, which Apple supports only on devices and which leaves the
    /// app waiting there. Without it a simulator answers adult. Devices never read it.
    static let simulatorAnswerKey = "AspenAgeCheck"

    /// Whether the system says the user is under `adult`. It is asked only where the user's
    /// region requires age assurance (`isEligibleForAgeFeatures`), so nobody elsewhere is shown
    /// the system's sheet. A system too old to say, one that does not say within
    /// `eligibilityWaitNanoseconds` whether the region requires it, a user who declines to share,
    /// or any failure answers `false`, as a device that reports no age does.
    @MainActor
    static func underage(in viewController: UIViewController?) async -> Bool {
        #if targetEnvironment(simulator)
        return UserDefaults.standard.string(forKey: simulatorAnswerKey) == "underage"
        #else
        guard #available(iOS 26.2, *), let viewController else {
            return false
        }
        guard await eligible() else {
            return false
        }
        do {
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
        #endif
    }

    /// Whether the user's region requires age assurance, or `false` once
    /// `eligibilityWaitNanoseconds` has passed without an answer. The answer and the wait race to
    /// resume one continuation, rather than run as a task group, which would wait on for a call
    /// that never returns.
    @available(iOS 26.2, *)
    @MainActor
    private static func eligible() async -> Bool {
        await withCheckedContinuation { continuation in
            let first = FirstAnswer(continuation)
            Task { @MainActor in
                first.give((try? await AgeRangeService.shared.isEligibleForAgeFeatures) ?? false)
            }
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: eligibilityWaitNanoseconds)
                first.give(false)
            }
        }
    }
}

/// Resumes its continuation with the first answer given, and ignores the rest.
@MainActor
private final class FirstAnswer {
    private var continuation: CheckedContinuation<Bool, Never>?

    nonisolated init(_ continuation: CheckedContinuation<Bool, Never>) {
        self.continuation = continuation
    }

    func give(_ answer: Bool) {
        continuation?.resume(returning: answer)
        continuation = nil
    }
}
