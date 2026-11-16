package com.aspen_chat.client.call

import android.content.Context
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.aspen_chat.client.R
import com.aspen_chat.client.voice.SoundCue

/**
 * The call's sounds, as the engine cues them (`CallObserver.playSound`), played on the call's own
 * audio path: each cue is a recording of the Aspen Sound Effects Pack (the same files the web
 * client plays, laid out as raw resources by the Gradle task `copyCallSounds`), played by a
 * `MediaPlayer` with the attributes of a voice call (`USAGE_VOICE_COMMUNICATION`), so the sound
 * goes where Telecom routes the call (earpiece, loudspeaker, headset), takes the call's volume,
 * and plays with the app in the background, mixed with the voices. No audio focus is asked for:
 * Telecom holds it for the call (`CallTelecom`), and a request of the app's own beside it would
 * displace Telecom's.
 *
 * At most `MAX_PLAYING_PER_CUE` of one cue play at once, as the web client plays at most two of
 * one sound; a cue past that is not heard rather than queued. The engine already spaces repeats
 * of a cue. A cue plays as it arrives: the audio mode is set for the call before the engine's
 * first snapshot of it reaches anyone, and the mode is given back only once Telecom has let the
 * call go, after the leaving's cue.
 *
 * `play` may be called from any thread (the engine's); everything else runs on the main thread.
 */
class CallSounds(context: Context) {
    fun interface Listener {
        /** A cue the layer has started playing: the player is playing as it is given. */
        fun played(cue: SoundCue, player: MediaPlayer)
    }

    /** Stops the listener it was given for. */
    inner class Subscription internal constructor(private val listener: Listener) {
        fun cancel() {
            synchronized(lock) { listeners.remove(listener) }
        }
    }

    /** One player for a cue, and whether its recording is ready to start. */
    private class Slot(val player: MediaPlayer) {
        var prepared = false
        /** Whether a start waits for the preparation. */
        var startWanted = false
    }

    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val lock = Any()
    private val listeners = ArrayList<Listener>()
    /** The players made for each cue, at most `MAX_PLAYING_PER_CUE`; one that is not playing is used again. */
    private val players = HashMap<SoundCue, MutableList<Slot>>()
    private var released = false

    /** How many sounds play now. Main thread. */
    val playing: Int
        get() = players.values.sumOf { slots -> slots.count { it.player.isPlaying } }

    /** Plays `cue` on the call's audio path. Any thread. */
    fun play(cue: SoundCue) {
        main.post { start(cue) }
    }

    /**
     * Calls `listener` on the main thread with each cue as it starts playing, until the
     * subscription is cancelled.
     */
    fun listen(listener: Listener): Subscription {
        synchronized(lock) { listeners.add(listener) }
        return Subscription(listener)
    }

    /** Releases every player; nothing plays after. Main thread. */
    fun release() {
        released = true
        for (slot in players.values.flatten()) {
            slot.player.release()
        }
        players.clear()
    }

    private fun start(cue: SoundCue) {
        if (released) {
            return
        }
        val slot = slot(cue) ?: return
        if (slot.prepared) {
            // A player that has played to its end starts over from the beginning.
            slot.player.start()
            started(cue, slot.player)
        } else {
            slot.startWanted = true
        }
    }

    private fun started(cue: SoundCue, player: MediaPlayer) {
        val told = synchronized(lock) { listeners.toList() }
        for (listener in told) {
            listener.played(cue, player)
        }
    }

    /**
     * A player for `cue` that is not playing, made when every one made so far plays and there is
     * room for another; `null` past `MAX_PLAYING_PER_CUE`, or when the recording cannot be read.
     */
    private fun slot(cue: SoundCue): Slot? {
        val made = players.getOrPut(cue) { ArrayList() }
        made.firstOrNull { it.prepared && !it.player.isPlaying }?.let { return it }
        if (made.size >= MAX_PLAYING_PER_CUE) {
            return null
        }
        val player = MediaPlayer()
        val slot = Slot(player)
        try {
            player.setAudioAttributes(ATTRIBUTES)
            context.resources.openRawResourceFd(resourceOf(cue)).use { file ->
                player.setDataSource(file.fileDescriptor, file.startOffset, file.length)
            }
            player.setOnPreparedListener {
                slot.prepared = true
                if (slot.startWanted) {
                    slot.startWanted = false
                    player.start()
                    started(cue, player)
                }
            }
            player.setOnErrorListener { _, what, extra ->
                Log.w(TAG, "the call's sound $cue failed ($what, $extra)")
                made.remove(slot)
                player.release()
                true
            }
            player.prepareAsync()
        } catch (failed: Exception) {
            Log.w(TAG, "the call's sound $cue could not be read", failed)
            player.release()
            return null
        }
        made.add(slot)
        return slot
    }

    companion object {
        private const val TAG = "AspenCall"

        /** How many of one cue play at once: the web client's `MAX_PLAYING_PER_SOUND`. */
        const val MAX_PLAYING_PER_CUE = 2

        /** A voice call's own sound: routed and set in volume as the call is. */
        val ATTRIBUTES: AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()

        /** The raw resource of each cue's recording: the web client's `SOUND_FILES`, named as Android names resources. */
        fun resourceOf(cue: SoundCue): Int = when (cue) {
            SoundCue.CALL_JOINED -> R.raw.call_joined
            SoundCue.CALL_LEFT -> R.raw.call_left
            SoundCue.DISCONNECTED -> R.raw.disconnected
            SoundCue.RECONNECTED -> R.raw.reconnected
            SoundCue.STREAM_STARTED -> R.raw.stream_started
            SoundCue.TRANSFER_ACCEPTED -> R.raw.transfer_accepted
            SoundCue.TRANSFER_COMPLETE -> R.raw.transfer_complete
        }
    }
}
