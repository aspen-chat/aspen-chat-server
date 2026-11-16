import CoreVideo
import Foundation
import WebRTC
@testable import App

/// A camera in software for the simulator, which has none (`CallCameraCapturer`): two cameras,
/// one facing each way, and fifteen frames a second of moving stripes, each a 320 by 240 NV12
/// buffer carried with the rotation a phone held upright gives its camera's frames (90 degrees),
/// so what is sent is turned upright by the sender, as a phone's camera is, and the self preview
/// is drawn turned. The stripes move, so the encoder has something to send in every frame.
final class SyntheticVideoCapturer: CallCameraCapturer {
    static let front = CallCameraDevice(id: "synthetic-front", label: "Synthetic front camera", facing: .front)
    static let back = CallCameraDevice(id: "synthetic-back", label: "Synthetic back camera", facing: .back)
    static let width = 320
    static let height = 240
    static let fps = 15

    private let queue = DispatchQueue(label: "synthetic-camera")
    private var timer: DispatchSourceTimer?
    private var frames = 0
    private var pool: CVPixelBufferPool?
    /// WebRTC's delegate is told which capturer the frame came from; this stands for it.
    private let capturer = RTCVideoCapturer()

    var devices: [CallCameraDevice] {
        [Self.front, Self.back]
    }

    private(set) var running: CallCameraDevice?

    func start(
        device: CallCameraDevice?, delegate: RTCVideoCapturerDelegate,
        completion: @escaping (Result<CallCameraDevice, CallCameraFailure>) -> Void
    ) {
        let chosen = device ?? Self.front
        guard devices.contains(chosen) else {
            completion(.failure(.none))
            return
        }
        queue.async { [self] in
            timer?.cancel()
            running = chosen
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now(), repeating: 1.0 / Double(Self.fps))
            timer.setEventHandler { [weak self, weak delegate] in
                guard let self, let frame = self.nextFrame() else {
                    return
                }
                delegate?.capturer(self.capturer, didCapture: frame)
            }
            timer.resume()
            self.timer = timer
            completion(.success(chosen))
        }
    }

    func stop(completion: @escaping () -> Void) {
        queue.async { [self] in
            timer?.cancel()
            timer = nil
            running = nil
            completion()
        }
    }

    private func nextFrame() -> RTCVideoFrame? {
        if pool == nil {
            let attributes: [CFString: Any] = [
                kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                kCVPixelBufferWidthKey: Self.width,
                kCVPixelBufferHeightKey: Self.height,
                kCVPixelBufferIOSurfacePropertiesKey: [:] as [CFString: Any],
            ]
            CVPixelBufferPoolCreate(kCFAllocatorDefault, nil, attributes as CFDictionary, &pool)
        }
        guard let pool else {
            return nil
        }
        var buffer: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, pool, &buffer) == kCVReturnSuccess, let buffer else {
            return nil
        }
        frames += 1
        CVPixelBufferLockBaseAddress(buffer, [])
        if let y = CVPixelBufferGetBaseAddressOfPlane(buffer, 0) {
            let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)
            for row in 0..<Self.height {
                let line = (y + row * stride).assumingMemoryBound(to: UInt8.self)
                for x in 0..<Self.width {
                    // Stripes forty pixels wide, between dark and light grey, moving four pixels a frame.
                    line[x] = ((x + frames * 4) / 40) % 2 == 0 ? 60 : 200
                }
            }
        }
        if let uv = CVPixelBufferGetBaseAddressOfPlane(buffer, 1) {
            let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 1)
            for row in 0..<(Self.height / 2) {
                memset(uv + row * stride, 128, Self.width)
            }
        }
        CVPixelBufferUnlockBaseAddress(buffer, [])
        let timeStampNs = Int64(frames) * Int64(1_000_000_000 / Self.fps)
        return RTCVideoFrame(buffer: RTCCVPixelBuffer(pixelBuffer: buffer), rotation: ._90, timeStampNs: timeStampNs)
    }
}
