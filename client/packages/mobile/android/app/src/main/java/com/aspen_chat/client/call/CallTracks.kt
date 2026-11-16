package com.aspen_chat.client.call

import com.aspen_chat.client.voice.TrackId
import org.webrtc.MediaStreamTrack

/**
 * The media tracks on this device, by the ids the call engine passes around and never looks
 * inside: the microphone the app opens, and the tracks each receiver gives, which whatever draws
 * or plays them looks up here. A track's id is WebRTC's own (`MediaStreamTrack.id()`), unique
 * within the process.
 *
 * A received track is not kept as an object: WebRTC's Java wrappers of a connection's
 * transceivers, their receivers, and the receivers' tracks are made anew by every
 * `PeerConnection.getTransceivers()` and the ones before disposed, so a wrapper held across one
 * is dead. Each is kept as the connection that receives it, which finds a live wrapper on demand
 * (`CallPeerConnection.receivedTrack`); so `track` is called each time one is needed, and what it
 * answers is used at once and never kept.
 */
class CallTracks {
    private val lock = Any()
    private val own = HashMap<TrackId, MediaStreamTrack>()
    private val received = HashMap<TrackId, CallPeerConnection>()

    /** Keeps `track`, the app's own, and answers its id. */
    fun add(track: MediaStreamTrack): TrackId {
        val id = track.id()
        synchronized(lock) { own[id] = track }
        return id
    }

    /** Notes that `connection` receives the track `id`. */
    fun addReceived(id: TrackId, connection: CallPeerConnection) {
        synchronized(lock) { received[id] = connection }
    }

    fun track(id: TrackId): MediaStreamTrack? {
        val (kept, connection) = synchronized(lock) { own[id] to received[id] }
        return kept ?: connection?.receivedTrack(id)
    }

    fun remove(id: TrackId) {
        synchronized(lock) {
            own.remove(id)
            received.remove(id)
        }
    }
}
