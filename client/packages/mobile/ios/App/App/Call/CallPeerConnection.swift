import Foundation
import WebRTC

/// The call engine's peer connections (`PeerConnectionFactory`) over `RTCPeerConnectionFactory`,
/// one for the process, with the platform's video codecs and WebRTC's own audio device module,
/// which captures the microphone and plays every received track through the shared audio
/// session (`CallAudio`). A test gives an `RTCAudioDevice` of its own instead, a microphone and
/// speaker in software, where the simulator's audio unit cannot be initialized.
final class CallPeerConnections: PeerConnectionFactory, @unchecked Sendable {
    let factory: RTCPeerConnectionFactory
    let tracks: CallTracks

    init(tracks: CallTracks, audioDevice: RTCAudioDevice? = nil) {
        RTCInitializeSSL()
        self.factory = RTCPeerConnectionFactory(
            encoderFactory: RTCDefaultVideoEncoderFactory(),
            decoderFactory: RTCDefaultVideoDecoderFactory(),
            audioDevice: audioDevice
        )
        self.tracks = tracks
    }

    func create(configuration: Configuration, observer: PeerConnectionObserver) async throws -> CreatedPeerConnection {
        let delegate = CallPeerConnectionDelegate(observer: observer)
        guard let connection = factory.peerConnection(
            with: CallPeerConnection.configuration(configuration),
            constraints: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil),
            delegate: delegate
        ) else {
            throw PeerConnectionError.Failed(detail: "WebRTC made no peer connection")
        }
        return CreatedPeerConnection(connection: CallPeerConnection(connection, delegate: delegate, tracks: tracks))
    }
}

/// What the engine hears of a connection without asking. The peer connection keeps its delegate
/// weakly, so `CallPeerConnection` holds this for it.
final class CallPeerConnectionDelegate: NSObject, RTCPeerConnectionDelegate {
    private let observer: PeerConnectionObserver

    init(observer: PeerConnectionObserver) {
        self.observer = observer
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {
        observer.connectionStateChanged(state: CallPeerConnection.connectionState(newState))
    }

    /// A candidate of this connection's own, for the other side of a transfer; the call's own
    /// connection has none the engine reads.
    func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {
        observer.iceCandidate(candidate: IceCandidate(
            candidate: candidate.sdp,
            sdpMid: candidate.sdpMid,
            sdpMLineIndex: UInt16(clamping: candidate.sdpMLineIndex),
            usernameFragment: nil
        ))
    }

    /// The other side opened a data channel, a transfer's: the engine's observer reads it from
    /// here on. The channel keeps its delegate weakly, so the wrapper, which the engine holds, is
    /// the delegate.
    func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {
        let channel = CallDataChannel(dataChannel)
        channel.attach(observer.dataChannelOpened(channel: channel))
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
    func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
}

/// One data channel as the engine drives it (`DataChannel`), over `RTCDataChannel`, and its
/// delegate, forwarding to the engine's observer from WebRTC's thread. WebRTC tells of each
/// change of the buffered amount with the amount before it, so the low mark is read here: the
/// observer hears `bufferedAmountLow` when the amount falls to the threshold or below from
/// above. The engine holds this object for the channel's life, which is why it, and not the
/// weakly held delegate, can be the delegate.
final class CallDataChannel: NSObject, DataChannel, RTCDataChannelDelegate, @unchecked Sendable {
    private let channel: RTCDataChannel
    private let lock = NSLock()
    private var observer: DataChannelObserver?
    private var threshold: UInt64 = 0

    init(_ channel: RTCDataChannel) {
        self.channel = channel
        super.init()
    }

    /// Reads the channel from here on, as `observer`.
    func attach(_ observer: DataChannelObserver) {
        lock.lock()
        self.observer = observer
        lock.unlock()
        channel.delegate = self
    }

    private var attached: DataChannelObserver? {
        lock.lock()
        defer { lock.unlock() }
        return observer
    }

    static func state(_ state: RTCDataChannelState) -> DataChannelState {
        switch state {
        case .connecting: return .connecting
        case .open: return .open
        case .closing: return .closing
        case .closed: return .closed
        @unknown default: return .closed
        }
    }

    func sendBinary(data: Data) throws {
        guard channel.sendData(RTCDataBuffer(data: data, isBinary: true)) else {
            throw PeerConnectionError.Failed(detail: "the data channel refused the message")
        }
    }

    func sendText(text: String) throws {
        guard channel.sendData(RTCDataBuffer(data: Data(text.utf8), isBinary: false)) else {
            throw PeerConnectionError.Failed(detail: "the data channel refused the message")
        }
    }

    func bufferedAmount() -> UInt64 {
        channel.bufferedAmount
    }

    func setBufferedAmountLowThreshold(bytes: UInt64) {
        lock.lock()
        threshold = bytes
        lock.unlock()
    }

    func state() -> DataChannelState {
        Self.state(channel.readyState)
    }

    func closeDataChannel() {
        channel.close()
    }

    func dataChannelDidChangeState(_ dataChannel: RTCDataChannel) {
        attached?.stateChanged(state: Self.state(dataChannel.readyState))
    }

    func dataChannel(_ dataChannel: RTCDataChannel, didReceiveMessageWith buffer: RTCDataBuffer) {
        guard let observer = attached else { return }
        if buffer.isBinary {
            observer.binary(data: buffer.data)
        } else {
            observer.text(text: String(decoding: buffer.data, as: UTF8.self))
        }
    }

    func dataChannel(_ dataChannel: RTCDataChannel, didChangeBufferedAmount amount: UInt64) {
        lock.lock()
        let threshold = self.threshold
        lock.unlock()
        // `amount` is what was buffered before the change.
        if amount > threshold && dataChannel.bufferedAmount <= threshold {
            attached?.bufferedAmountLow()
        }
    }
}

/// One peer connection as the engine drives it (`PeerConnection`), each method the W3C operation
/// of its name over `RTCPeerConnection`; a transfer's connection has its data channel
/// (`CallDataChannel`) and trickles its candidates through the delegate. Where the two differ:
///
/// - `RTCPeerConnection.transceivers` wraps the native transceivers anew on every read, so a
///   transceiver is told apart by its sender's id, which WebRTC keeps for its life, and numbered
///   here in the order first seen; the number is the engine's `TransceiverId`.
/// - `getParameters` and `setParameters` are one property, `RTCRtpSender.parameters`, whose
///   object must be the one last read: the object is kept here between the two, and the
///   engine's `SenderParameters.rest` stays empty.
/// - A statistics report is a dictionary of `RTCStatistics` with `timestamp_us`; it is written
///   as the W3C array, each member with `id`, `type`, and `timestamp` in milliseconds beside its
///   values, which already carry their W3C names.
/// - A receiver's volume is its audio track's source's (`RTCAudioSource.volume`, 0 to 10).
final class CallPeerConnection: PeerConnection, @unchecked Sendable {
    private let connection: RTCPeerConnection
    private let delegate: CallPeerConnectionDelegate
    private let tracks: CallTracks
    private let lock = NSLock()
    /// By `TransceiverId`.
    private var transceivers: [RTCRtpTransceiver] = []
    /// The parameters last read of each sender, for `setSenderParameters` to write back.
    private var parameters: [TransceiverId: RTCRtpParameters] = [:]
    /// The receivers' tracks put in `tracks`, taken out again when the connection closes.
    private var received: Set<TrackId> = []

    init(_ connection: RTCPeerConnection, delegate: CallPeerConnectionDelegate, tracks: CallTracks) {
        self.connection = connection
        self.delegate = delegate
        self.tracks = tracks
    }

    // MARK: Conversions

    /// Unified plan, one ICE and DTLS association for every section, RTCP multiplexed, as the
    /// voice server requires of a client.
    static func configuration(_ configuration: Configuration) -> RTCConfiguration {
        let made = RTCConfiguration()
        made.sdpSemantics = .unifiedPlan
        made.bundlePolicy = .maxBundle
        made.rtcpMuxPolicy = .require
        made.iceServers = configuration.iceServers.map { server in
            RTCIceServer(urlStrings: server.urls, username: server.username, credential: server.credential)
        }
        made.iceTransportPolicy = configuration.iceTransportPolicy == "relay" ? .relay : .all
        return made
    }

    static func connectionState(_ state: RTCPeerConnectionState) -> ConnectionState {
        switch state {
        case .new: return .new
        case .connecting: return .connecting
        case .connected: return .connected
        case .disconnected: return .disconnected
        case .failed: return .failed
        case .closed: return .closed
        @unknown default: return .failed
        }
    }

    private static func signalingState(_ state: RTCSignalingState) -> SignalingState {
        switch state {
        case .stable: return .stable
        case .haveLocalOffer: return .haveLocalOffer
        case .haveLocalPrAnswer: return .haveLocalPranswer
        case .haveRemoteOffer: return .haveRemoteOffer
        case .haveRemotePrAnswer: return .haveRemotePranswer
        case .closed: return .closed
        @unknown default: return .closed
        }
    }

    private static func sdpType(_ kind: DescriptionKind) -> RTCSdpType {
        switch kind {
        case .offer: return .offer
        case .answer: return .answer
        case .rollback: return .rollback
        }
    }

    private static func descriptionKind(_ type: RTCSdpType) -> DescriptionKind? {
        switch type {
        case .offer: return .offer
        case .answer, .prAnswer: return .answer
        case .rollback: return .rollback
        @unknown default: return nil
        }
    }

    private static func direction(_ direction: Direction) -> RTCRtpTransceiverDirection {
        switch direction {
        case .sendRecv: return .sendRecv
        case .sendOnly: return .sendOnly
        case .recvOnly: return .recvOnly
        case .inactive: return .inactive
        }
    }

    /// The W3C `RTCPriorityType` names and WebRTC's `bitratePriority`, whose values the
    /// specification fixes (very-low 0.5, low 1, medium 2, high 4).
    private static let priorities: [(name: String, priority: RTCPriority, bitrate: Double)] = [
        ("very-low", .veryLow, 0.5),
        ("low", .low, 1),
        ("medium", .medium, 2),
        ("high", .high, 4),
    ]

    /// Writes `encoding` onto `parameters`, leaving what it does not say as it was.
    private static func apply(_ encoding: SendEncoding, to parameters: RTCRtpEncodingParameters) {
        if let rid = encoding.rid {
            parameters.rid = rid
        }
        if let active = encoding.active {
            parameters.isActive = active
        }
        if let maxBitrate = encoding.maxBitrate {
            parameters.maxBitrateBps = NSNumber(value: maxBitrate)
        }
        if let maxFramerate = encoding.maxFramerate {
            parameters.maxFramerate = NSNumber(value: maxFramerate)
        }
        if let scale = encoding.scaleResolutionDownBy {
            parameters.scaleResolutionDownBy = NSNumber(value: scale)
        }
        if let network = priorities.first(where: { $0.name == encoding.networkPriority }) {
            parameters.networkPriority = network.priority
        }
        if let bitrate = priorities.first(where: { $0.name == encoding.priority }) {
            parameters.bitratePriority = bitrate.bitrate
        }
    }

    private static func encoding(_ parameters: RTCRtpEncodingParameters) -> SendEncoding {
        SendEncoding(
            rid: parameters.rid,
            active: parameters.isActive,
            maxBitrate: parameters.maxBitrateBps.map { UInt32(clamping: $0.int64Value) },
            maxFramerate: parameters.maxFramerate?.doubleValue,
            scaleResolutionDownBy: parameters.scaleResolutionDownBy?.doubleValue,
            networkPriority: priorities.first { $0.priority == parameters.networkPriority }?.name,
            priority: priorities.first { $0.bitrate == parameters.bitratePriority }?.name
        )
    }

    /// The report as the W3C `RTCStatsReport` reads when iterated: an array of its stats
    /// objects, each the values WebRTC gathered under their W3C names, with `id`, `type`, and
    /// `timestamp` (milliseconds, as `DOMHighResTimeStamp` is) beside them.
    static func json(_ report: RTCStatisticsReport) throws -> String {
        let entries: [[String: Any]] = report.statistics.values.map { statistics in
            var entry: [String: Any] = statistics.values
            entry["id"] = statistics.id
            entry["type"] = statistics.type
            entry["timestamp"] = statistics.timestamp_us / 1000
            return entry
        }
        guard JSONSerialization.isValidJSONObject(entries) else {
            throw PeerConnectionError.Failed(detail: "the statistics report does not write as JSON")
        }
        let data = try JSONSerialization.data(withJSONObject: entries)
        return String(decoding: data, as: UTF8.self)
    }

    // MARK: Transceivers

    /// The number of `transceiver`, given the first time it is seen.
    private func number(_ transceiver: RTCRtpTransceiver) -> TransceiverId {
        let id = transceiver.sender.senderId
        if let at = transceivers.firstIndex(where: { $0.sender.senderId == id }) {
            return TransceiverId(at)
        }
        transceivers.append(transceiver)
        return TransceiverId(transceivers.count - 1)
    }

    /// Learns every transceiver the connection has, those a remote description made included.
    private func learn() {
        for transceiver in connection.transceivers {
            _ = number(transceiver)
        }
    }

    /// Runs `body` holding the lock; a call rather than `lock()` and `unlock()` inline, since those
    /// are not for async methods.
    private func locked<T>(_ body: () -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return body()
    }

    private func transceiver(_ id: TransceiverId) throws -> RTCRtpTransceiver {
        lock.lock()
        defer { lock.unlock() }
        guard let transceiver = transceivers[safe: Int(id)] else {
            throw PeerConnectionError.Failed(detail: "no transceiver \(id)")
        }
        return transceiver
    }

    // MARK: PeerConnection

    func createOffer(iceRestart: Bool) async throws -> String {
        let constraints = RTCMediaConstraints(
            mandatoryConstraints: iceRestart ? [kRTCMediaConstraintsIceRestart: kRTCMediaConstraintsValueTrue] : nil,
            optionalConstraints: nil
        )
        let description: RTCSessionDescription = try await Self.completed { done in
            connection.offer(for: constraints) { description, error in
                done(description, error)
            }
        }
        return description.sdp
    }

    func createAnswer() async throws -> String {
        let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
        let description: RTCSessionDescription = try await Self.completed { done in
            connection.answer(for: constraints) { description, error in
                done(description, error)
            }
        }
        return description.sdp
    }

    func setLocalDescription(description: SessionDescription) async throws {
        let sdp = RTCSessionDescription(type: Self.sdpType(description.kind), sdp: description.sdp)
        try await Self.finished { done in
            connection.setLocalDescription(sdp, completionHandler: done)
        }
    }

    func setRemoteDescription(description: SessionDescription) async throws {
        let sdp = RTCSessionDescription(type: Self.sdpType(description.kind), sdp: description.sdp)
        try await Self.finished { done in
            connection.setRemoteDescription(sdp, completionHandler: done)
        }
        locked { learn() }
    }

    func addIceCandidate(candidate: IceCandidate) async throws {
        let added = RTCIceCandidate(
            sdp: candidate.candidate,
            sdpMLineIndex: Int32(clamping: candidate.sdpMLineIndex ?? 0),
            sdpMid: candidate.sdpMid
        )
        try await Self.finished { done in
            connection.add(added, completionHandler: done)
        }
    }

    /// Ordered and reliable, which `RTCDataChannelConfiguration` is unless told otherwise.
    func createDataChannel(label: String, observer: DataChannelObserver) async throws -> CreatedDataChannel {
        let configuration = RTCDataChannelConfiguration()
        configuration.isOrdered = true
        guard let made = connection.dataChannel(forLabel: label, configuration: configuration) else {
            throw PeerConnectionError.Failed(detail: "the data channel could not be opened")
        }
        let channel = CallDataChannel(made)
        channel.attach(observer)
        return CreatedDataChannel(channel: channel)
    }

    func localDescription() -> SessionDescription? {
        guard let local = connection.localDescription, let kind = Self.descriptionKind(local.type) else {
            return nil
        }
        return SessionDescription(kind: kind, sdp: local.sdp)
    }

    func signalingState() -> SignalingState {
        Self.signalingState(connection.signalingState)
    }

    func addTransceiver(
        kind: MediaKind, track: TrackId?, direction: Direction, streamId: String?, sendEncodings: [SendEncoding]
    ) async throws -> TransceiverId {
        let initial = RTCRtpTransceiverInit()
        initial.direction = Self.direction(direction)
        if let streamId {
            initial.streamIds = [streamId]
        }
        initial.sendEncodings = sendEncodings.map { encoding in
            let parameters = RTCRtpEncodingParameters()
            Self.apply(encoding, to: parameters)
            return parameters
        }
        let made: RTCRtpTransceiver?
        if let track {
            guard let media = tracks.track(track) else {
                throw PeerConnectionError.Failed(detail: "no track \(track)")
            }
            made = connection.addTransceiver(with: media, init: initial)
        } else {
            made = connection.addTransceiver(of: kind == .audio ? .audio : .video, init: initial)
        }
        guard let made else {
            throw PeerConnectionError.Failed(detail: "the transceiver could not be added")
        }
        return locked { number(made) }
    }

    func transceiverMid(transceiver: TransceiverId) -> String? {
        guard let transceiver = try? self.transceiver(transceiver) else {
            return nil
        }
        // WebRTC answers an empty mid for a transceiver no description has placed yet.
        return transceiver.mid.isEmpty ? nil : transceiver.mid
    }

    func transceiverByMid(mid: String) -> TransceiverId? {
        lock.lock()
        defer { lock.unlock() }
        learn()
        return transceivers.firstIndex { $0.mid == mid }.map { TransceiverId($0) }
    }

    func setDirection(transceiver: TransceiverId, direction: Direction) async throws {
        let transceiver = try self.transceiver(transceiver)
        var error: NSError?
        transceiver.setDirection(Self.direction(direction), error: &error)
        if let error {
            throw PeerConnectionError.Failed(detail: error.localizedDescription)
        }
    }

    func stopTransceiver(transceiver: TransceiverId) async throws {
        try self.transceiver(transceiver).stopInternal()
    }

    func removeTrack(transceiver: TransceiverId) async throws {
        let sender = try self.transceiver(transceiver).sender
        guard connection.removeTrack(sender) else {
            throw PeerConnectionError.Failed(detail: "the sender's track could not be removed")
        }
    }

    func replaceTrack(transceiver: TransceiverId, track: TrackId?) async throws {
        let sender = try self.transceiver(transceiver).sender
        if let track {
            guard let media = tracks.track(track) else {
                throw PeerConnectionError.Failed(detail: "no track \(track)")
            }
            sender.track = media
        } else {
            sender.track = nil
        }
    }

    func senderParameters(transceiver: TransceiverId) async throws -> SenderParameters {
        let read = try self.transceiver(transceiver).sender.parameters
        locked { parameters[transceiver] = read }
        return SenderParameters(encodings: read.encodings.map(Self.encoding), rest: "")
    }

    func setSenderParameters(transceiver: TransceiverId, parameters: SenderParameters) async throws {
        let sender = try self.transceiver(transceiver).sender
        let kept = locked { self.parameters.removeValue(forKey: transceiver) }
        let written = kept ?? sender.parameters
        for (encoding, target) in zip(parameters.encodings, written.encodings) {
            Self.apply(encoding, to: target)
        }
        sender.parameters = written
    }

    func receiverTrack(transceiver: TransceiverId) -> TrackId? {
        guard let track = try? self.transceiver(transceiver).receiver.track else {
            return nil
        }
        let id = tracks.add(track)
        lock.lock()
        received.insert(id)
        lock.unlock()
        return id
    }

    func setReceiverVolume(transceiver: TransceiverId, volume: Double) async throws {
        let received = try self.transceiver(transceiver).receiver.track
        guard let track = received as? RTCAudioTrack else {
            throw PeerConnectionError.Failed(detail: "transceiver \(transceiver) receives no audio")
        }
        track.source.volume = volume
    }

    func setConfiguration(configuration: Configuration) async throws {
        guard connection.setConfiguration(Self.configuration(configuration)) else {
            throw PeerConnectionError.Failed(detail: "the configuration was refused")
        }
    }

    func stats(of: StatsOf) async throws -> String {
        let report: RTCStatisticsReport = await withCheckedContinuation { continuation in
            switch of {
            case .connection:
                connection.statistics { continuation.resume(returning: $0) }
            case .sender(let id):
                guard let transceiver = try? self.transceiver(id) else {
                    connection.statistics { continuation.resume(returning: $0) }
                    return
                }
                connection.statistics(for: transceiver.sender) { continuation.resume(returning: $0) }
            case .receiver(let id):
                guard let transceiver = try? self.transceiver(id) else {
                    connection.statistics { continuation.resume(returning: $0) }
                    return
                }
                connection.statistics(for: transceiver.receiver) { continuation.resume(returning: $0) }
            }
        }
        return try Self.json(report)
    }

    func connectionState() -> ConnectionState {
        Self.connectionState(connection.connectionState)
    }

    func closePeerConnection() {
        connection.close()
        lock.lock()
        let received = self.received
        self.received = []
        lock.unlock()
        for id in received {
            tracks.remove(id)
        }
    }

    // MARK: Completion handlers as throwing calls

    private static func completed<T>(
        _ start: (@escaping (T?, Error?) -> Void) -> Void
    ) async throws -> T {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<T, Error>) in
            start { value, error in
                if let value {
                    continuation.resume(returning: value)
                } else {
                    continuation.resume(throwing: PeerConnectionError.Failed(
                        detail: error?.localizedDescription ?? "WebRTC answered nothing"
                    ))
                }
            }
        }
    }

    private static func finished(_ start: (@escaping (Error?) -> Void) -> Void) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            start { error in
                if let error {
                    continuation.resume(throwing: PeerConnectionError.Failed(detail: error.localizedDescription))
                } else {
                    continuation.resume()
                }
            }
        }
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
