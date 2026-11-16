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
 * processing and the receiver and loudspeaker take a call's volume. `CallTelecom` sets the mode
 * through `configureForCall` as it adds the call to Telecom and gives it back through `release`
 * when the call ends; Telecom sets the same mode for a call it holds, so the two agree. The audio
 * focus is Telecom's alone: it takes the focus for every call it holds, as a voice communication,
 * and a request of the app's own beside it is displaced by Telecom's at once (a transient loss the
 * app would mistake for an interruption), so an interruption reaches the call as Telecom's hold.
 * The route within the mode (receiver, loudspeaker, a headset) is Telecom's to change too
 * (`CallRoutes`), never `AudioManager`'s beside it. The camera is a later piece.
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
