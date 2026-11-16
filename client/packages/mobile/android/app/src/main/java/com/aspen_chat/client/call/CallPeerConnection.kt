package com.aspen_chat.client.call

import android.content.Context
import com.aspen_chat.client.voice.Configuration
import com.aspen_chat.client.voice.ConnectionState
import com.aspen_chat.client.voice.CreatedDataChannel
import com.aspen_chat.client.voice.CreatedPeerConnection
import com.aspen_chat.client.voice.DataChannelObserver
import com.aspen_chat.client.voice.DataChannelState
import com.aspen_chat.client.voice.DescriptionKind
import com.aspen_chat.client.voice.Direction
import com.aspen_chat.client.voice.IceCandidate
import com.aspen_chat.client.voice.MediaKind
import com.aspen_chat.client.voice.PeerConnection
import com.aspen_chat.client.voice.PeerConnectionException
import com.aspen_chat.client.voice.PeerConnectionFactory
import com.aspen_chat.client.voice.PeerConnectionObserver
import com.aspen_chat.client.voice.SendEncoding
import com.aspen_chat.client.voice.SenderParameters
import com.aspen_chat.client.voice.SessionDescription
import com.aspen_chat.client.voice.SignalingState
import com.aspen_chat.client.voice.StatsOf
import com.aspen_chat.client.voice.TrackId
import com.aspen_chat.client.voice.TransceiverId
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.suspendCoroutine
import kotlin.math.roundToInt
import org.json.JSONArray
import org.json.JSONObject
import org.webrtc.AudioTrack
import org.webrtc.DefaultVideoDecoderFactory
import org.webrtc.DefaultVideoEncoderFactory
import org.webrtc.EglBase
import org.webrtc.MediaConstraints
import org.webrtc.MediaStream
import org.webrtc.MediaStreamTrack
import org.webrtc.RTCStatsCollectorCallback
import org.webrtc.RTCStatsReport
import org.webrtc.RtpParameters
import org.webrtc.RtpTransceiver
import org.webrtc.SdpObserver
import org.webrtc.audio.AudioDeviceModule
import org.webrtc.audio.JavaAudioDeviceModule
import org.webrtc.DataChannel as WebRtcDataChannel
import org.webrtc.IceCandidate as WebRtcIceCandidate
import org.webrtc.PeerConnection as WebRtcPeerConnection
import org.webrtc.PeerConnectionFactory as WebRtcPeerConnectionFactory
import org.webrtc.SessionDescription as WebRtcSessionDescription

/**
 * The call engine's peer connections (`PeerConnectionFactory`) over WebRTC's
 * `PeerConnectionFactory`, one for the process, with the device's video codecs (its hardware
 * encoders and decoders through one EGL context, software ones behind them) and the Java audio
 * device module, which records the microphone and plays every received track, with the device's
 * own echo canceller and noise suppressor where it has them; `eglContext` is that context, which
 * the camera's texture and the video renderers share. A test may give an audio device module of
 * its own instead.
 */
class CallPeerConnections(
    context: Context,
    val tracks: CallTracks,
    audioDevice: AudioDeviceModule? = null,
) : PeerConnectionFactory {
    val factory: WebRtcPeerConnectionFactory
    private val egl: EglBase

    /** The context the codecs share; the camera's texture and the renderers draw on it too. */
    val eglContext: EglBase.Context
        get() = egl.eglBaseContext

    init {
        initialize(context.applicationContext)
        egl = EglBase.create()
        val audio = audioDevice ?: JavaAudioDeviceModule.builder(context.applicationContext)
            .setUseHardwareAcousticEchoCanceler(JavaAudioDeviceModule.isBuiltInAcousticEchoCancelerSupported())
            .setUseHardwareNoiseSuppressor(JavaAudioDeviceModule.isBuiltInNoiseSuppressorSupported())
            .createAudioDeviceModule()
        factory = WebRtcPeerConnectionFactory.builder()
            .setAudioDeviceModule(audio)
            .setVideoEncoderFactory(DefaultVideoEncoderFactory(egl.eglBaseContext, true, true))
            .setVideoDecoderFactory(DefaultVideoDecoderFactory(egl.eglBaseContext))
            .createPeerConnectionFactory()
    }

    override suspend fun create(configuration: Configuration, observer: PeerConnectionObserver): CreatedPeerConnection {
        val connection = factory.createPeerConnection(
            CallPeerConnection.configuration(configuration),
            CallPeerConnectionObserver(observer),
        ) ?: throw PeerConnectionException.Failed("WebRTC made no peer connection")
        return CreatedPeerConnection(CallPeerConnection(connection, tracks))
    }

    private companion object {
        private var initialized = false

        /** WebRTC's native library is loaded and set up once for the process. */
        @Synchronized
        fun initialize(context: Context) {
            if (initialized) {
                return
            }
            WebRtcPeerConnectionFactory.initialize(
                WebRtcPeerConnectionFactory.InitializationOptions.builder(context).createInitializationOptions(),
            )
            initialized = true
        }
    }
}

/** What the engine hears of a connection without asking, from WebRTC's signalling thread. */
class CallPeerConnectionObserver(private val observer: PeerConnectionObserver) : WebRtcPeerConnection.Observer {
    override fun onConnectionChange(newState: WebRtcPeerConnection.PeerConnectionState) {
        observer.connectionStateChanged(CallPeerConnection.connectionState(newState))
    }

    /** A candidate of this connection's own, for the other side of a transfer. */
    override fun onIceCandidate(candidate: WebRtcIceCandidate) {
        observer.iceCandidate(
            IceCandidate(candidate.sdp, candidate.sdpMid, candidate.sdpMLineIndex.toUShort(), null),
        )
    }

    /** The other side opened a data channel, a transfer's: the engine's observer reads it from here on. */
    override fun onDataChannel(dataChannel: WebRtcDataChannel) {
        val channel = CallDataChannel(dataChannel)
        channel.attach(observer.dataChannelOpened(channel))
    }

    override fun onSignalingChange(newState: WebRtcPeerConnection.SignalingState) {}
    override fun onIceConnectionChange(newState: WebRtcPeerConnection.IceConnectionState) {}
    override fun onIceConnectionReceivingChange(receiving: Boolean) {}
    override fun onIceGatheringChange(newState: WebRtcPeerConnection.IceGatheringState) {}
    override fun onIceCandidatesRemoved(candidates: Array<out WebRtcIceCandidate>) {}
    override fun onAddStream(stream: MediaStream) {}
    override fun onRemoveStream(stream: MediaStream) {}
    override fun onRenegotiationNeeded() {}
}

/**
 * One data channel as the engine drives it (`DataChannel`), over `org.webrtc.DataChannel`, and
 * its observer, forwarding to the engine's from WebRTC's signalling thread. WebRTC tells of each
 * change of the buffered amount with the amount before it, so the low mark is read here: the
 * engine hears `bufferedAmountLow` when the amount falls to the threshold or below from above. A
 * message's buffer is WebRTC's own and good only within `onMessage`, so its bytes are copied.
 * Closing disposes the channel, freeing its native side; `state` answers `CLOSED` from then on
 * and sending refuses. No lock is held while WebRTC is called: each of its methods runs on its
 * signalling thread and waits for it, and that thread calls the observer, so a lock held across
 * one would deadlock against a callback taking it. What is shared is atomic instead.
 */
class CallDataChannel(private val channel: WebRtcDataChannel) : com.aspen_chat.client.voice.DataChannel, WebRtcDataChannel.Observer {
    private val observer = AtomicReference<DataChannelObserver?>(null)
    private val threshold = AtomicLong(0)
    private val closed = AtomicBoolean(false)

    /** Reads the channel from here on, as `observer`. */
    fun attach(observer: DataChannelObserver) {
        this.observer.set(observer)
        channel.registerObserver(this)
    }

    override fun sendBinary(data: ByteArray) {
        send(WebRtcDataChannel.Buffer(ByteBuffer.wrap(data), true))
    }

    override fun sendText(text: String) {
        send(WebRtcDataChannel.Buffer(ByteBuffer.wrap(text.toByteArray(Charsets.UTF_8)), false))
    }

    private fun send(buffer: WebRtcDataChannel.Buffer) {
        // A disposed channel refuses with `IllegalStateException`, which is a refusal too.
        val sent = !closed.get() && try { channel.send(buffer) } catch (disposed: IllegalStateException) { false }
        if (!sent) {
            throw PeerConnectionException.Failed("the data channel refused the message")
        }
    }

    override fun bufferedAmount(): ULong =
        if (closed.get()) 0uL else try { channel.bufferedAmount().toULong() } catch (disposed: IllegalStateException) { 0uL }

    override fun setBufferedAmountLowThreshold(bytes: ULong) {
        threshold.set(bytes.toLong())
    }

    override fun state(): DataChannelState =
        if (closed.get()) DataChannelState.CLOSED else try { state(channel.state()) } catch (disposed: IllegalStateException) { DataChannelState.CLOSED }

    /**
     * Closes once: the observer is taken off first, which waits for a callback under way, so no
     * callback runs on a disposed channel.
     */
    override fun closeDataChannel() {
        if (!closed.compareAndSet(false, true)) {
            return
        }
        channel.unregisterObserver()
        channel.close()
        channel.dispose()
    }

    override fun onBufferedAmountChange(previousAmount: Long) {
        if (closed.get()) {
            return
        }
        val threshold = threshold.get().toULong()
        if (previousAmount.toULong() > threshold && channel.bufferedAmount().toULong() <= threshold) {
            observer.get()?.bufferedAmountLow()
        }
    }

    override fun onStateChange() {
        if (closed.get()) {
            return
        }
        observer.get()?.stateChanged(state(channel.state()))
    }

    override fun onMessage(buffer: WebRtcDataChannel.Buffer) {
        val bytes = ByteArray(buffer.data.remaining())
        buffer.data.get(bytes)
        val observer = observer.get() ?: return
        if (buffer.binary) {
            observer.binary(bytes)
        } else {
            observer.text(String(bytes, Charsets.UTF_8))
        }
    }

    companion object {
        fun state(state: WebRtcDataChannel.State): DataChannelState = when (state) {
            WebRtcDataChannel.State.CONNECTING -> DataChannelState.CONNECTING
            WebRtcDataChannel.State.OPEN -> DataChannelState.OPEN
            WebRtcDataChannel.State.CLOSING -> DataChannelState.CLOSING
            WebRtcDataChannel.State.CLOSED -> DataChannelState.CLOSED
        }
    }
}

/**
 * One peer connection as the engine drives it (`PeerConnection`), each method the W3C operation
 * of its name over `org.webrtc.PeerConnection`; a transfer's connection has its data channel
 * (`CallDataChannel`) and trickles its candidates through the observer. Where the two differ:
 *
 * - `getTransceivers()` makes new Java wrappers of the native transceivers on every call and
 *   disposes those it made before, their senders, receivers, and the receivers' tracks with
 *   them, so no wrapper is kept: a transceiver is told apart by its sender's id, which WebRTC
 *   keeps for its life, and numbered here in the order first seen; the number is the engine's
 *   `TransceiverId`. Each use reads the transceivers afresh under the lock and is done with the
 *   wrapper before it lets go. A received track is given to `CallTracks` as this connection's,
 *   which asks `receivedTrack` for a live wrapper each time one is needed.
 * - `getParameters` gives an `RtpParameters` whose `transactionId` `setParameters` checks, so
 *   the object is kept here between the two and the engine's `SenderParameters.rest` stays
 *   empty.
 * - A statistics report is a map of `RTCStats` with a timestamp in microseconds; it is written
 *   as the W3C array, each member with `id`, `type`, and `timestamp` in milliseconds beside its
 *   values, which already carry their W3C names.
 * - A receiver's volume is its `AudioTrack.setVolume`, 0 to 10.
 * - `close` disposes the connection too, freeing its native side; `connectionState` answers
 *   `Closed` from then on and every other method refuses.
 */
class CallPeerConnection(
    private val connection: WebRtcPeerConnection,
    private val tracks: CallTracks,
) : PeerConnection {
    private val lock = Any()
    /** Sender ids, by `TransceiverId`. */
    private val senders = ArrayList<String>()
    /** The parameters last read of each sender, for `setSenderParameters` to write back. */
    private val parameters = HashMap<TransceiverId, RtpParameters>()
    /** The receivers' tracks put in `tracks`, taken out again when the connection closes. */
    private val received = HashSet<TrackId>()
    private var closed = false

    // --- Conversions --------------------------------------------------------------------------

    companion object {
        /**
         * Unified plan, one ICE and DTLS association for every section, RTCP multiplexed, as the
         * voice server requires of a client.
         */
        fun configuration(configuration: Configuration): WebRtcPeerConnection.RTCConfiguration {
            val servers = configuration.iceServers.map { server ->
                val built = WebRtcPeerConnection.IceServer.builder(server.urls)
                server.username?.let { built.setUsername(it) }
                server.credential?.let { built.setPassword(it) }
                built.createIceServer()
            }
            return WebRtcPeerConnection.RTCConfiguration(servers).apply {
                sdpSemantics = WebRtcPeerConnection.SdpSemantics.UNIFIED_PLAN
                bundlePolicy = WebRtcPeerConnection.BundlePolicy.MAXBUNDLE
                rtcpMuxPolicy = WebRtcPeerConnection.RtcpMuxPolicy.REQUIRE
                iceTransportsType = if (configuration.iceTransportPolicy == "relay") {
                    WebRtcPeerConnection.IceTransportsType.RELAY
                } else {
                    WebRtcPeerConnection.IceTransportsType.ALL
                }
            }
        }

        fun connectionState(state: WebRtcPeerConnection.PeerConnectionState): ConnectionState = when (state) {
            WebRtcPeerConnection.PeerConnectionState.NEW -> ConnectionState.NEW
            WebRtcPeerConnection.PeerConnectionState.CONNECTING -> ConnectionState.CONNECTING
            WebRtcPeerConnection.PeerConnectionState.CONNECTED -> ConnectionState.CONNECTED
            WebRtcPeerConnection.PeerConnectionState.DISCONNECTED -> ConnectionState.DISCONNECTED
            WebRtcPeerConnection.PeerConnectionState.FAILED -> ConnectionState.FAILED
            WebRtcPeerConnection.PeerConnectionState.CLOSED -> ConnectionState.CLOSED
        }

        private fun signalingState(state: WebRtcPeerConnection.SignalingState): SignalingState = when (state) {
            WebRtcPeerConnection.SignalingState.STABLE -> SignalingState.STABLE
            WebRtcPeerConnection.SignalingState.HAVE_LOCAL_OFFER -> SignalingState.HAVE_LOCAL_OFFER
            WebRtcPeerConnection.SignalingState.HAVE_LOCAL_PRANSWER -> SignalingState.HAVE_LOCAL_PRANSWER
            WebRtcPeerConnection.SignalingState.HAVE_REMOTE_OFFER -> SignalingState.HAVE_REMOTE_OFFER
            WebRtcPeerConnection.SignalingState.HAVE_REMOTE_PRANSWER -> SignalingState.HAVE_REMOTE_PRANSWER
            WebRtcPeerConnection.SignalingState.CLOSED -> SignalingState.CLOSED
        }

        private fun sdpType(kind: DescriptionKind): WebRtcSessionDescription.Type = when (kind) {
            DescriptionKind.OFFER -> WebRtcSessionDescription.Type.OFFER
            DescriptionKind.ANSWER -> WebRtcSessionDescription.Type.ANSWER
            DescriptionKind.ROLLBACK -> WebRtcSessionDescription.Type.ROLLBACK
        }

        private fun descriptionKind(type: WebRtcSessionDescription.Type): DescriptionKind = when (type) {
            WebRtcSessionDescription.Type.OFFER -> DescriptionKind.OFFER
            WebRtcSessionDescription.Type.ANSWER, WebRtcSessionDescription.Type.PRANSWER -> DescriptionKind.ANSWER
            WebRtcSessionDescription.Type.ROLLBACK -> DescriptionKind.ROLLBACK
        }

        private fun direction(direction: Direction): RtpTransceiver.RtpTransceiverDirection = when (direction) {
            Direction.SEND_RECV -> RtpTransceiver.RtpTransceiverDirection.SEND_RECV
            Direction.SEND_ONLY -> RtpTransceiver.RtpTransceiverDirection.SEND_ONLY
            Direction.RECV_ONLY -> RtpTransceiver.RtpTransceiverDirection.RECV_ONLY
            Direction.INACTIVE -> RtpTransceiver.RtpTransceiverDirection.INACTIVE
        }

        /**
         * The W3C `RTCPriorityType` names beside WebRTC's `Priority` constants and
         * `bitratePriority` values, which the specification fixes (very-low 0.5, low 1, medium 2,
         * high 4).
         */
        private val priorities = listOf(
            Triple("very-low", org.webrtc.Priority.VERY_LOW, 0.5),
            Triple("low", org.webrtc.Priority.LOW, 1.0),
            Triple("medium", org.webrtc.Priority.MEDIUM, 2.0),
            Triple("high", org.webrtc.Priority.HIGH, 4.0),
        )

        /** Writes `encoding` onto `parameters`, leaving what it does not say as it was. */
        private fun apply(encoding: SendEncoding, parameters: RtpParameters.Encoding) {
            encoding.rid?.let { parameters.rid = it }
            encoding.active?.let { parameters.active = it }
            encoding.maxBitrate?.let { parameters.maxBitrateBps = it.toInt() }
            // WebRTC's Java encodings take a whole number of frames a second.
            encoding.maxFramerate?.let { parameters.maxFramerate = it.roundToInt() }
            encoding.scaleResolutionDownBy?.let { parameters.scaleResolutionDownBy = it }
            priorities.firstOrNull { it.first == encoding.networkPriority }?.let { parameters.networkPriority = it.second }
            priorities.firstOrNull { it.first == encoding.priority }?.let { parameters.bitratePriority = it.third }
        }

        private fun encoding(parameters: RtpParameters.Encoding) = SendEncoding(
            rid = parameters.rid,
            active = parameters.active,
            maxBitrate = parameters.maxBitrateBps?.toUInt(),
            maxFramerate = parameters.maxFramerate?.toDouble(),
            scaleResolutionDownBy = parameters.scaleResolutionDownBy,
            networkPriority = priorities.firstOrNull { it.second == parameters.networkPriority }?.first,
            priority = priorities.firstOrNull { it.third == parameters.bitratePriority }?.first,
        )

        /**
         * The report as the W3C `RTCStatsReport` reads when iterated: an array of its stats
         * objects, each the values WebRTC gathered under their W3C names, with `id`, `type`, and
         * `timestamp` (milliseconds, as `DOMHighResTimeStamp` is) beside them.
         */
        fun json(report: RTCStatsReport): String {
            val entries = JSONArray()
            for (statistics in report.statsMap.values) {
                val entry = JSONObject()
                for ((name, value) in statistics.members) {
                    entry.put(name, json(value))
                }
                entry.put("id", statistics.id)
                entry.put("type", statistics.type)
                entry.put("timestamp", statistics.timestampUs / 1000)
                entries.put(entry)
            }
            return entries.toString()
        }

        /**
         * One statistic's value as `org.json` writes it: arrays and maps member by member, and a
         * `BigInteger` (WebRTC's unsigned 64-bit counters) as the number it is, which
         * `JSONObject.wrap` would write as text.
         */
        private fun json(value: Any?): Any? = when (value) {
            null -> JSONObject.NULL
            is Array<*> -> JSONArray().also { array -> value.forEach { array.put(json(it)) } }
            is Map<*, *> -> JSONObject().also { map -> value.forEach { (k, v) -> map.put(k.toString(), json(v)) } }
            else -> value
        }
    }

    // --- Transceivers -------------------------------------------------------------------------

    /** The number of `transceiver`, given the first time it is seen. Called holding the lock. */
    private fun number(transceiver: RtpTransceiver): TransceiverId {
        val id = transceiver.sender.id()
        val at = senders.indexOf(id)
        if (at >= 0) {
            return at.toUInt()
        }
        senders.add(id)
        return (senders.size - 1).toUInt()
    }

    /**
     * Runs `body` holding the lock with the connection's transceivers read afresh, those a remote
     * description made included, each numbered; the wrappers are good only within `body`.
     */
    private fun <T> withTransceivers(body: (List<RtpTransceiver>) -> T): T = synchronized(lock) {
        if (closed) {
            throw PeerConnectionException.Failed("the connection is closed")
        }
        val transceivers = connection.transceivers
        for (transceiver in transceivers) {
            number(transceiver)
        }
        body(transceivers)
    }

    private fun <T> withTransceiver(id: TransceiverId, body: (RtpTransceiver) -> T): T = withTransceivers { all ->
        val sender = senders.getOrNull(id.toInt()) ?: throw PeerConnectionException.Failed("no transceiver $id")
        val transceiver = all.firstOrNull { it.sender.id() == sender }
            ?: throw PeerConnectionException.Failed("transceiver $id is gone")
        body(transceiver)
    }

    /** A live wrapper of the received track `id`, good until the transceivers are next read. */
    fun receivedTrack(id: TrackId): MediaStreamTrack? = synchronized(lock) {
        if (closed) {
            return null
        }
        connection.transceivers.map { it.receiver.track() }.firstOrNull { it?.id() == id }
    }

    // --- PeerConnection -----------------------------------------------------------------------

    override suspend fun createOffer(iceRestart: Boolean): String {
        val constraints = MediaConstraints()
        if (iceRestart) {
            constraints.mandatory.add(MediaConstraints.KeyValuePair("IceRestart", "true"))
        }
        return created { observer -> connection.createOffer(observer, constraints) }.description
    }

    override suspend fun createAnswer(): String =
        created { observer -> connection.createAnswer(observer, MediaConstraints()) }.description

    override suspend fun setLocalDescription(description: SessionDescription) {
        val sdp = WebRtcSessionDescription(sdpType(description.kind), description.sdp)
        finished { observer -> connection.setLocalDescription(observer, sdp) }
    }

    override suspend fun setRemoteDescription(description: SessionDescription) {
        val sdp = WebRtcSessionDescription(sdpType(description.kind), description.sdp)
        finished { observer -> connection.setRemoteDescription(observer, sdp) }
        withTransceivers {}
    }

    override suspend fun addIceCandidate(candidate: IceCandidate) {
        val added = WebRtcIceCandidate(candidate.sdpMid, candidate.sdpMLineIndex?.toInt() ?: 0, candidate.candidate)
        val taken = synchronized(lock) { !closed && connection.addIceCandidate(added) }
        if (!taken) {
            throw PeerConnectionException.Failed("the candidate was refused")
        }
    }

    /** Ordered and reliable, which `DataChannel.Init` is unless told otherwise. */
    override suspend fun createDataChannel(label: String, observer: DataChannelObserver): CreatedDataChannel {
        val made = synchronized(lock) {
            if (closed) {
                throw PeerConnectionException.Failed("the connection is closed")
            }
            connection.createDataChannel(label, WebRtcDataChannel.Init().apply { ordered = true })
        } ?: throw PeerConnectionException.Failed("the data channel could not be opened")
        val channel = CallDataChannel(made)
        channel.attach(observer)
        return CreatedDataChannel(channel)
    }

    override fun localDescription(): SessionDescription? {
        val local = connection.localDescription ?: return null
        return SessionDescription(descriptionKind(local.type), local.description)
    }

    override fun signalingState(): SignalingState = signalingState(connection.signalingState())

    override suspend fun addTransceiver(
        kind: MediaKind,
        track: TrackId?,
        direction: Direction,
        streamId: String?,
        sendEncodings: List<SendEncoding>,
    ): TransceiverId {
        val encodings = sendEncodings.map { encoding ->
            RtpParameters.Encoding(encoding.rid, encoding.active ?: true, encoding.scaleResolutionDownBy)
                .also { apply(encoding, it) }
        }
        val initial = RtpTransceiver.RtpTransceiverInit(direction(direction), listOfNotNull(streamId), encodings)
        val mediaType = when (kind) {
            MediaKind.AUDIO -> MediaStreamTrack.MediaType.MEDIA_TYPE_AUDIO
            MediaKind.VIDEO -> MediaStreamTrack.MediaType.MEDIA_TYPE_VIDEO
        }
        return synchronized(lock) {
            if (closed) {
                throw PeerConnectionException.Failed("the connection is closed")
            }
            val made = try {
                if (track != null) {
                    val media = tracks.track(track) ?: throw PeerConnectionException.Failed("no track $track")
                    connection.addTransceiver(media, initial)
                } else {
                    connection.addTransceiver(mediaType, initial)
                }
            } catch (e: IllegalStateException) {
                throw PeerConnectionException.Failed(e.toString())
            }
            number(made)
        }
    }

    override fun transceiverMid(transceiver: TransceiverId): String? =
        try {
            withTransceiver(transceiver) { it.mid?.takeIf { mid -> mid.isNotEmpty() } }
        } catch (refused: PeerConnectionException) {
            null
        }

    override fun transceiverByMid(mid: String): TransceiverId? = withTransceivers { all ->
        all.firstOrNull { it.mid == mid }?.let { number(it) }
    }

    override suspend fun setDirection(transceiver: TransceiverId, direction: Direction) {
        withTransceiver(transceiver) {
            if (!it.setDirection(direction(direction))) {
                throw PeerConnectionException.Failed("the direction was refused")
            }
        }
    }

    override suspend fun stopTransceiver(transceiver: TransceiverId) {
        withTransceiver(transceiver) { it.stopInternal() }
    }

    override suspend fun removeTrack(transceiver: TransceiverId) {
        withTransceiver(transceiver) {
            if (!connection.removeTrack(it.sender)) {
                throw PeerConnectionException.Failed("the sender's track could not be removed")
            }
        }
    }

    override suspend fun replaceTrack(transceiver: TransceiverId, track: TrackId?) {
        withTransceiver(transceiver) {
            val media = if (track != null) {
                tracks.track(track) ?: throw PeerConnectionException.Failed("no track $track")
            } else {
                null
            }
            // The track stays the app's: the sender is not to dispose it when it is replaced.
            if (!it.sender.setTrack(media, false)) {
                throw PeerConnectionException.Failed("the sender's track could not be replaced")
            }
        }
    }

    override suspend fun senderParameters(transceiver: TransceiverId): SenderParameters =
        withTransceiver(transceiver) {
            val read = it.sender.parameters
            parameters[transceiver] = read
            SenderParameters(read.encodings.map(::encoding), "")
        }

    override suspend fun setSenderParameters(transceiver: TransceiverId, parameters: SenderParameters) {
        withTransceiver(transceiver) {
            val written = this.parameters.remove(transceiver) ?: it.sender.parameters
            for ((encoding, target) in parameters.encodings.zip(written.encodings)) {
                apply(encoding, target)
            }
            if (!it.sender.setParameters(written)) {
                throw PeerConnectionException.Failed("the sender's parameters were refused")
            }
        }
    }

    override fun receiverTrack(transceiver: TransceiverId): TrackId? =
        try {
            withTransceiver(transceiver) {
                val id = it.receiver.track()?.id() ?: return@withTransceiver null
                tracks.addReceived(id, this)
                received.add(id)
                id
            }
        } catch (refused: PeerConnectionException) {
            null
        }

    override suspend fun setReceiverVolume(transceiver: TransceiverId, volume: Double) {
        withTransceiver(transceiver) {
            val track = it.receiver.track() as? AudioTrack
                ?: throw PeerConnectionException.Failed("transceiver $transceiver receives no audio")
            track.setVolume(volume)
        }
    }

    override suspend fun setConfiguration(configuration: Configuration) {
        if (!connection.setConfiguration(configuration(configuration))) {
            throw PeerConnectionException.Failed("the configuration was refused")
        }
    }

    override suspend fun stats(of: StatsOf): String {
        val report = suspendCoroutine { continuation ->
            val deliver = RTCStatsCollectorCallback { report -> continuation.resume(report) }
            when (of) {
                is StatsOf.Connection -> connection.getStats(deliver)
                is StatsOf.Sender -> withTransceiver(of.v1) { connection.getStats(it.sender, deliver) }
                is StatsOf.Receiver -> withTransceiver(of.v1) { connection.getStats(it.receiver, deliver) }
            }
        }
        return json(report)
    }

    override fun connectionState(): ConnectionState = synchronized(lock) {
        if (closed) ConnectionState.CLOSED else connectionState(connection.connectionState())
    }

    override fun closePeerConnection() {
        val released = synchronized(lock) {
            if (closed) {
                return
            }
            closed = true
            connection.close()
            connection.dispose()
            val released = received.toList()
            received.clear()
            released
        }
        for (id in released) {
            tracks.remove(id)
        }
    }

    // --- WebRTC's observers as suspending calls -----------------------------------------------

    private suspend fun created(start: (SdpObserver) -> Unit): WebRtcSessionDescription =
        suspendCoroutine { continuation ->
            start(object : SdpObserver {
                override fun onCreateSuccess(sdp: WebRtcSessionDescription) = continuation.resume(sdp)
                override fun onCreateFailure(error: String) =
                    continuation.resumeWithException(PeerConnectionException.Failed(error))
                override fun onSetSuccess() {}
                override fun onSetFailure(error: String) {}
            })
        }

    private suspend fun finished(start: (SdpObserver) -> Unit) {
        suspendCoroutine { continuation ->
            start(object : SdpObserver {
                override fun onSetSuccess() = continuation.resume(Unit)
                override fun onSetFailure(error: String) =
                    continuation.resumeWithException(PeerConnectionException.Failed(error))
                override fun onCreateSuccess(sdp: WebRtcSessionDescription) {}
                override fun onCreateFailure(error: String) {}
            })
        }
    }
}
