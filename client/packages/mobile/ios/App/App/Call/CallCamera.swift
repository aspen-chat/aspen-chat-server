import AVFoundation
import Foundation
import WebRTC

/// A camera the phone has, as the system lists it: its id (`AVCaptureDevice.uniqueID`), its
/// name for display alone, and which way it faces as the system says (`PhoneCamera` in the
/// app's `nativeCall.ts`).
struct CallCameraDevice: Equatable {
    enum Facing: String {
        case front
        case back
    }

    let id: String
    let label: String?
    let facing: Facing?
}

/// Why the camera could not be opened, as the page's `CameraFailure` names it and the plugin
/// rejects with it.
enum CallCameraFailure: String, Error {
    /// The system refused the camera.
    case denied
    /// The phone has no camera, or not the one asked for.
    case none
    /// The camera would not start.
    case failed
}

/// What captures the camera's frames: the phone's cameras through `RTCCameraVideoCapturer`
/// (`DeviceCameraCapturer`), or, in a test on the simulator, which has no camera, frames drawn
/// in software. Frames go to `delegate`, the `RTCVideoSource` the engine's track is made of.
protocol CallCameraCapturer: AnyObject {
    /// The cameras this capturer can open.
    var devices: [CallCameraDevice] { get }
    /// Starts capturing `device`, or the capturer's default (a phone's front camera) for `nil`,
    /// into `delegate`; answers the camera opened, or why none was. A capturer already running
    /// moves to the device, with no gap in the frames but the camera's own.
    func start(
        device: CallCameraDevice?, delegate: RTCVideoCapturerDelegate,
        completion: @escaping (Result<CallCameraDevice, CallCameraFailure>) -> Void)
    func stop(completion: @escaping () -> Void)
}

/// The phone's cameras through WebRTC's capturer. The format chosen is the largest no wider or
/// taller than `maxWidth` by `maxHeight` in the capturer's preferred pixel format, at the most
/// frames a second it allows up to `maxFps`, which bounds what the encoder is given; the engine's
/// encoding bounds what is sent. Where the system allows it, the capture session is told the
/// camera is used while the app is not in front (multitasking camera access, which an app with
/// the VoIP background mode may ask), so a call's camera stays on in a picture in picture.
final class DeviceCameraCapturer: CallCameraCapturer {
    static let maxWidth: Int32 = 1280
    static let maxHeight: Int32 = 720
    static let maxFps = 30
    private var capturer: RTCCameraVideoCapturer?

    var devices: [CallCameraDevice] {
        RTCCameraVideoCapturer.captureDevices().map(Self.device)
    }

    static func device(_ device: AVCaptureDevice) -> CallCameraDevice {
        CallCameraDevice(id: device.uniqueID, label: device.localizedName, facing: facing(device.position))
    }

    static func facing(_ position: AVCaptureDevice.Position) -> CallCameraDevice.Facing? {
        switch position {
        case .front: return .front
        case .back: return .back
        case .unspecified: return nil
        @unknown default: return nil
        }
    }

    /// The format to capture `device` in and its frame rate (`maxWidth`, `maxHeight`, `maxFps`).
    static func format(for device: AVCaptureDevice, preferring pixelFormat: FourCharCode) -> (AVCaptureDevice.Format, Int)? {
        let formats = RTCCameraVideoCapturer.supportedFormats(for: device)
        let fitting = formats.filter { format in
            let size = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
            return size.width <= maxWidth && size.height <= maxHeight
        }
        let candidates = fitting.isEmpty ? formats : fitting
        let chosen = candidates.max { a, b in
            let sizeA = CMVideoFormatDescriptionGetDimensions(a.formatDescription)
            let sizeB = CMVideoFormatDescriptionGetDimensions(b.formatDescription)
            let areaA = Int(sizeA.width) * Int(sizeA.height)
            let areaB = Int(sizeB.width) * Int(sizeB.height)
            if areaA != areaB {
                return areaA < areaB
            }
            let preferredA = CMFormatDescriptionGetMediaSubType(a.formatDescription) == pixelFormat
            let preferredB = CMFormatDescriptionGetMediaSubType(b.formatDescription) == pixelFormat
            return !preferredA && preferredB
        }
        guard let chosen else {
            return nil
        }
        let fastest = chosen.videoSupportedFrameRateRanges.map { Int($0.maxFrameRate) }.max() ?? maxFps
        return (chosen, min(fastest, maxFps))
    }

    func start(
        device: CallCameraDevice?, delegate: RTCVideoCapturerDelegate,
        completion: @escaping (Result<CallCameraDevice, CallCameraFailure>) -> Void
    ) {
        let available = RTCCameraVideoCapturer.captureDevices()
        let chosen: AVCaptureDevice?
        if let device {
            chosen = available.first { $0.uniqueID == device.id }
        } else {
            chosen = available.first { $0.position == .front } ?? available.first
        }
        guard let chosen else {
            completion(.failure(.none))
            return
        }
        let capturer = self.capturer ?? RTCCameraVideoCapturer(delegate: delegate)
        self.capturer = capturer
        capturer.delegate = delegate
        guard let formatChosen = Self.format(for: chosen, preferring: capturer.preferredOutputPixelFormat()) else {
            completion(.failure(.failed))
            return
        }
        let (format, fps) = formatChosen
        let session = capturer.captureSession
        if #available(iOS 16.0, *), session.isMultitaskingCameraAccessSupported, !session.isMultitaskingCameraAccessEnabled {
            session.beginConfiguration()
            session.isMultitaskingCameraAccessEnabled = true
            session.commitConfiguration()
        }
        capturer.startCapture(with: chosen, format: format, fps: fps) { error in
            if let error {
                NSLog("Aspen could not start the camera %@: %@", chosen.localizedName, String(describing: error))
                completion(.failure(.failed))
            } else {
                completion(.success(Self.device(chosen)))
            }
        }
    }

    func stop(completion: @escaping () -> Void) {
        guard let capturer else {
            completion()
            return
        }
        self.capturer = nil
        capturer.stopCapture(completionHandler: completion)
    }
}

/// The call's camera on this device: one capturer and one track for the process, opened through
/// `start` as the engine's `camera` source and kept until `stop`, moved between cameras by
/// `switchTo` with the track unchanged, so the engine's sender needs nothing. The frames go to
/// the track's `RTCVideoSource`, which WebRTC adapts and encodes; the self preview is drawn from
/// the same track (`CallVideo`), with the camera's own rotation. `makeCapturer` is where a test
/// on the simulator, which has no camera, puts a capturer of its own. Call on the main queue;
/// completions arrive there.
final class CallCamera {
    private let factory: RTCPeerConnectionFactory
    private let tracks: CallTracks
    var makeCapturer: () -> CallCameraCapturer = { DeviceCameraCapturer() }
    private var capturer: CallCameraCapturer?
    private var source: RTCVideoSource?
    private(set) var track: RTCVideoTrack?
    /// The camera open now.
    private(set) var device: CallCameraDevice?
    /// The account whose engine was given the track (`startSource`), while the camera is on.
    private(set) var account: CallAccount?

    init(factory: RTCPeerConnectionFactory, tracks: CallTracks) {
        self.factory = factory
        self.tracks = tracks
    }

    /// The cameras the phone has, listed without asking the user, who is asked as one opens.
    var devices: [CallCameraDevice] {
        (capturer ?? makeCapturer()).devices
    }

    var isOn: Bool {
        track != nil
    }

    /// Asks the system for the camera; granted, or refused, once per install, and answered at
    /// once after.
    static func requestAccess(_ answer: @escaping (Bool) -> Void) {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            answer(true)
        case .denied, .restricted:
            answer(false)
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { granted in
                DispatchQueue.main.async { answer(granted) }
            }
        @unknown default:
            answer(false)
        }
    }

    /// Opens the camera `device` names (the phone's default for `nil`) for `account`'s engine,
    /// answering the track to send and the camera opened. A camera already on is moved instead.
    func start(
        device id: String?, account: CallAccount,
        completion: @escaping (Result<(track: TrackId, device: CallCameraDevice), CallCameraFailure>) -> Void
    ) {
        let capturer = self.capturer ?? makeCapturer()
        self.capturer = capturer
        let wanted = id.flatMap { id in capturer.devices.first { $0.id == id } }
        if id != nil, wanted == nil {
            completion(.failure(.none))
            return
        }
        let source = self.source ?? factory.videoSource()
        self.source = source
        let track = self.track ?? factory.videoTrack(with: source, trackId: "camera-\(UUID().uuidString.lowercased())")
        if self.track == nil {
            self.track = track
            tracks.add(track)
        }
        self.account = account
        capturer.start(device: wanted, delegate: source) { [weak self] result in
            DispatchQueue.main.async {
                guard let self else {
                    return
                }
                switch result {
                case .success(let opened):
                    self.device = opened
                    completion(.success((track.trackId, opened)))
                case .failure(let failure):
                    // Nothing captures: the track is let go, so the engine is given none.
                    if self.device == nil {
                        self.release()
                    }
                    completion(.failure(failure))
                }
            }
        }
    }

    /// Stops the camera and lets its track go. `completion` runs once the capturer has stopped.
    func stop(completion: @escaping () -> Void = {}) {
        guard let capturer else {
            release()
            completion()
            return
        }
        release()
        capturer.stop {
            DispatchQueue.main.async(execute: completion)
        }
    }

    private func release() {
        if let track {
            tracks.remove(track.trackId)
        }
        track = nil
        source = nil
        device = nil
        account = nil
        capturer = nil
    }
}
