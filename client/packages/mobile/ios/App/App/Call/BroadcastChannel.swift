import CoreMedia
import CoreVideo
import Foundation

/// The channel between the broadcast upload extension (`AspenBroadcast`, which iOS before 27
/// runs the screen capture in, as a process of its own) and the app, over the App Group the two
/// share: a Unix socket in the group's container for messages, and a file there both map into
/// memory for the samples themselves. Compiled into both targets.
///
/// Frames are large (a phone's screen at 1080p is three megabytes a frame as NV12, thirty times a
/// second), so they never cross the socket: the mapped file holds a ring of `videoSlots` frame
/// slots and a ring of audio bytes. The extension copies each frame's planes out of its own
/// pixel buffer into a free slot (one copy, which cannot be avoided: an IOSurface cannot be handed
/// between an extension and its app), tells the app which slot over the socket (`Message.frame`),
/// and the app wraps the slot's memory as a pixel buffer of its own without copying it
/// (`CVPixelBufferCreateWithPlanarBytes`), hands it to WebRTC, and gives the slot back
/// (`Message.release`) when WebRTC has let the buffer go. A frame that finds no free slot is
/// dropped by the extension, so a slow app never backs the capture up. Audio is small: each
/// buffer is written at the audio ring's write offset and read out by the app as its message
/// arrives, the ring being seconds deep.
///
/// The app listens (`BroadcastListener`) while a share is asked for; the extension connects
/// (`BroadcastSender`) when its broadcast starts. The app ends a share by sending `stop`, on
/// which the extension finishes its broadcast; the extension ending (the user's stop in the
/// system's control, or the system's) closes the socket, which the app hears as the share's end.
enum BroadcastChannel {
    /// The App Group both targets are entitled to (`com.apple.security.application-groups`).
    static let appGroup = "group.com.aspen-chat.client"
    /// The extension's bundle id, which the system's broadcast picker is pointed at.
    static let extensionBundleId = "com.aspen-chat.client.broadcast"
    static let videoSlots = 3
    static let maxWidth = 1920
    static let maxHeight = 1920
    /// One NV12 frame of `maxWidth` by `maxHeight` plus a line of slack for the strides.
    static let videoSlotBytes = (maxWidth + 64) * (maxHeight + 2) * 3 / 2
    static let audioRingBytes = 1 << 20
    static let ringBytes = videoSlots * videoSlotBytes + audioRingBytes
    static let version: UInt32 = 1

    static func containerURL() -> URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
    }

    static func socketPath() -> String? {
        containerURL()?.appendingPathComponent("broadcast.sock").path
    }

    static func ringURL() -> URL? {
        containerURL()?.appendingPathComponent("broadcast.ring")
    }

    /// A message over the socket, 48 bytes little-endian: a kind and seven 32-bit fields, then
    /// a 64-bit timestamp, whose meanings the kind decides.
    struct Message: Equatable {
        enum Kind: UInt32 {
            /// The extension is connected and speaks `version` (`a`).
            case hello = 1
            /// A frame in video slot `a`: `b` by `c` pixels, planes at strides `d` and `e`,
            /// rotation `f` (degrees clockwise to upright), pixel format `g` (a four-character
            /// code), presented at `timestamp` nanoseconds.
            case frame = 2
            /// Audio at `a` bytes into the audio ring, `b` bytes long, `c` Hz, `d` channels, `e`
            /// bits a channel, `f` set when float, `g` set when interleaved (or mono), presented
            /// at `timestamp` nanoseconds.
            case audio = 3
            /// The app is done with video slot `a`.
            case release = 4
            /// The app ends the share: the extension finishes its broadcast.
            case stop = 5
        }

        static let size = 48

        var kind: Kind
        var a: UInt32 = 0
        var b: UInt32 = 0
        var c: UInt32 = 0
        var d: UInt32 = 0
        var e: UInt32 = 0
        var f: UInt32 = 0
        var g: UInt32 = 0
        var timestamp: UInt64 = 0

        func encoded() -> Data {
            var data = Data(capacity: Self.size)
            for field in [kind.rawValue, a, b, c, d, e, f, g] {
                var little = field.littleEndian
                data.append(Data(bytes: &little, count: 4))
            }
            var time = timestamp.littleEndian
            data.append(Data(bytes: &time, count: 8))
            return data
        }

        /// Reads one message off the front of `data`, or `nil` for too little or a kind unknown.
        static func decode(_ data: Data) -> Message? {
            guard data.count >= size else {
                return nil
            }
            let fields: [UInt32] = (0..<8).map { at in
                data.subdata(in: data.startIndex + at * 4..<data.startIndex + at * 4 + 4)
                    .withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
            }
            let time = data.subdata(in: data.startIndex + 32..<data.startIndex + 40)
                .withUnsafeBytes { UInt64(littleEndian: $0.loadUnaligned(as: UInt64.self)) }
            guard let kind = Kind(rawValue: fields[0]) else {
                return nil
            }
            return Message(
                kind: kind, a: fields[1], b: fields[2], c: fields[3], d: fields[4], e: fields[5], f: fields[6],
                g: fields[7], timestamp: time)
        }
    }

    /// The shared file mapped into this process. Each side maps the whole; `create` (the app's)
    /// makes the file and sizes it first.
    final class Ring {
        let base: UnsafeMutableRawPointer
        private let fd: Int32

        private init(base: UnsafeMutableRawPointer, fd: Int32) {
            self.base = base
            self.fd = fd
        }

        static func open(create: Bool) -> Ring? {
            guard let url = ringURL() else {
                return nil
            }
            let flags = create ? O_RDWR | O_CREAT : O_RDWR
            let fd = Darwin.open(url.path, flags, 0o600)
            guard fd >= 0 else {
                return nil
            }
            if create, ftruncate(fd, off_t(ringBytes)) != 0 {
                close(fd)
                return nil
            }
            guard let mapped = mmap(nil, ringBytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0), mapped != MAP_FAILED
            else {
                close(fd)
                return nil
            }
            return Ring(base: mapped, fd: fd)
        }

        func slot(_ index: Int) -> UnsafeMutableRawPointer {
            base + index * videoSlotBytes
        }

        var audio: UnsafeMutableRawPointer {
            base + videoSlots * videoSlotBytes
        }

        deinit {
            munmap(base, ringBytes)
            Darwin.close(fd)
        }
    }

    /// Writes whole messages to `fd` and reads them back; the socket is a stream, so a read may
    /// hold part of a message, which is kept for the next.
    final class Framing {
        private var pending = Data()

        func read(from fd: Int32, into messages: inout [Message]) -> Bool {
            var buffer = [UInt8](repeating: 0, count: 4096)
            let got = Darwin.read(fd, &buffer, buffer.count)
            if got <= 0 {
                return false
            }
            pending.append(contentsOf: buffer[0..<got])
            while pending.count >= Message.size {
                let message = Message.decode(pending.prefix(Message.size))
                pending.removeFirst(Message.size)
                if let message {
                    messages.append(message)
                }
            }
            return true
        }

        static func write(_ message: Message, to fd: Int32) -> Bool {
            let data = message.encoded()
            return data.withUnsafeBytes { bytes in
                var sent = 0
                while sent < data.count {
                    let wrote = Darwin.write(fd, bytes.baseAddress! + sent, data.count - sent)
                    if wrote <= 0 {
                        return false
                    }
                    sent += wrote
                }
                return true
            }
        }
    }

    static func socketAddress(_ path: String) -> sockaddr_un? {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            return nil
        }
        withUnsafeMutableBytes(of: &address.sun_path) { sun in
            for (at, byte) in bytes.enumerated() {
                sun[at] = byte
            }
            sun[bytes.count] = 0
        }
        return address
    }
}

/// What `BroadcastListener` tells of the broadcast, on the listener's own queue.
protocol BroadcastListenerDelegate: AnyObject {
    func broadcastConnected()
    /// A frame, wrapped around the slot's memory; the slot is given back when `pixelBuffer` is
    /// released.
    func broadcast(frame pixelBuffer: CVPixelBuffer, rotation: Int, timestampNs: Int64)
    func broadcast(audio sampleBuffer: CMSampleBuffer)
    /// The extension's socket closed: the broadcast ended, by the user, the system, or `stop`.
    func broadcastEnded()
}

/// The app's end: listens on the socket while a share is asked for, maps the ring, and hands
/// each frame and audio buffer to `delegate` on its own queue. One extension connects at a time.
final class BroadcastListener {

    private let queue = DispatchQueue(label: "aspen-broadcast-listener")
    private var listening: Int32 = -1
    private var accepting: DispatchSourceRead?
    private var client: Int32 = -1
    private var reading: DispatchSourceRead?
    private var ring: BroadcastChannel.Ring?
    private let framing = BroadcastChannel.Framing()
    private weak var delegate: BroadcastListenerDelegate?
    private var audioFormats: [String: CMAudioFormatDescription] = [:]

    /// Starts listening; `false` when the App Group container, the ring, or the socket cannot
    /// be had (an app built without the group's entitlement).
    func start(delegate: BroadcastListenerDelegate) -> Bool {
        queue.sync {
            self.delegate = delegate
            guard listening < 0 else {
                return true
            }
            guard let path = BroadcastChannel.socketPath(), var address = BroadcastChannel.socketAddress(path),
                  let ring = BroadcastChannel.Ring.open(create: true)
            else {
                return false
            }
            unlink(path)
            let fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else {
                return false
            }
            let bound = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
            }
            guard bound == 0, listen(fd, 1) == 0 else {
                Darwin.close(fd)
                return false
            }
            self.ring = ring
            listening = fd
            let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
            source.setEventHandler { [weak self] in self?.accept() }
            source.resume()
            accepting = source
            return true
        }
    }

    private func accept() {
        let fd = Darwin.accept(listening, nil, nil)
        guard fd >= 0 else {
            return
        }
        if client >= 0 {
            // One broadcast at a time: a second extension is turned away.
            Darwin.close(fd)
            return
        }
        client = fd
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in self?.readMessages() }
        source.resume()
        reading = source
    }

    private func readMessages() {
        var messages: [BroadcastChannel.Message] = []
        if !framing.read(from: client, into: &messages) {
            dropClient()
            delegate?.broadcastEnded()
            return
        }
        for message in messages {
            switch message.kind {
            case .hello:
                delegate?.broadcastConnected()
            case .frame:
                deliverFrame(message)
            case .audio:
                deliverAudio(message)
            case .release, .stop:
                break
            }
        }
    }

    private func deliverFrame(_ message: BroadcastChannel.Message) {
        guard let ring, let delegate else {
            return
        }
        let slot = Int(message.a)
        guard slot < BroadcastChannel.videoSlots else {
            return
        }
        let width = Int(message.b), height = Int(message.c)
        let strideY = Int(message.d), strideUV = Int(message.e)
        let base = ring.slot(slot)
        let planes = [base, base + strideY * height]
        var planeStrides = [strideY, strideUV]
        var planeWidths = [width, (width + 1) / 2]
        var planeHeights = [height, (height + 1) / 2]
        var planeBases: [UnsafeMutableRawPointer?] = planes
        let fd = client
        var pixelBuffer: CVPixelBuffer?
        // Released once WebRTC has let the buffer go: the slot is the extension's again.
        let release: CVPixelBufferReleasePlanarBytesCallback = { context, _, _, _, _ in
            guard let context else { return }
            let kept = Unmanaged<SlotRelease>.fromOpaque(context).takeRetainedValue()
            kept.release()
        }
        let kept = Unmanaged.passRetained(SlotRelease(fd: fd, slot: slot, queue: queue)).toOpaque()
        let made = CVPixelBufferCreateWithPlanarBytes(
            kCFAllocatorDefault, width, height, OSType(message.g), nil, 0, 2, &planeBases, &planeWidths,
            &planeHeights, &planeStrides, release, kept, nil, &pixelBuffer)
        guard made == kCVReturnSuccess, let pixelBuffer else {
            Unmanaged<SlotRelease>.fromOpaque(kept).takeRetainedValue().release()
            return
        }
        delegate.broadcast(frame: pixelBuffer, rotation: Int(message.f), timestampNs: Int64(message.timestamp))
    }

    private func deliverAudio(_ message: BroadcastChannel.Message) {
        guard let ring, let delegate else {
            return
        }
        let offset = Int(message.a), bytes = Int(message.b)
        guard offset + bytes <= BroadcastChannel.audioRingBytes, bytes > 0 else {
            return
        }
        let channels = Int(message.d), bits = Int(message.e)
        let isFloat = message.f != 0, interleaved = message.g != 0
        var description = AudioStreamBasicDescription()
        description.mSampleRate = Float64(message.c)
        description.mFormatID = kAudioFormatLinearPCM
        description.mFormatFlags = (isFloat ? kAudioFormatFlagIsFloat : kAudioFormatFlagIsSignedInteger) | kAudioFormatFlagIsPacked
            | (interleaved ? 0 : kAudioFormatFlagIsNonInterleaved)
        description.mBitsPerChannel = UInt32(bits)
        description.mChannelsPerFrame = UInt32(channels)
        description.mFramesPerPacket = 1
        description.mBytesPerFrame = UInt32(interleaved ? channels * bits / 8 : bits / 8)
        description.mBytesPerPacket = description.mBytesPerFrame
        let key = "\(message.c)/\(channels)/\(bits)/\(isFloat)/\(interleaved)"
        let format: CMAudioFormatDescription
        if let known = audioFormats[key] {
            format = known
        } else {
            var made: CMAudioFormatDescription?
            guard CMAudioFormatDescriptionCreate(
                allocator: kCFAllocatorDefault, asbd: &description, layoutSize: 0, layout: nil, magicCookieSize: 0,
                magicCookie: nil, extensions: nil, formatDescriptionOut: &made) == noErr, let made
            else {
                return
            }
            audioFormats[key] = made
            format = made
        }
        // The bytes are copied out of the ring at once, since the extension writes on.
        var block: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(
            allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: bytes, blockAllocator: kCFAllocatorDefault,
            customBlockSource: nil, offsetToData: 0, dataLength: bytes, flags: 0, blockBufferOut: &block) == noErr,
            let block
        else {
            return
        }
        CMBlockBufferReplaceDataBytes(with: ring.audio + offset, blockBuffer: block, offsetIntoDestination: 0, dataLength: bytes)
        let frames = bytes / Int(description.mBytesPerFrame) / (interleaved ? 1 : channels)
        var sampleBuffer: CMSampleBuffer?
        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1, timescale: CMTimeScale(message.c)),
            presentationTimeStamp: CMTime(value: CMTimeValue(message.timestamp), timescale: 1_000_000_000),
            decodeTimeStamp: .invalid)
        guard CMSampleBufferCreate(
            allocator: kCFAllocatorDefault, dataBuffer: block, dataReady: true, makeDataReadyCallback: nil, refcon: nil,
            formatDescription: format, sampleCount: frames, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
            sampleSizeEntryCount: 0, sampleSizeArray: nil, sampleBufferOut: &sampleBuffer) == noErr,
            let sampleBuffer
        else {
            return
        }
        delegate.broadcast(audio: sampleBuffer)
    }

    /// Tells the extension to finish its broadcast; the share ends when its socket closes.
    func stop() {
        queue.async { [self] in
            if client >= 0 {
                _ = BroadcastChannel.Framing.write(BroadcastChannel.Message(kind: .stop), to: client)
            }
        }
    }

    /// Stops listening and drops any extension connected.
    func close() {
        queue.sync {
            dropClient()
            accepting?.cancel()
            accepting = nil
            if listening >= 0 {
                Darwin.close(listening)
                listening = -1
            }
            if let path = BroadcastChannel.socketPath() {
                unlink(path)
            }
            ring = nil
        }
    }

    private func dropClient() {
        reading?.cancel()
        reading = nil
        if client >= 0 {
            Darwin.close(client)
            client = -1
        }
    }

    /// Gives a video slot back to the extension when the pixel buffer wrapping it is released.
    private final class SlotRelease {
        let fd: Int32
        let slot: Int
        let queue: DispatchQueue

        init(fd: Int32, slot: Int, queue: DispatchQueue) {
            self.fd = fd
            self.slot = slot
            self.queue = queue
        }

        func release() {
            queue.async { [fd, slot] in
                _ = BroadcastChannel.Framing.write(BroadcastChannel.Message(kind: .release, a: UInt32(slot)), to: fd)
            }
        }
    }
}

/// The extension's end: connects to the app's socket, maps the ring, and writes each sample
/// into it. Frames go to free video slots, dropped when none is free; audio to the audio ring.
final class BroadcastSender {
    private let queue = DispatchQueue(label: "aspen-broadcast-sender")
    private var fd: Int32 = -1
    private var reading: DispatchSourceRead?
    private var ring: BroadcastChannel.Ring?
    private let framing = BroadcastChannel.Framing()
    private var free = Set(0..<BroadcastChannel.videoSlots)
    private var audioOffset = 0
    /// Called when the app ends the share, or its socket closes.
    var onStop: (() -> Void)?

    /// Connects to the app; `false` when it is not listening (no share was asked for).
    func connect() -> Bool {
        queue.sync {
            guard let path = BroadcastChannel.socketPath(), var address = BroadcastChannel.socketAddress(path),
                  let ring = BroadcastChannel.Ring.open(create: false)
            else {
                return false
            }
            let fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else {
                return false
            }
            let connected = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard connected == 0 else {
                Darwin.close(fd)
                return false
            }
            self.fd = fd
            self.ring = ring
            let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
            source.setEventHandler { [weak self] in self?.readMessages() }
            source.resume()
            reading = source
            return BroadcastChannel.Framing.write(
                BroadcastChannel.Message(kind: .hello, a: BroadcastChannel.version), to: fd)
        }
    }

    private func readMessages() {
        var messages: [BroadcastChannel.Message] = []
        if !framing.read(from: fd, into: &messages) {
            disconnect()
            onStop?()
            return
        }
        for message in messages {
            switch message.kind {
            case .release:
                free.insert(Int(message.a))
            case .stop:
                disconnect()
                onStop?()
            case .hello, .frame, .audio:
                break
            }
        }
    }

    /// Copies a bi-planar 8-bit frame (`420v` or `420f`, which ReplayKit's screen frames are)
    /// into a free slot and tells the app; a frame of another format, or one too large for a
    /// slot, is dropped.
    func send(frame pixelBuffer: CVPixelBuffer, rotation: Int, timestampNs: Int64) {
        queue.sync {
            guard fd >= 0, let ring, let slot = free.first else {
                return
            }
            let format = CVPixelBufferGetPixelFormatType(pixelBuffer)
            guard format == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
                || format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                CVPixelBufferGetPlaneCount(pixelBuffer) == 2
            else {
                return
            }
            let width = CVPixelBufferGetWidth(pixelBuffer), height = CVPixelBufferGetHeight(pixelBuffer)
            CVPixelBufferLockBaseAddress(pixelBuffer, .readOnly)
            defer { CVPixelBufferUnlockBaseAddress(pixelBuffer, .readOnly) }
            guard let y = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 0),
                  let uv = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 1)
            else {
                return
            }
            let strideY = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 0)
            let strideUV = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 1)
            let rowsUV = CVPixelBufferGetHeightOfPlane(pixelBuffer, 1)
            guard strideY * height + strideUV * rowsUV <= BroadcastChannel.videoSlotBytes else {
                return
            }
            let base = ring.slot(slot)
            memcpy(base, y, strideY * height)
            memcpy(base + strideY * height, uv, strideUV * rowsUV)
            free.remove(slot)
            let message = BroadcastChannel.Message(
                kind: .frame, a: UInt32(slot), b: UInt32(width), c: UInt32(height), d: UInt32(strideY), e: UInt32(strideUV),
                f: UInt32(rotation), g: format, timestamp: UInt64(max(0, timestampNs)))
            if !BroadcastChannel.Framing.write(message, to: fd) {
                disconnect()
                onStop?()
            }
        }
    }

    /// Copies a PCM buffer into the audio ring and tells the app.
    func send(audio sampleBuffer: CMSampleBuffer) {
        queue.sync {
            guard fd >= 0, let ring,
                  let description = CMSampleBufferGetFormatDescription(sampleBuffer),
                  let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee,
                  asbd.mFormatID == kAudioFormatLinearPCM
            else {
                return
            }
            let bytes = CMSampleBufferGetTotalSampleSize(sampleBuffer)
            guard bytes > 0, bytes <= BroadcastChannel.audioRingBytes else {
                return
            }
            if audioOffset + bytes > BroadcastChannel.audioRingBytes {
                audioOffset = 0
            }
            guard let block = CMSampleBufferGetDataBuffer(sampleBuffer),
                  CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: bytes, destination: ring.audio + audioOffset) == noErr
            else {
                return
            }
            let interleaved = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved == 0
            let isFloat = asbd.mFormatFlags & kAudioFormatFlagIsFloat != 0
            let presented = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
            let timestampNs = presented.isNumeric ? Int64(CMTimeGetSeconds(presented) * 1_000_000_000) : 0
            let message = BroadcastChannel.Message(
                kind: .audio, a: UInt32(audioOffset), b: UInt32(bytes), c: UInt32(asbd.mSampleRate), d: asbd.mChannelsPerFrame,
                e: asbd.mBitsPerChannel, f: isFloat ? 1 : 0, g: interleaved ? 1 : 0, timestamp: UInt64(max(0, timestampNs)))
            audioOffset += bytes
            if !BroadcastChannel.Framing.write(message, to: fd) {
                disconnect()
                onStop?()
            }
        }
    }

    func close() {
        queue.sync { disconnect() }
    }

    private func disconnect() {
        reading?.cancel()
        reading = nil
        if fd >= 0 {
            Darwin.close(fd)
            fd = -1
        }
        ring = nil
    }
}
