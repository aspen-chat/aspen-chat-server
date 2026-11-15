import AudioToolbox
import Foundation
import WebRTC

/// A microphone and speaker in software for WebRTC (`RTCAudioDevice`): every ten milliseconds
/// it records a warble, `frequency` and the fifth above it in turn with a gap between, and plays
/// out what the call decoded, into a buffer it drops. A warble rather than one steady tone, which
/// WebRTC's noise suppression takes for noise and its encoder's silence detection then stops
/// sending. The simulator's own audio unit cannot be initialized from a test (its
/// `AudioUnitInitialize` aborts the process on an RPC timeout), so the call engine's proof runs
/// on this instead of WebRTC's audio device module, which a device check keeps for the built-in
/// module. 16-bit mono at 48 kHz, the format the delegate expects.
final class ToneAudioDevice: NSObject, RTCAudioDevice {
    private static let rate = 48000.0
    private static let frames: UInt32 = 480
    /// Each note sounds for this long, then rests as long again.
    private static let note = 0.15
    private let frequency: Double
    private let amplitude: Double
    private var delegate: RTCAudioDeviceDelegate?
    private let queue = DispatchQueue(label: "tone-audio-device")
    private var timer: DispatchSourceTimer?
    private var phase = 0.0
    private var sampleTime = 0.0
    private var recorded = [Int16](repeating: 0, count: 480)
    private var played = [Int16](repeating: 0, count: 480)

    private(set) var isInitialized = false
    private(set) var isPlayoutInitialized = false
    private(set) var isPlaying = false
    private(set) var isRecordingInitialized = false
    private(set) var isRecording = false

    init(frequency: Double = 440, amplitude: Double = 0.3) {
        self.frequency = frequency
        self.amplitude = amplitude
    }

    var deviceInputSampleRate: Double { Self.rate }
    var inputIOBufferDuration: TimeInterval { 0.01 }
    var inputNumberOfChannels: Int { 1 }
    var inputLatency: TimeInterval { 0 }
    var deviceOutputSampleRate: Double { Self.rate }
    var outputIOBufferDuration: TimeInterval { 0.01 }
    var outputNumberOfChannels: Int { 1 }
    var outputLatency: TimeInterval { 0 }

    func initialize(with delegate: RTCAudioDeviceDelegate) -> Bool {
        self.delegate = delegate
        isInitialized = true
        return true
    }

    func terminateDevice() -> Bool {
        _ = stopRecording()
        _ = stopPlayout()
        delegate = nil
        isInitialized = false
        return true
    }

    func initializePlayout() -> Bool {
        isPlayoutInitialized = true
        return true
    }

    func startPlayout() -> Bool {
        isPlaying = true
        tick()
        return true
    }

    func stopPlayout() -> Bool {
        isPlaying = false
        tick()
        return true
    }

    func initializeRecording() -> Bool {
        isRecordingInitialized = true
        return true
    }

    func startRecording() -> Bool {
        isRecording = true
        tick()
        return true
    }

    func stopRecording() -> Bool {
        isRecording = false
        tick()
        return true
    }

    /// Runs the ten-millisecond clock while anything plays or records.
    private func tick() {
        queue.sync {
            if isPlaying || isRecording {
                if timer == nil {
                    let timer = DispatchSource.makeTimerSource(queue: queue)
                    timer.schedule(deadline: .now(), repeating: .milliseconds(10), leeway: .milliseconds(1))
                    timer.setEventHandler { [weak self] in self?.cycle() }
                    timer.resume()
                    self.timer = timer
                }
            } else {
                timer?.cancel()
                timer = nil
            }
        }
    }

    private func cycle() {
        guard let delegate else {
            return
        }
        var timestamp = AudioTimeStamp()
        timestamp.mSampleTime = sampleTime
        timestamp.mHostTime = mach_absolute_time()
        timestamp.mFlags = [.sampleTimeValid, .hostTimeValid]
        sampleTime += Double(Self.frames)
        var flags = AudioUnitRenderActionFlags()
        if isRecording {
            // Where the cycle stands: the first note, a rest, the second note, a rest.
            let cycle = (sampleTime / Self.rate).truncatingRemainder(dividingBy: 4 * Self.note) / Self.note
            let sounding = cycle.truncatingRemainder(dividingBy: 2) < 1
            let pitch = cycle < 2 ? frequency : frequency * 1.5
            for at in recorded.indices {
                recorded[at] = sounding ? Int16(amplitude * 32767 * sin(phase)) : 0
                phase += 2 * .pi * pitch / Self.rate
                if phase > 2 * .pi {
                    phase -= 2 * .pi
                }
            }
            recorded.withUnsafeMutableBytes { bytes in
                var list = AudioBufferList(
                    mNumberBuffers: 1,
                    mBuffers: AudioBuffer(
                        mNumberChannels: 1, mDataByteSize: UInt32(bytes.count), mData: bytes.baseAddress
                    )
                )
                _ = delegate.deliverRecordedData(&flags, &timestamp, 1, Self.frames, &list, nil, nil)
            }
        }
        if isPlaying {
            played.withUnsafeMutableBytes { bytes in
                var list = AudioBufferList(
                    mNumberBuffers: 1,
                    mBuffers: AudioBuffer(
                        mNumberChannels: 1, mDataByteSize: UInt32(bytes.count), mData: bytes.baseAddress
                    )
                )
                _ = delegate.getPlayoutData(&flags, &timestamp, 0, Self.frames, &list)
            }
        }
    }
}
