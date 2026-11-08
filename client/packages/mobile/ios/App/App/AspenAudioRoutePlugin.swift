import AVFoundation
import Capacitor
import UIKit

/// Whether a call plays through the iPhone's receiver or its loudspeaker (`AspenAudioRoute` in
/// the app's `src/api/audioRoute.ts`, which Android's plugin of the same name answers too), which
/// the web view's own WebRTC cannot choose. The app holds the route from a call's start to its
/// end, and only while nothing else that takes a call on its own (headphones, a headset, a car)
/// is connected. The web view sets the shared audio session's category while it captures the
/// microphone; the route is chosen within it, and chosen again when the web view sets it anew.
@objc(AspenAudioRoutePlugin)
public class AspenAudioRoutePlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "AspenAudioRoutePlugin"
    public let jsName = "AspenAudioRoute"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "outputs", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setSpeakerphone", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "release", returnType: CAPPluginReturnPromise),
    ]

    /// The loudspeaker (`true`) or the receiver while the app holds the route; `nil` while not.
    private var wanted: Bool?
    /// Whether choosing the receiver took `defaultToSpeaker` off the web view's category, which
    /// `release` puts back.
    private var removedDefaultToSpeaker = false
    /// The outputs last told to the page, so a route change that changes none tells nothing.
    private var told: [String: Bool]?

    override public func load() {
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(routeChanged(_:)),
            name: AVAudioSession.routeChangeNotification,
            object: nil
        )
    }

    /// Answers `{ receiver, speaker, other }`.
    @objc func outputs(_ call: CAPPluginCall) {
        call.resolve(Self.outputs())
    }

    /// Plays the call through the loudspeaker (`on`) or the receiver until `release`.
    @objc func setSpeakerphone(_ call: CAPPluginCall) {
        let on = call.getBool("on", true)
        wanted = on
        apply(on)
        call.resolve()
    }

    /// Gives the route back to the web view's own choice.
    @objc func release(_ call: CAPPluginCall) {
        if wanted != nil {
            wanted = nil
            let session = AVAudioSession.sharedInstance()
            if session.category == .playAndRecord {
                try? session.overrideOutputAudioPort(.none)
                if removedDefaultToSpeaker {
                    try? session.setCategory(
                        .playAndRecord,
                        mode: session.mode,
                        options: session.categoryOptions.union(.defaultToSpeaker)
                    )
                }
            }
            removedDefaultToSpeaker = false
        }
        call.resolve()
    }

    /// What the device can play a call through. Only an iPhone has a receiver; anything else on
    /// the route, or a microphone other than the built-in one (a headset's), is something else.
    static func outputs() -> [String: Bool] {
        let session = AVAudioSession.sharedInstance()
        let builtIn: Set<AVAudioSession.Port> = [.builtInReceiver, .builtInSpeaker]
        let otherOutput = session.currentRoute.outputs.contains { !builtIn.contains($0.portType) }
        let otherInput = session.availableInputs?.contains { $0.portType != .builtInMic } ?? false
        return [
            "receiver": UIDevice.current.userInterfaceIdiom == .phone,
            "speaker": true,
            "other": otherOutput || otherInput,
        ]
    }

    /// Chooses the loudspeaker or the receiver within the web view's category. Outside a call's
    /// capture there is no category to choose within, and the web view's setting one is a route
    /// change that chooses again.
    private func apply(_ on: Bool) {
        let session = AVAudioSession.sharedInstance()
        guard session.category == .playAndRecord else {
            return
        }
        if on {
            try? session.overrideOutputAudioPort(.speaker)
            return
        }
        // With `defaultToSpeaker` the session's own route is the loudspeaker, which no override
        // of the output takes back to the receiver.
        if session.categoryOptions.contains(.defaultToSpeaker) {
            do {
                try session.setCategory(
                    .playAndRecord,
                    mode: session.mode,
                    options: session.categoryOptions.subtracting(.defaultToSpeaker)
                )
                removedDefaultToSpeaker = true
            } catch {
                return
            }
        }
        try? session.overrideOutputAudioPort(.none)
    }

    /// Tells the page what it can play through when that changes, and, while the app holds the
    /// route, chooses again after the web view set its category, which resets the choice.
    @objc private func routeChanged(_ notification: Notification) {
        let outputs = Self.outputs()
        if outputs != told {
            told = outputs
            notifyListeners("outputsChange", data: outputs)
        }
        let raw = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt
        guard
            let wanted,
            let raw,
            AVAudioSession.RouteChangeReason(rawValue: raw) == .categoryChange,
            outputs["other"] == false
        else {
            return
        }
        let speaker = AVAudioSession.sharedInstance().currentRoute.outputs
            .contains { $0.portType == .builtInSpeaker }
        if speaker != wanted {
            apply(wanted)
        }
    }
}
