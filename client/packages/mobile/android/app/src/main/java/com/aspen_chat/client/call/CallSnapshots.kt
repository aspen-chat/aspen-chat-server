package com.aspen_chat.client.call

import android.os.Handler
import android.os.Looper
import com.aspen_chat.client.voice.CallObserver
import com.aspen_chat.client.voice.CallState
import com.aspen_chat.client.voice.SoundCue

/**
 * Every snapshot the call engine publishes (its `CallObserver`), kept for whoever asks what the
 * call is now and handed to each listener on the main thread, where the interface reads it: as
 * the record, and as the JSON the engine writes beside it (`listenJson`, `latestJson`), which
 * `AspenCallPlugin` hands the web view untouched; and each of the call's sounds the engine cues
 * (`playSound`), handed to `CallSounds` to play. The engine calls both on its own thread and
 * waits for them to return, so nothing here blocks or reaches back into the engine.
 */
class CallSnapshots(private val sounds: CallSounds) : CallObserver {
    fun interface Listener {
        fun changed(state: CallState)
    }

    fun interface JsonListener {
        fun changed(json: String)
    }

    /** Stops the listener it was given for. */
    inner class Subscription internal constructor(private val listener: Listener?, private val jsonListener: JsonListener?) {
        fun cancel() {
            synchronized(lock) {
                listeners.remove(listener)
                jsonListeners.remove(jsonListener)
            }
        }
    }

    private val lock = Any()
    private val main = Handler(Looper.getMainLooper())
    private var current: CallState? = null
    private var currentJson: String? = null
    private val listeners = ArrayList<Listener>()
    private val jsonListeners = ArrayList<JsonListener>()

    /** The latest snapshot, or `null` before the engine published one. */
    val latest: CallState?
        get() = synchronized(lock) { current }

    /** The latest snapshot's JSON, or `null` before the engine published one. */
    val latestJson: String?
        get() = synchronized(lock) { currentJson }

    override fun playSound(cue: SoundCue) {
        sounds.play(cue)
    }

    override fun stateChanged(state: CallState, json: String) {
        val (told, toldJson) = synchronized(lock) {
            current = state
            currentJson = json
            listeners.toList() to jsonListeners.toList()
        }
        main.post {
            for (listener in told) {
                listener.changed(state)
            }
            for (listener in toldJson) {
                listener.changed(json)
            }
        }
    }

    /** As `listen`, with each snapshot's JSON. */
    fun listenJson(listener: JsonListener): Subscription {
        val latest = synchronized(lock) {
            jsonListeners.add(listener)
            currentJson
        }
        if (latest != null) {
            main.post { listener.changed(latest) }
        }
        return Subscription(null, listener)
    }

    /**
     * Calls `listener` on the main thread with the latest snapshot, when there is one, and with
     * every one after it, until the subscription is cancelled.
     */
    fun listen(listener: Listener): Subscription {
        val latest = synchronized(lock) {
            listeners.add(listener)
            current
        }
        if (latest != null) {
            main.post { listener.changed(latest) }
        }
        return Subscription(listener, null)
    }
}
