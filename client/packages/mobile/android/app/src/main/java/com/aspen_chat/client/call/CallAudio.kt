package com.aspen_chat.client.call

import android.content.Context
import android.media.AudioManager
import com.aspen_chat.client.voice.TrackId
import java.util.UUID
import org.webrtc.AudioSource
import org.webrtc.AudioTrack
import org.webrtc.MediaConstraints
import org.webrtc.PeerConnectionFactory

/**
 * The call's sound on this device: the audio mode set for a call, and the microphone, opened
 * through WebRTC's audio device module as a track the engine sends.
 *
 * A call plays and records in `MODE_IN_COMMUNICATION`, where the system applies its voice
 * processing and the receiver and loudspeaker take a call's volume. The app sets the mode itself
 * here until Telecom owns the call: a self-managed `ConnectionService` sets it for the call it
 * holds, and `configureForCall` and `release` are the seams its connection's `onStateChanged`
 * and `onDisconnect` will drive. The route (receiver, loudspeaker, a headset) is chosen within
 * that mode by a later piece, as `AspenAudioRoutePlugin` chooses it for the web view's calls; so
 * is the camera.
 */
class CallAudio(context: Context, private val factory: PeerConnectionFactory, private val tracks: CallTracks) {
    private val audioManager = context.applicationContext.getSystemService(AudioManager::class.java)
    private var modeBefore: Int? = null
    private var source: AudioSource? = null
    private var microphone: AudioTrack? = null

    /** Puts the device in the communication mode for the call, remembering the mode it was in. */
    fun configureForCall() {
        if (modeBefore == null) {
            modeBefore = audioManager.mode
        }
        audioManager.mode = AudioManager.MODE_IN_COMMUNICATION
    }

    /** Gives the device back the mode it was in before `configureForCall`. */
    fun release() {
        val before = modeBefore ?: return
        modeBefore = null
        if (audioManager.mode == AudioManager.MODE_IN_COMMUNICATION) {
            audioManager.mode = before
        }
    }

    /**
     * The microphone as a track for the engine (`Engine.setMicrophone`), opened on the first call
     * and kept until `closeMicrophone`. Recording starts when it is sent on a connected peer
     * connection, which needs `RECORD_AUDIO` granted.
     */
    fun openMicrophone(): TrackId {
        microphone?.let { return it.id() }
        val source = factory.createAudioSource(MediaConstraints())
        val track = factory.createAudioTrack("microphone-${UUID.randomUUID()}", source)
        this.source = source
        microphone = track
        return tracks.add(track)
    }

    fun closeMicrophone() {
        val track = microphone ?: return
        tracks.remove(track.id())
        microphone = null
        track.dispose()
        source?.dispose()
        source = null
    }
}
