import AVFoundation
import CoreMedia
import Foundation
import WebRTC

/// The audio device of the share's WebRTC factory (`CallPeerConnections.captureFactory`): a
/// microphone in software whose sound is whatever the screen capture hands it (`deliver`), the
/// app audio ScreenCaptureKit or the broadcast extension gives with the picture. A WebRTC factory
/// has one audio capture for every connection it makes, so the share's sound, a second capture
/// beside the microphone, needs a factory of its own; this is what that factory records from.
/// Every buffer is converted to the one format WebRTC is told of (16-bit, stereo, 48 kHz,
/// interleaved) and handed on in the buffer's own length, which WebRTC cuts into its ten
/// millisecond frames itself. It is handed on through the delegate's render block, which fills
/// the buffer WebRTC sizes for the frames and channels, rather than as a pre-filled buffer list,
/// whose samples WebRTC's device module counts as frames and so reads half of when stereo.
/// Nothing plays out: the share's connection receives nothing. Delivery runs on one queue, as
/// the protocol asks of a device that is not interrupted.
final class ShareAudioDevice: NSObject, RTCAudioDevice {
    static let rate = 48000.0
    static let channels = 2
    static let format = AVAudioFormat(
        commonFormat: .pcmFormatInt16, sampleRate: rate, channels: AVAudioChannelCount(channels), interleaved: true)!

    private let queue = DispatchQueue(label: "aspen-share-audio")
    private var delegate: RTCAudioDeviceDelegate?
    private var converter: AVAudioConverter?
    private var converterInput: AVAudioFormat?
    private var sampleTime = 0.0
    /// How many frames have been handed to WebRTC, and how many buffers were dropped and the
    /// last reason, for the proof's diagnosis.
    private(set) var deliveredFrames = 0
    private(set) var droppedBuffers = 0
    private(set) var lastDrop = ""

    private(set) var isInitialized = false
    private(set) var isPlayoutInitialized = false
    private(set) var isPlaying = false
    private(set) var isRecordingInitialized = false
    private(set) var isRecording = false

    var deviceInputSampleRate: Double { Self.rate }
    var inputIOBufferDuration: TimeInterval { 0.01 }
    var inputNumberOfChannels: Int { Self.channels }
    var inputLatency: TimeInterval { 0 }
    var deviceOutputSampleRate: Double { Self.rate }
    var outputIOBufferDuration: TimeInterval { 0.01 }
    var outputNumberOfChannels: Int { Self.channels }
    var outputLatency: TimeInterval { 0 }

    func initialize(with delegate: RTCAudioDeviceDelegate) -> Bool {
        queue.sync { self.delegate = delegate }
        isInitialized = true
        return true
    }

    func terminateDevice() -> Bool {
        _ = stopRecording()
        _ = stopPlayout()
        queue.sync { delegate = nil }
        isInitialized = false
        return true
    }

    func initializePlayout() -> Bool {
        isPlayoutInitialized = true
        return true
    }

    func startPlayout() -> Bool {
        isPlaying = true
        return true
    }

    func stopPlayout() -> Bool {
        isPlaying = false
        return true
    }

    func initializeRecording() -> Bool {
        isRecordingInitialized = true
        return true
    }

    func startRecording() -> Bool {
        queue.sync { isRecording = true }
        return true
    }

    func stopRecording() -> Bool {
        queue.sync { isRecording = false }
        return true
    }

    /// Hands the capture's audio, in whatever PCM format it came, to WebRTC as recorded sound.
    /// Dropped while nothing records, as before the share's connection sends.
    func deliver(_ sampleBuffer: CMSampleBuffer) {
        queue.async { [self] in
            guard isRecording, let delegate else {
                drop("not recording")
                return
            }
            guard let description = CMSampleBufferGetFormatDescription(sampleBuffer),
                  let streamDescription = CMAudioFormatDescriptionGetStreamBasicDescription(description),
                  let input = AVAudioFormat(streamDescription: streamDescription)
            else {
                drop("no PCM format")
                return
            }
            let frames = AVAudioFrameCount(CMSampleBufferGetNumSamples(sampleBuffer))
            guard frames > 0, let inBuffer = AVAudioPCMBuffer(pcmFormat: input, frameCapacity: frames) else {
                drop("no frames")
                return
            }
            inBuffer.frameLength = frames
            let copied = CMSampleBufferCopyPCMDataIntoAudioBufferList(
                sampleBuffer, at: 0, frameCount: Int32(frames), into: inBuffer.mutableAudioBufferList)
            guard copied == noErr else {
                drop("copy failed \(copied)")
                return
            }
            let converted: AVAudioPCMBuffer
            if input == Self.format {
                converted = inBuffer
            } else {
                if converterInput != input {
                    converter = AVAudioConverter(from: input, to: Self.format)
                    converterInput = input
                }
                guard let converter else {
                    drop("no converter from \(input)")
                    return
                }
                let capacity = AVAudioFrameCount(Double(frames) * Self.rate / input.sampleRate) + 16
                guard let out = AVAudioPCMBuffer(pcmFormat: Self.format, frameCapacity: capacity) else {
                    drop("no output buffer")
                    return
                }
                var consumed = false
                var error: NSError?
                let status = converter.convert(to: out, error: &error) { _, status in
                    if consumed {
                        status.pointee = .noDataNow
                        return nil
                    }
                    consumed = true
                    status.pointee = .haveData
                    return inBuffer
                }
                if let error {
                    drop("conversion failed: \(error)")
                    return
                }
                if out.frameLength == 0 {
                    drop("conversion gave no frames (\(status.rawValue))")
                    return
                }
                converted = out
            }
            var timestamp = AudioTimeStamp()
            timestamp.mSampleTime = sampleTime
            timestamp.mHostTime = mach_absolute_time()
            timestamp.mFlags = [.sampleTimeValid, .hostTimeValid]
            sampleTime += Double(converted.frameLength)
            var flags = AudioUnitRenderActionFlags()
            let frameCount = UInt32(converted.frameLength)
            let source = converted.mutableAudioBufferList.pointee.mBuffers
            let result = delegate.deliverRecordedData(&flags, &timestamp, 1, frameCount, nil, nil) { _, _, _, count, into, _ in
                // WebRTC sized `into` for `count` frames of the device's channels; the samples
                // are what was converted, interleaved as WebRTC takes them.
                let target = into.pointee.mBuffers
                let bytes = min(Int(target.mDataByteSize), Int(source.mDataByteSize), Int(count) * Self.channels * 2)
                if let to = target.mData, let from = source.mData {
                    memcpy(to, from, bytes)
                }
                return noErr
            }
            if result == noErr {
                deliveredFrames += Int(frameCount)
            } else {
                drop("WebRTC refused the frames (\(result))")
            }
        }
    }

    private func drop(_ reason: String) {
        droppedBuffers += 1
        lastDrop = reason
    }
}

/// The constraints of the share's audio source: the sound is an application's, not a voice in
/// a room, so none of WebRTC's voice processing (echo cancellation, noise suppression, gain
/// control, the high-pass filter) is applied over it.
func shareAudioConstraints() -> RTCMediaConstraints {
    RTCMediaConstraints(
        mandatoryConstraints: [
            "googEchoCancellation": "false",
            "googNoiseSuppression": "false",
            "googAutoGainControl": "false",
            "googHighpassFilter": "false",
        ],
        optionalConstraints: nil)
}
