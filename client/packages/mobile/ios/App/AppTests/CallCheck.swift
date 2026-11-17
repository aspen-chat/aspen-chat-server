import Foundation
import WebRTC
import XCTest

/// Where the call tests run, as `scripts/check_mobile_call.py` says in their environment
/// (`TEST_RUNNER_ASPEN_CALL_*`): the simulator and a phone differ in what the platform can do, and
/// each difference is decided here by name rather than tolerated.
///
/// | Variable                 | Values           | What                                                                                                                                                                                                                            |
/// | ------------------------ | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
/// | `ASPEN_CALL_MICROPHONE`  | `tone`, `live`   | What WebRTC records and plays through: `ToneAudioDevice` on the simulator, whose own audio unit aborts the process when a test initializes it; the built-in audio device module on a phone, whose microphone hears the room      |
/// | `ASPEN_CALL_SYSTEM_CALL` | `ends`, `stands` | What CallKit does with the outgoing call: the simulator's ends it half a second after the start action, having no in-call app to open (`ios-app.md`, What only a device can show); a phone's keeps it and activates the session |
///
/// Unset, each is the platform's own (the simulator's, or a device's), so the tests run from Xcode
/// without the check; a value that is neither is refused by name.
enum CallCheck {
    enum Microphone: String, CaseIterable {
        case tone
        case live
    }

    enum SystemCall: String, CaseIterable {
        case ends
        case stands
    }

    struct Refused: Error, CustomStringConvertible {
        let description: String
    }

    static func microphone() throws -> Microphone {
        try read("ASPEN_CALL_MICROPHONE", simulator: .tone, device: .live)
    }

    static func systemCall() throws -> SystemCall {
        try read("ASPEN_CALL_SYSTEM_CALL", simulator: .ends, device: .stands)
    }

    /// The audio device the tests' peer connection factories run on: the one in software for
    /// `tone`, and for `live` none, which is WebRTC's own module on the phone's microphone and
    /// speaker.
    static func audioDevice() throws -> RTCAudioDevice? {
        switch try microphone() {
        case .tone:
            return ToneAudioDevice()
        case .live:
            return nil
        }
    }

    private static func read<Value: RawRepresentable & CaseIterable>(
        _ key: String, simulator: Value, device: Value
    ) throws -> Value where Value.RawValue == String {
        guard let given = ProcessInfo.processInfo.environment[key], !given.isEmpty else {
            #if targetEnvironment(simulator)
                return simulator
            #else
                return device
            #endif
        }
        guard let value = Value(rawValue: given) else {
            let known = Value.allCases.map(\.rawValue).joined(separator: " or ")
            throw Refused(description: "\(key) is \(given); it is \(known)")
        }
        return value
    }

    /// Reaches `url` once, before anything else of a run asks the network, and fails by name
    /// when it cannot, since each cause is the device's to fix: `trustFailure` when the device
    /// does not trust the certificate it is served (a phone without the development authority's
    /// profile turned on), `unreachable` when nothing answers (a phone on another network, or one
    /// that has not allowed Aspen on the local network, which iOS asks on the first request and
    /// which the wait here leaves time to answer). Any HTTP answer is a reach.
    static func reach(_ url: URL, seconds: Double) async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = seconds
        configuration.timeoutIntervalForResource = seconds
        do {
            _ = try await URLSession(configuration: configuration).data(from: url)
        } catch let error as URLError {
            let trust: Set<URLError.Code> = [
                .serverCertificateUntrusted, .serverCertificateHasUnknownRoot, .serverCertificateHasBadDate,
                .serverCertificateNotYetValid, .secureConnectionFailed, .clientCertificateRejected,
            ]
            let message: String
            if trust.contains(error.code) {
                message = "trustFailure: this device does not trust the certificate \(url.host ?? "the server") serves "
                    + "(\(error.code.rawValue), \(error.localizedDescription)): install the development authority's "
                    + "profile (scripts/dev_ca.py profile) and turn it on under Settings, General, About, Certificate "
                    + "Trust Settings"
            } else {
                message = "unreachable: \(url) answered nothing in \(Int(seconds)) s (\(error.code.rawValue), "
                    + "\(error.localizedDescription)): the device must be on the network the check serves on, and "
                    + "allow Aspen on the local network when iOS asks"
            }
            XCTFail(message)
            throw Refused(description: message)
        }
    }
}
