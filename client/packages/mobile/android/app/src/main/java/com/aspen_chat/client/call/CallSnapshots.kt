package com.aspen_chat.client.call

import android.os.Handler
import android.os.Looper
import com.aspen_chat.client.voice.CallObserver
import com.aspen_chat.client.voice.CallState

/**
 * Every snapshot the call engine publishes (its `CallObserver`), kept for whoever asks what the
 * call is now and handed to each listener on the main thread, where the interface reads it. The
 * engine calls `stateChanged` on its own thread and waits for it to return, so nothing here
 * blocks or reaches back into the engine.
 */
class CallSnapshots : CallObserver {
    fun interface Listener {
        fun changed(state: CallState)
    }

    /** Stops the listener it was given for. */
    inner class Subscription internal constructor(private val listener: Listener) {
        fun cancel() {
            synchronized(lock) { listeners.remove(listener) }
        }
    }

    private val lock = Any()
    private val main = Handler(Looper.getMainLooper())
    private var current: CallState? = null
    private val listeners = ArrayList<Listener>()

    /** The latest snapshot, or `null` before the engine published one. */
    val latest: CallState?
        get() = synchronized(lock) { current }

    override fun stateChanged(state: CallState) {
        val told = synchronized(lock) {
            current = state
            listeners.toList()
        }
        main.post {
            for (listener in told) {
                listener.changed(state)
            }
        }
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
        return Subscription(listener)
    }
}
