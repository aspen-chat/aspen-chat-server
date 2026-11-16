import CoreMedia
import ReplayKit

/// The broadcast upload extension: on iOS before 27 (and wherever ScreenCaptureKit's picker is
/// not available) the system captures the screen in this process, started from the system's
/// broadcast picker, and hands each sample here; they go to the app over `BroadcastChannel`,
/// which is compiled into this target too, and the app sends them into its call. The broadcast
/// is for a share the app asked for: when the app is not listening (the broadcast started from
/// Control Center with no call sharing) it is finished at once with a message. The app ends the
/// share by sending `stop`, on which the broadcast is finished without an alert (the user's own
/// stop, `RPRecordingErrorCode.userDeclined`, which the system shows nothing for). The
/// microphone's samples are never sent: the call's microphone is the app's own.
final class SampleHandler: RPBroadcastSampleHandler {
    private let sender = BroadcastSender()

    override func broadcastStarted(withSetupInfo setupInfo: [String: NSObject]?) {
        sender.onStop = { [weak self] in
            self?.finishBroadcastWithError(SampleHandler.stoppedByUser)
        }
        if !sender.connect() {
            finishBroadcastWithError(
                NSError(
                    domain: Bundle.main.bundleIdentifier ?? "com.aspen-chat.client.broadcast", code: 1,
                    userInfo: [
                        NSLocalizedFailureReasonErrorKey: NSLocalizedString(
                            "broadcastNotInCall", comment: "The broadcast was started outside a call that shares"),
                    ]))
        }
    }

    override func broadcastFinished() {
        sender.close()
    }

    override func processSampleBuffer(_ sampleBuffer: CMSampleBuffer, with sampleBufferType: RPSampleBufferType) {
        switch sampleBufferType {
        case .video:
            guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else {
                return
            }
            let presented = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
            let timestampNs = presented.isNumeric ? Int64(CMTimeGetSeconds(presented) * 1_000_000_000) : 0
            sender.send(frame: pixelBuffer, rotation: Self.rotation(of: sampleBuffer), timestampNs: timestampNs)
        case .audioApp:
            sender.send(audio: sampleBuffer)
        case .audioMic:
            break
        @unknown default:
            break
        }
    }

    /// The degrees clockwise a frame must turn to be upright, from the orientation ReplayKit
    /// attaches to each (`RPVideoSampleOrientationKey`, a `CGImagePropertyOrientation`): the
    /// screen is captured as the device holds it, upright in portrait.
    static func rotation(of sampleBuffer: CMSampleBuffer) -> Int {
        guard let value = CMGetAttachment(sampleBuffer, key: RPVideoSampleOrientationKey as CFString, attachmentModeOut: nil)
            as? NSNumber,
            let orientation = CGImagePropertyOrientation(rawValue: value.uint32Value)
        else {
            return 0
        }
        switch orientation {
        case .up, .upMirrored: return 0
        case .down, .downMirrored: return 180
        case .left, .leftMirrored: return 90
        case .right, .rightMirrored: return 270
        @unknown default: return 0
        }
    }

    /// Ends the broadcast as the user's own stop, which the system shows no alert for.
    static let stoppedByUser = NSError(
        domain: RPRecordingErrorDomain, code: RPRecordingErrorCode.userDeclined.rawValue, userInfo: nil)
}
