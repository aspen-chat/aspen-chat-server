import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import WebRTC
@testable import App

/// A screen capture in software for the simulator, which has neither ScreenCaptureKit nor the
/// broadcast extension's picker (`CallScreenCapturer`): consents at once, then fifteen frames a
/// second of a moving checkerboard, each a 640 by 360 NV12 buffer upright (a screen's frames
/// are), and, when audio is asked for, the app audio ReplayKit would give, as 1024-frame buffers
/// of a 660 Hz tone, 16-bit stereo at 44.1 kHz (a format `ShareAudioDevice` must convert), every
/// 23 ms. A `declining` capturer stands for the user dismissing the system's picker.
final class SyntheticScreenCapturer: CallScreenCapturer {
    static let width = 640
    static let height = 360
    static let fps = 15
    static let audioRate = 44100.0
    static let audioFrames = 1024

    private let queue = DispatchQueue(label: "synthetic-screen")
    private var timer: DispatchSourceTimer?
    private var audioTimer: DispatchSourceTimer?
    private var frames = 0
    private var phase = 0.0
    private var audioSamples = 0
    private var pool: CVPixelBufferPool?
    private var audioFormat: CMAudioFormatDescription?
    private weak var sink: CallScreenSink?
    let declining: Bool
    private(set) var running = false
    private(set) var framesSent = 0
    private(set) var audioBuffersSent = 0

    init(declining: Bool = false) {
        self.declining = declining
    }

    func start(audio: Bool, sink: CallScreenSink) {
        self.sink = sink
        if declining {
            sink.captureEnded(.denied)
            return
        }
        queue.async { [self] in
            running = true
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now(), repeating: 1.0 / Double(Self.fps))
            timer.setEventHandler { [weak self] in
                guard let self, let frame = self.nextFrame() else {
                    return
                }
                self.framesSent += 1
                self.sink?.capture(didCapture: frame)
            }
            timer.resume()
            self.timer = timer
            if audio {
                let audioTimer = DispatchSource.makeTimerSource(queue: queue)
                audioTimer.schedule(
                    deadline: .now(), repeating: .milliseconds(Int(1000.0 * Double(Self.audioFrames) / Self.audioRate)))
                audioTimer.setEventHandler { [weak self] in
                    guard let self, let buffer = self.nextAudio() else {
                        return
                    }
                    self.audioBuffersSent += 1
                    self.sink?.capture(didCaptureAudio: buffer)
                }
                audioTimer.resume()
                self.audioTimer = audioTimer
            }
            sink.captureStarted()
        }
    }

    func stop() {
        queue.async { [self] in
            timer?.cancel()
            timer = nil
            audioTimer?.cancel()
            audioTimer = nil
            running = false
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
                    // Checks sixty pixels on a side, between dark and light grey, moving three
                    // pixels a frame.
                    let check = ((x + frames * 3) / 60 + row / 60) % 2 == 0
                    line[x] = check ? 50 : 210
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
        return RTCVideoFrame(buffer: RTCCVPixelBuffer(pixelBuffer: buffer), rotation: ._0, timeStampNs: timeStampNs)
    }

    private func nextAudio() -> CMSampleBuffer? {
        if audioFormat == nil {
            var description = AudioStreamBasicDescription(
                mSampleRate: Self.audioRate, mFormatID: kAudioFormatLinearPCM,
                mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked, mBytesPerPacket: 4,
                mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 2, mBitsPerChannel: 16, mReserved: 0)
            CMAudioFormatDescriptionCreate(
                allocator: kCFAllocatorDefault, asbd: &description, layoutSize: 0, layout: nil, magicCookieSize: 0,
                magicCookie: nil, extensions: nil, formatDescriptionOut: &audioFormat)
        }
        guard let audioFormat else {
            return nil
        }
        var samples = [Int16](repeating: 0, count: Self.audioFrames * 2)
        for frame in 0..<Self.audioFrames {
            let value = Int16(0.3 * 32767 * sin(phase))
            samples[frame * 2] = value
            samples[frame * 2 + 1] = value
            phase += 2 * .pi * 660 / Self.audioRate
            if phase > 2 * .pi {
                phase -= 2 * .pi
            }
        }
        let bytes = samples.count * 2
        var block: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(
            allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: bytes, blockAllocator: kCFAllocatorDefault,
            customBlockSource: nil, offsetToData: 0, dataLength: bytes, flags: 0, blockBufferOut: &block) == noErr,
            let block
        else {
            return nil
        }
        samples.withUnsafeBytes { raw in
            _ = CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: bytes)
        }
        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1, timescale: CMTimeScale(Self.audioRate)),
            presentationTimeStamp: CMTime(value: CMTimeValue(audioSamples), timescale: CMTimeScale(Self.audioRate)),
            decodeTimeStamp: .invalid)
        audioSamples += Self.audioFrames
        var sampleBuffer: CMSampleBuffer?
        guard CMSampleBufferCreate(
            allocator: kCFAllocatorDefault, dataBuffer: block, dataReady: true, makeDataReadyCallback: nil, refcon: nil,
            formatDescription: audioFormat, sampleCount: Self.audioFrames, sampleTimingEntryCount: 1,
            sampleTimingArray: &timing, sampleSizeEntryCount: 0, sampleSizeArray: nil, sampleBufferOut: &sampleBuffer) == noErr
        else {
            return nil
        }
        return sampleBuffer
    }
}
