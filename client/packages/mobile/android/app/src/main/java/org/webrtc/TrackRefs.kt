package org.webrtc

/**
 * A wrapper of a received video track that outlives WebRTC's own. `PeerConnection.getTransceivers()`
 * makes new Java wrappers of the transceivers, their receivers, and the receivers' tracks on every
 * call and disposes those it made before, and a `VideoTrack` wrapper takes the sinks it added with
 * it when disposed, so a renderer added through one of those wrappers stops drawing at the next
 * read of the transceivers. This holds a reference of its own on the native track and wraps it
 * anew, so sinks added through the wrapper stay until the app disposes it. In WebRTC's package
 * because the native pointer and the reference count are package-private there; the app's call
 * layer (`CallTracks`) is its one caller.
 */
object TrackRefs {
    /** A wrapper of `track`'s native track with a reference of its own; dispose it when done. */
    @JvmStatic
    fun retain(track: VideoTrack): VideoTrack {
        val nativeTrack = track.nativeMediaStreamTrack
        JniCommon.nativeAddRef(nativeTrack)
        return VideoTrack(nativeTrack)
    }
}
