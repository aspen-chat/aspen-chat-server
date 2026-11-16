package com.aspen_chat.client.call

import com.aspen_chat.client.voice.TrackId
import org.webrtc.MediaStreamTrack
import org.webrtc.TrackRefs
import org.webrtc.VideoTrack

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
 * answers is used at once and never kept. What draws a received video track holds a wrapper of
 * its own instead (`retainedVideoTrack`, through `TrackRefs`), since a sink added through one of
 * WebRTC's wrappers is taken off the track when that wrapper is disposed.
 */
class CallTracks {
    private val lock = Any()
    private val own = HashMap<TrackId, MediaStreamTrack>()
    private val received = HashMap<TrackId, CallPeerConnection>()
    /** Wrappers of received video tracks handed out and not yet given back, by id. */
    private val retained = HashMap<TrackId, ArrayList<VideoTrack>>()

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

    /**
     * A wrapper of the video track `id` that stays good until `releaseVideoTrack`, for a sink to
     * be added through: the app's own track as it is, or a received track retained anew
     * (`TrackRefs.retain`). `null` for a track that is not here, or not video.
     */
    fun retainedVideoTrack(id: TrackId): VideoTrack? {
        val kept = synchronized(lock) { own[id] }
        if (kept != null) {
            return kept as? VideoTrack
        }
        val live = track(id) as? VideoTrack ?: return null
        val wrapper = TrackRefs.retain(live)
        synchronized(lock) { retained.getOrPut(id) { ArrayList() }.add(wrapper) }
        return wrapper
    }

    /** Gives back a wrapper from `retainedVideoTrack`, disposing it where it was one of a received track. */
    fun releaseVideoTrack(id: TrackId, wrapper: VideoTrack) {
        val disposed = synchronized(lock) { retained[id]?.remove(wrapper) == true }
        if (disposed) {
            wrapper.dispose()
        }
    }

    /** Forgets the track; a retained wrapper still out holds a reference of its own until given back. */
    fun remove(id: TrackId) {
        synchronized(lock) {
            own.remove(id)
            received.remove(id)
        }
    }
}
