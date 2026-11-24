import AVFoundation
import CoreMedia
import Foundation
import ReplayKit
import UIKit
import WebRTC

#if canImport(ScreenCaptureKit)
    import ScreenCaptureKit
#endif

/// Why the screen could not be shared, as the page's `ShareFailure` names it and the plugin
/// rejects or its `share` event says.
enum CallScreenFailure: String, Error {
    /// The user declined the system's consent, or dismissed its picker.
    case denied
    /// This device cannot capture its screen.
    case unsupported
    /// The capture would not start, or ended on its own.
    case failed
}

/// What a screen capturer tells `CallScreen` of the capture, from any thread.
protocol CallScreenSink: AnyObject {
    /// The user consented and media is on its way: the share's tracks are made and sent.
    func captureStarted()
    func capture(didCapture frame: RTCVideoFrame)
    /// App audio with the picture, as PCM in whatever format the capture gives.
    func capture(didCaptureAudio sampleBuffer: CMSampleBuffer)
    /// The capture is over: `nil` when the user or the system ended it, a failure otherwise.
    func captureEnded(_ failure: CallScreenFailure?)
}

/// What captures the screen: ScreenCaptureKit in this process on iOS 27 and later
/// (`SystemScreenCapturer`), the broadcast upload extension before it (`BroadcastScreenCapturer`),
/// or, in a test on the simulator, which can do neither, frames drawn in software.
protocol CallScreenCapturer: AnyObject {
    /// Asks the system for the screen through its own consent and starts capturing, with the
    /// app's audio when `audio`; what follows reaches `sink`.
    func start(audio: Bool, sink: CallScreenSink)
    func stop()
}

/// The call's share on this device: the screen captured by whichever capturer the system
/// offers (`makeCapturer`), as a video track and, with its sound, an audio track of the share's
/// WebRTC factory (`CallPeerConnections.captureFactory`, whose audio device the sound is fed to),
/// which the engine sends on the capture connection. The peer connections are read as each
/// share starts (`media`), so the share's tracks and sound belong to the factory the engines run
/// on then, whichever a test has put there. The tracks are made only once the capture has
/// started, so the engine asks to send nothing the user has not consented to; the self preview
/// is drawn from the same video track (`CallVideo`). Call on the main queue; the callbacks
/// arrive there.
final class CallScreen: CallScreenSink {
    private let media: () -> CallPeerConnections
    private let tracks: CallTracks
    /// The share's audio device, of the factory the tracks were made from, while a share is on.
    private var shareAudio: ShareAudioDevice?
    /// Where a test on the simulator puts a capturer of its own.
    var makeCapturer: () -> CallScreenCapturer = { CallScreen.systemCapturer() }
    /// The capturer of the share asked for or on; readable for the proof's diagnosis.
    private(set) var capturer: CallScreenCapturer?
    private var source: RTCVideoSource?
    private let frameSource = RTCVideoCapturer()
    private(set) var videoTrack: RTCVideoTrack?
    private(set) var audioTrack: RTCAudioTrack?
    /// The account whose engine the share is for, while one is asked for or on.
    private(set) var account: CallAccount?
    private var wantsAudio = false
    private var onStarted: ((TrackId, TrackId?) -> Void)?
    private var onEnded: ((CallScreenFailure?) -> Void)?

    init(media: @escaping () -> CallPeerConnections, tracks: CallTracks) {
        self.media = media
        self.tracks = tracks
    }

    /// Whether a share is asked for or on.
    var isActive: Bool {
        capturer != nil
    }

    /// Whether the share's media flows.
    var isOn: Bool {
        videoTrack != nil
    }

    /// The capturer this system offers: ScreenCaptureKit's picker where it is available, else
    /// the broadcast extension through the system's broadcast picker.
    static func systemCapturer() -> CallScreenCapturer {
        #if canImport(ScreenCaptureKit)
            if #available(iOS 27.0, *), SCContentSharingPicker.shared.isAvailable {
                return SystemScreenCapturer()
            }
        #endif
        return BroadcastScreenCapturer()
    }

    /// Asks the system for the screen for `account`'s engine. `onStarted` is called with the
    /// video track and, with `audio`, the audio track once the capture runs, which is when the
    /// engine is given them; `onEnded` when it is over, with why when it did not simply stop.
    /// A share already asked for is left as it is.
    func start(
        account: CallAccount, audio: Bool, onStarted: @escaping (TrackId, TrackId?) -> Void,
        onEnded: @escaping (CallScreenFailure?) -> Void
    ) {
        guard capturer == nil else {
            return
        }
        self.account = account
        wantsAudio = audio
        self.onStarted = onStarted
        self.onEnded = onEnded
        let capturer = makeCapturer()
        self.capturer = capturer
        capturer.start(audio: audio, sink: self)
    }

    /// Ends the share and lets its tracks go.
    func stop() {
        let capturer = self.capturer
        release()
        capturer?.stop()
    }

    private func release() {
        if let videoTrack {
            tracks.remove(videoTrack.trackId)
        }
        if let audioTrack {
            tracks.remove(audioTrack.trackId)
        }
        videoTrack = nil
        audioTrack = nil
        source = nil
        shareAudio = nil
        capturer = nil
        account = nil
        onStarted = nil
        onEnded = nil
    }

    // MARK: CallScreenSink

    func captureStarted() {
        DispatchQueue.main.async { [self] in
            guard capturer != nil, videoTrack == nil else {
                return
            }
            let media = media()
            let factory = media.captureFactory
            shareAudio = media.shareAudio
            let source = factory.videoSource(forScreenCast: true)
            let video = factory.videoTrack(with: source, trackId: "share-\(UUID().uuidString.lowercased())")
            self.source = source
            videoTrack = video
            tracks.add(video)
            var audio: RTCAudioTrack?
            if wantsAudio {
                let made = factory.audioTrack(
                    with: factory.audioSource(with: shareAudioConstraints()),
                    trackId: "share-audio-\(UUID().uuidString.lowercased())")
                audioTrack = made
                tracks.add(made)
                audio = made
            }
            onStarted?(video.trackId, audio?.trackId)
        }
    }

    func capture(didCapture frame: RTCVideoFrame) {
        source?.capturer(frameSource, didCapture: frame)
    }

    func capture(didCaptureAudio sampleBuffer: CMSampleBuffer) {
        shareAudio?.deliver(sampleBuffer)
    }

    func captureEnded(_ failure: CallScreenFailure?) {
        DispatchQueue.main.async { [self] in
            guard capturer != nil else {
                return
            }
            let ended = onEnded
            release()
            ended?(failure)
        }
    }
}

#if canImport(ScreenCaptureKit)
    /// The screen through ScreenCaptureKit, in this process, on iOS 27 and later: the system's
    /// content sharing picker is the consent, in which the user chooses the whole screen (or
    /// this app alone) and may later change or stop it from the system's own control; the stream
    /// it answers with is captured at the display's own size, no larger than
    /// `BroadcastChannel.maxWidth` on a side, with the app audio of every other app (this one's
    /// own sound, the call, is left out) at the format `ShareAudioDevice` takes. The capture goes
    /// on with the app in the background under the `screen-capture` background mode
    /// (`Info.plist`); without it the system stops the stream with `missingBackgroundMode`.
    @available(iOS 27.0, *)
    final class SystemScreenCapturer: NSObject, CallScreenCapturer, SCContentSharingPickerObserver, SCStreamOutput,
        SCStreamDelegate
    {
        private let queue = DispatchQueue(label: "aspen-screen-capture")
        private weak var sink: CallScreenSink?
        private var stream: SCStream?
        private var audio = false
        private var observing = false

        func start(audio: Bool, sink: CallScreenSink) {
            self.sink = sink
            self.audio = audio
            let picker = SCContentSharingPicker.shared
            var configuration = SCContentSharingPickerConfiguration()
            configuration.showsMicrophoneControl = false
            configuration.showsCameraControl = false
            picker.defaultConfiguration = configuration
            picker.add(self)
            observing = true
            picker.isActive = true
            picker.present()
        }

        func stop() {
            let stream = self.stream
            self.stream = nil
            stream?.stopCapture { _ in }
            endPicker()
        }

        private func endPicker() {
            let picker = SCContentSharingPicker.shared
            if observing {
                picker.remove(self)
                observing = false
            }
            picker.isActive = false
        }

        // MARK: SCContentSharingPickerObserver

        func contentSharingPicker(_ picker: SCContentSharingPicker, didCancelFor stream: SCStream?) {
            guard self.stream == nil else {
                return
            }
            endPicker()
            sink?.captureEnded(.denied)
        }

        func contentSharingPicker(_ picker: SCContentSharingPicker, didUpdateWith filter: SCContentFilter, for stream: SCStream?) {
            guard self.stream == nil else {
                return
            }
            let configuration = SCStreamConfiguration()
            let scale = CGFloat(filter.pointPixelScale)
            var width = Int(filter.contentRect.width * scale)
            var height = Int(filter.contentRect.height * scale)
            let longest = max(width, height)
            if longest > BroadcastChannel.maxWidth, longest > 0 {
                let ratio = Double(BroadcastChannel.maxWidth) / Double(longest)
                width = Int(Double(width) * ratio) & ~1
                height = Int(Double(height) * ratio) & ~1
            }
            if width > 0, height > 0 {
                configuration.width = width
                configuration.height = height
            }
            configuration.capturesAudio = audio
            configuration.excludesCurrentProcessAudio = true
            configuration.sampleRate = Int(ShareAudioDevice.rate)
            configuration.channelCount = ShareAudioDevice.channels
            let made = SCStream(filter: filter, configuration: configuration, delegate: self)
            do {
                try made.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
                if audio {
                    try made.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
                }
            } catch {
                NSLog("Aspen could not add the screen stream's outputs: %@", String(describing: error))
                endPicker()
                sink?.captureEnded(.failed)
                return
            }
            self.stream = made
            made.startCapture { [weak self] error in
                guard let self else {
                    return
                }
                if let error {
                    NSLog("Aspen could not start the screen stream: %@", String(describing: error))
                    self.stream = nil
                    self.endPicker()
                    self.sink?.captureEnded(.failed)
                } else {
                    self.sink?.captureStarted()
                }
            }
        }

        func contentSharingPickerStartDidFailWithError(_ error: Error) {
            NSLog("Aspen could not show the screen picker: %@", String(describing: error))
            endPicker()
            sink?.captureEnded(.failed)
        }

        // MARK: SCStreamOutput

        func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
            switch type {
            case .screen:
                guard CMSampleBufferIsValid(sampleBuffer), let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else {
                    return
                }
                // A frame with no new picture (idle, blank, or a status change) draws nothing.
                if let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false)
                    as? [[SCStreamFrameInfo: Any]],
                    let status = attachments.first?[.status] as? Int, status != SCFrameStatus.complete.rawValue
                {
                    return
                }
                let presented = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
                let timestampNs = presented.isNumeric ? Int64(CMTimeGetSeconds(presented) * 1_000_000_000) : 0
                sink?.capture(
                    didCapture: RTCVideoFrame(
                        buffer: RTCCVPixelBuffer(pixelBuffer: pixelBuffer), rotation: ._0, timeStampNs: timestampNs))
            case .audio:
                sink?.capture(didCaptureAudio: sampleBuffer)
            case .microphone:
                break
            @unknown default:
                break
            }
        }

        // MARK: SCStreamDelegate

        func stream(_ stream: SCStream, didStopWithError error: Error) {
            guard self.stream === stream else {
                return
            }
            self.stream = nil
            endPicker()
            let code = (error as NSError).code
            let byUser = code == SCStreamError.Code.userStopped.rawValue || code == SCStreamError.Code.systemStoppedStream.rawValue
            if !byUser {
                NSLog("Aspen's screen stream stopped: %@", String(describing: error))
            }
            sink?.captureEnded(byUser ? nil : .failed)
        }
    }
#endif

/// The screen through the broadcast upload extension (`AspenBroadcast`), on iOS before 27 or
/// where ScreenCaptureKit's picker is not available: the system's broadcast picker is the
/// consent, shown pointed at the extension, in which the user starts the broadcast; the
/// extension then connects over `BroadcastChannel` and its frames and app audio arrive from the
/// shared ring. The picker tells nothing of a user who dismisses it, so a broadcast that has not
/// connected within `pickerTimeout` is taken as declined. Stopping sends the extension `stop`;
/// the broadcast ending from the system's control closes the socket, which ends the share.
final class BroadcastScreenCapturer: CallScreenCapturer, BroadcastListenerDelegate {
    static let pickerTimeout: TimeInterval = 60
    private let listener = BroadcastListener()
    private weak var sink: CallScreenSink?
    private var connected = false
    private var pickerView: RPSystemBroadcastPickerView?
    private var timeout: DispatchWorkItem?

    func start(audio: Bool, sink: CallScreenSink) {
        self.sink = sink
        guard listener.start(delegate: self) else {
            sink.captureEnded(.unsupported)
            return
        }
        let picker = RPSystemBroadcastPickerView(frame: CGRect(x: 0, y: 0, width: 1, height: 1))
        picker.preferredExtension = BroadcastChannel.extensionBundleId
        picker.showsMicrophoneButton = false
        picker.isHidden = true
        let window = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first(where: \.isKeyWindow)
        window?.addSubview(picker)
        pickerView = picker
        // The picker is one button; the system's sheet opens on its action.
        for case let button as UIButton in picker.subviews {
            button.sendActions(for: .touchUpInside)
        }
        let timeout = DispatchWorkItem { [weak self] in
            guard let self, !connected else {
                return
            }
            finish()
            self.sink?.captureEnded(.denied)
        }
        self.timeout = timeout
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.pickerTimeout, execute: timeout)
    }

    func stop() {
        listener.stop()
        finish()
    }

    private func finish() {
        timeout?.cancel()
        timeout = nil
        pickerView?.removeFromSuperview()
        pickerView = nil
        listener.close()
    }

    // MARK: BroadcastListenerDelegate

    func broadcastConnected() {
        DispatchQueue.main.async { [self] in
            connected = true
            timeout?.cancel()
            timeout = nil
            sink?.captureStarted()
        }
    }

    func broadcast(frame pixelBuffer: CVPixelBuffer, rotation: Int, timestampNs: Int64) {
        let turned: RTCVideoRotation
        switch rotation {
        case 90: turned = ._90
        case 180: turned = ._180
        case 270: turned = ._270
        default: turned = ._0
        }
        sink?.capture(
            didCapture: RTCVideoFrame(buffer: RTCCVPixelBuffer(pixelBuffer: pixelBuffer), rotation: turned, timeStampNs: timestampNs))
    }

    func broadcast(audio sampleBuffer: CMSampleBuffer) {
        sink?.capture(didCaptureAudio: sampleBuffer)
    }

    func broadcastEnded() {
        DispatchQueue.main.async { [self] in
            let wasConnected = connected
            connected = false
            finish()
            sink?.captureEnded(wasConnected ? nil : .failed)
        }
    }
}
